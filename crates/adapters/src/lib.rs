//! loopfleet adapters: the `AgentAdapter` trait and per-agent implementations
//! (Claude Code, pi, cursor-agent) that normalize each agent's stream into the
//! shared event enum. Implemented in M1.
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

use tokio::process::Command;

mod claude;
mod cursor;
mod pi;
mod stub;
pub use claude::ClaudeAdapter;
pub use cursor::CursorAdapter;
pub use pi::PiAdapter;
pub use stub::StubAdapter;

/// Build the base [`Command`] for spawning `program`, honoring the run's opaque
/// sandbox `wrapper` prefix ([`RunSpec::wrapper`]).
///
/// When `wrapper` is non-empty the process spawned is
/// `wrapper[0] wrapper[1..] program …`, so the agent runs confined; when it is
/// empty the agent is spawned directly (unsandboxed dev/test runs). Each adapter
/// appends its own flags to the returned command. The tokens are opaque here —
/// the adapter never learns the backend is Seatbelt, keeping `Sandbox` details
/// out of the adapters (PRD: Sandbox).
pub(crate) fn base_command(wrapper: &[OsString], program: &str) -> Command {
    match wrapper.split_first() {
        Some((launcher, prefix_args)) => {
            let mut cmd = Command::new(launcher);
            cmd.args(prefix_args).arg(program);
            cmd
        }
        None => Command::new(program),
    }
}

// The `AgentAdapter` trait and its launch/handle types live in `core` — next to
// the run loop that drives them — so `core` can compose over a `&dyn AgentAdapter`
// without depending on this crate (which depends on `core`, so the reverse would
// be a cycle). The concrete adapters above `impl` that trait; it is re-exported
// here so `crate::AgentAdapter` (and friends) resolve for them and for existing
// consumers of this crate.
pub use loopfleet_core::{
    AdapterError, AgentAdapter, RunHandle, RunSpec, SessionHandle, SessionSeed,
};
