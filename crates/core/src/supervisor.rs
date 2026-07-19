//! Supervisor foundations (M3): the run lifecycle state machine and per-run
//! process-group spawning with group-wide SIGTERM.
//!
//! These are the supervisor's building blocks. The driving loop (next M3 task)
//! composes them with an [`AgentAdapter`](../../loopfleet_adapters), the
//! `SeatbeltSandbox`, and the serialized [`GitActor`](loopfleet_gitx::GitActor)
//! (the single mutation point for worktree/shadow ops) to run N iterations.
//!
//! Two pieces live here:
//! - [`RunState`] — the `queued → running → (completed | failed | stopped)`
//!   machine the store persists as `runs.status` and the UI derives task state
//!   from. Transitions are validated so an illegal edge is a caught bug, not a
//!   silently corrupt row.
//! - [`RunProcess`] — spawns an agent in its **own process group** so a stop can
//!   SIGTERM the whole group, catching any child the agent forked (a shell it
//!   ran, a language server it started), not just the direct child. The PRD's
//!   stop semantics are "SIGTERM that group at the next iteration boundary".

use std::fmt;
use std::io;
use std::process::ExitStatus;

use tokio::process::{Child, Command};

/// The run lifecycle state machine: `queued → running → (completed | failed |
/// stopped | limit-reached)`. `completed` = the bound task's `STATUS: COMPLETE`
/// marker appeared within N iterations; `failed` = N reached still incomplete,
/// or a crash; `stopped` = the user stopped the run; `limit-reached` = the agent
/// hit a rate limit, so the run ended early to wait it out (the app schedules a
/// re-run once the limit resets). Acceptance is a separate flag, not a state
/// (see [`Run`](loopfleet_store)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Queued,
    Running,
    Completed,
    Failed,
    Stopped,
    LimitReached,
}

/// An attempt to move a run along an edge the machine does not allow.
#[derive(Debug, PartialEq, Eq)]
pub struct InvalidTransition {
    pub from: RunState,
    pub to: RunState,
}

impl fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid run state transition: {} -> {}",
            self.from.as_str(),
            self.to.as_str()
        )
    }
}

impl std::error::Error for InvalidTransition {}

impl RunState {
    /// The lowercase token persisted in `runs.status` and shown in the UI.
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Queued => "queued",
            RunState::Running => "running",
            RunState::Completed => "completed",
            RunState::Failed => "failed",
            RunState::Stopped => "stopped",
            RunState::LimitReached => "limit-reached",
        }
    }

    /// Parse a `runs.status` token back into a state. Returns `None` for any
    /// value not written by [`as_str`](RunState::as_str).
    pub fn from_token(s: &str) -> Option<RunState> {
        Some(match s {
            "queued" => RunState::Queued,
            "running" => RunState::Running,
            "completed" => RunState::Completed,
            "failed" => RunState::Failed,
            "stopped" => RunState::Stopped,
            "limit-reached" => RunState::LimitReached,
            _ => return None,
        })
    }

    /// A terminal state has no outgoing transitions; the run is over.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            RunState::Completed
                | RunState::Failed
                | RunState::Stopped
                | RunState::LimitReached
        )
    }

    /// Whether `self -> to` is a legal edge of the machine.
    pub fn can_transition(self, to: RunState) -> bool {
        matches!(
            (self, to),
            (RunState::Queued, RunState::Running)
                | (RunState::Running, RunState::Completed)
                | (RunState::Running, RunState::Failed)
                | (RunState::Running, RunState::Stopped)
                | (RunState::Running, RunState::LimitReached)
        )
    }

    /// Advance the machine, rejecting any edge the PRD's diagram does not draw.
    pub fn transition(self, to: RunState) -> Result<RunState, InvalidTransition> {
        if self.can_transition(to) {
            Ok(to)
        } else {
            Err(InvalidTransition { from: self, to })
        }
    }
}

/// An agent process running in its own process group.
///
/// Spawned with `process_group(0)`, so the child becomes the leader of a fresh
/// group whose id equals its pid. [`terminate`](RunProcess::terminate) SIGTERMs
/// that whole group, so descendants the agent forked die too — the reason the
/// group exists. Unix-only; v1 is macOS-only and the `Sandbox`/process layers
/// are the platform-specific seam (PRD).
pub struct RunProcess {
    child: Child,
    pgid: libc::pid_t,
}

impl RunProcess {
    /// Spawn `command` in a new process group. The caller configures program,
    /// args, cwd, and stdio; this only pins the group so the process is
    /// stoppable as a unit.
    pub fn spawn(mut command: Command) -> io::Result<Self> {
        // process_group(0): child leads a new group, pgid == child pid.
        command.process_group(0);
        let child = command.spawn()?;
        let pgid = child
            .id()
            .expect("child has a pid before it is awaited") as libc::pid_t;
        Ok(RunProcess { child, pgid })
    }

    /// The process-group id (equal to the leader's pid). Stable for the lifetime
    /// of the run.
    pub fn pgid(&self) -> libc::pid_t {
        self.pgid
    }

    /// SIGTERM the entire process group. Catches children the agent forked, not
    /// just the direct child. Idempotent from the caller's side: signalling a
    /// group whose members have all exited is a no-op error the caller may
    /// ignore.
    pub fn terminate(&self) -> io::Result<()> {
        // SAFETY: killpg is a thin libc syscall wrapper with no memory effects.
        let rc = unsafe { libc::killpg(self.pgid, libc::SIGTERM) };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Wait for the process to exit and reap it. The loop awaits this after an
    /// iteration's agent invocation ends (naturally or after `terminate`).
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn state_tokens_round_trip() {
        for s in [
            RunState::Queued,
            RunState::Running,
            RunState::Completed,
            RunState::Failed,
            RunState::Stopped,
            RunState::LimitReached,
        ] {
            assert_eq!(RunState::from_token(s.as_str()), Some(s));
        }
        assert_eq!(RunState::from_token("bogus"), None);
    }

    #[test]
    fn only_prd_edges_are_legal() {
        use RunState::*;
        // The exactly-legal set: queued->running, running->{completed,failed,stopped}.
        assert!(Queued.transition(Running).is_ok());
        assert!(Running.transition(Completed).is_ok());
        assert!(Running.transition(Failed).is_ok());
        assert!(Running.transition(Stopped).is_ok());
        assert!(Running.transition(LimitReached).is_ok());

        // A few edges that must be rejected: no skipping queued->completed, no
        // resurrecting a terminal state, no self-loops.
        assert!(Queued.transition(Completed).is_err());
        assert!(Completed.transition(Running).is_err());
        assert!(Stopped.transition(Running).is_err());
        assert!(Running.transition(Running).is_err());
    }

    #[test]
    fn terminal_states_have_no_exits() {
        assert!(!RunState::Queued.is_terminal());
        assert!(!RunState::Running.is_terminal());
        assert!(RunState::Completed.is_terminal());
        assert!(RunState::Failed.is_terminal());
        assert!(RunState::Stopped.is_terminal());
        assert!(RunState::LimitReached.is_terminal());
    }

    /// `kill(pid, 0)` probes existence without signalling: Ok(alive) via rc == 0.
    fn is_alive(pid: libc::pid_t) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// terminate() SIGTERMs the whole group: a grandchild the agent forked (here
    /// a `sleep` backgrounded by the shell) dies too, not just the direct child.
    #[tokio::test(flavor = "multi_thread")]
    async fn terminate_kills_the_whole_group() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("grandchild.pid");

        // The shell backgrounds a long sleep, records its pid, then waits on it.
        // sleep is a distinct process in the same group — the grandchild.
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(format!(
            "sleep 300 & echo $! > {} ; wait",
            pidfile.display()
        ));
        let mut proc = RunProcess::spawn(cmd).unwrap();

        // Wait for the shell to write the grandchild pid.
        let grandchild = loop {
            if let Ok(s) = std::fs::read_to_string(&pidfile) {
                if let Ok(pid) = s.trim().parse::<libc::pid_t>() {
                    break pid;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert!(is_alive(grandchild), "sleep should be running before terminate");

        proc.terminate().unwrap();
        // The group leader (the shell) exits from SIGTERM.
        proc.wait().await.unwrap();

        // The grandchild received the group signal and is gone. Poll briefly to
        // let signal delivery + reaping settle.
        let deadline = Instant::now() + Duration::from_secs(5);
        while is_alive(grandchild) {
            assert!(Instant::now() < deadline, "grandchild survived group SIGTERM");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
