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
//! - the user requests a stop via `cancel` → [`RunState::Stopped`];
//! - the agent hit a rate limit during the pass and it did not otherwise finish
//!   → [`RunState::LimitReached`] (the caller schedules a re-run once it resets);
//! - `max_iterations` reached still incomplete → [`RunState::Failed`];
//! - a hard failure (adapter cannot spawn, or a snapshot fails) →
//!   [`RunState::Failed`].
//!
//! A per-pass `Failed` *event* is deliberately NOT fatal: the ralph pattern is
//! resilient across passes, so a pass that ends without completing the task just
//! rolls into the next fresh pass; only exhausting N without completion, or an
//! inability to spawn/snapshot at all, fails the run.
//!
//! Stop (PRD "SIGTERM that group at the next iteration boundary"): the loop
//! watches a `cancel` [`watch`](tokio::sync::watch) channel. Between passes,
//! nothing is running, so a requested stop just returns [`RunState::Stopped`]
//! without spawning the next pass. During a pass, a stop breaks the drain and
//! drops the [`RunHandle`](crate::adapter::RunHandle); every adapter honors that
//! by SIGTERMing the agent's process group (so forked descendants die too — the
//! adapters spawn each agent as a group leader). Either way the pass's worktree
//! is still snapshotted, so all shadow refs are kept.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use loopfleet_gitx::GitActor;
use tokio::sync::watch;

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
    /// Terminal state: [`Completed`](RunState::Completed),
    /// [`Failed`](RunState::Failed), [`Stopped`](RunState::Stopped), or
    /// [`LimitReached`](RunState::LimitReached).
    pub state: RunState,
    /// The snapshots taken, in pass order. Empty if the very first pass could
    /// not spawn.
    pub iterations: Vec<IterationRecord>,
    /// When `state` is [`LimitReached`](RunState::LimitReached), the rate
    /// limit's reset instant as the agent reported it (ISO-8601), if any — the
    /// caller schedules a re-run at this time. `None` for every other state,
    /// and for a rate limit the agent gave no reset time for.
    pub reset_at: Option<String>,
}

/// Drive `adapter` through up to `cfg.max_iterations` passes against one task.
///
/// Sending `true` on `cancel` requests a stop: the run ends in
/// [`RunState::Stopped`] at the current pass boundary (dropping the handle so
/// the adapter SIGTERMs the agent), keeping every snapshot taken so far.
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
    cancel: &mut watch::Receiver<bool>,
    on_event: &mut (dyn FnMut(u32, &NormalizedEvent) + Send),
) -> LoopOutcome {
    let mut iterations = Vec::new();

    for n in 1..=cfg.max_iterations {
        // Boundary stop (PRD default): a stop requested between passes returns
        // without spawning the next one — nothing is running, so it is clean.
        if *cancel.borrow() {
            return LoopOutcome {
                state: RunState::Stopped,
                iterations,
                reset_at: None,
            };
        }

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
                    reset_at: None,
                }
            }
        };

        // Drain the pass. The stream ends on `Ended`/`Failed` or when the
        // adapter's child exits. A mid-pass stop breaks out and drops the handle
        // below, which the adapter honors by SIGTERMing the agent's group.
        let mut cancelled = false;
        // The most recent rate-limit notice seen this pass, if any. `Some(inner)`
        // means the agent hit a limit (`inner` = its reported reset time, which
        // may itself be `None`); the run ends limit-reached if the pass doesn't
        // otherwise complete or get cancelled.
        let mut rate_limited: Option<Option<String>> = None;
        loop {
            tokio::select! {
                event = handle.events.recv() => match event {
                    Some(event) => {
                        if let NormalizedEvent::RateLimited { reset_at, .. } = &event {
                            rate_limited = Some(reset_at.clone());
                        }
                        on_event(n, &event);
                    }
                    None => break,
                },
                changed = cancel.changed() => {
                    // A stop request (or all senders dropped, i.e. app shutdown)
                    // ends the pass.
                    if changed.is_err() || *cancel.borrow_and_update() {
                        cancelled = true;
                        break;
                    }
                }
            }
        }
        // Stop the agent promptly if we broke early (drop closes the receiver →
        // the adapter SIGTERMs the process group). A no-op if the pass ended.
        drop(handle);

        // App-owned snapshot of this pass's worktree state (the agent never
        // commits). Taken even on stop, so shadow refs are kept. A snapshot
        // failure is a hard failure.
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
                    reset_at: None,
                }
            }
        }

        // A stop honored during this pass ends the run once its snapshot is safe.
        if cancelled {
            return LoopOutcome {
                state: RunState::Stopped,
                iterations,
                reset_at: None,
            };
        }

        // Done when the agent has written the completion marker for its task.
        if crate::progress::file_marks_complete(&cfg.progress_path) {
            return LoopOutcome {
                state: RunState::Completed,
                iterations,
                reset_at: None,
            };
        }

        // The agent hit a rate limit and the pass didn't otherwise finish. Rolling
        // into the next fresh pass would just hit the same wall, so end the run
        // limit-reached; the caller schedules a re-run once the limit resets.
        if let Some(reset_at) = rate_limited {
            return LoopOutcome {
                state: RunState::LimitReached,
                iterations,
                reset_at,
            };
        }
    }

    // Exhausted all passes without a completion marker.
    LoopOutcome {
        state: RunState::Failed,
        iterations,
        reset_at: None,
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
        /// 1-based pass on which to emit a `RateLimited` event (carrying this
        /// reset time) instead of finishing normally; `None` = never.
        rate_limit_on: Option<(u32, Option<String>)>,
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
                rate_limit_on: None,
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

            // Replay a minimal but realistic event stream. On a rate-limited pass
            // the agent surfaces the notice and stops without completing.
            let rate_limit = self
                .rate_limit_on
                .as_ref()
                .filter(|(pass, _)| *pass == n)
                .map(|(_, reset_at)| reset_at.clone());
            let (tx, rx) = mpsc::channel(8);
            tokio::spawn(async move {
                let mut events = vec![
                    NormalizedEvent::TurnStarted,
                    NormalizedEvent::AssistantText {
                        text: "working".into(),
                    },
                ];
                if let Some(reset_at) = rate_limit {
                    events.push(NormalizedEvent::RateLimited {
                        reset_at,
                        message: Some("rate limit hit".into()),
                    });
                } else {
                    events.push(NormalizedEvent::TurnCompleted {
                        usage: Default::default(),
                    });
                }
                events.push(NormalizedEvent::Ended);
                for ev in events {
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

        let (_ctx, mut cancel) = watch::channel(false);
        let mut seen = Vec::new();
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut |n, ev| {
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

        let (_ctx, mut cancel) = watch::channel(false);
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut |_, _| {}).await;

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
    async fn rate_limit_ends_the_run_and_surfaces_the_reset_time() {
        let git = GitActor::spawn();
        // 3 passes available, but pass 1 hits a rate limit, so the run ends
        // limit-reached after that pass rather than rolling into pass 2.
        let (cfg, _repo, _root, _prog) = setup("loop-ratelimit", 3, &git).await;
        let mut adapter = ScriptedAdapter::new(cfg.progress_path.clone(), None);
        adapter.rate_limit_on = Some((1, Some("2025-01-15T10:30:00Z".into())));

        let (_ctx, mut cancel) = watch::channel(false);
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut |_, _| {}).await;

        assert_eq!(outcome.state, RunState::LimitReached);
        assert_eq!(outcome.reset_at.as_deref(), Some("2025-01-15T10:30:00Z"));
        // The limited pass is still snapshotted; no further pass ran.
        assert_eq!(outcome.iterations.len(), 1);
        assert_eq!(adapter.call.load(Ordering::SeqCst), 1, "must not roll into pass 2");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rate_limit_without_reset_time_still_ends_limit_reached() {
        let git = GitActor::spawn();
        let (cfg, _repo, _root, _prog) = setup("loop-ratelimit-noreset", 3, &git).await;
        let mut adapter = ScriptedAdapter::new(cfg.progress_path.clone(), None);
        adapter.rate_limit_on = Some((1, None));

        let (_ctx, mut cancel) = watch::channel(false);
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut |_, _| {}).await;

        assert_eq!(outcome.state, RunState::LimitReached);
        assert_eq!(outcome.reset_at, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hard_failure_when_adapter_cannot_spawn() {
        let git = GitActor::spawn();
        let (cfg, _repo, _root, _prog) = setup("loop-nospawn", 3, &git).await;
        let mut adapter = ScriptedAdapter::new(cfg.progress_path.clone(), None);
        adapter.fail_start = true;

        let (_ctx, mut cancel) = watch::channel(false);
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut |_, _| {}).await;

        assert_eq!(outcome.state, RunState::Failed);
        assert!(outcome.iterations.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prompt_carries_task_and_prior_progress() {
        let git = GitActor::spawn();
        let (cfg, _repo, _root, _prog) = setup("loop-prompt", 3, &git).await;
        let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), Some(2));

        let (_ctx, mut cancel) = watch::channel(false);
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut |_, _| {}).await;
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

        let (_ctx, mut cancel) = watch::channel(false);
        run_loop(&adapter, &git, &cfg, &mut cancel, &mut |_, _| {}).await;

        // Every pass's RunSpec carried the wrapper verbatim.
        let wrappers = adapter.wrappers.lock().unwrap();
        assert_eq!(wrappers.len(), 2);
        assert!(wrappers.iter().all(|w| *w == cfg.wrapper));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stops_at_boundary_when_cancelled() {
        let git = GitActor::spawn();
        // 5 passes available, but a stop is requested before the loop starts, so
        // no pass ever runs and the run ends Stopped with no snapshots.
        let (cfg, _repo, _root, _prog) = setup("loop-stop-boundary", 5, &git).await;
        let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), None);

        let (ctx, mut cancel) = watch::channel(false);
        ctx.send(true).unwrap();
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut |_, _| {}).await;

        assert_eq!(outcome.state, RunState::Stopped);
        assert!(outcome.iterations.is_empty());
        assert_eq!(adapter.call.load(Ordering::SeqCst), 0, "no pass should spawn");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_requested_mid_run_ends_after_the_current_pass() {
        let git = GitActor::spawn();
        // Never completes on its own; a stop is requested during pass 1, so the
        // run stops after that pass's snapshot rather than exhausting all 5.
        let (cfg, _repo, _root, _prog) = setup("loop-stop-mid", 5, &git).await;
        let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), None);

        let (ctx, mut cancel) = watch::channel(false);
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut |n, _| {
            // Request the stop while pass 1 is streaming.
            if n == 1 {
                let _ = ctx.send(true);
            }
        })
        .await;

        assert_eq!(outcome.state, RunState::Stopped);
        // The in-flight pass is still snapshotted, so its shadow ref is kept.
        assert_eq!(outcome.iterations.len(), 1);
        assert_eq!(
            outcome.iterations[0].shadow_ref,
            "refs/agentapp/run-loop-stop-mid/iter-1"
        );
    }
}
