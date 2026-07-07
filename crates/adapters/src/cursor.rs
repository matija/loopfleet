//! cursor-agent adapter, headless. Spawns
//! `cursor-agent -p --output-format stream-json --force <prompt>` in the run's
//! worktree and maps its newline-delimited JSONL into [`NormalizedEvent`]s.
//!
//! The transport is one JSON object per line. The shapes this adapter maps
//! (captured from cursor-agent 2026.07.x, `--output-format stream-json`):
//! - `{"type":"system","subtype":"init",…}` → `TurnStarted`.
//! - `{"type":"assistant","message":{"role":"assistant","content":[…]}}` — each
//!   content block maps: `text` → `AssistantText`. cursor-agent's stream-json
//!   does not surface model reasoning, so no `Reasoning` is emitted.
//! - `{"type":"tool_call","subtype":"started","call_id":…,"tool_call":{…}}` — the
//!   `tool_call` object is keyed by tool kind (`readToolCall`, `shellToolCall`,
//!   …). A non-shell tool → `ToolCall`; a `shellToolCall` emits nothing here and
//!   waits for its `completed` event (which carries the exit code).
//! - `{"type":"tool_call","subtype":"completed",…}` — a non-shell tool →
//!   `ToolResult` (correlated by `call_id`); a `shellToolCall` →
//!   `CommandRun { cmd, exit }`. Unlike Claude and pi, cursor-agent reports a
//!   real numeric `exitCode` for shell commands, so `CommandRun.exit` is
//!   populated.
//! - `{"type":"result",…,"is_error":…,"usage":{…}}` — terminal →
//!   `TurnCompleted { usage }` (or `Failed` when `is_error`) followed by `Ended`.
//!
//! Other line types (`user` — the echoed prompt — and anything unrecognized)
//! carry nothing the enum represents and are ignored.

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

/// The cursor-agent headless adapter. Stateless; each `start_run` spawns its own
/// process and mapper.
pub struct CursorAdapter;

#[async_trait]
impl AgentAdapter for CursorAdapter {
    async fn start_run(&self, spec: &RunSpec) -> Result<RunHandle, AdapterError> {
        // `-p` is non-interactive; `--output-format stream-json` selects the JSONL
        // transport; `--force` bypasses cursor-agent's own permission prompts. In
        // headless mode cursor-agent fabricates a "user skipped" answer for any
        // prompt anyway (see PRD), so the Seatbelt profile (M2) is the real
        // boundary — `--force` just keeps the run from stalling.
        let mut child = crate::base_command(&spec.wrapper, "cursor-agent")
            .arg("-p")
            .args(["--output-format", "stream-json"])
            .arg("--force")
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
/// and forwards them. When the stream ends without a terminal `result` line,
/// synthesizes a `Failed`/`Ended` pair from stderr so consumers always see a
/// termination.
async fn drive(
    mut child: tokio::process::Child,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    tx: mpsc::Sender<NormalizedEvent>,
) {
    let mut mapper = CursorMapper::new();
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
            .map(|s| format!("agent exited without result: {s}"))
            .unwrap_or_else(|| "agent exited without result".to_string());
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

/// Stateless mapper from cursor-agent stream-json lines to [`NormalizedEvent`]s.
/// Every `tool_call` event carries the full typed `tool_call` object, so shell
/// vs. non-shell is decided per line with no cross-line state.
struct CursorMapper;

impl CursorMapper {
    fn new() -> Self {
        Self
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
            Some("system") if subtype(&v) == Some("init") => Ok(vec![NormalizedEvent::TurnStarted]),
            Some("assistant") => Ok(map_assistant(&v)),
            Some("tool_call") => Ok(map_tool_call(&v)),
            Some("result") => Ok(map_result(&v)),
            _ => Ok(vec![]),
        }
    }
}

/// Maps an assistant message's content blocks. Only `text` → `AssistantText`
/// (empty skipped); cursor-agent does not surface reasoning blocks.
fn map_assistant(v: &Value) -> Vec<NormalizedEvent> {
    v.get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| {
                    if b.get("type").and_then(Value::as_str) != Some("text") {
                        return None;
                    }
                    let text = b.get("text").and_then(Value::as_str).unwrap_or_default();
                    (!text.is_empty()).then(|| NormalizedEvent::AssistantText { text: text.into() })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Maps a `tool_call` line. A `shellToolCall` is normalized to `CommandRun` at
/// its `completed` event (which carries the exit code); its `started` event is
/// dropped. Every other tool emits `ToolCall` at `started` and `ToolResult` at
/// `completed`, correlated by `call_id`.
fn map_tool_call(v: &Value) -> Vec<NormalizedEvent> {
    let (name, entry) = match v.get("tool_call").and_then(tool_entry) {
        Some(x) => x,
        None => return vec![],
    };
    let call_id = v.get("call_id").and_then(Value::as_str).unwrap_or_default();
    let is_shell = name == "shell";

    match subtype(v) {
        Some("started") => {
            if is_shell {
                // Wait for `completed` — it carries the numeric exit code.
                return vec![];
            }
            let args = entry.get("args").cloned().unwrap_or(Value::Null);
            vec![NormalizedEvent::ToolCall {
                call_id: call_id.into(),
                name: name.into(),
                input_excerpt: excerpt(&compact(&args)),
            }]
        }
        Some("completed") => {
            if is_shell {
                let cmd = entry
                    .get("args")
                    .and_then(|a| a.get("command"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let exit = entry
                    .get("result")
                    .and_then(|r| r.get("success"))
                    .and_then(|s| s.get("exitCode"))
                    .and_then(Value::as_i64)
                    .map(|n| n as i32);
                return vec![NormalizedEvent::CommandRun {
                    cmd: cmd.into(),
                    exit,
                }];
            }
            let result = entry.get("result");
            // A completed tool_call reports success via a `success` key; anything
            // else (an `error`/`failure` variant) means the tool failed.
            let ok = result.and_then(|r| r.get("success")).is_some();
            let output = result.map(result_excerpt).unwrap_or_default();
            vec![NormalizedEvent::ToolResult {
                call_id: call_id.into(),
                ok,
                output_excerpt: excerpt(&output),
            }]
        }
        _ => vec![],
    }
}

/// Maps the terminal `result` line: `TurnCompleted` with usage on success,
/// `Failed` when `is_error`, always followed by `Ended`.
fn map_result(v: &Value) -> Vec<NormalizedEvent> {
    let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
    let terminal = if is_error {
        let reason = v
            .get("result")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| "agent run failed".to_string());
        NormalizedEvent::Failed { reason }
    } else {
        NormalizedEvent::TurnCompleted {
            usage: parse_usage(v.get("usage")),
        }
    };
    vec![terminal, NormalizedEvent::Ended]
}

/// Finds the typed tool object inside a `tool_call` map: the single key ending in
/// `ToolCall` (e.g. `readToolCall`), returning its normalized name (`read`) and
/// the object. `toolCallId` ends in `Id`, so it is never matched.
fn tool_entry(tool_call: &Value) -> Option<(&str, &Value)> {
    tool_call
        .as_object()?
        .iter()
        .find_map(|(k, val)| k.strip_suffix("ToolCall").map(|name| (name, val)))
}

/// A completed tool's `result` is `{"success":{…}}` or an error variant.
/// Prefers a plain `content` string; otherwise compacts the inner object.
fn result_excerpt(result: &Value) -> String {
    if let Some(success) = result.get("success") {
        if let Some(s) = success.get("content").and_then(Value::as_str) {
            return s.to_string();
        }
        return compact(success);
    }
    compact(result)
}

/// Reads `inputTokens` / `outputTokens` from a `result`'s `usage`. cursor-agent's
/// cache-token fields are not part of the normalized `Usage`.
fn parse_usage(usage: Option<&Value>) -> Usage {
    let field = |name: &str| {
        usage
            .and_then(|u| u.get(name))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    Usage {
        input_tokens: field("inputTokens"),
        output_tokens: field("outputTokens"),
    }
}

/// The `subtype` discriminator common to `system`, `tool_call`, and `result`.
fn subtype(v: &Value) -> Option<&str> {
    v.get("subtype").and_then(Value::as_str)
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
        let mut mapper = CursorMapper::new();
        text.lines()
            .flat_map(|line| mapper.map_line(line).unwrap())
            .collect()
    }

    /// The captured real-world stream maps to the expected event sequence:
    /// TurnStarted, the assistant text, the read call/result pair, the bash
    /// CommandRun with its real exit code, the final text, TurnCompleted, Ended.
    #[test]
    fn maps_captured_stream() {
        let fixture = include_str!("../fixtures/cursor_stream.jsonl");
        let events = map_all(fixture);

        assert_eq!(events.first(), Some(&NormalizedEvent::TurnStarted));
        assert_eq!(events.last(), Some(&NormalizedEvent::Ended));

        // The user prompt echo produces nothing; assistant text is surfaced.
        assert!(events.iter().any(|e| matches!(
            e,
            NormalizedEvent::AssistantText { text } if text == "done"
        )));

        // read tool → ToolCall/ToolResult correlated by id (even though the id
        // contains an embedded newline).
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
        assert!(call_id.unwrap().contains('\n'));

        // shell → CommandRun carrying the real exit code (0), not None.
        assert!(events.iter().any(|e| matches!(
            e,
            NormalizedEvent::CommandRun { cmd, exit: Some(0) } if cmd == "echo hi"
        )));
        // Exactly one ToolResult: the read result. The shell has no ToolResult.
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, NormalizedEvent::ToolResult { .. }))
                .count(),
            1
        );

        // Single terminal turn with parsed usage.
        let completed: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                NormalizedEvent::TurnCompleted { usage } => Some(usage.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            completed,
            vec![Usage {
                input_tokens: 38786,
                output_tokens: 244,
            }]
        );
    }

    #[test]
    fn system_init_maps_to_turn_started() {
        let line = r#"{"type":"system","subtype":"init","model":"x"}"#;
        assert_eq!(map_all(line), vec![NormalizedEvent::TurnStarted]);
    }

    #[test]
    fn non_init_system_line_is_ignored() {
        let line = r#"{"type":"system","subtype":"other"}"#;
        assert!(map_all(line).is_empty());
    }

    #[test]
    fn assistant_text_maps_and_empty_is_skipped() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"},{"type":"text","text":""}]}}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::AssistantText { text: "hi".into() }]
        );
    }

    #[test]
    fn user_message_is_ignored() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"do it"}]}}"#;
        assert!(map_all(line).is_empty());
    }

    #[test]
    fn non_shell_tool_start_becomes_tool_call_with_excerpt() {
        let line = r#"{"type":"tool_call","subtype":"started","call_id":"c1","tool_call":{"readToolCall":{"args":{"path":"x"}},"toolCallId":"c1"}}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::ToolCall {
                call_id: "c1".into(),
                name: "read".into(),
                input_excerpt: r#"{"path":"x"}"#.into(),
            }]
        );
    }

    #[test]
    fn non_shell_tool_completed_becomes_tool_result() {
        let line = r#"{"type":"tool_call","subtype":"completed","call_id":"c1","tool_call":{"readToolCall":{"result":{"success":{"content":"hello"}}},"toolCallId":"c1"}}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::ToolResult {
                call_id: "c1".into(),
                ok: true,
                output_excerpt: "hello".into(),
            }]
        );
    }

    #[test]
    fn tool_completed_error_variant_sets_ok_false() {
        let line = r#"{"type":"tool_call","subtype":"completed","call_id":"c1","tool_call":{"editToolCall":{"result":{"error":{"message":"boom"}}},"toolCallId":"c1"}}"#;
        let events = map_all(line);
        assert!(matches!(
            events.as_slice(),
            [NormalizedEvent::ToolResult { ok: false, .. }]
        ));
    }

    #[test]
    fn shell_start_is_dropped_and_completed_becomes_command_run() {
        let text = concat!(
            r#"{"type":"tool_call","subtype":"started","call_id":"s1","tool_call":{"shellToolCall":{"args":{"command":"ls -la"}},"toolCallId":"s1"}}"#,
            "\n",
            r#"{"type":"tool_call","subtype":"completed","call_id":"s1","tool_call":{"shellToolCall":{"args":{"command":"ls -la"},"result":{"success":{"exitCode":2}}},"toolCallId":"s1"}}"#
        );
        assert_eq!(
            map_all(text),
            vec![NormalizedEvent::CommandRun {
                cmd: "ls -la".into(),
                exit: Some(2),
            }]
        );
    }

    #[test]
    fn shell_completed_without_exit_code_has_none() {
        let line = r#"{"type":"tool_call","subtype":"completed","call_id":"s1","tool_call":{"shellToolCall":{"args":{"command":"x"},"result":{"error":{"message":"nope"}}},"toolCallId":"s1"}}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::CommandRun {
                cmd: "x".into(),
                exit: None,
            }]
        );
    }

    #[test]
    fn result_success_maps_to_turn_completed_then_ended() {
        let line = r#"{"type":"result","subtype":"success","is_error":false,"usage":{"inputTokens":10,"outputTokens":3}}"#;
        assert_eq!(
            map_all(line),
            vec![
                NormalizedEvent::TurnCompleted {
                    usage: Usage {
                        input_tokens: 10,
                        output_tokens: 3,
                    }
                },
                NormalizedEvent::Ended,
            ]
        );
    }

    #[test]
    fn result_error_maps_to_failed_then_ended() {
        let line = r#"{"type":"result","subtype":"error","is_error":true,"result":"model overloaded"}"#;
        assert_eq!(
            map_all(line),
            vec![
                NormalizedEvent::Failed {
                    reason: "model overloaded".into()
                },
                NormalizedEvent::Ended,
            ]
        );
    }

    #[test]
    fn invalid_json_is_a_protocol_error() {
        let mut mapper = CursorMapper::new();
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

    /// End-to-end against the real `cursor-agent` binary in a throwaway git repo.
    /// Ignored by default: it needs the CLI installed, network, and credits.
    /// Run with `cargo test -p loopfleet-adapters -- --ignored live_run`.
    #[tokio::test]
    #[ignore = "spawns the real cursor-agent CLI; needs network + credits"]
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
        let mut handle = CursorAdapter.start_run(&spec).await.unwrap();

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
