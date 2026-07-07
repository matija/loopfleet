//! Claude Code adapter, headless. Spawns
//! `claude -p <prompt> --output-format stream-json --verbose
//! --dangerously-skip-permissions` in the run's worktree and maps its
//! newline-delimited stream-json into [`NormalizedEvent`]s.
//!
//! The transport is one JSON object per line. The shapes this adapter maps
//! (captured from Claude Code 2.1.x):
//! - `{"type":"system","subtype":"init",…}` — session start → `TurnStarted`.
//! - `{"type":"assistant","message":{"content":[…blocks…]}}` — each content
//!   block maps: `text` → `AssistantText`, `thinking` → `Reasoning`,
//!   `tool_use` → `CommandRun` when the tool is `Bash`, else `ToolCall`.
//! - `{"type":"user","message":{"content":[{"type":"tool_result",…}]}}` —
//!   `tool_result` → `ToolResult`, correlated to its `tool_use` by id. Results
//!   of `Bash` calls are dropped: they were already normalized to `CommandRun`,
//!   which the enum carries as a single event with no result pairing.
//! - `{"type":"result","subtype":"success"|…,"usage":{…}}` — terminal line →
//!   `TurnCompleted` (success) or `Failed` (error), then `Ended`.
//!
//! Other line types Claude emits (hook lifecycle, rate-limit notices, partial
//! deltas) carry nothing the enum represents and are ignored.

use std::collections::HashSet;
use std::path::Path;

use async_trait::async_trait;
use loopfleet_core::{NormalizedEvent, Usage};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::{AdapterError, AgentAdapter, RunHandle, RunSpec, SessionHandle, SessionSeed};

/// Longest excerpt (in bytes, on a char boundary) kept for tool inputs and
/// results — the event log stores excerpts, not full payloads.
const EXCERPT_LIMIT: usize = 2000;

/// The Claude Code headless adapter. Stateless; each `start_run` spawns its own
/// process and mapper.
pub struct ClaudeAdapter;

#[async_trait]
impl AgentAdapter for ClaudeAdapter {
    async fn start_run(&self, spec: &RunSpec) -> Result<RunHandle, AdapterError> {
        let mut child = crate::base_command(&spec.wrapper, "claude")
            .arg("-p")
            .arg(&spec.prompt)
            .args(["--output-format", "stream-json", "--verbose"])
            .arg("--dangerously-skip-permissions")
            .current_dir(&spec.cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(AdapterError::Spawn)?;

        let stdout = child
            .stdout
            .take()
            .expect("stdout was piped so it is present");
        let stderr = child
            .stderr
            .take()
            .expect("stderr was piped so it is present");

        // Bounded channel: the backpressure contract (a slow consumer stalls the
        // reader) matching the stub and the M1 event-log writer.
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(drive(child, stdout, stderr, tx));
        Ok(RunHandle { events: rx })
    }

    async fn open_session(
        &self,
        _cwd: &Path,
        _seed: SessionSeed,
    ) -> Result<SessionHandle, AdapterError> {
        Err(AdapterError::SessionsUnsupported)
    }
}

/// Reads the process's stdout line by line, maps each into normalized events,
/// and forwards them. When the stream ends without a terminal `result` line,
/// synthesizes a `Failed`/`Ended` pair from stderr so consumers always see a
/// termination.
async fn drive(
    mut child: tokio::process::Child,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    tx: mpsc::Sender<NormalizedEvent>,
) {
    let mut mapper = ClaudeMapper::new();
    let mut lines = BufReader::new(stdout).lines();
    let mut saw_terminal = false;

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let events = match mapper.map_line(&line) {
                    Ok(events) => events,
                    // A single unparseable line shouldn't kill the run; surface
                    // it and keep reading.
                    Err(e) => vec![NormalizedEvent::Failed {
                        reason: e.to_string(),
                    }],
                };
                for ev in events {
                    if matches!(ev, NormalizedEvent::Ended) {
                        saw_terminal = true;
                    }
                    if tx.send(ev).await.is_err() {
                        // Consumer dropped: SIGTERM the agent's group and stop.
                        crate::stop_agent(&mut child);
                        return;
                    }
                }
            }
            Ok(None) => break,
            Err(e) => {
                let _ = tx
                    .send(NormalizedEvent::Failed {
                        reason: format!("reading agent stdout: {e}"),
                    })
                    .await;
                break;
            }
        }
    }

    if !saw_terminal {
        let reason = read_stderr(stderr)
            .await
            .filter(|s| !s.is_empty())
            .map(|s| format!("agent exited without a result: {s}"))
            .unwrap_or_else(|| "agent exited without a result".to_string());
        let _ = tx.send(NormalizedEvent::Failed { reason }).await;
        let _ = tx.send(NormalizedEvent::Ended).await;
    }

    let _ = child.wait().await;
}

/// Drains stderr into a string for a failure reason. Best-effort.
async fn read_stderr(stderr: tokio::process::ChildStderr) -> Option<String> {
    let mut lines = BufReader::new(stderr).lines();
    let mut collected = Vec::new();
    while let Ok(Some(line)) = lines.next_line().await {
        collected.push(line);
    }
    if collected.is_empty() {
        None
    } else {
        Some(collected.join("\n"))
    }
}

/// Stateful mapper from Claude stream-json lines to [`NormalizedEvent`]s. Holds
/// the set of `Bash` tool-call ids so their `tool_result` lines can be dropped
/// (already normalized to `CommandRun`).
struct ClaudeMapper {
    bash_calls: HashSet<String>,
}

impl ClaudeMapper {
    fn new() -> Self {
        Self {
            bash_calls: HashSet::new(),
        }
    }

    /// Maps one line into zero or more normalized events. Blank lines and line
    /// types the enum does not represent yield an empty vec.
    fn map_line(&mut self, line: &str) -> Result<Vec<NormalizedEvent>, AdapterError> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(vec![]);
        }
        let v: Value = serde_json::from_str(line)
            .map_err(|e| AdapterError::Protocol(format!("invalid stream-json line: {e}")))?;

        match v.get("type").and_then(Value::as_str) {
            Some("system") if v.get("subtype").and_then(Value::as_str) == Some("init") => {
                Ok(vec![NormalizedEvent::TurnStarted])
            }
            Some("assistant") => Ok(self.map_content(&v, Self::map_assistant_block())),
            Some("user") => Ok(self.map_content(&v, Self::map_user_block())),
            Some("result") => Ok(self.map_result(&v)),
            _ => Ok(vec![]),
        }
    }

    /// Applies a per-block mapper to `message.content[]`, in stream order.
    fn map_content(
        &mut self,
        v: &Value,
        mapper: impl Fn(&mut Self, &Value) -> Option<NormalizedEvent>,
    ) -> Vec<NormalizedEvent> {
        v.get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| mapper(self, b))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    /// Block mapper for `type:"assistant"` messages.
    fn map_assistant_block() -> impl Fn(&mut Self, &Value) -> Option<NormalizedEvent> {
        |this: &mut Self, b: &Value| match b.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = b.get("text").and_then(Value::as_str).unwrap_or_default();
                (!text.is_empty()).then(|| NormalizedEvent::AssistantText { text: text.into() })
            }
            Some("thinking") => {
                let text = b.get("thinking").and_then(Value::as_str).unwrap_or_default();
                (!text.is_empty()).then(|| NormalizedEvent::Reasoning { text: text.into() })
            }
            Some("tool_use") => {
                let name = b.get("name").and_then(Value::as_str).unwrap_or_default();
                let id = b.get("id").and_then(Value::as_str).unwrap_or_default();
                let input = b.get("input").cloned().unwrap_or(Value::Null);
                if name == "Bash" {
                    // Shell-exec is normalized to CommandRun (no result pairing);
                    // remember the id so we drop its tool_result.
                    this.bash_calls.insert(id.to_string());
                    let cmd = input
                        .get("command")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    // Claude's bash tool_result carries no numeric exit code, so
                    // exit is unknown at invocation and stays absent.
                    Some(NormalizedEvent::CommandRun {
                        cmd: cmd.into(),
                        exit: None,
                    })
                } else {
                    Some(NormalizedEvent::ToolCall {
                        call_id: id.into(),
                        name: name.into(),
                        input_excerpt: excerpt(&compact(&input)),
                    })
                }
            }
            _ => None,
        }
    }

    /// Block mapper for `type:"user"` messages (tool results).
    fn map_user_block() -> impl Fn(&mut Self, &Value) -> Option<NormalizedEvent> {
        |this: &mut Self, b: &Value| {
            if b.get("type").and_then(Value::as_str) != Some("tool_result") {
                return None;
            }
            let id = b
                .get("tool_use_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // Result of a Bash call: already emitted as CommandRun, and the enum
            // has no way to correlate a result to it. Drop it.
            if this.bash_calls.contains(id) {
                return None;
            }
            // `is_error` absent means success.
            let ok = !b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            let output = stringify_content(b.get("content"));
            Some(NormalizedEvent::ToolResult {
                call_id: id.into(),
                ok,
                output_excerpt: excerpt(&output),
            })
        }
    }

    /// Maps the terminal `result` line: `TurnCompleted` on success or `Failed`
    /// on error, always followed by `Ended`.
    fn map_result(&self, v: &Value) -> Vec<NormalizedEvent> {
        let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false)
            || v.get("subtype").and_then(Value::as_str) != Some("success");
        let terminal = if is_error {
            let reason = v
                .get("result")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    v.get("subtype")
                        .and_then(Value::as_str)
                        .map(|s| format!("agent reported {s}"))
                })
                .unwrap_or_else(|| "agent failed".to_string());
            NormalizedEvent::Failed { reason }
        } else {
            NormalizedEvent::TurnCompleted {
                usage: parse_usage(v.get("usage")),
            }
        };
        vec![terminal, NormalizedEvent::Ended]
    }
}

/// Reads `input_tokens` / `output_tokens` from a result's `usage` object.
/// Claude's cache-token fields are not part of the normalized `Usage`.
fn parse_usage(usage: Option<&Value>) -> Usage {
    let field = |name: &str| {
        usage
            .and_then(|u| u.get(name))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    Usage {
        input_tokens: field("input_tokens"),
        output_tokens: field("output_tokens"),
    }
}

/// A `tool_result`'s `content` is either a plain string or an array of
/// `{type:"text",text:…}` blocks; flatten either into text.
fn stringify_content(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Compact (no-whitespace) JSON, for tool-input excerpts.
fn compact(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// Truncates to [`EXCERPT_LIMIT`] on a char boundary, marking elision.
fn excerpt(s: &str) -> String {
    if s.len() <= EXCERPT_LIMIT {
        return s.to_string();
    }
    let mut end = EXCERPT_LIMIT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_all(text: &str) -> Vec<NormalizedEvent> {
        let mut mapper = ClaudeMapper::new();
        text.lines()
            .flat_map(|line| mapper.map_line(line).unwrap())
            .collect()
    }

    /// The captured real-world stream maps to the expected event sequence:
    /// TurnStarted, the Read call/result pair, the Bash CommandRun (its result
    /// dropped), the final text, then TurnCompleted + Ended.
    #[test]
    fn maps_captured_stream() {
        let fixture = include_str!("../fixtures/claude_stream.jsonl");
        let events = map_all(fixture);

        assert_eq!(events.first(), Some(&NormalizedEvent::TurnStarted));
        assert_eq!(events.last(), Some(&NormalizedEvent::Ended));

        // Read tool → ToolCall/ToolResult correlated by id; Bash → CommandRun.
        assert!(events.iter().any(|e| matches!(
            e,
            NormalizedEvent::ToolCall { name, .. } if name == "Read"
        )));
        let call_id = events.iter().find_map(|e| match e {
            NormalizedEvent::ToolCall { call_id, .. } => Some(call_id.clone()),
            _ => None,
        });
        let result_id = events.iter().find_map(|e| match e {
            NormalizedEvent::ToolResult { call_id, .. } => Some(call_id.clone()),
            _ => None,
        });
        assert!(call_id.is_some() && call_id == result_id);

        assert!(events.iter().any(|e| matches!(
            e,
            NormalizedEvent::CommandRun { cmd, exit: None } if cmd == "echo hi"
        )));
        // Exactly one ToolResult: the Read result. The Bash result is dropped.
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, NormalizedEvent::ToolResult { .. }))
                .count(),
            1
        );

        assert_eq!(
            events.iter().find(|e| matches!(e, NormalizedEvent::TurnCompleted { .. })),
            Some(&NormalizedEvent::TurnCompleted {
                usage: Usage {
                    input_tokens: 4363,
                    output_tokens: 180,
                },
            })
        );
    }

    #[test]
    fn thinking_maps_to_reasoning_and_text_to_assistant() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"answer"}]}}"#;
        assert_eq!(
            map_all(line),
            vec![
                NormalizedEvent::Reasoning { text: "hmm".into() },
                NormalizedEvent::AssistantText {
                    text: "answer".into()
                },
            ]
        );
    }

    #[test]
    fn empty_text_and_thinking_blocks_are_skipped() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"","signature":"s"},{"type":"text","text":""}]}}"#;
        assert!(map_all(line).is_empty());
    }

    #[test]
    fn non_bash_tool_use_becomes_tool_call_with_excerpt() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Grep","input":{"pattern":"fn main"}}]}}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::ToolCall {
                call_id: "t1".into(),
                name: "Grep".into(),
                input_excerpt: r#"{"pattern":"fn main"}"#.into(),
            }]
        );
    }

    #[test]
    fn tool_result_error_flag_sets_ok_false() {
        let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"boom","is_error":true}]}}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::ToolResult {
                call_id: "t1".into(),
                ok: false,
                output_excerpt: "boom".into(),
            }]
        );
    }

    #[test]
    fn error_result_maps_to_failed_then_ended() {
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"something broke"}"#;
        assert_eq!(
            map_all(line),
            vec![
                NormalizedEvent::Failed {
                    reason: "something broke".into()
                },
                NormalizedEvent::Ended,
            ]
        );
    }

    #[test]
    fn unknown_line_types_and_blanks_are_ignored() {
        let text = "\n{\"type\":\"rate_limit_event\"}\n{\"type\":\"system\",\"subtype\":\"hook_started\"}\n";
        assert!(map_all(text).is_empty());
    }

    #[test]
    fn invalid_json_is_a_protocol_error() {
        let mut mapper = ClaudeMapper::new();
        assert!(matches!(
            mapper.map_line("{not json}"),
            Err(AdapterError::Protocol(_))
        ));
    }

    #[test]
    fn array_content_tool_result_is_flattened() {
        let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t9","content":[{"type":"text","text":"line one"},{"type":"text","text":"line two"}]}]}}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::ToolResult {
                call_id: "t9".into(),
                ok: true,
                output_excerpt: "line one\nline two".into(),
            }]
        );
    }

    #[test]
    fn excerpt_truncates_on_char_boundary() {
        let long = "x".repeat(EXCERPT_LIMIT + 50);
        let out = excerpt(&long);
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().filter(|c| *c == 'x').count(), EXCERPT_LIMIT);
    }

    /// End-to-end against the real `claude` binary in a throwaway git repo.
    /// Ignored by default: it needs the CLI installed, network, and credits.
    /// Run with `cargo test -p loopfleet-adapters -- --ignored live_run`.
    #[tokio::test]
    #[ignore = "spawns the real claude CLI; needs network + credits"]
    async fn live_run_against_fixture_repo() {
        use crate::AgentAdapter;
        use std::process::Command as StdCommand;

        let dir = tempfile::tempdir().unwrap();
        StdCommand::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::fs::write(dir.path().join("README.md"), "hello\n").unwrap();

        let spec = RunSpec {
            cwd: dir.path().to_path_buf(),
            prompt: "Read README.md and then say the single word done.".into(),
            wrapper: Vec::new(),
        };
        let mut handle = ClaudeAdapter.start_run(&spec).await.unwrap();

        let mut events = Vec::new();
        while let Some(ev) = handle.events.recv().await {
            events.push(ev);
        }

        assert_eq!(events.first(), Some(&NormalizedEvent::TurnStarted));
        assert_eq!(events.last(), Some(&NormalizedEvent::Ended));
        assert!(events
            .iter()
            .any(|e| matches!(e, NormalizedEvent::TurnCompleted { .. })));
    }
}
