use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;
use loopfleet_core::{NormalizedEvent, Usage};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::{AdapterError, AgentAdapter, RunHandle, RunSpec, SessionHandle, SessionSeed};
use loopfleet_core::adapter::SteerRequest;

const EXCERPT_LIMIT: usize = 2000;

pub struct CodexAdapter;

#[async_trait]
impl AgentAdapter for CodexAdapter {
    fn can_steer(&self) -> bool {
        crate::discovery::can_steer("codex")
    }

    async fn start_run(&self, spec: &RunSpec) -> Result<RunHandle, AdapterError> {
        let can_steer = self.can_steer();
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
        let (steer, requests) = mpsc::channel(64);
        tokio::spawn(drive(
            child,
            stdin,
            stdout,
            stderr,
            tx,
            spec.clone(),
            requests,
        ));
        Ok(RunHandle {
            events: rx,
            steer: can_steer.then_some(steer),
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
    mut requests: mpsc::Receiver<SteerRequest>,
) {
    let mut stderr = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut text = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            text = excerpt(&format!("{text}\n{line}"));
        }
        text
    });
    let result = tokio::select! {
        biased;
        _ = tx.closed() => Ok(()),
        result = async {
        send(&mut stdin, json!({"id":1,"method":"initialize","params":{
            "clientInfo":{"name":"loopfleet","title":"Loopfleet","version":env!("CARGO_PKG_VERSION")}
        }})).await?;
        let mut lines = BufReader::new(stdout).lines();
        let mut mapper = CodexMapper::default();
        let mut expected = 1;
        let mut thread_id = String::new();
        let mut turn_id = None::<String>;
        let mut pending = HashMap::new();
        loop {
            let line = tokio::select! {
                _ = tx.closed() => return Ok(()),
                Some(request) = requests.recv() => {
                    let Some(turn) = &turn_id else {
                        let _ = request.ack.send(Err(AdapterError::Protocol("no active Codex turn".into())));
                        continue;
                    };
                    let id = format!("steer:{}", request.tap_id);
                    send(&mut stdin, json!({"id":id,"method":"turn/steer","params":{
                        "threadId":thread_id,"expectedTurnId":turn,"input":[{"type":"text","text":request.text}]
                    }})).await?;
                    pending.insert(id, (turn.clone(), request));
                    continue;
                },
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
                if let Some((turn, request)) = value["id"].as_str().and_then(|id| pending.remove(id)) {
                    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
                        let _ = request.ack.send(Err(AdapterError::Protocol(error_message(error).into())));
                    } else if let Some(accepted) = value["result"]["turnId"].as_str() {
                        let _ = request.ack.send(if accepted == turn { Ok(()) } else {
                            Err(AdapterError::Protocol("stale Codex turn".into()))
                        });
                    }
                    continue;
                }
                if value["id"].as_u64() != Some(expected) || expected > 3 {
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
                        thread_id = thread.to_owned();
                        send(&mut stdin, json!({"id":3,"method":"turn/start","params":{
                            "threadId":thread,"input":[{"type":"text","text":spec.prompt}]
                        }})).await?;
                    }
                    3 => {
                        turn_id = Some(value["result"]["turn"]["id"].as_str()
                            .ok_or_else(|| AdapterError::Protocol("missing turn id".into()))?.to_owned());
                    }
                    _ => {}
                }
                expected += 1;
                continue;
            }
            if value["params"]["threadId"].as_str().is_some_and(|id| id != thread_id) {
                continue;
            }
            if value["method"] == "turn/started" {
                if let Some(id) = value["params"]["turn"]["id"].as_str() {
                    turn_id = Some(id.to_owned());
                }
            }
            if value["params"]["turnId"].as_str().is_some_and(|id| turn_id.as_deref() != Some(id))
                || (value["method"] == "turn/completed"
                    && value["params"]["turn"]["id"].as_str().is_some_and(|id| turn_id.as_deref() != Some(id))) {
                continue;
            }
            for event in mapper.map_line(&line)? {
                let ended = matches!(event, NormalizedEvent::Ended);
                if tx.send(event).await.is_err() || ended {
                    return Ok(());
                }
            }
        }
        } => result,
    };
    requests.close();
    while let Ok(request) = requests.try_recv() {
        let _ = request
            .ack
            .send(Err(AdapterError::Protocol("no active Codex turn".into())));
    }
    drop(stdin);
    crate::codex::shutdown(&mut child, &tx).await;
    let stderr = match tokio::select! {
        biased;
        _ = tx.closed() => None,
        result = tokio::time::timeout(std::time::Duration::from_secs(2), &mut stderr) => Some(result),
    } {
        Some(Ok(result)) => result.unwrap_or_default(),
        _ => {
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

pub(crate) async fn shutdown(
    child: &mut tokio::process::Child,
    tx: &mpsc::Sender<NormalizedEvent>,
) {
    let pid = child.id();
    let exited = tokio::select! {
        biased;
        _ = tx.closed() => false,
        result = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()) => matches!(result, Ok(Ok(_))),
    };
    #[cfg(unix)]
    if let Some(pid) = pid {
        unsafe {
            libc::killpg(pid as libc::pid_t, libc::SIGTERM);
        }
    }
    if !exited {
        let _ = tokio::time::timeout(std::time::Duration::from_millis(200), child.wait()).await;
    }
    #[cfg(unix)]
    if let Some(pid) = pid {
        unsafe {
            libc::killpg(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    let _ = child.start_kill();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
}

#[derive(Default)]
struct CodexMapper {
    usage: Usage,
    failed: bool,
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
                Some("completed") if !self.failed => Ok(vec![
                    NormalizedEvent::TurnCompleted {
                        usage: self.usage.clone(),
                    },
                    NormalizedEvent::Ended,
                ]),
                Some("interrupted") => Ok(map_error("Codex turn interrupted")),
                _ if self.failed => Ok(vec![NormalizedEvent::Ended]),
                _ => Ok(map_error(error_message(&params["turn"]))),
            },
            Some("error") if params["willRetry"].as_bool() != Some(true) => {
                self.failed = true;
                let mut events = map_error(error_message(params));
                events.pop();
                Ok(events)
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
                    .filter(|result| !result.is_null())
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

    #[cfg(unix)]
    #[tokio::test]
    async fn both_transports_stop_and_reap_at_every_wait() {
        for adapter in [&CodexAdapter as &dyn AgentAdapter, &crate::PiAdapter] {
            for phase in [
                "startup",
                "prompt",
                "steer",
                "ack",
                "backpressure",
                "shutdown",
                "untapped",
                "bounded",
            ] {
                let dir = tempfile::tempdir().unwrap();
                let script = dir.path().join("shutdown.py");
                std::fs::write(&script, r#"
import json, os, signal, sys, time
phase = sys.argv[1]
signal.signal(signal.SIGTERM, signal.SIG_IGN)
with open('pid', 'w') as f:
    f.write(str(os.getpid()))
def mark():
    with open('ready', 'w') as f:
        f.write('ready')
def stall():
    mark()
    time.sleep(60)
def read():
    return json.loads(sys.stdin.readline())
def emit(v):
    print(json.dumps(v), flush=True)
if phase == 'startup':
    stall()
codex = sys.argv[2] == 'codex'
if codex:
    assert read()['method'] == 'initialize'
    emit(dict(id=1, result={}))
    assert read()['method'] == 'initialized'
    assert read()['method'] == 'thread/start'
    emit(dict(id=2, result=dict(thread=dict(id='thread'))))
else:
    mode = read()
    emit(dict(type='response', id='0', command='set_steering_mode', success=True))
if phase == 'prompt':
    stall()
read()
if codex:
    emit(dict(id=3, result=dict(turn=dict(id='turn'))))
    emit(dict(method='turn/started', params={}))
else:
    emit(dict(type='response', id='1', command='prompt', success=True, data=dict(disposition='started')))
    emit(dict(type='turn_start'))
if phase == 'steer':
    stall()
if phase == 'ack':
    read()
    stall()
if phase == 'backpressure':
    for _ in range(100):
        if codex:
            emit(dict(method='item/completed', params=dict(item=dict(type='agentMessage', text='text'))))
        else:
            emit(dict(type='message_end', message=dict(role='assistant', content=[dict(type='text', text='text')])) )
    stall()
if phase == 'untapped':
    descendant = os.fork()
    if descendant == 0:
        time.sleep(60)
        os._exit(0)
    with open('descendant', 'w') as f:
        f.write(str(descendant))
if codex:
    emit(dict(method='turn/completed', params=dict(turn=dict(status='completed'))))
else:
    emit(dict(type='agent_settled'))
mark()
sys.stdin.read()
with open('eof', 'w') as f:
    f.write('closed')
if phase in ['shutdown', 'bounded']:
    time.sleep(60)
"#).unwrap();
                let mut run = adapter
                    .start_run(&RunSpec {
                        cwd: dir.path().into(),
                        prompt: if phase == "prompt" {
                            "x".repeat(2_000_000)
                        } else {
                            "initial".into()
                        },
                        wrapper: vec!["python3".into(), script.into_os_string(), phase.into()],
                        model: None,
                    })
                    .await
                    .unwrap();
                let mut ack = None;
                if matches!(phase, "steer" | "ack") {
                    assert_eq!(
                        tokio::time::timeout(std::time::Duration::from_secs(5), run.events.recv())
                            .await
                            .unwrap(),
                        Some(NormalizedEvent::TurnStarted)
                    );
                    let (sender, receiver) = tokio::sync::oneshot::channel();
                    run.steer
                        .as_ref()
                        .unwrap()
                        .send(SteerRequest {
                            tap_id: "tap".into(),
                            text: if phase == "steer" {
                                "x".repeat(2_000_000)
                            } else {
                                "steer".into()
                            },
                            ack: sender,
                        })
                        .await
                        .unwrap();
                    ack = Some(receiver);
                }
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    while !dir.path().join("ready").exists() {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
                if matches!(phase, "untapped" | "bounded") {
                    tokio::time::timeout(std::time::Duration::from_secs(5), async {
                        while run.events.recv().await.is_some() {}
                    })
                    .await
                    .unwrap();
                    assert!(dir.path().join("eof").exists());
                } else {
                    if phase == "shutdown" {
                        while run.events.recv().await != Some(NormalizedEvent::Ended) {}
                    }
                    drop(run);
                }
                if let Some(ack) = ack {
                    assert!(!matches!(
                        tokio::time::timeout(std::time::Duration::from_secs(1), ack)
                            .await
                            .unwrap(),
                        Ok(Ok(()))
                    ));
                }
                let pid: i32 = std::fs::read_to_string(dir.path().join("pid"))
                    .unwrap()
                    .parse()
                    .unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    while unsafe { libc::kill(pid, 0) } == 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
                if phase == "untapped" {
                    let pid: i32 = std::fs::read_to_string(dir.path().join("descendant"))
                        .unwrap()
                        .parse()
                        .unwrap();
                    tokio::time::timeout(std::time::Duration::from_secs(1), async {
                        while unsafe { libc::kill(pid, 0) } == 0 {
                            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        }
                    })
                    .await
                    .unwrap();
                }
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn scripted_handshake_and_steering_outcomes() {
        for (codex, adapter) in [
            (true, &CodexAdapter as &dyn AgentAdapter),
            (false, &crate::PiAdapter),
        ] {
            for scenario in [
                "accepted",
                "rejected",
                "stale",
                "unknown",
                "completion-first",
                "exit",
            ] {
                let dir = tempfile::tempdir().unwrap();
                let script = dir.path().join("harness.py");
                std::fs::write(&script, include_str!("../fixtures/harness-v1.py")).unwrap();
                let mut run = adapter
                    .start_run(&RunSpec {
                        cwd: dir.path().into(),
                        prompt: "initial\nprompt".into(),
                        wrapper: vec!["python3".into(), script.into_os_string(), scenario.into()],
                        model: Some("test-model".into()),
                    })
                    .await
                    .unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    assert_eq!(run.events.recv().await, Some(NormalizedEvent::TurnStarted), "{codex} {scenario}");
                    let (ack, receiver) = tokio::sync::oneshot::channel();
                    run.steer.as_ref().unwrap().send(SteerRequest {
                        tap_id: "tap".into(), text: "steer".into(), ack,
                    }).await.unwrap();
                    let mut events = Vec::new();
                    while let Some(event) = run.events.recv().await {
                        events.push(event);
                    }
                    let accepted = matches!(receiver.await, Ok(Ok(())));
                    assert_eq!(accepted, scenario == "accepted" || (!codex && scenario == "completion-first"), "{codex} {scenario}");
                    assert_eq!(events.last(), Some(&NormalizedEvent::Ended));
                    assert_eq!(events.iter().filter(|event| matches!(event, NormalizedEvent::Ended)).count(), 1);
                    assert!(!events.iter().any(|event| matches!(event, NormalizedEvent::TurnStarted)));
                    if scenario == "exit" || (!codex && scenario == "unknown") {
                        assert!(events.iter().any(|event| matches!(event, NormalizedEvent::Failed { reason } if reason.contains("agent exited without"))));
                    } else {
                        assert!(!events.iter().any(|event| matches!(event, NormalizedEvent::Failed { .. })));
                    }
                    let text: Vec<_> = events.iter().filter_map(|event| match event {
                        NormalizedEvent::AssistantText { text } => Some(text.as_str()),
                        _ => None,
                    }).collect();
                    assert_eq!(text, if matches!(scenario, "exit" | "completion-first") { vec![] } else { vec!["answer"] });
                    let pid: i32 = std::fs::read_to_string(dir.path().join("pid")).unwrap().parse().unwrap();
                    assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
                    if scenario != "exit" && (scenario != "unknown" || codex) {
                        assert!(dir.path().join("eof").exists());
                    }
                }).await.unwrap();
            }
        }
    }

    #[test]
    fn maps_versioned_app_server_fixture() {
        assert_eq!(
            map_all(include_str!("../fixtures/codex-app-server-v1.jsonl")),
            vec![
                NormalizedEvent::TurnStarted,
                NormalizedEvent::AssistantText {
                    text: "answer".into()
                },
                NormalizedEvent::TurnCompleted {
                    usage: Usage {
                        input_tokens: 12,
                        output_tokens: 3
                    }
                },
                NormalizedEvent::Ended,
            ]
        );
    }

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
        assert!(matches!(
            limited.last(),
            Some(NormalizedEvent::Failed { .. })
        ));
    }

    #[test]
    fn maps_completed_items_and_tool_failures() {
        assert_eq!(
            map_completed_item(Some(
                &json!({"type":"mcpToolCall","id":"tool","status":"failed","result":null,"error":{"message":"tool failed"}})
            )),
            vec![NormalizedEvent::ToolResult {
                call_id: "tool".into(),
                ok: false,
                output_excerpt: r#"{"message":"tool failed"}"#.into()
            }]
        );
        assert_eq!(
            map_completed_item(Some(
                &json!({"type":"reasoning","summary":[],"content":["reasoning"]})
            )),
            vec![NormalizedEvent::Reasoning {
                text: "reasoning".into()
            }]
        );
        let search = json!({"type":"webSearch","id":"search","query":"query"});
        assert_eq!(
            map_started_item(Some(&search)),
            vec![NormalizedEvent::ToolCall {
                call_id: "search".into(),
                name: "web_search".into(),
                input_excerpt: "query".into()
            }]
        );
        assert_eq!(
            map_completed_item(Some(&search)),
            vec![NormalizedEvent::ToolResult {
                call_id: "search".into(),
                ok: true,
                output_excerpt: "query".into()
            }]
        );
    }

    #[test]
    fn waits_for_completion_and_preserves_terminal_status() {
        let mut mapper = CodexMapper::default();
        assert_eq!(mapper.map_line(r#"{"method":"error","params":{"error":{"message":"failed upstream"},"willRetry":false}}"#).unwrap(), vec![NormalizedEvent::Failed { reason: "failed upstream".into() }]);
        assert_eq!(
            mapper
                .map_line(r#"{"method":"turn/completed","params":{"turn":{"status":"completed"}}}"#)
                .unwrap(),
            vec![NormalizedEvent::Ended]
        );
        assert_eq!(
            map_all(r#"{"method":"turn/completed","params":{"turn":{"status":"interrupted"}}}"#),
            vec![
                NormalizedEvent::Failed {
                    reason: "Codex turn interrupted".into()
                },
                NormalizedEvent::Ended
            ]
        );
        assert!(map_all(
            r#"{"method":"error","params":{"error":{"message":"retrying"},"willRetry":true}}"#
        )
        .is_empty());
    }

    #[tokio::test]
    async fn correlates_steer_replies_and_requires_acknowledgement() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("steer.py");
        std::fs::write(&script, r#"
import json, os, sys, time
def read():
    return json.loads(sys.stdin.readline())
def emit(value):
    print(json.dumps(value), flush=True)
with open('pid', 'w') as file:
    file.write(str(os.getpid()))
read()
emit({'method': 'item/completed', 'params': {'item': {'type': 'agentMessage', 'text': 'initializing'}}})
while not os.path.exists('ready'):
    time.sleep(0.001)
emit({'id': 1, 'result': {}})
read()
read()
emit({'id': 2, 'result': {'thread': {'id': 'thread'}}})
read()
emit({'method': 'turn/started', 'params': {'threadId': 'thread', 'turn': {'id': 'turn'}}})
requests = [read()]
emit({'id': 3, 'result': {'turn': {'id': 'turn'}}})
requests += [read() for _ in range(4)]
for index, request in enumerate(requests):
    assert request == {'id': 'steer:tap-' + str(index), 'method': 'turn/steer', 'params': {'threadId': 'thread', 'expectedTurnId': 'turn', 'input': [{'type': 'text', 'text': 'correction ' + str(index)}]}}
emit({'id': requests[2]['id'], 'result': {'turnId': 'stale'}})
emit({'id': requests[1]['id'], 'error': {'message': 'explicit rejection'}})
emit({'id': requests[0]['id'], 'result': {'turnId': 'turn'}})
emit({'id': requests[3]['id'], 'result': {}})
emit({'method': 'turn/completed', 'params': {'threadId': 'other', 'turn': {'id': 'turn', 'status': 'completed'}}})
emit({'method': 'turn/completed', 'params': {'threadId': 'thread', 'turn': {'id': 'stale', 'status': 'completed'}}})
emit({'method': 'error', 'params': {'threadId': 'thread', 'turnId': 'turn', 'error': {'message': 'turn failed'}, 'willRetry': False}})
emit({'method': 'item/completed', 'params': {'threadId': 'thread', 'turnId': 'turn', 'item': {'type': 'agentMessage', 'text': 'after error'}}})
emit({'method': 'turn/completed', 'params': {'threadId': 'thread', 'turn': {'id': 'turn', 'status': 'failed', 'error': {'message': 'turn failed'}}}})
sys.stdin.read()
"#).unwrap();
        let spec = RunSpec {
            cwd: dir.path().to_path_buf(),
            prompt: "prompt".into(),
            wrapper: vec!["python3".into(), script.into_os_string()],
            model: None,
        };
        assert!(CodexAdapter.can_steer());
        let mut run = CodexAdapter.start_run(&spec).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            assert_eq!(
                run.events.recv().await,
                Some(NormalizedEvent::AssistantText {
                    text: "initializing".into()
                })
            );
            let (ack, receiver) = tokio::sync::oneshot::channel();
            run.steer
                .as_ref()
                .unwrap()
                .send(SteerRequest {
                    tap_id: "early".into(),
                    text: "early correction".into(),
                    ack,
                })
                .await
                .unwrap();
            assert!(receiver.await.unwrap().is_err());
            std::fs::write(dir.path().join("ready"), "").unwrap();
            assert_eq!(run.events.recv().await, Some(NormalizedEvent::TurnStarted));
            let mut acknowledgements = Vec::new();
            for index in 0..5 {
                let (ack, receiver) = tokio::sync::oneshot::channel();
                run.steer
                    .as_ref()
                    .unwrap()
                    .send(SteerRequest {
                        tap_id: format!("tap-{index}"),
                        text: format!("correction {index}"),
                        ack,
                    })
                    .await
                    .unwrap();
                acknowledgements.push(receiver);
            }
            for (index, receiver) in acknowledgements.into_iter().enumerate() {
                match index {
                    0 => assert!(receiver.await.unwrap().is_ok()),
                    1 | 2 => assert!(receiver.await.unwrap().is_err()),
                    _ => assert!(receiver.await.is_err()),
                }
            }
            assert_eq!(
                run.events.recv().await,
                Some(NormalizedEvent::Failed {
                    reason: "turn failed".into()
                })
            );
            assert_eq!(
                run.events.recv().await,
                Some(NormalizedEvent::AssistantText {
                    text: "after error".into()
                })
            );
            assert_eq!(run.events.recv().await, Some(NormalizedEvent::Ended));
            assert_eq!(run.events.recv().await, None);
        })
        .await
        .unwrap();
        #[cfg(unix)]
        assert_eq!(
            unsafe {
                libc::kill(
                    std::fs::read_to_string(dir.path().join("pid"))
                        .unwrap()
                        .parse()
                        .unwrap(),
                    0,
                )
            },
            -1
        );
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
