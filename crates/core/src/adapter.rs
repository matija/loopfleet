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

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::NormalizedEvent;

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
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdapterError::Spawn(e) => write!(f, "failed to spawn agent: {e}"),
            AdapterError::Protocol(m) => write!(f, "agent protocol error: {m}"),
            AdapterError::SessionsUnsupported => {
                write!(f, "interactive sessions are not supported in v1")
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
}
