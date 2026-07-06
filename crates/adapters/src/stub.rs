//! A stub adapter that replays a canned event log instead of spawning a real
//! agent. This is the fixture the UI work builds against before any real
//! adapter exists: deterministic, no process, no network — feed it an event
//! list (or a JSONL fixture) and it streams those events through the same
//! [`RunHandle`] path a real run uses.

use std::path::Path;

use async_trait::async_trait;
use loopfleet_core::NormalizedEvent;
use tokio::sync::mpsc;

use crate::{AdapterError, AgentAdapter, RunHandle, RunSpec, SessionHandle, SessionSeed};

/// Replays a fixed sequence of [`NormalizedEvent`]s. Each `start_run` yields a
/// fresh stream of the same events, in order.
pub struct StubAdapter {
    events: Vec<NormalizedEvent>,
}

impl StubAdapter {
    /// Build a stub from an in-memory event list.
    pub fn new(events: Vec<NormalizedEvent>) -> Self {
        Self { events }
    }

    /// Build a stub from a JSONL fixture: one [`NormalizedEvent`] per non-empty
    /// line (the same `{"kind":…}` shape the store persists). Blank lines are
    /// skipped so fixtures can use spacing for readability.
    pub fn from_jsonl(text: &str) -> Result<Self, serde_json::Error> {
        let events = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(serde_json::from_str)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::new(events))
    }
}

#[async_trait]
impl AgentAdapter for StubAdapter {
    async fn start_run(&self, _spec: &RunSpec) -> Result<RunHandle, AdapterError> {
        // Bounded channel: same backpressure contract as a real run — a slow
        // consumer stalls replay rather than buffering without limit.
        let (tx, rx) = mpsc::channel(64);
        let events = self.events.clone();
        tokio::spawn(async move {
            for ev in events {
                // Receiver dropped: consumer went away, stop replaying.
                if tx.send(ev).await.is_err() {
                    break;
                }
            }
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use loopfleet_core::Usage;
    use std::path::PathBuf;

    fn spec() -> RunSpec {
        RunSpec {
            cwd: PathBuf::from("/tmp/worktree"),
            prompt: "do the task".into(),
        }
    }

    async fn drain(mut handle: RunHandle) -> Vec<NormalizedEvent> {
        let mut out = Vec::new();
        while let Some(ev) = handle.events.recv().await {
            out.push(ev);
        }
        out
    }

    /// The stub replays exactly the events it was given, in order, then closes.
    #[tokio::test]
    async fn replays_events_in_order() {
        let events = vec![
            NormalizedEvent::TurnStarted,
            NormalizedEvent::AssistantText { text: "hi".into() },
            NormalizedEvent::TurnCompleted {
                usage: Usage::default(),
            },
            NormalizedEvent::Ended,
        ];
        let adapter = StubAdapter::new(events.clone());
        let handle = adapter.start_run(&spec()).await.unwrap();
        assert_eq!(drain(handle).await, events);
    }

    /// Each run gets its own fresh stream of the same events.
    #[tokio::test]
    async fn each_run_is_independent() {
        let adapter = StubAdapter::new(vec![NormalizedEvent::TurnStarted, NormalizedEvent::Ended]);
        let first = drain(adapter.start_run(&spec()).await.unwrap()).await;
        let second = drain(adapter.start_run(&spec()).await.unwrap()).await;
        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
    }

    /// A JSONL fixture parses into the same events and replays through the run.
    #[tokio::test]
    async fn loads_and_replays_jsonl_fixture() {
        let fixture = include_str!("../fixtures/basic_run.jsonl");
        let adapter = StubAdapter::from_jsonl(fixture).unwrap();
        let events = drain(adapter.start_run(&spec()).await.unwrap()).await;

        assert_eq!(events.first(), Some(&NormalizedEvent::TurnStarted));
        assert_eq!(events.last(), Some(&NormalizedEvent::Ended));
        // ToolCall / ToolResult in the fixture correlate by call_id.
        let call_id = events.iter().find_map(|e| match e {
            NormalizedEvent::ToolCall { call_id, .. } => Some(call_id.clone()),
            _ => None,
        });
        let result_id = events.iter().find_map(|e| match e {
            NormalizedEvent::ToolResult { call_id, .. } => Some(call_id.clone()),
            _ => None,
        });
        assert!(call_id.is_some());
        assert_eq!(call_id, result_id);
    }

    /// Blank lines in a fixture are ignored; malformed JSON is a hard error.
    #[tokio::test]
    async fn blank_lines_skipped_bad_json_errors() {
        let ok = "{\"kind\":\"turn_started\"}\n\n{\"kind\":\"ended\"}\n";
        assert_eq!(StubAdapter::from_jsonl(ok).unwrap().events.len(), 2);
        assert!(StubAdapter::from_jsonl("{not json}").is_err());
    }

    /// v1 adapters do not implement interactive sessions.
    #[tokio::test]
    async fn open_session_is_unsupported() {
        let adapter = StubAdapter::new(vec![]);
        let seed = SessionSeed {
            plan_file: PathBuf::from("PRD.md"),
        };
        let err = adapter
            .open_session(Path::new("/tmp/repo"), seed)
            .await
            .unwrap_err();
        assert!(matches!(err, AdapterError::SessionsUnsupported));
    }
}
