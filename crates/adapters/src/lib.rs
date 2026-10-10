//! Loopfleet adapters: the `AgentAdapter` trait and per-agent implementations
//! (Claude Code, Codex, pi, cursor-agent) that normalize each agent's stream into the
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

use tokio::process::{Child, Command};

mod claude;
mod codex;
mod cursor;
mod discovery;
mod pi;
mod stub;
pub use claude::ClaudeAdapter;
pub use codex::CodexAdapter;
pub use cursor::CursorAdapter;
pub use discovery::{discover, discover_all, spec_for, AgentSpec, AgentStatus, KNOWN_AGENTS};
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
///
/// The Seatbelt wrapper (`sandbox-exec`) execs the agent and passes stdin
/// through, including the piped stdin used by Pi and Codex for steering JSON.
/// Supervision uses this inherited channel without weakening the boundary:
/// the rendered Seatbelt profile still confines the agent and its descendants.
///
/// The child is put in its own process group (`process_group(0)`, so its pgid
/// equals its pid) so a stop can SIGTERM the whole group via [`stop_agent`],
/// catching shells and tools the agent forked — the PRD's stop semantics.
pub(crate) fn base_command(wrapper: &[OsString], program: &str) -> Command {
    let mut cmd = match wrapper.split_first() {
        Some((launcher, prefix_args)) => {
            let mut cmd = Command::new(launcher);
            cmd.args(prefix_args).arg(program);
            cmd
        }
        None => Command::new(program),
    };
    if !wrapper.is_empty() {
        if let Some(home) = std::env::var_os("HOME") {
            cmd.env(
                "CARGO_HOME",
                std::path::PathBuf::from(home).join(".cache/loopfleet/cargo"),
            );
        }
    }
    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

/// Stop a running agent when the consumer drops its [`RunHandle`]. SIGTERMs the
/// child's process group (per [`base_command`], the child leads its own group,
/// so its forked descendants get the signal too), matching the PRD's stop
/// semantics ("SIGTERM that group"). Robust reaping of an agent that ignores
/// SIGTERM is left to M6 (orphan reaping).
pub(crate) fn stop_agent(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // SAFETY: killpg is a thin libc syscall wrapper with no memory effects.
        unsafe {
            libc::killpg(pid as libc::pid_t, libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    let _ = child.start_kill();
}

#[cfg(test)]
mod command_tests {
    use super::*;

    #[test]
    fn sandboxed_agents_receive_writable_cargo_home() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let cmd = base_command(&[OsString::from("sandbox-exec")], "pi");
        let expected = std::path::PathBuf::from(home).join(".cache/loopfleet/cargo");
        assert!(cmd.as_std().get_envs().any(|(key, value)| {
            key == "CARGO_HOME" && value == Some(expected.as_os_str())
        }));
    }

    #[test]
    fn unsandboxed_agents_preserve_inherited_cargo_home() {
        let cmd = base_command(&[], "pi");
        assert!(!cmd.as_std().get_envs().any(|(key, _)| key == "CARGO_HOME"));
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
