use std::path::Path;

use async_trait::async_trait;
use loopfleet_core::{NormalizedEvent, Usage};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::{AdapterError, AgentAdapter, RunHandle, RunSpec, SessionHandle, SessionSeed};

const EXCERPT_LIMIT: usize = 2000;

pub struct CodexAdapter;

#[async_trait]
impl AgentAdapter for CodexAdapter {
    async fn start_run(&self, spec: &RunSpec) -> Result<RunHandle, AdapterError> {
        let mut cmd = crate::base_command(&spec.wrapper, "codex");
        let mut child = cmd
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .current_dir(&spec.cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(AdapterError::Spawn)?;
        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout = child
            .stdout
            .take()
            .expect("stdout was piped so it is present");
        let stderr = child
            .stderr
            .take()
            .expect("stderr was piped so it is present");
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(drive(child, stdin, stdout, stderr, tx, spec.clone()));
        Ok(RunHandle {
            events: rx,
            steer: None,
        })
    }

    async fn open_session(
        &self,
        _cwd: &Path,
        _seed: SessionSeed,
    ) -> Result<SessionHandle, AdapterError> {
        Err(AdapterError::SessionsUnsupported)
    }
}

async fn send(stdin: &mut tokio::process::ChildStdin, value: Value) -> Result<(), AdapterError> {
    stdin
        .write_all(format!("{value}\n").as_bytes())
        .await
        .map_err(|e| AdapterError::Protocol(format!("writing agent stdin: {e}")))
}

async fn drive(
    mut child: tokio::process::Child,
    mut stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    tx: mpsc::Sender<NormalizedEvent>,
    spec: RunSpec,
) {
    let mut stderr = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut text = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            text = excerpt(&format!("{text}\n{line}"));
        }
        text
    });
    let result = async {
        send(&mut stdin, json!({"id":1,"method":"initialize","params":{
            "clientInfo":{"name":"loopfleet","title":"Loopfleet","version":env!("CARGO_PKG_VERSION")}
        }})).await?;
        let mut lines = BufReader::new(stdout).lines();
        let mut mapper = CodexMapper::default();
        let mut expected = 1;
        loop {
            let line = tokio::select! {
                _ = tx.closed() => return Ok(()),
                line = lines.next_line() => line.map_err(|e| AdapterError::Protocol(format!("reading agent stdout: {e}")))?,
            };
            let Some(line) = line else {
                return Err(AdapterError::Protocol("agent exited without turn/completed".into()));
            };
            let value: Value = serde_json::from_str(&line)
                .map_err(|e| AdapterError::Protocol(format!("invalid JSONL line: {e}")))?;
            if value.get("id").is_some() {
                if value.get("method").is_some() {
                    return Err(AdapterError::Protocol(format!("unexpected server request: {}", value["method"])));
                }
                if value["id"].as_u64() != Some(expected) {
                    return Err(AdapterError::Protocol("unexpected response id".into()));
                }
                if let Some(error) = value.get("error") {
                    return Err(AdapterError::Protocol(error.get("message").and_then(Value::as_str).unwrap_or("Codex request failed").into()));
                }
                match expected {
                    1 => {
                        send(&mut stdin, json!({"method":"initialized"})).await?;
                        send(&mut stdin, json!({"id":2,"method":"thread/start","params":{
                            "model":spec.model,"cwd":spec.cwd,"approvalPolicy":"never","sandbox":"danger-full-access"
                        }})).await?;
                    }
                    2 => {
                        let thread = value["result"]["thread"]["id"].as_str()
                            .ok_or_else(|| AdapterError::Protocol("missing thread id".into()))?;
                        send(&mut stdin, json!({"id":3,"method":"turn/start","params":{
                            "threadId":thread,"input":[{"type":"text","text":spec.prompt}]
                        }})).await?;
                    }
                    _ => {}
                }
                expected += 1;
                continue;
            }
            for event in mapper.map_line(&line)? {
                let ended = matches!(event, NormalizedEvent::Ended);
                if tx.send(event).await.is_err() || ended {
                    return Ok(());
                }
            }
        }
    }.await;
    crate::stop_agent(&mut child);
    drop(stdin);
    if tokio::time::timeout(std::time::Duration::from_secs(2), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
    let stderr = match tokio::time::timeout(std::time::Duration::from_secs(2), &mut stderr).await {
        Ok(result) => result.unwrap_or_default(),
        Err(_) => {
            stderr.abort();
            let _ = stderr.await;
            String::new()
        }
    };
    if let Err(error) = result {
        for event in map_error(&format!(
            "{error}{}",
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        )) {
            let _ = tx.send(event).await;
        }
    }
}

#[derive(Default)]
struct CodexMapper {
    usage: Usage,
}

impl CodexMapper {
    fn map_line(&mut self, line: &str) -> Result<Vec<NormalizedEvent>, AdapterError> {
        if line.trim().is_empty() {
            return Ok(vec![]);
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|e| AdapterError::Protocol(format!("invalid JSONL line: {e}")))?;
        let params = &value["params"];
        match value.get("method").and_then(Value::as_str) {
            Some("turn/started") => Ok(vec![NormalizedEvent::TurnStarted]),
            Some("item/started") => Ok(map_started_item(params.get("item"))),
            Some("item/completed") => Ok(map_completed_item(params.get("item"))),
            Some("thread/tokenUsage/updated") => {
                self.usage = parse_usage(params["tokenUsage"].get("total"));
                Ok(vec![])
            }
            Some("turn/completed") => match params["turn"]["status"].as_str() {
                Some("completed") => Ok(vec![
                    NormalizedEvent::TurnCompleted {
                        usage: self.usage.clone(),
                    },
                    NormalizedEvent::Ended,
                ]),
                _ => Ok(map_error(error_message(&params["turn"]))),
            },
            Some("error") if params["willRetry"].as_bool() != Some(true) => {
                Ok(map_error(error_message(params)))
            }
            _ => Ok(vec![]),
        }
    }
}

fn map_started_item(item: Option<&Value>) -> Vec<NormalizedEvent> {
    let Some(item) = item else { return vec![] };
    match item.get("type").and_then(Value::as_str) {
        Some("mcpToolCall") => vec![NormalizedEvent::ToolCall {
            call_id: string_field(item, "id"),
            name: mcp_name(item),
            input_excerpt: excerpt(&compact(item.get("arguments").unwrap_or(&Value::Null))),
        }],
        Some("webSearch") => vec![NormalizedEvent::ToolCall {
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
        Some("agentMessage") => nonempty_text(item, "text")
            .map(|text| vec![NormalizedEvent::AssistantText { text }])
            .unwrap_or_default(),
        Some("reasoning") => item
            .get("summary")
            .filter(|summary| summary.as_array().is_some_and(|parts| !parts.is_empty()))
            .or_else(|| item.get("content"))
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .filter(|text| !text.is_empty())
            .map(|text| vec![NormalizedEvent::Reasoning { text }])
            .unwrap_or_default(),
        Some("commandExecution") => vec![NormalizedEvent::CommandRun {
            cmd: string_field(item, "command"),
            exit: item
                .get("exitCode")
                .and_then(Value::as_i64)
                .map(|exit| exit as i32),
        }],
        Some("mcpToolCall") => vec![NormalizedEvent::ToolResult {
            call_id: string_field(item, "id"),
            ok: item.get("error").is_none_or(Value::is_null)
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
        Some("webSearch") => vec![NormalizedEvent::ToolResult {
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
        input_tokens: field("inputTokens"),
        output_tokens: field("outputTokens"),
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
        let mut mapper = CodexMapper::default();
        text.lines()
            .flat_map(|line| mapper.map_line(line).unwrap())
            .collect()
    }

    #[test]
    fn maps_captured_stream() {
        let events = map_all(concat!(
            r#"{"method":"turn/started","params":{}}"#,
            "\n",
            r#"{"method":"item/completed","params":{"item":{"type":"reasoning","summary":["I will inspect the project."]}}}"#,
            "\n",
            r#"{"method":"item/completed","params":{"item":{"type":"commandExecution","command":"cargo test","exitCode":0}}}"#,
            "\n",
            r#"{"method":"item/started","params":{"item":{"type":"mcpToolCall","id":"item_3","server":"docs","tool":"read"}}}"#,
            "\n",
            r#"{"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"total":{"inputTokens":120,"outputTokens":25}}}}"#,
            "\n",
            r#"{"method":"turn/completed","params":{"turn":{"status":"completed"}}}"#
        ));
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
        let failed = map_all(
            r#"{"method":"turn/completed","params":{"turn":{"status":"failed","error":{"message":"bad model"}}}}"#,
        );
        assert_eq!(
            failed,
            vec![
                NormalizedEvent::Failed {
                    reason: "bad model".into()
                },
                NormalizedEvent::Ended
            ]
        );

        let limited = map_all(
            r#"{"method":"error","params":{"error":{"message":"Usage limit reached"},"willRetry":false}}"#,
        );
        assert!(matches!(
            limited.first(),
            Some(NormalizedEvent::RateLimited { message: Some(message), .. })
                if message == "Usage limit reached"
        ));
        assert_eq!(limited.last(), Some(&NormalizedEvent::Ended));
    }

    #[test]
    fn rejects_invalid_json() {
        let mut mapper = CodexMapper::default();
        assert!(matches!(
            mapper.map_line("not json"),
            Err(AdapterError::Protocol(_))
        ));
    }
    #[tokio::test]
    async fn initializes_fresh_threads_and_reaps_each_pass() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("server.py");
        std::fs::write(&script, r#"
import json, os, sys
assert sys.argv[1:] == ['codex', 'app-server', '--listen', 'stdio://']
def read():
    return json.loads(sys.stdin.readline())
def emit(value):
    print(json.dumps(value), flush=True)
def reply(id, result):
    emit({'id': id, 'result': result})
msg = read()
assert msg['method'] == 'initialize' and msg['id'] == 1
assert msg['params']['clientInfo']['name'] == 'loopfleet'
reply(1, {'userAgent': 'test'})
assert read() == {'method': 'initialized'}
msg = read()
assert msg['method'] == 'thread/start' and msg['id'] == 2
assert msg['params'] == {'cwd': os.getcwd(), 'model': 'test-model', 'approvalPolicy': 'never', 'sandbox': 'danger-full-access'}
thread = str(os.getpid())
with open('passes', 'a') as file:
    file.write(thread + '\n')
reply(2, {'thread': {'id': thread}})
emit({'method': 'thread/started', 'params': {'thread': {'id': thread}}})
msg = read()
assert msg['method'] == 'turn/start' and msg['id'] == 3
assert msg['params'] == {'threadId': thread, 'input': [{'type': 'text', 'text': 'initial\nprompt "quoted"'}]}
reply(3, {'turn': {'id': 'turn'}})
sys.stderr.write('diagnostic\n' * 10000)
sys.stderr.flush()
emit({'method': 'turn/started', 'params': {}})
emit({'method': 'item/completed', 'params': {'item': {'type': 'agentMessage', 'text': 'done'}}})
emit({'method': 'turn/completed', 'params': {'turn': {'status': 'completed'}}})
sys.stdin.read()
"#).unwrap();
        let spec = RunSpec {
            cwd: dir.path().canonicalize().unwrap(),
            prompt: "initial\nprompt \"quoted\"".into(),
            wrapper: vec!["python3".into(), script.into_os_string()],
            model: Some("test-model".into()),
        };
        for _ in 0..2 {
            let mut run = CodexAdapter.start_run(&spec).await.unwrap();
            let events = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                let mut events = Vec::new();
                while let Some(event) = run.events.recv().await {
                    events.push(event);
                }
                events
            })
            .await
            .unwrap();
            assert_eq!(
                events,
                vec![
                    NormalizedEvent::TurnStarted,
                    NormalizedEvent::AssistantText {
                        text: "done".into()
                    },
                    NormalizedEvent::TurnCompleted {
                        usage: Usage::default()
                    },
                    NormalizedEvent::Ended,
                ]
            );
        }
        let passes = std::fs::read_to_string(dir.path().join("passes")).unwrap();
        let pids: Vec<i32> = passes.lines().map(|pid| pid.parse().unwrap()).collect();
        assert_eq!(pids.len(), 2);
        assert_ne!(pids[0], pids[1]);
        #[cfg(unix)]
        for pid in pids {
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        }
    }
    #[tokio::test]
    async fn reports_request_and_transport_failures() {
        let dir = tempfile::tempdir().unwrap();
        for output in [
            r#"{"id":1,"error":{"message":"initialization failed"}}"#,
            "invalid JSON",
            "",
        ] {
            let spec = RunSpec {
                cwd: dir.path().to_path_buf(),
                prompt: "prompt".into(),
                wrapper: vec![
                    "python3".into(),
                    "-c".into(),
                    format!(
                    "import sys; sys.stdin.readline(); print({}); sys.stderr.write('diagnostic')",
                    serde_json::to_string(output).unwrap()
                )
                    .into(),
                ],
                model: None,
            };
            let mut run = CodexAdapter.start_run(&spec).await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                assert!(matches!(run.events.recv().await, Some(NormalizedEvent::Failed { reason }) if reason.contains("diagnostic")));
                assert_eq!(run.events.recv().await, Some(NormalizedEvent::Ended));
                assert_eq!(run.events.recv().await, None);
            }).await.unwrap();
        }
    }
}
