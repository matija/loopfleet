//! The iteration loop (M3): drive an agent through N fresh passes against one
//! bound task, snapshotting the worktree after each pass, until the task is
//! marked complete, N is exhausted, or a hard failure occurs.
//!
//! This composes the M3 primitives — the [`AgentAdapter`](crate::adapter) trait,
//! the run-lifecycle [`RunState`](crate::RunState) machine, and the serialized
//! [`GitActor`](loopfleet_gitx::GitActor) (the single mutation point for
//! app-owned shadow commits) — into the ralph-style loop the PRD describes:
//! fresh context every pass, the external progress file as durable state, an
//! app-owned snapshot between passes.
//!
//! Each pass: seed the prompt with the bound task plus the prior progress-file
//! contents → `start_run` a fresh agent invocation → drain its normalized events
//! → snapshot the worktree to `refs/agentapp/run-<id>/iter-<n>` → check the
//! progress file for the `STATUS: COMPLETE` marker.
//!
//! Stop conditions (PRD run status machine):
//! - the bound task's `STATUS: COMPLETE` appears → [`RunState::Completed`];
//! - `max_iterations` reached still incomplete → [`RunState::Failed`];
//! - a hard failure (adapter cannot spawn, or a snapshot fails) →
//!   [`RunState::Failed`].
//!
//! A per-pass `Failed` *event* is deliberately NOT fatal: the ralph pattern is
//! resilient across passes, so a pass that ends without completing the task just
//! rolls into the next fresh pass; only exhausting N without completion, or an
//! inability to spawn/snapshot at all, fails the run.
//!
//! NB (deferred integration): today `start_run` owns the agent process and this
//! loop's stop is dropping the [`RunHandle`](crate::adapter::RunHandle) (which
//! every adapter honors by killing its child). Threading `SeatbeltSandbox`
//! command-wrapping and process-group SIGTERM *through* the adapter spawn — so a
//! forked descendant dies too and the Seatbelt profile is the boundary — is the
//! job of the end-to-end wiring (last M3 bullet), where a real sandboxed agent
//! is actually spawned. This module is transport- and sandbox-agnostic on
//! purpose.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use loopfleet_gitx::GitActor;

use crate::adapter::{AgentAdapter, RunSpec};
use crate::{NormalizedEvent, RunState};

/// Everything the loop needs to run one task to completion (or failure).
#[derive(Debug, Clone)]
pub struct LoopConfig {
    /// Stable run id; keys the worktree, branch, and shadow refs.
    pub run_id: String,
    /// The parent repository (where shadow refs and objects live).
    pub repo: PathBuf,
    /// The per-run worktree the agent runs in and the snapshotter captures.
    pub worktree: PathBuf,
    /// The external, app-managed progress file. Read for seed context and the
    /// completion marker; written by the agent (never by this loop).
    pub progress_path: PathBuf,
    /// The bound task's text, injected into every pass's prompt.
    pub task_text: String,
    /// Maximum passes before the run fails as incomplete.
    pub max_iterations: u32,
    /// The opaque sandbox wrapper prefix prepended to each pass's agent spawn
    /// (see [`RunSpec::wrapper`](crate::adapter::RunSpec::wrapper)). The wiring
    /// layer fills it with the Seatbelt invocation (`sandbox-exec -f <profile>`,
    /// via [`confine_prefix`](loopfleet_sandbox)); empty runs the agent directly.
    pub wrapper: Vec<OsString>,
}

/// One completed pass's app-owned snapshot. The caller persists these as
/// `Iteration` rows (`run_id`, `n`, `shadow_ref`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IterationRecord {
    pub n: u32,
    pub shadow_ref: String,
    pub commit: String,
}

/// The result of driving a run to a terminal state.
#[derive(Debug, Clone)]
pub struct LoopOutcome {
    /// Terminal state: [`Completed`](RunState::Completed) or
    /// [`Failed`](RunState::Failed).
    pub state: RunState,
    /// The snapshots taken, in pass order. Empty if the very first pass could
    /// not spawn.
    pub iterations: Vec<IterationRecord>,
}

/// Drive `adapter` through up to `cfg.max_iterations` passes against one task.
///
/// `on_event` receives every normalized event, tagged with its 1-based pass
/// number, as it arrives — the caller forwards these to the SQLite event log
/// (serialize + send through the bounded [`EventSender`](loopfleet_store)). It
/// is a plain callback so the loop stays decoupled from the store and unit
/// testable.
pub async fn run_loop(
    adapter: &dyn AgentAdapter,
    git: &GitActor,
    cfg: &LoopConfig,
    on_event: &mut dyn FnMut(u32, &NormalizedEvent),
) -> LoopOutcome {
    let mut iterations = Vec::new();

    for n in 1..=cfg.max_iterations {
        // Fresh context each pass: seed with the task and whatever the agent has
        // recorded in the external progress file so far.
        let prior = read_progress(&cfg.progress_path);
        let spec = RunSpec {
            cwd: cfg.worktree.clone(),
            prompt: build_prompt(cfg, &prior),
            wrapper: cfg.wrapper.clone(),
        };

        let mut handle = match adapter.start_run(&spec).await {
            Ok(h) => h,
            // Could not even spawn the agent: a hard failure (a crash).
            Err(_) => {
                return LoopOutcome {
                    state: RunState::Failed,
                    iterations,
                }
            }
        };

        // Drain the pass to completion. The stream ends on `Ended`/`Failed` or
        // when the adapter's child exits; dropping the handle here would stop it.
        while let Some(event) = handle.events.recv().await {
            on_event(n, &event);
        }

        // App-owned snapshot of this pass's worktree state (the agent never
        // commits). A snapshot failure is a hard failure.
        match git
            .snapshot(
                cfg.repo.clone(),
                cfg.worktree.clone(),
                cfg.run_id.clone(),
                n,
            )
            .await
        {
            Ok(snap) => iterations.push(IterationRecord {
                n,
                shadow_ref: snap.git_ref,
                commit: snap.commit,
            }),
            Err(_) => {
                return LoopOutcome {
                    state: RunState::Failed,
                    iterations,
                }
            }
        }

        // Done when the agent has written the completion marker for its task.
        if crate::progress::file_marks_complete(&cfg.progress_path) {
            return LoopOutcome {
                state: RunState::Completed,
                iterations,
            };
        }
    }

    // Exhausted all passes without a completion marker.
    LoopOutcome {
        state: RunState::Failed,
        iterations,
    }
}

/// Read the progress file, or `""` if it does not exist yet (first pass).
fn read_progress(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Assemble a pass's prompt: the bound task, the progress-file protocol, and the
/// prior progress as the durable memory the fresh context reads back.
fn build_prompt(cfg: &LoopConfig, prior: &str) -> String {
    let prior = if prior.trim().is_empty() {
        "(no prior progress yet)"
    } else {
        prior
    };
    format!(
        "Task:\n{task}\n\n\
You are running in a loop with fresh context each pass. Your durable memory is \
the progress file at:\n  {progress}\n\n\
Read your prior progress below, continue the work, and append what you did this \
pass to that file. When the task is fully done, write a line containing exactly \
`{marker}` to the progress file.\n\n\
--- prior progress ---\n{prior}\n",
        task = cfg.task_text,
        progress = cfg.progress_path.display(),
        marker = crate::progress::COMPLETION_MARKER,
        prior = prior,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{AdapterError, RunHandle, SessionHandle, SessionSeed};
    use async_trait::async_trait;
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    /// A scripted adapter: on each `start_run` it records the prompt, appends a
    /// line to the external progress file (optionally the completion marker on a
    /// chosen pass), touches a file in the worktree so snapshots have content,
    /// and replays a fixed event list.
    struct ScriptedAdapter {
        progress_path: PathBuf,
        /// 1-based pass on which to write `STATUS: COMPLETE`; `None` = never.
        complete_on: Option<u32>,
        /// If set, `start_run` fails instead of streaming — a spawn crash.
        fail_start: bool,
        call: AtomicU32,
        prompts: Arc<Mutex<Vec<String>>>,
        /// The `wrapper` prefix each pass's `RunSpec` carried — records that the
        /// loop threads `LoopConfig::wrapper` through to the adapter.
        wrappers: Arc<Mutex<Vec<Vec<OsString>>>>,
    }

    impl ScriptedAdapter {
        fn new(progress_path: PathBuf, complete_on: Option<u32>) -> Self {
            Self {
                progress_path,
                complete_on,
                fail_start: false,
                call: AtomicU32::new(0),
                prompts: Arc::new(Mutex::new(Vec::new())),
                wrappers: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl AgentAdapter for ScriptedAdapter {
        async fn start_run(&self, spec: &RunSpec) -> Result<RunHandle, AdapterError> {
            if self.fail_start {
                return Err(AdapterError::Spawn(std::io::Error::other("cannot spawn")));
            }
            let n = self.call.fetch_add(1, Ordering::SeqCst) + 1;
            self.prompts.lock().unwrap().push(spec.prompt.clone());
            self.wrappers.lock().unwrap().push(spec.wrapper.clone());

            // Append this pass's progress to the external file.
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.progress_path)
                .unwrap();
            writeln!(f, "pass {n} did work").unwrap();
            if self.complete_on == Some(n) {
                writeln!(f, "STATUS: COMPLETE").unwrap();
            }

            // Change the worktree so each snapshot has real content.
            std::fs::write(spec.cwd.join(format!("pass-{n}.txt")), "x\n").unwrap();

            // Replay a minimal but realistic event stream.
            let (tx, rx) = mpsc::channel(8);
            tokio::spawn(async move {
                for ev in [
                    NormalizedEvent::TurnStarted,
                    NormalizedEvent::AssistantText {
                        text: "working".into(),
                    },
                    NormalizedEvent::TurnCompleted {
                        usage: Default::default(),
                    },
                    NormalizedEvent::Ended,
                ] {
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

    /// A repo with one commit, so `worktree add -b` has a HEAD to branch from.
    fn repo_with_commit() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(p)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t.test"]);
        run(&["config", "user.name", "t"]);
        run(&["config", "commit.gpgsign", "false"]);
        std::fs::write(p.join("README.md"), "hi\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
        dir
    }

    /// Cut a worktree and build a `LoopConfig` pointing at an external progress
    /// file. Returns the config plus tempdirs to keep alive.
    async fn setup(
        run_id: &str,
        max_iterations: u32,
        git: &GitActor,
    ) -> (
        LoopConfig,
        tempfile::TempDir,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        let repo = repo_with_commit();
        let root = tempfile::tempdir().unwrap();
        let progress_dir = tempfile::tempdir().unwrap();
        let wt = git
            .worktree_add(repo.path().into(), root.path().into(), run_id.into())
            .await
            .unwrap();
        let cfg = LoopConfig {
            run_id: run_id.into(),
            repo: repo.path().into(),
            worktree: wt.path,
            progress_path: progress_dir.path().join("progress.md"),
            task_text: "Implement the widget".into(),
            max_iterations,
            wrapper: Vec::new(),
        };
        (cfg, repo, root, progress_dir)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn completes_when_marker_written() {
        let git = GitActor::spawn();
        let (cfg, _repo, _root, _prog) = setup("loop-complete", 3, &git).await;
        let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), Some(1));

        let mut seen = Vec::new();
        let outcome = run_loop(&adapter, &git, &cfg, &mut |n, ev| {
            seen.push((n, ev.clone()))
        })
        .await;

        assert_eq!(outcome.state, RunState::Completed);
        assert_eq!(outcome.iterations.len(), 1);
        assert_eq!(
            outcome.iterations[0].shadow_ref,
            "refs/agentapp/run-loop-complete/iter-1"
        );
        // Every event from the one pass was forwarded, tagged pass 1.
        assert_eq!(seen.len(), 4);
        assert!(seen.iter().all(|(n, _)| *n == 1));
        assert!(matches!(seen[0].1, NormalizedEvent::TurnStarted));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fails_after_exhausting_iterations() {
        let git = GitActor::spawn();
        let (cfg, _repo, _root, _prog) = setup("loop-exhaust", 3, &git).await;
        let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), None);

        let outcome = run_loop(&adapter, &git, &cfg, &mut |_, _| {}).await;

        assert_eq!(outcome.state, RunState::Failed);
        assert_eq!(outcome.iterations.len(), 3);
        // One shadow ref per pass; distinct commits (the parent chain advances).
        let refs: Vec<_> = outcome.iterations.iter().map(|i| i.shadow_ref.as_str()).collect();
        assert_eq!(
            refs,
            [
                "refs/agentapp/run-loop-exhaust/iter-1",
                "refs/agentapp/run-loop-exhaust/iter-2",
                "refs/agentapp/run-loop-exhaust/iter-3",
            ]
        );
        assert_ne!(outcome.iterations[0].commit, outcome.iterations[1].commit);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hard_failure_when_adapter_cannot_spawn() {
        let git = GitActor::spawn();
        let (cfg, _repo, _root, _prog) = setup("loop-nospawn", 3, &git).await;
        let mut adapter = ScriptedAdapter::new(cfg.progress_path.clone(), None);
        adapter.fail_start = true;

        let outcome = run_loop(&adapter, &git, &cfg, &mut |_, _| {}).await;

        assert_eq!(outcome.state, RunState::Failed);
        assert!(outcome.iterations.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prompt_carries_task_and_prior_progress() {
        let git = GitActor::spawn();
        let (cfg, _repo, _root, _prog) = setup("loop-prompt", 3, &git).await;
        let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), Some(2));

        let outcome = run_loop(&adapter, &git, &cfg, &mut |_, _| {}).await;
        assert_eq!(outcome.state, RunState::Completed);

        let prompts = adapter.prompts.lock().unwrap();
        assert_eq!(prompts.len(), 2);
        // Pass 1: the task text, and no prior progress yet.
        assert!(prompts[0].contains("Implement the widget"));
        assert!(prompts[0].contains("(no prior progress yet)"));
        // Pass 2: the prior progress the agent wrote in pass 1 is fed back.
        assert!(prompts[1].contains("pass 1 did work"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn threads_wrapper_into_each_pass_spec() {
        let git = GitActor::spawn();
        let (mut cfg, _repo, _root, _prog) = setup("loop-wrapper", 3, &git).await;
        // Stand in for the Seatbelt prefix the wiring layer builds.
        cfg.wrapper = vec![
            OsString::from("/usr/bin/sandbox-exec"),
            OsString::from("-f"),
            OsString::from("/tmp/run-loop-wrapper.sb"),
        ];
        let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), Some(2));

        run_loop(&adapter, &git, &cfg, &mut |_, _| {}).await;

        // Every pass's RunSpec carried the wrapper verbatim.
        let wrappers = adapter.wrappers.lock().unwrap();
        assert_eq!(wrappers.len(), 2);
        assert!(wrappers.iter().all(|w| *w == cfg.wrapper));
    }
}
