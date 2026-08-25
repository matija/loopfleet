//! A stub adapter that replays a canned event log instead of spawning a real
//! agent. This is the fixture the UI work builds against before any real
//! adapter exists: deterministic, no process, no network — feed it an event
//! list (or a JSONL fixture) and it streams those events through the same
//! [`RunHandle`] path a real run uses.

use std::path::Path;

use async_trait::async_trait;
use loopfleet_core::{NormalizedEvent, UsageSnapshot};
use tokio::sync::mpsc;

use crate::{AdapterError, AgentAdapter, RunHandle, RunSpec, SessionHandle, SessionSeed};

/// Replays a fixed sequence of [`NormalizedEvent`]s. Each `start_run` yields a
/// fresh stream of the same events, in order.
pub struct StubAdapter {
    events: Vec<NormalizedEvent>,
    /// Headroom this stub reports from [`AgentAdapter::usage_snapshot`], when
    /// configured. `None` (the default) makes the stub behave like the real v1
    /// adapters: usage reporting is unsupported.
    usage: Option<UsageSnapshot>,
}

impl StubAdapter {
    /// Build a stub from an in-memory event list.
    pub fn new(events: Vec<NormalizedEvent>) -> Self {
        Self {
            events,
            usage: None,
        }
    }

    /// Builder-style: make this stub an adapter that *can* report headroom,
    /// answering [`AgentAdapter::usage_snapshot`] with `usage`. This is the
    /// fixture side of the optional capability — the UI and any scheduling that
    /// reads headroom can be driven end to end without a real agent, since no
    /// v1 agent CLI exposes a headless usage query.
    ///
    /// The snapshot is re-stamped with the caller's `now_ms` on each read, so a
    /// fixture never has to be rebuilt to stay fresh; set `reset_at_ms` if you
    /// want it to age out (see [`loopfleet_core::is_stale`]).
    pub fn with_usage(mut self, usage: UsageSnapshot) -> Self {
        self.usage = Some(usage);
        self
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

    /// Reports the configured snapshot, stamped `now_ms`. An unconfigured stub
    /// falls through to the trait default (`UsageUnsupported`).
    async fn usage_snapshot(&self, now_ms: i64) -> Result<UsageSnapshot, AdapterError> {
        match &self.usage {
            Some(usage) => Ok(UsageSnapshot {
                observed_at_ms: now_ms,
                ..usage.clone()
            }),
            None => Err(AdapterError::UsageUnsupported),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loopfleet_core::usage::DEFAULT_STALE_AFTER_MS;
    use loopfleet_core::{is_stale, Usage, UsageSource};
    use std::path::PathBuf;

    /// A fixed "now" (epoch millis) for the usage tests.
    const NOW: i64 = 1_760_000_000_000;

    fn spec() -> RunSpec {
        RunSpec {
            cwd: PathBuf::from("/tmp/worktree"),
            prompt: "do the task".into(),
            wrapper: Vec::new(),
            model: None,
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

    /// The capability is opt-in: a stub built the plain way answers exactly as
    /// the trait default does, so the real adapters' behavior is what tests
    /// against an unconfigured stub see.
    #[tokio::test]
    async fn usage_is_unsupported_unless_configured() {
        let adapter = StubAdapter::new(vec![]);
        let err = adapter.usage_snapshot(NOW).await.unwrap_err();
        assert!(matches!(err, AdapterError::UsageUnsupported));
    }

    /// A configured stub reports the snapshot it was given, re-stamped to the
    /// caller's clock so it reads as freshly observed.
    #[tokio::test]
    async fn configured_usage_is_reported_stamped_now() {
        let adapter = StubAdapter::new(vec![])
            .with_usage(UsageSnapshot::reported("claude", 0.75, 0).with_limit_window("weekly"));

        let snap = adapter.usage_snapshot(NOW).await.unwrap();
        assert_eq!(snap.agent_key, "claude");
        assert_eq!(snap.used_fraction, 0.75);
        assert_eq!(snap.limit_window.as_deref(), Some("weekly"));
        assert_eq!(snap.observed_at_ms, NOW);
        assert_eq!(snap.source, UsageSource::Reported);
        assert!(!is_stale(&snap, NOW, DEFAULT_STALE_AFTER_MS));
    }

    /// Reading through `dyn AgentAdapter` — how the supervisor holds an adapter
    /// — still dispatches to the impl, so the capability survives boxing.
    #[tokio::test]
    async fn capability_dispatches_through_dyn_adapter() {
        let configured: Box<dyn AgentAdapter> =
            Box::new(StubAdapter::new(vec![]).with_usage(UsageSnapshot::unknown("claude", 0)));
        let snap = configured.usage_snapshot(NOW).await.unwrap();
        // `unknown` is a successful answer meaning "nothing said yet" — not the
        // same as the adapter having no way to answer at all.
        assert_eq!(snap.source, UsageSource::Unknown);

        let plain: Box<dyn AgentAdapter> = Box::new(StubAdapter::new(vec![]));
        assert!(matches!(
            plain.usage_snapshot(NOW).await.unwrap_err(),
            AdapterError::UsageUnsupported
        ));
    }
}
