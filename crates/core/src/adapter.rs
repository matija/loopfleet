//! The `AgentAdapter` trait and its associated types.
//!
//! Per the PRD architecture, both the supervisor/run-loop and the adapter trait
//! live in `core`; the per-agent implementations (Claude Code, pi, cursor-agent)
//! live in the `loopfleet-adapters` crate and `impl` this trait. Keeping the
//! trait here lets the run loop (also in `core`) compose over a
//! `&dyn AgentAdapter` without `core` depending on `adapters` (which would be a
//! cycle, since `adapters` depends on `core` for [`NormalizedEvent`]).
//!
//! An adapter's only job is to turn one agent's native transport into the
//! [`NormalizedEvent`] vocabulary. Everything downstream consumes only that
//! enum and never learns which agent produced it.
//!
//! v1 is headless-only: [`AgentAdapter::start_run`] is real; `open_session`
//! stays in the signature (per the PRD's frozen trait) but every v1 adapter
//! returns [`AdapterError::SessionsUnsupported`] — interactive sessions land in
//! M5.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::{NormalizedEvent, UsageSnapshot};

/// Everything an adapter needs to launch a headless run. Grows as real adapters
/// land (per-agent flag sets, model selection); v1 carries only what the stub
/// and the supervisor need.
#[derive(Debug, Clone)]
pub struct RunSpec {
    /// Working directory for the agent process — the per-run worktree.
    pub cwd: PathBuf,
    /// The seeded prompt: the bound task plus the progress-file instructions the
    /// supervisor injects.
    pub prompt: String,
    /// An opaque argv prefix the adapter prepends to its own `program args…`,
    /// spawning `wrapper[0] wrapper[1..] program args…` instead. The wiring layer
    /// fills this with the Seatbelt sandbox invocation (`sandbox-exec -f
    /// <profile>`) so every pass runs confined; empty means spawn the agent
    /// directly (unsandboxed dev/test runs). The adapter treats the tokens as
    /// opaque — it never learns the backend is Seatbelt, keeping the `Sandbox`
    /// details from leaking into adapters (PRD: Sandbox).
    pub wrapper: Vec<OsString>,
    /// Model override to run this agent with (e.g. Claude's "opus", "sonnet",
    /// or a pinned version string like "claude-opus-4-1-20250805"). `None`
    /// means the agent CLI's own default. Adapters that have no notion of
    /// model selection ignore this.
    pub model: Option<String>,
}

/// Seed context for an interactive plan-editing session (M5). Present so the
/// trait signature is frozen now; no v1 adapter consumes it.
#[derive(Debug, Clone)]
pub struct SessionSeed {
    /// The plan file the session is rooted on.
    pub plan_file: PathBuf,
}

/// A live headless run. Consumers receive [`NormalizedEvent`]s in order until
/// the channel closes; a well-behaved stream is terminated by `Ended` or
/// `Failed`. The bounded channel is the backpressure — a slow consumer stalls
/// the producer rather than growing an unbounded buffer.
///
/// Process-group ownership and stop/SIGTERM handling belong to the supervisor,
/// not the handle.
#[derive(Debug)]
pub struct RunHandle {
    pub events: mpsc::Receiver<NormalizedEvent>,
}

/// A live interactive session (M5). Mirrors [`RunHandle`]; unused in v1.
#[derive(Debug)]
pub struct SessionHandle {
    pub events: mpsc::Receiver<NormalizedEvent>,
}

/// Why an adapter operation failed.
#[derive(Debug)]
pub enum AdapterError {
    /// Failed to spawn or drive the agent process / transport.
    Spawn(std::io::Error),
    /// The agent emitted output the adapter could not map to the enum.
    Protocol(String),
    /// Interactive sessions are not implemented in v1 (M5).
    SessionsUnsupported,
    /// This agent has no way to report how much of its limit window is left,
    /// so [`AgentAdapter::usage_snapshot`] has nothing to answer with.
    UsageUnsupported,
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdapterError::Spawn(e) => write!(f, "failed to spawn agent: {e}"),
            AdapterError::Protocol(m) => write!(f, "agent protocol error: {m}"),
            AdapterError::SessionsUnsupported => {
                write!(f, "interactive sessions are not supported in v1")
            }
            AdapterError::UsageUnsupported => {
                write!(f, "this agent does not report usage/limit headroom")
            }
        }
    }
}

impl std::error::Error for AdapterError {}

/// Normalizes one agent's transport into [`NormalizedEvent`]s. Object-safe (via
/// `async_trait`) so the supervisor can hold a `Box<dyn AgentAdapter>` chosen by
/// agent name at run time.
#[async_trait]
pub trait AgentAdapter: Send + Sync {
    /// Launch a headless run and return a handle streaming its normalized events.
    async fn start_run(&self, spec: &RunSpec) -> Result<RunHandle, AdapterError>;

    /// Open an interactive session (M5). v1 adapters return
    /// [`AdapterError::SessionsUnsupported`].
    async fn open_session(
        &self,
        cwd: &Path,
        seed: SessionSeed,
    ) -> Result<SessionHandle, AdapterError>;

    /// Report how much of the agent's limit window is spent right now, as a
    /// normalized [`UsageSnapshot`] stamped `now_ms` (epoch millis, supplied by
    /// the caller so this stays clock-free and testable, like `usage`'s
    /// functions).
    ///
    /// This is an *optional capability*, not part of the run path: an agent
    /// that can be asked for its headroom out of band answers here, and
    /// scheduling can read it before committing a run. Agents that only ever
    /// mention limits mid-stream keep reaching the same state the other way —
    /// [`NormalizedEvent::RateLimited`] folded through
    /// [`fold_rate_limit`](crate::fold_rate_limit) — so nothing is lost by
    /// leaving this unimplemented.
    ///
    /// The default returns [`AdapterError::UsageUnsupported`], which is the
    /// honest answer for every agent whose CLI has no headless usage query
    /// (all three v1 agents: `claude`, `pi`, `cursor-agent`). Callers must
    /// treat that as "ask again some other way", not as "plenty left" — it is
    /// distinct from a successful [`UsageSnapshot::unknown`], which says the
    /// agent *can* report but has said nothing yet.
    async fn usage_snapshot(&self, now_ms: i64) -> Result<UsageSnapshot, AdapterError> {
        let _ = now_ms;
        Err(AdapterError::UsageUnsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An adapter that implements only the required methods — the shape every
    /// v1 adapter has.
    struct MinimalAdapter;

    #[async_trait]
    impl AgentAdapter for MinimalAdapter {
        async fn start_run(&self, _spec: &RunSpec) -> Result<RunHandle, AdapterError> {
            let (_tx, rx) = mpsc::channel(1);
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

    /// Not implementing the capability compiles, and answers "unsupported" —
    /// never a zero-used snapshot, which would read as headroom.
    #[tokio::test]
    async fn default_usage_snapshot_is_unsupported() {
        let err = MinimalAdapter.usage_snapshot(0).await.unwrap_err();
        assert!(matches!(err, AdapterError::UsageUnsupported));
        assert_eq!(
            err.to_string(),
            "this agent does not report usage/limit headroom"
        );
    }

    /// The capability must not have broken object safety: the supervisor holds
    /// adapters as `Box<dyn AgentAdapter>`.
    #[tokio::test]
    async fn default_is_reachable_through_dyn_adapter() {
        let adapter: Box<dyn AgentAdapter> = Box::new(MinimalAdapter);
        assert!(matches!(
            adapter.usage_snapshot(0).await.unwrap_err(),
            AdapterError::UsageUnsupported
        ));
    }
}
