use std::path::Path;

use async_trait::async_trait;
use loopfleet_core::{NormalizedEvent, Usage};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::{AdapterError, AgentAdapter, RunHandle, RunSpec, SessionHandle, SessionSeed};

const EXCERPT_LIMIT: usize = 2000;

pub struct CodexAdapter;

#[async_trait]
impl AgentAdapter for CodexAdapter {
    async fn start_run(&self, spec: &RunSpec) -> Result<RunHandle, AdapterError> {
        let mut cmd = crate::base_command(&spec.wrapper, "codex");
        cmd.arg("exec")
            .arg("--json")
            .arg("--dangerously-bypass-approvals-and-sandbox")
            .arg("--dangerously-bypass-hook-trust");
        if let Some(model) = &spec.model {
            cmd.arg("--model").arg(model);
        }
        let mut child = cmd
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

async fn drive(
    mut child: tokio::process::Child,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    tx: mpsc::Sender<NormalizedEvent>,
) {
    let mut mapper = CodexMapper;
    let mut lines = BufReader::new(stdout).lines();
    let mut saw_terminal = false;

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let events = mapper.map_line(&line).unwrap_or_else(|e| {
                    vec![NormalizedEvent::Failed {
                        reason: e.to_string(),
                    }]
                });
                for event in events {
                    if matches!(event, NormalizedEvent::Ended) {
                        saw_terminal = true;
                    }
                    if tx.send(event).await.is_err() {
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
            .map(|s| format!("agent exited without turn.completed: {s}"))
            .unwrap_or_else(|| "agent exited without turn.completed".to_string());
        let _ = tx.send(NormalizedEvent::Failed { reason }).await;
        let _ = tx.send(NormalizedEvent::Ended).await;
    }

    let _ = child.wait().await;
}

async fn read_stderr(stderr: tokio::process::ChildStderr) -> Option<String> {
    let mut lines = BufReader::new(stderr).lines();
    let mut collected = Vec::new();
    while let Ok(Some(line)) = lines.next_line().await {
        collected.push(line);
    }
    (!collected.is_empty()).then(|| collected.join("\n"))
}

struct CodexMapper;

impl CodexMapper {
    fn map_line(&mut self, line: &str) -> Result<Vec<NormalizedEvent>, AdapterError> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(vec![]);
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|e| AdapterError::Protocol(format!("invalid JSONL line: {e}")))?;
        match value.get("type").and_then(Value::as_str) {
            Some("turn.started") => Ok(vec![NormalizedEvent::TurnStarted]),
            Some("item.started") => Ok(map_started_item(value.get("item"))),
            Some("item.completed") => Ok(map_completed_item(value.get("item"))),
            Some("turn.completed") => Ok(vec![
                NormalizedEvent::TurnCompleted {
                    usage: parse_usage(value.get("usage")),
                },
                NormalizedEvent::Ended,
            ]),
            Some("turn.failed") => Ok(map_error(error_message(&value))),
            Some("error") => Ok(map_error(
                value
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex failed"),
            )),
            _ => Ok(vec![]),
        }
    }
}

fn map_started_item(item: Option<&Value>) -> Vec<NormalizedEvent> {
    let Some(item) = item else { return vec![] };
    match item.get("type").and_then(Value::as_str) {
        Some("mcp_tool_call") => vec![NormalizedEvent::ToolCall {
            call_id: string_field(item, "id"),
            name: mcp_name(item),
            input_excerpt: excerpt(&compact(item.get("arguments").unwrap_or(&Value::Null))),
        }],
        Some("web_search") => vec![NormalizedEvent::ToolCall {
            call_id: string_field(item, "id"),
            name: "web_search".into(),
            input_excerpt: excerpt(&string_field(item, "query")),
        }],
        _ => vec![],
    }
}

fn map_completed_item(item: Option<&Value>) -> Vec<NormalizedEvent> {
    let Some(item) = item else { return vec![] };
    match item.get("type").and_then(Value::as_str) {
        Some("agent_message") => nonempty_text(item, "text")
            .map(|text| vec![NormalizedEvent::AssistantText { text }])
            .unwrap_or_default(),
        Some("reasoning") => nonempty_text(item, "text")
            .map(|text| vec![NormalizedEvent::Reasoning { text }])
            .unwrap_or_default(),
        Some("command_execution") => vec![NormalizedEvent::CommandRun {
            cmd: string_field(item, "command"),
            exit: item
                .get("exit_code")
                .and_then(Value::as_i64)
                .map(|exit| exit as i32),
        }],
        Some("mcp_tool_call") => vec![NormalizedEvent::ToolResult {
            call_id: string_field(item, "id"),
            ok: item.get("error").is_none()
                && !matches!(
                    item.get("status").and_then(Value::as_str),
                    Some("failed" | "declined")
                ),
            output_excerpt: excerpt(&compact(
                item.get("result")
                    .or_else(|| item.get("error"))
                    .unwrap_or(&Value::Null),
            )),
        }],
        Some("web_search") => vec![NormalizedEvent::ToolResult {
            call_id: string_field(item, "id"),
            ok: true,
            output_excerpt: excerpt(&string_field(item, "query")),
        }],
        _ => vec![],
    }
}

fn map_error(message: &str) -> Vec<NormalizedEvent> {
    let failed = NormalizedEvent::Failed {
        reason: message.to_string(),
    };
    let lower = message.to_ascii_lowercase();
    if lower.contains("rate limit") || lower.contains("usage limit") {
        vec![
            NormalizedEvent::RateLimited {
                reset_at: None,
                message: Some(message.to_string()),
            },
            failed,
            NormalizedEvent::Ended,
        ]
    } else {
        vec![failed, NormalizedEvent::Ended]
    }
}

fn error_message(value: &Value) -> &str {
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("Codex turn failed")
}

fn parse_usage(usage: Option<&Value>) -> Usage {
    let field = |name| {
        usage
            .and_then(|usage| usage.get(name))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    Usage {
        input_tokens: field("input_tokens"),
        output_tokens: field("output_tokens"),
    }
}

fn string_field(value: &Value, name: &str) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn nonempty_text(value: &Value, name: &str) -> Option<String> {
    let text = string_field(value, name);
    (!text.is_empty()).then_some(text)
}

fn mcp_name(item: &Value) -> String {
    let server = string_field(item, "server");
    let tool = string_field(item, "tool");
    if server.is_empty() {
        tool
    } else {
        format!("{server}.{tool}")
    }
}

fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn excerpt(text: &str) -> String {
    if text.len() <= EXCERPT_LIMIT {
        return text.to_string();
    }
    let mut end = EXCERPT_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_all(text: &str) -> Vec<NormalizedEvent> {
        let mut mapper = CodexMapper;
        text.lines()
            .flat_map(|line| mapper.map_line(line).unwrap())
            .collect()
    }

    #[test]
    fn maps_captured_stream() {
        let events = map_all(include_str!("../fixtures/codex_stream.jsonl"));
        assert_eq!(events.first(), Some(&NormalizedEvent::TurnStarted));
        assert_eq!(events.last(), Some(&NormalizedEvent::Ended));
        assert!(events.iter().any(|event| matches!(
            event,
            NormalizedEvent::Reasoning { text } if text == "I will inspect the project."
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            NormalizedEvent::CommandRun { cmd, exit: Some(0) } if cmd == "cargo test"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            NormalizedEvent::ToolCall { call_id, name, .. }
                if call_id == "item_3" && name == "docs.read"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            NormalizedEvent::TurnCompleted { usage }
                if usage.input_tokens == 120 && usage.output_tokens == 25
        )));
    }

    #[test]
    fn maps_failed_and_limited_turns() {
        let failed = map_all(r#"{"type":"turn.failed","error":{"message":"bad model"}}"#);
        assert_eq!(
            failed,
            vec![
                NormalizedEvent::Failed {
                    reason: "bad model".into()
                },
                NormalizedEvent::Ended
            ]
        );

        let limited = map_all(r#"{"type":"error","message":"Usage limit reached"}"#);
        assert!(matches!(
            limited.first(),
            Some(NormalizedEvent::RateLimited { message: Some(message), .. })
                if message == "Usage limit reached"
        ));
        assert_eq!(limited.last(), Some(&NormalizedEvent::Ended));
    }

    #[test]
    fn rejects_invalid_json() {
        let mut mapper = CodexMapper;
        assert!(matches!(
            mapper.map_line("not json"),
            Err(AdapterError::Protocol(_))
        ));
    }
}
