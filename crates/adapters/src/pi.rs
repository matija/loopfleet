//! pi adapter, headless. Spawns `pi -p --mode json <prompt>` in the run's
//! worktree and maps its newline-delimited AgentEvent JSONL into
//! [`NormalizedEvent`]s.
//!
//! The transport is one JSON object per line. The shapes this adapter maps
//! (captured from pi 0.80.x, `--mode json`):
//! - `{"type":"turn_start"}` → `TurnStarted`; `{"type":"turn_end","message":{…}}`
//!   → `TurnCompleted` (its `message.usage`) or `Failed` when the message's
//!   `stopReason` is `"error"`. A headless run has one or more agent turns.
//! - `{"type":"message_end","message":{"role":"assistant","content":[…]}}` — each
//!   content block maps: `text` → `AssistantText`, `thinking` → `Reasoning`.
//!   `toolCall` blocks are *not* mapped here; tool activity comes from the
//!   `tool_execution_*` events, which also carry the results.
//! - `{"type":"tool_execution_start","toolCallId":…,"toolName":…,"args":{…}}` →
//!   `CommandRun` when the tool is `bash`, else `ToolCall`.
//! - `{"type":"tool_execution_end","toolCallId":…,"result":{…},"isError":…}` →
//!   `ToolResult`, correlated by `toolCallId`. Results of `bash` calls are
//!   dropped: they were already normalized to `CommandRun`, which the enum
//!   carries as a single event with no result pairing.
//! - `{"type":"agent_end",…}` — the run wrapper's terminal line → `Ended`.
//!
//! Other line types pi emits (`session`, `agent_start`, `message_start`,
//! streaming `message_update` / `tool_execution_update` deltas, non-assistant
//! `message_end`s) carry nothing the enum represents and are ignored — the
//! complete `message_end` and `tool_execution_end` events are the source of
//! truth, so the deltas are noise for a headless run.

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

/// The pi headless adapter. Stateless; each `start_run` spawns its own process
/// and mapper.
pub struct PiAdapter;

#[async_trait]
impl AgentAdapter for PiAdapter {
    async fn start_run(&self, spec: &RunSpec) -> Result<RunHandle, AdapterError> {
        // `-p` is non-interactive (process the prompt and exit); `--mode json`
        // selects the AgentEvent JSONL transport. In headless mode pi resolves
        // permission prompts automatically — the Seatbelt profile (M2) is the
        // real boundary, so no per-agent bypass flag is passed here.
        let mut child = crate::base_command(&spec.wrapper, "pi")
            .arg("-p")
            .args(["--mode", "json"])
            .arg(&spec.prompt)
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
/// and forwards them. When the stream ends without a terminal `agent_end` line,
/// synthesizes a `Failed`/`Ended` pair from stderr so consumers always see a
/// termination.
async fn drive(
    mut child: tokio::process::Child,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    tx: mpsc::Sender<NormalizedEvent>,
) {
    let mut mapper = PiMapper::new();
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
            .map(|s| format!("agent exited without agent_end: {s}"))
            .unwrap_or_else(|| "agent exited without agent_end".to_string());
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

/// Stateful mapper from pi AgentEvent lines to [`NormalizedEvent`]s. Holds the
/// set of `bash` tool-call ids so their `tool_execution_end` lines can be
/// dropped (already normalized to `CommandRun`).
struct PiMapper {
    bash_calls: HashSet<String>,
}

impl PiMapper {
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
            .map_err(|e| AdapterError::Protocol(format!("invalid agent-event line: {e}")))?;

        match v.get("type").and_then(Value::as_str) {
            Some("turn_start") => Ok(vec![NormalizedEvent::TurnStarted]),
            Some("message_end") => Ok(self.map_message_end(&v)),
            Some("tool_execution_start") => Ok(self.map_tool_start(&v).into_iter().collect()),
            Some("tool_execution_end") => Ok(self.map_tool_end(&v).into_iter().collect()),
            Some("turn_end") => Ok(vec![self.map_turn_end(&v)]),
            Some("agent_end") => Ok(vec![NormalizedEvent::Ended]),
            _ => Ok(vec![]),
        }
    }

    /// Maps an assistant `message_end`'s content blocks. Non-assistant messages
    /// (the echoed prompt, tool-result messages) yield nothing; `toolCall`
    /// blocks are skipped because tool activity is sourced from the
    /// `tool_execution_*` events.
    fn map_message_end(&self, v: &Value) -> Vec<NormalizedEvent> {
        let message = match v.get("message") {
            Some(m) => m,
            None => return vec![],
        };
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            return vec![];
        }
        message
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| blocks.iter().filter_map(map_assistant_block).collect())
            .unwrap_or_default()
    }

    /// Maps `tool_execution_start`: `bash` → `CommandRun`; anything else →
    /// `ToolCall`. Records `bash` ids so the matching end is dropped.
    fn map_tool_start(&mut self, v: &Value) -> Option<NormalizedEvent> {
        let id = v.get("toolCallId").and_then(Value::as_str).unwrap_or_default();
        let name = v.get("toolName").and_then(Value::as_str).unwrap_or_default();
        let args = v.get("args").cloned().unwrap_or(Value::Null);
        if name == "bash" {
            // Shell-exec is normalized to CommandRun (no result pairing);
            // remember the id so we drop its tool_execution_end.
            self.bash_calls.insert(id.to_string());
            let cmd = args.get("command").and_then(Value::as_str).unwrap_or_default();
            // pi's bash result carries no numeric exit code (only an isError
            // flag), so exit is unknown at invocation and stays absent.
            Some(NormalizedEvent::CommandRun {
                cmd: cmd.into(),
                exit: None,
            })
        } else {
            Some(NormalizedEvent::ToolCall {
                call_id: id.into(),
                name: name.into(),
                input_excerpt: excerpt(&compact(&args)),
            })
        }
    }

    /// Maps `tool_execution_end` to a `ToolResult`, dropping the results of
    /// `bash` calls (already emitted as `CommandRun`).
    fn map_tool_end(&self, v: &Value) -> Option<NormalizedEvent> {
        let id = v.get("toolCallId").and_then(Value::as_str).unwrap_or_default();
        if self.bash_calls.contains(id) {
            return None;
        }
        let ok = !v.get("isError").and_then(Value::as_bool).unwrap_or(false);
        let output = stringify_content(v.get("result").and_then(|r| r.get("content")));
        Some(NormalizedEvent::ToolResult {
            call_id: id.into(),
            ok,
            output_excerpt: excerpt(&output),
        })
    }

    /// Maps a `turn_end`: `Failed` when the turn's message errored, else
    /// `TurnCompleted` with the turn's usage. Terminal `Ended` comes separately
    /// from `agent_end`.
    fn map_turn_end(&self, v: &Value) -> NormalizedEvent {
        let message = v.get("message");
        let stop_reason = message
            .and_then(|m| m.get("stopReason"))
            .and_then(Value::as_str);
        if stop_reason == Some("error") {
            let reason = message
                .and_then(|m| m.get("errorMessage"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| "agent turn failed".to_string());
            NormalizedEvent::Failed { reason }
        } else {
            NormalizedEvent::TurnCompleted {
                usage: parse_usage(message.and_then(|m| m.get("usage"))),
            }
        }
    }
}

/// Block mapper for an assistant message's content: `text` → `AssistantText`,
/// `thinking` → `Reasoning`. `toolCall` (and anything else) yields nothing.
fn map_assistant_block(b: &Value) -> Option<NormalizedEvent> {
    match b.get("type").and_then(Value::as_str) {
        Some("text") => {
            let text = b.get("text").and_then(Value::as_str).unwrap_or_default();
            (!text.is_empty()).then(|| NormalizedEvent::AssistantText { text: text.into() })
        }
        Some("thinking") => {
            let text = b.get("thinking").and_then(Value::as_str).unwrap_or_default();
            (!text.is_empty()).then(|| NormalizedEvent::Reasoning { text: text.into() })
        }
        _ => None,
    }
}

/// Reads `input` / `output` from a turn's `usage` object. pi's cache and cost
/// fields are not part of the normalized `Usage`.
fn parse_usage(usage: Option<&Value>) -> Usage {
    let field = |name: &str| {
        usage
            .and_then(|u| u.get(name))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    Usage {
        input_tokens: field("input"),
        output_tokens: field("output"),
    }
}

/// A tool result's `content` is either a plain string or an array of
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
        let mut mapper = PiMapper::new();
        text.lines()
            .flat_map(|line| mapper.map_line(line).unwrap())
            .collect()
    }

    /// The captured real-world stream maps to the expected event sequence:
    /// two turns, the Reasoning + read call/result pair, the bash CommandRun
    /// (its result dropped), the final text, then Ended.
    #[test]
    fn maps_captured_stream() {
        let fixture = include_str!("../fixtures/pi_stream.jsonl");
        let events = map_all(fixture);

        assert_eq!(events.first(), Some(&NormalizedEvent::TurnStarted));
        assert_eq!(events.last(), Some(&NormalizedEvent::Ended));

        // The user prompt echo and streaming deltas produce nothing; the first
        // mapped content is the assistant's Reasoning block.
        assert!(events.iter().any(|e| matches!(
            e,
            NormalizedEvent::Reasoning { text } if text.starts_with("The user wants")
        )));

        // read tool → ToolCall/ToolResult correlated by id; bash → CommandRun.
        assert!(events.iter().any(|e| matches!(
            e,
            NormalizedEvent::ToolCall { name, .. } if name == "read"
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
        // Exactly one ToolResult: the read result. The bash result is dropped.
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, NormalizedEvent::ToolResult { .. }))
                .count(),
            1
        );

        // Two turns, each with its own usage.
        let completed: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                NormalizedEvent::TurnCompleted { usage } => Some(usage.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            completed,
            vec![
                Usage {
                    input_tokens: 5545,
                    output_tokens: 47,
                },
                Usage {
                    input_tokens: 59,
                    output_tokens: 31,
                },
            ]
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, NormalizedEvent::TurnStarted))
                .count(),
            2
        );
    }

    #[test]
    fn thinking_maps_to_reasoning_and_text_to_assistant() {
        let line = r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"answer"}]}}"#;
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
    fn non_assistant_message_end_is_ignored() {
        let user = r#"{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"do the thing"}]}}"#;
        let tool = r#"{"type":"message_end","message":{"role":"toolResult","content":[{"type":"text","text":"result"}]}}"#;
        assert!(map_all(&format!("{user}\n{tool}")).is_empty());
    }

    #[test]
    fn empty_text_and_thinking_blocks_are_skipped() {
        let line = r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":""},{"type":"text","text":""}]}}"#;
        assert!(map_all(line).is_empty());
    }

    #[test]
    fn tool_call_blocks_in_message_are_not_mapped() {
        // The assistant message carries the toolCall too, but tool activity is
        // sourced from tool_execution_* — mapping it here would double-emit.
        let line = r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"toolCall","id":"t1","name":"read","arguments":{"path":"x"}}]}}"#;
        assert!(map_all(line).is_empty());
    }

    #[test]
    fn non_bash_tool_start_becomes_tool_call_with_excerpt() {
        let line = r#"{"type":"tool_execution_start","toolCallId":"t1","toolName":"grep","args":{"pattern":"fn main"}}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::ToolCall {
                call_id: "t1".into(),
                name: "grep".into(),
                input_excerpt: r#"{"pattern":"fn main"}"#.into(),
            }]
        );
    }

    #[test]
    fn bash_tool_start_becomes_command_run_and_its_end_is_dropped() {
        let text = concat!(
            r#"{"type":"tool_execution_start","toolCallId":"b1","toolName":"bash","args":{"command":"ls -la"}}"#,
            "\n",
            r#"{"type":"tool_execution_end","toolCallId":"b1","toolName":"bash","result":{"content":[{"type":"text","text":"files"}]},"isError":false}"#
        );
        assert_eq!(
            map_all(text),
            vec![NormalizedEvent::CommandRun {
                cmd: "ls -la".into(),
                exit: None,
            }]
        );
    }

    #[test]
    fn tool_end_error_flag_sets_ok_false() {
        let line = r#"{"type":"tool_execution_end","toolCallId":"t1","result":{"content":[{"type":"text","text":"boom"}]},"isError":true}"#;
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
    fn errored_turn_maps_to_failed() {
        let line = r#"{"type":"turn_end","message":{"role":"assistant","stopReason":"error","errorMessage":"400 something broke","usage":{"input":0,"output":0}}}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::Failed {
                reason: "400 something broke".into()
            }]
        );
    }

    #[test]
    fn agent_end_maps_to_ended() {
        let line = r#"{"type":"agent_end","willRetry":false}"#;
        assert_eq!(map_all(line), vec![NormalizedEvent::Ended]);
    }

    #[test]
    fn unknown_line_types_and_blanks_are_ignored() {
        let text = "\n{\"type\":\"session\",\"version\":3}\n{\"type\":\"agent_start\"}\n{\"type\":\"message_update\"}\n{\"type\":\"tool_execution_update\"}\n";
        assert!(map_all(text).is_empty());
    }

    #[test]
    fn invalid_json_is_a_protocol_error() {
        let mut mapper = PiMapper::new();
        assert!(matches!(
            mapper.map_line("{not json}"),
            Err(AdapterError::Protocol(_))
        ));
    }

    #[test]
    fn excerpt_truncates_on_char_boundary() {
        let long = "x".repeat(EXCERPT_LIMIT + 50);
        let out = excerpt(&long);
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().filter(|c| *c == 'x').count(), EXCERPT_LIMIT);
    }

    /// End-to-end against the real `pi` binary in a throwaway git repo.
    /// Ignored by default: it needs the CLI installed, network, and credits.
    /// Run with `cargo test -p loopfleet-adapters -- --ignored live_run`.
    #[tokio::test]
    #[ignore = "spawns the real pi CLI; needs network + credits"]
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
        let mut handle = PiAdapter.start_run(&spec).await.unwrap();

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
