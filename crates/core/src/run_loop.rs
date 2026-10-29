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
use tokio::sync::{mpsc, oneshot, watch};

use crate::adapter::{AgentAdapter, RunSpec, SteerRequest};
use crate::execution::{self, AgentResult, ExecutionConfig};
use crate::tap::{self, Delivery};
use crate::{NormalizedEvent, RunState, Tap};

/// Everything the loop needs to run one task to completion (or failure).
#[derive(Debug, Clone)]
pub struct LoopConfig {
    /// Stable run id; keys the worktree, branch, and shadow refs.
    pub run_id: String,
    /// The parent repository (where shadow refs and objects live).
    pub repo: PathBuf,
    /// The per-run worktree the agent runs in and the snapshotter captures.
    pub worktree: PathBuf,
    pub progress_path: PathBuf,
    /// The bound task's text, injected into every pass's prompt.
    pub task_text: String,
    pub plan_context: String,
    pub execution: ExecutionConfig,
    pub repair_context: Option<String>,
    /// Maximum passes before the run fails as incomplete.
    pub max_iterations: u32,
    /// The opaque sandbox wrapper prefix prepended to each pass's agent spawn
    /// (see [`RunSpec::wrapper`](crate::adapter::RunSpec::wrapper)). The wiring
    /// layer fills it with the Seatbelt invocation (`sandbox-exec -f <profile>`,
    /// via [`confine_prefix`](loopfleet_sandbox)); empty runs the agent directly.
    pub wrapper: Vec<OsString>,
    /// Model override passed through to every pass's [`RunSpec::model`].
    pub model: Option<String>,
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
    taps: &mut mpsc::Receiver<Tap>,
    on_event: &mut (dyn FnMut(u32, &NormalizedEvent) + Send),
) -> LoopOutcome {
    let mut iterations = Vec::new();
    let mut pending = Vec::new();

    for (index, check) in cfg.execution.preflight.iter().enumerate() {
        let result = execution::run_check(check, &cfg.worktree, &cfg.wrapper, cancel).await;
        on_event(0, &NormalizedEvent::AssistantText {
            text: format!("Loopfleet preflight: {}\n{}", result.name, result.output),
        });
        if let Err(error) = execution::write_record(&cfg.progress_path, &format!("preflight-{index}.json"), &serde_json::to_string_pretty(&result).unwrap()) {
            on_event(0, &NormalizedEvent::Failed { reason: format!("Cannot record preflight: {error}") });
            return LoopOutcome { state: RunState::Failed, iterations, reset_at: None };
        }
        if !result.passed {
            let reason = format!("Preflight blocked: {}. Restore this prerequisite and resume the run.\n{}", result.name, result.output);
            let _ = execution::append_progress(&cfg.progress_path, &reason);
            on_event(0, &NormalizedEvent::Failed { reason });
            return LoopOutcome {
                state: if *cancel.borrow() || cancel.has_changed().is_err() { RunState::Stopped } else { RunState::Blocked },
                iterations, reset_at: None,
            };
        }
    }

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
        while let Ok(tap) = taps.try_recv() {
            let delivery = tap::route(adapter.can_steer(), false, false);
            on_event(
                n,
                &NormalizedEvent::UserMessage {
                    id: tap.id.clone(),
                    text: tap.text.as_str().to_owned(),
                    delivery,
                },
            );
            if delivery == Delivery::Queued {
                pending.push(tap);
            }
        }
        let prompt = build_prompt(cfg, &prior, &pending, n);
        for tap in pending.drain(..) {
            on_event(
                n,
                &NormalizedEvent::UserMessage {
                    id: tap.id,
                    text: tap.text.into(),
                    delivery: Delivery::Delivered { pass: n },
                },
            );
        }
        let spec = RunSpec {
            cwd: cfg.worktree.clone(),
            prompt,
            wrapper: cfg.wrapper.clone(),
            model: cfg.model.clone(),
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
        let mut acknowledgements: Vec<(
            Tap,
            oneshot::Receiver<Result<(), crate::adapter::AdapterError>>,
        )> = Vec::new();
        loop {
            tokio::select! {
                event = handle.events.recv() => match event {
                    _ if *cancel.borrow() || cancel.has_changed().is_err() => {
                        cancelled = true;
                        break;
                    }
                    Some(event) => {
                        if let NormalizedEvent::RateLimited { reset_at, .. } = &event {
                            rate_limited = Some(reset_at.clone());
                        }
                        on_event(n, &event);
                    }
                    None => break,
                },
                Some(tap) = taps.recv() => {
                    if *cancel.borrow() || cancel.has_changed().is_err() {
                        cancelled = true;
                        break;
                    }
                    let mut delivery = tap::route(adapter.can_steer(), true, handle.steer.is_some());
                    if delivery == Delivery::Steered {
                        let (ack, receiver) = oneshot::channel();
                        let request = SteerRequest {
                            tap_id: tap.id.clone(),
                            text: tap.text.as_str().to_owned(),
                            ack,
                        };
                        if handle.steer.as_ref().unwrap().try_send(request).is_err() {
                            delivery = Delivery::Queued;
                        } else {
                            acknowledgements.push((tap, receiver));
                            continue;
                        }
                    }
                    resolve_tap(n, tap, delivery, &mut pending, on_event);
                },
                (index, result) = std::future::poll_fn(|cx| {
                    for (index, (_, receiver)) in acknowledgements.iter_mut().enumerate() {
                        if let std::task::Poll::Ready(result) = std::future::Future::poll(std::pin::Pin::new(receiver), cx) {
                            return std::task::Poll::Ready((index, result.ok()));
                        }
                    }
                    std::task::Poll::Pending
                }), if !acknowledgements.is_empty() => {
                    let (tap, _) = acknowledgements.remove(index);
                    resolve_tap(n, tap, tap::acknowledged(result), &mut pending, on_event);
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
        for (tap, mut receiver) in acknowledgements {
            resolve_tap(
                n,
                tap,
                tap::acknowledged(receiver.try_recv().ok()),
                &mut pending,
                on_event,
            );
        }

        let mut ready = false;
        let mut blocked = false;
        if !cancelled {
            match execution::read_result(&cfg.progress_path, n) {
                Ok(AgentResult::Blocked { reason }) => {
                    blocked = true;
                    let _ = execution::append_progress(&cfg.progress_path, &format!("Blocked: {reason}"));
                    on_event(n, &NormalizedEvent::AssistantText { text: format!("Blocked: {reason}") });
                }
                Ok(AgentResult::Ready { summary, checks }) => {
                    let mut seen = std::collections::HashSet::new();
                    let checks: Vec<_> = cfg.execution.verify.iter().chain(&checks)
                        .filter(|check| seen.insert(check.command.clone())).collect();
                    ready = !checks.is_empty();
                    if checks.is_empty() {
                        on_event(n, &NormalizedEvent::Failed { reason: "No verification checks supplied. Report the task's executable checks or configure .loopfleet.json.".into() });
                    }
                    let mut results = Vec::new();
                    for check in checks {
                        let result = execution::run_check(check, &cfg.worktree, &cfg.wrapper, cancel).await;
                        on_event(n, &NormalizedEvent::AssistantText {
                            text: format!("Loopfleet verification: {} — {}\n{}", result.name, if result.passed { "passed" } else { "failed" }, result.output),
                        });
                        let passed = result.passed;
                        results.push(result);
                        if !passed {
                            ready = false;
                            break;
                        }
                    }
                    let evidence = serde_json::to_string_pretty(&results).unwrap();
                    if let Err(error) = execution::write_record(&cfg.progress_path, &format!("verification-{n}.json"), &evidence) {
                        ready = false;
                        on_event(n, &NormalizedEvent::Failed { reason: format!("Cannot record verification: {error}") });
                    }
                    let failures = results.iter().filter(|result| !result.passed)
                        .map(|result| format!("{}: {}", result.name, result.output.chars().take(2000).collect::<String>()))
                        .collect::<Vec<_>>().join("\n");
                    let feedback = if ready { format!("Runner verification passed.\nSUMMARY: {summary}") } else { format!("Runner verification failed. Fix these checks before reporting ready:\n{failures}") };
                    if let Err(error) = execution::append_progress(&cfg.progress_path, &feedback) {
                        ready = false;
                        on_event(n, &NormalizedEvent::Failed { reason: format!("Cannot record verification progress: {error}") });
                    }
                }
                Err(reason) => {
                    let _ = execution::append_progress(&cfg.progress_path, &reason);
                    on_event(n, &NormalizedEvent::Failed { reason });
                }
            }
        }
        cancelled |= *cancel.borrow() || cancel.has_changed().is_err();

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

        if ready || blocked {
            return LoopOutcome {
                state: if ready { RunState::Completed } else { RunState::Blocked },
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

    LoopOutcome {
        state: RunState::Failed,
        iterations,
        reset_at: None,
    }
}

fn resolve_tap(
    pass: u32,
    tap: Tap,
    delivery: Delivery,
    pending: &mut Vec<Tap>,
    on_event: &mut (dyn FnMut(u32, &NormalizedEvent) + Send),
) {
    on_event(
        pass,
        &NormalizedEvent::UserMessage {
            id: tap.id.clone(),
            text: tap.text.as_str().to_owned(),
            delivery,
        },
    );
    if delivery == Delivery::Queued {
        pending.push(tap);
    }
}

/// Read the progress file, or `""` if it does not exist yet (first pass).
fn read_progress(path: &Path) -> String {
    crate::safe_file::read_bounded(path.parent().and_then(Path::parent).unwrap_or_else(|| Path::new("/")), path, 256 * 1024).unwrap_or_default()
}

/// Assemble a pass's prompt: the bound task, the loop rules, the progress-file
/// protocol, and the prior progress the fresh context reads back.
fn build_prompt(cfg: &LoopConfig, prior: &str, pending: &[Tap], pass: u32) -> String {
    let prior = if prior.trim().is_empty() {
        "(no prior progress yet)"
    } else {
        prior
    };
    format!(
        "{contract}\n\nSelected task:\n{task}\n\n\
Full plan reference captured at launch (only implement the selected task):\n<plan-reference>\n{plan}\n</plan-reference>\n\n\
Complete the entire task in this attempt. Do all required tests and checks.\n\
The required result is the completed task, not a progress report or a plan.\n\
If a task requirement remains unfinished, this attempt failed.\n\
A problem that prevents further work does not make the task completed.\n\
A missing test, an omitted check, or a failed check means that the task remains unfinished.\n\n\
Progress file:\n  {progress}\n\n\
Do these steps:\n\
1. Read the previous progress at the end of this message.\n\
2. Examine the existing changes.\n\
3. Complete each unfinished task requirement.\n\
4. Do all required tests and checks.\n\
5. Compare the result with each task requirement.\n\
6. Make sure that the result satisfies each task requirement.\n\
7. Make sure that all required tests and checks passed.\n\n\
Obey these rules:\n\
- Do not stop after one part of the task.\n\
- Do not leave required work for another attempt.\n\
- Do not ask for permission to continue the task.\n\
- Do not describe unfinished requirements as optional work.\n\
- Write only the code that the task requires.\n\
- Do not add options, layers, or hooks for possible future requirements.\n\
- Use the shortest correct solution.\n\
- If one line is sufficient, write one line.\n\
- Change only the files that the task requires.\n\
- Preserve the task requirements in plan files.\n\
- Add results to plan files without replacement of the plan.\n\
- Preserve processes that were active before this attempt.\n\
- Never quit, kill, or restart the Loopfleet application hosting this run.\n\
- For native checks, build and launch this worktree's app, not the installed Loopfleet app.\n\
- Track native app PIDs individually; never quit applications by name or kill by a shared executable path.\n\
- Before you stop, stop each background process that you started for this attempt.\n\
- Make sure that each process you stopped exited.\n\n\
Process inspection:\n\
The macOS sandbox blocks setuid programs, including `ps` and `top`.\n\
- Find processes with `pgrep` instead of `ps`.\n\
- Measure process resources with `libproc` instead of `top`.\n\
- Inspect the rendered sandbox profile before diagnosing native automation permissions.\n\
- If it already allows lsopen and appleevent-send, do not claim the project permission is disabled.\n\
- Do not remove the sandbox or change its permissions.\n\n\
Record the result:\n\
1. Add completed work, exact check commands, results, and unfinished requirements to the progress file.\n\
2. Write a JSON result to {result_path}. This is required even if task text mentions an older completion marker.\n\
3. For implemented work with all required checks passed, use {{\"status\":\"ready\",\"summary\":\"Imperative description under 72 characters\",\"checks\":[{{\"name\":\"Task acceptance\",\"command\":[\"program\",\"argument\"],\"timeout_seconds\":300}}]}}. Supply actual executable checks, not placeholder commands.\n\
4. For an external prerequisite that prevents required work or verification, use {{\"status\":\"blocked\",\"reason\":\"Specific prerequisite, unfinished checks, and recovery action\"}}. Do not start host infrastructure or weaken the sandbox.\n\
5. For code/test failures, continue fixing the task. Do not label implementation failures as external blockers.\n\
6. Loopfleet independently runs verification before marking work ready for review. A completion marker alone never completes the run.\n\
7. Preserve useful worktree changes. Loopfleet handles snapshots and commits. Do not request Git write access.\n\n\
--- prior progress ---\n{prior}\n{repair}\n{notes}",
        contract = execution::RUNNER_CONTRACT,
        task = cfg.task_text,
        plan = cfg.plan_context,
        progress = cfg.progress_path.display(),
        result_path = execution::result_path(&cfg.progress_path, pass).display(),
        prior = prior,
        repair = cfg.repair_context.as_deref().unwrap_or_default(),
        notes = tap::render_notes(pending).map_or(String::new(), |notes| format!("\n{notes}\n")),
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

    struct TapScript {
        taps: mpsc::Sender<Tap>,
        can_steer: bool,
        steer_open: Option<bool>,
        steer_full: bool,
        acknowledgement: Option<bool>,
        wait_for_stop: bool,
        steered: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl TapScript {
        fn start_run(&self, handle: RunHandle, pass: u32) -> RunHandle {
            if pass != 1 {
                return handle;
            }
            let (events, rx) = mpsc::channel(1);
            let (steer, mut requests) = mpsc::channel::<SteerRequest>(1);
            if self.steer_full {
                steer
                    .try_send(SteerRequest {
                        tap_id: String::new(),
                        text: String::new(),
                        ack: oneshot::channel().0,
                    })
                    .unwrap();
            }
            if self.steer_open != Some(true) {
                requests.close();
            }
            let acknowledgement = self.acknowledgement;
            let wait_for_stop = self.wait_for_stop;
            let taps = self.taps.clone();
            let steered = self.steered.clone();
            let live_steering = self.can_steer && self.steer_open == Some(true) && !self.steer_full;
            tokio::spawn(async move {
                let mut held = Vec::new();
                for text in ["First correction", "Second correction"] {
                    taps.send(test_tap(text)).await.unwrap();
                    drop(taps.reserve().await.unwrap());
                    if live_steering {
                        let request = requests.recv().await.unwrap();
                        steered.lock().unwrap().push((request.tap_id, request.text));
                        match acknowledgement {
                            Some(true) => {
                                let _ = request.ack.send(Ok(()));
                            }
                            Some(false) => {
                                let _ = request
                                    .ack
                                    .send(Err(AdapterError::Protocol("rejected".into())));
                            }
                            None if wait_for_stop => held.push(request.ack),
                            None => drop(request.ack),
                        }
                    }
                }
                let _ = events
                    .send(NormalizedEvent::AssistantText {
                        text: "still draining".into(),
                    })
                    .await;
                if wait_for_stop {
                    events.closed().await;
                }
                drop(events);
            });
            RunHandle {
                events: rx,
                steer: self.steer_open.map(|_| steer),
            }
        }
    }

    fn test_tap(text: &str) -> Tap {
        Tap {
            id: text.into(),
            text: text.to_owned().try_into().unwrap(),
            created_at_ms: 0,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn routes_live_taps_and_delivers_pending_notes_once() {
        for (can_steer, steer_open, steer_full) in [
            (false, None, false),
            (false, Some(true), false),
            (true, None, false),
            (true, Some(false), false),
            (true, Some(true), false),
            (true, Some(true), true),
        ] {
            let git = GitActor::spawn();
            let (cfg, _repo, _root, _prog) = setup("loop-taps", 3, &git).await;
            let (tx, mut taps) = mpsc::channel(1);
            let mut adapter = ScriptedAdapter::new(cfg.progress_path.clone(), Some(3));
            adapter.taps = Some(TapScript {
                taps: tx,
                can_steer,
                steer_open,
                steer_full,
                acknowledgement: Some(true),
                wait_for_stop: false,
                steered: Arc::new(Mutex::new(Vec::new())),
            });
            let (_ctx, mut cancel) = watch::channel(false);
            let mut deliveries = Vec::new();
            let outcome = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                run_loop(
                    &adapter,
                    &git,
                    &cfg,
                    &mut cancel,
                    &mut taps,
                    &mut |_, event| {
                        if let NormalizedEvent::UserMessage { delivery, .. } = event {
                            deliveries.push(*delivery);
                        }
                    },
                ),
            )
            .await
            .unwrap();
            assert_eq!(outcome.state, RunState::Completed);
            let prompts = adapter.prompts.lock().unwrap();
            let steered = adapter.taps.as_ref().unwrap().steered.lock().unwrap();
            assert!(!prompts[0].contains("correction"));
            assert!(!prompts[2].contains("correction"));
            if can_steer && steer_open == Some(true) && !steer_full {
                assert_eq!(deliveries, [Delivery::Steered, Delivery::Steered]);
                assert_eq!(
                    steered.as_slice(),
                    &[
                        ("First correction".into(), "First correction".into()),
                        ("Second correction".into(), "Second correction".into())
                    ]
                );
                assert!(!prompts[1].contains("correction"));
            } else {
                assert_eq!(
                    deliveries,
                    [
                        Delivery::Queued,
                        Delivery::Queued,
                        Delivery::Delivered { pass: 2 },
                        Delivery::Delivered { pass: 2 }
                    ]
                );
                assert!(steered.is_empty());
                assert!(prompts[1].contains("## Corrections from the human"));
                assert!(prompts[1].contains("First correction\n\nSecond correction"));
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn acknowledges_rejection_and_lost_ack_without_blocking_events_or_stop() {
        for (acknowledgement, stop) in [(Some(false), false), (None, false), (None, true)] {
            let git = GitActor::spawn();
            let (cfg, _repo, _root, _prog) = setup("loop-ack", 2, &git).await;
            let (tx, mut taps) = mpsc::channel(1);
            let mut adapter = ScriptedAdapter::new(cfg.progress_path.clone(), Some(2));
            adapter.taps = Some(TapScript {
                taps: tx,
                can_steer: true,
                steer_open: Some(true),
                steer_full: false,
                acknowledgement,
                wait_for_stop: stop,
                steered: Arc::new(Mutex::new(Vec::new())),
            });
            let (ctx, mut cancel) = watch::channel(false);
            let mut deliveries = Vec::new();
            let mut drained = false;
            let outcome = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                run_loop(&adapter, &git, &cfg, &mut cancel, &mut taps, &mut |_, event| {
                    if let NormalizedEvent::UserMessage { delivery, .. } = event {
                        deliveries.push(*delivery);
                    }
                    if matches!(event, NormalizedEvent::AssistantText { text } if text == "still draining") {
                        drained = true;
                        if stop { ctx.send(true).unwrap(); }
                    }
                }),
            ).await.unwrap();
            assert!(drained);
            assert_eq!(
                outcome.state,
                if stop {
                    RunState::Stopped
                } else {
                    RunState::Completed
                }
            );
            let expected = if acknowledgement == Some(false) {
                Delivery::Queued
            } else {
                Delivery::Unknown
            };
            assert_eq!(&deliveries[..2], &[expected, expected]);
            if !stop {
                let prompts = adapter.prompts.lock().unwrap();
                assert_eq!(
                    prompts[1].contains("First correction"),
                    acknowledgement == Some(false)
                );
                assert_eq!(
                    prompts[1].contains("Second correction"),
                    acknowledgement == Some(false)
                );
            }
            assert!(!deliveries.contains(&Delivery::Steered));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delivers_boundary_taps_in_the_next_prompt_once() {
        let git = GitActor::spawn();
        let (cfg, _repo, _root, _prog) = setup("loop-boundary-taps", 2, &git).await;
        let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), Some(2));
        let (tx, mut taps) = mpsc::channel(2);
        tx.send(test_tap("First correction")).await.unwrap();
        tx.send(test_tap("Second correction")).await.unwrap();
        let (_ctx, mut cancel) = watch::channel(false);
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut taps, &mut |_, _| {}).await;
        assert_eq!(outcome.state, RunState::Completed);
        let prompts = adapter.prompts.lock().unwrap();
        assert!(prompts[0]
            .contains("## Corrections from the human\n\nFirst correction\n\nSecond correction"));
        assert!(!prompts[1].contains("correction"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_before_tap_leaves_it_undelivered() {
        for at_boundary in [true, false] {
            let git = GitActor::spawn();
            let (cfg, _repo, _root, _prog) = setup("loop-stop-tap", 2, &git).await;
            let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), Some(2));
            let (tx, mut taps) = mpsc::channel(1);
            let (ctx, mut cancel) = watch::channel(at_boundary);
            if at_boundary {
                tx.send(test_tap("Late correction")).await.unwrap();
            }
            let mut deliveries = Vec::new();
            let outcome = run_loop(
                &adapter,
                &git,
                &cfg,
                &mut cancel,
                &mut taps,
                &mut |_, event| {
                    if matches!(event, NormalizedEvent::TurnStarted) {
                        ctx.send(true).unwrap();
                        tx.try_send(test_tap("Late correction")).unwrap();
                    }
                    if let NormalizedEvent::UserMessage { delivery, .. } = event {
                        deliveries.push(*delivery);
                    }
                },
            )
            .await;
            assert_eq!(outcome.state, RunState::Stopped);
            assert_eq!(outcome.iterations.len(), usize::from(!at_boundary));
            let prompts = adapter.prompts.lock().unwrap();
            assert_eq!(prompts.len(), usize::from(!at_boundary));
            assert!(prompts
                .iter()
                .all(|prompt| !prompt.contains("Late correction")));
            assert!(deliveries.is_empty());
            if at_boundary {
                assert_eq!(taps.try_recv().unwrap().id, "Late correction");
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tap_after_last_pass_does_not_start_another_pass() {
        for complete_on in [Some(1), None] {
            let git = GitActor::spawn();
            let (cfg, _repo, _root, _prog) = setup("loop-last-tap", 1, &git).await;
            let mut adapter = ScriptedAdapter::new(cfg.progress_path.clone(), complete_on);
            let (tx, mut taps) = mpsc::channel(1);
            adapter.taps = Some(TapScript {
                taps: tx.clone(),
                can_steer: false,
                steer_open: None,
                steer_full: false,
                acknowledgement: Some(true),
                wait_for_stop: false,
                steered: Arc::new(Mutex::new(Vec::new())),
            });
            let (_ctx, mut cancel) = watch::channel(false);
            let mut deliveries = Vec::new();
            let outcome = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                run_loop(
                    &adapter,
                    &git,
                    &cfg,
                    &mut cancel,
                    &mut taps,
                    &mut |_, event| {
                        if let NormalizedEvent::UserMessage { delivery, .. } = event {
                            deliveries.push(*delivery);
                        }
                    },
                ),
            )
            .await
            .unwrap();
            assert_eq!(deliveries, [Delivery::Queued, Delivery::Queued]);
            assert!(!adapter.prompts.lock().unwrap()[0].contains("correction"));
            tx.send(test_tap("Late correction")).await.unwrap();
            assert_eq!(
                outcome.state,
                if complete_on.is_some() {
                    RunState::Completed
                } else {
                    RunState::Failed
                }
            );
            assert_eq!(outcome.iterations.len(), 1);
            assert_eq!(adapter.prompts.lock().unwrap().len(), 1);
            assert_eq!(taps.try_recv().unwrap().id, "Late correction");
        }
    }

    /// A scripted adapter: on each `start_run` it records the prompt, appends a
    /// line to the external progress file (optionally the completion marker on a
    /// chosen pass), touches a file in the worktree so snapshots have content,
    /// and replays a fixed event list.
    struct ScriptedAdapter {
        taps: Option<TapScript>,
        progress_path: PathBuf,
        /// 1-based pass on which to write `<STATUS>COMPLETE</STATUS>`; `None` = never.
        complete_on: Option<u32>,
        result_override: Option<&'static str>,
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
                taps: None,
                progress_path,
                complete_on,
                result_override: None,
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
        fn can_steer(&self) -> bool {
            self.taps.as_ref().is_some_and(|taps| taps.can_steer)
        }

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
                writeln!(f, "<STATUS>COMPLETE</STATUS>").unwrap();
                let result = self.result_override.unwrap_or(r#"{"status":"ready","summary":"Implement the widget","checks":[{"name":"Fixture","command":["/usr/bin/true"]}]}"#);
                if !result.is_empty() {
                    std::fs::write(execution::result_path(&self.progress_path, n), result).unwrap();
                }
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
            let handle = RunHandle {
                events: rx,
                steer: None,
            };
            Ok(match &self.taps {
                Some(taps) => taps.start_run(handle, n),
                None => handle,
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
            repair_context: None,
            run_id: run_id.into(),
            repo: repo.path().into(),
            worktree: wt.path,
            progress_path: progress_dir.path().join("progress.md"),
            task_text: "Implement the widget".into(),
            plan_context: "# Plan\nImplementation decisions and nested acceptance details".into(),
            execution: ExecutionConfig::default(),
            max_iterations,
            wrapper: Vec::new(),
            model: None,
        };
        (cfg, repo, root, progress_dir)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn verifies_handoff_and_preserves_blocked_work() {
        let git = GitActor::spawn();
        let (cfg, _repo, _root, _prog) = setup("loop-complete", 3, &git).await;
        let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), Some(1));

        let (_ctx, mut cancel) = watch::channel(false);
        let mut taps = mpsc::channel(1).1;
        let mut seen = Vec::new();
        let outcome = run_loop(
            &adapter,
            &git,
            &cfg,
            &mut cancel,
            &mut taps,
            &mut |n, ev| seen.push((n, ev.clone())),
        )
        .await;

        assert_eq!(outcome.state, RunState::Completed);
        assert_eq!(outcome.iterations.len(), 1);
        assert_eq!(
            outcome.iterations[0].shadow_ref,
            "refs/agentapp/run-loop-complete/iter-1"
        );
        // Every event from the one pass was forwarded, tagged pass 1.
        assert_eq!(seen.len(), 5);
        assert!(seen.iter().all(|(n, _)| *n == 1));
        assert!(matches!(seen[0].1, NormalizedEvent::TurnStarted));
        let evidence = std::fs::read_to_string(cfg.progress_path.with_file_name("verification-1.json")).unwrap();
        assert!(evidence.contains("\"passed\": true"));

        for (id, report, expected) in [
            ("marker-only", "", RunState::Failed),
            ("bad-report", r#"{"status":"ready","summary":"Done","checks":[]}"#, RunState::Failed),
            ("blocked-report", r#"{"status":"blocked","reason":"Docker unavailable. Start Docker and resume."}"#, RunState::Blocked),
            ("failed-verification", r#"{"status":"ready","summary":"Implement feature","checks":[{"name":"Acceptance","command":["/usr/bin/false"]}]}"#, RunState::Failed),
        ] {
            let (cfg, _repo, _root, _prog) = setup(id, 1, &git).await;
            let mut adapter = ScriptedAdapter::new(cfg.progress_path.clone(), Some(1));
            adapter.result_override = Some(report);
            let (_tx, mut cancel) = watch::channel(false);
            let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut mpsc::channel(1).1, &mut |_, _| {}).await;
            assert_eq!(outcome.state, expected, "{id}");
            assert_eq!(outcome.iterations.len(), 1);
            assert!(cfg.worktree.join("pass-1.txt").exists());
        }

        let (mut cfg, repo, _root, _prog) = setup("preflight-blocked", 4, &git).await;
        let config = repo.path().join(".loopfleet.json");
        std::fs::write(&config, r#"{"version":1,"preflight":[{"name":"Docker available","command":["/usr/bin/false"]}]}"#).unwrap();
        cfg.execution = ExecutionConfig::load(repo.path()).unwrap();
        let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), Some(1));
        let (_tx, mut cancel) = watch::channel(false);
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut mpsc::channel(1).1, &mut |_, _| {}).await;
        assert_eq!(outcome.state, RunState::Blocked);
        assert!(outcome.iterations.is_empty());
        assert_eq!(adapter.call.load(Ordering::SeqCst), 0);
        assert!(std::fs::read_to_string(&cfg.progress_path).unwrap().contains("Docker available"));
        std::fs::write(&config, r#"{"version":99}"#).unwrap();
        assert!(ExecutionConfig::load(repo.path()).is_err());
        std::fs::write(&config, r#"{"version":1,"verify":[{"name":"Invalid","command":[],"timeout_seconds":0}]}"#).unwrap();
        assert!(ExecutionConfig::load(repo.path()).is_err());

        let (mut cfg, _repo, _root, _prog) = setup("pinned-verification", 1, &git).await;
        cfg.execution.verify.push(execution::Check { name: "Required acceptance".into(), command: vec!["/usr/bin/false".into()], timeout_seconds: 5 });
        let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), Some(1));
        let (_tx, mut cancel) = watch::channel(false);
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut mpsc::channel(1).1, &mut |_, _| {}).await;
        assert_eq!(outcome.state, RunState::Failed);
        assert!(std::fs::read_to_string(&cfg.progress_path).unwrap().contains("Runner verification failed"));

        let check = execution::Check { name: "Timeout cleanup".into(), command: vec!["/bin/sh".into(), "-c".into(), "sleep 60 & wait".into()], timeout_seconds: 1 };
        let started = std::time::Instant::now();
        let result = execution::run_check(&check, &cfg.worktree, &[], &mut cancel).await;
        assert!(!result.passed);
        assert!(result.output.contains("Timed out"));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        let (tx, mut cancelled) = watch::channel(false);
        tx.send(true).unwrap();
        let result = execution::run_check(&check, &cfg.worktree, &[], &mut cancelled).await;
        assert!(!result.passed);
        assert_eq!(result.output, "Stopped");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fails_after_exhausting_iterations() {
        let git = GitActor::spawn();
        let (cfg, _repo, _root, _prog) = setup("loop-exhaust", 3, &git).await;
        let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), None);

        let (_ctx, mut cancel) = watch::channel(false);
        let mut taps = mpsc::channel(1).1;
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut taps, &mut |_, _| {}).await;

        assert_eq!(outcome.state, RunState::Failed);
        assert_eq!(outcome.iterations.len(), 3);
        // One shadow ref per pass; distinct commits (the parent chain advances).
        let refs: Vec<_> = outcome
            .iterations
            .iter()
            .map(|i| i.shadow_ref.as_str())
            .collect();
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
        let mut taps = mpsc::channel(1).1;
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut taps, &mut |_, _| {}).await;

        assert_eq!(outcome.state, RunState::LimitReached);
        assert_eq!(outcome.reset_at.as_deref(), Some("2025-01-15T10:30:00Z"));
        // The limited pass is still snapshotted; no further pass ran.
        assert_eq!(outcome.iterations.len(), 1);
        assert_eq!(
            adapter.call.load(Ordering::SeqCst),
            1,
            "must not roll into pass 2"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rate_limit_without_reset_time_still_ends_limit_reached() {
        let git = GitActor::spawn();
        let (cfg, _repo, _root, _prog) = setup("loop-ratelimit-noreset", 3, &git).await;
        let mut adapter = ScriptedAdapter::new(cfg.progress_path.clone(), None);
        adapter.rate_limit_on = Some((1, None));

        let (_ctx, mut cancel) = watch::channel(false);
        let mut taps = mpsc::channel(1).1;
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut taps, &mut |_, _| {}).await;

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
        let mut taps = mpsc::channel(1).1;
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut taps, &mut |_, _| {}).await;

        assert_eq!(outcome.state, RunState::Failed);
        assert!(outcome.iterations.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prompt_carries_task_and_prior_progress() {
        let git = GitActor::spawn();
        let (cfg, _repo, _root, _prog) = setup("loop-prompt", 3, &git).await;
        let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), Some(2));

        let (_ctx, mut cancel) = watch::channel(false);
        let mut taps = mpsc::channel(1).1;
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut taps, &mut |_, _| {}).await;
        assert_eq!(outcome.state, RunState::Completed);

        let prompts = adapter.prompts.lock().unwrap();
        assert_eq!(prompts.len(), 2);
        // Pass 1: the task text, and no prior progress yet.
        assert!(prompts[0].contains("Implement the widget"));
        assert!(prompts[0].contains("(no prior progress yet)"));
        // Pass 2: the prior progress the agent wrote in pass 1 is fed back.
        assert!(prompts[1].contains("pass 1 did work"));
        for prompt in prompts.iter() {
            assert!(prompt.contains("Loopfleet owns all Git mutations"));
            assert!(prompt.contains("Implementation decisions and nested acceptance details"));
            assert!(prompt.contains("result-"));
            assert!(prompt.contains("Loopfleet independently runs verification"));
            assert!(prompt.contains("Complete the entire task in this attempt"));
            assert!(prompt.contains("Do all required tests and checks."));
            assert!(prompt.contains("Do not stop after one part of the task."));
            assert!(
                prompt.contains("If a task requirement remains unfinished, this attempt failed.")
            );
            assert!(prompt.contains(
                "A problem that prevents further work does not make the task completed."
            ));
            assert!(prompt.contains("A missing test, an omitted check, or a failed check"));
            assert!(prompt.contains("Do not label implementation failures as external blockers"));
            assert!(!prompt.contains("Do the next part of the task"));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn threads_wrapper_into_each_pass_spec() {
        let git = GitActor::spawn();
        let (mut cfg, _repo, _root, _prog) = setup("loop-wrapper", 3, &git).await;
        // Stand in for the Seatbelt prefix the wiring layer builds.
        cfg.wrapper = vec![
            OsString::from("/usr/bin/env"),
            OsString::from("LOOPFLEET_CHECK_WRAPPER=1"),
        ];
        let mut adapter = ScriptedAdapter::new(cfg.progress_path.clone(), Some(2));
        adapter.result_override = Some(r#"{"status":"ready","summary":"Verify wrapper","checks":[{"name":"Wrapped acceptance","command":["/bin/sh","-c","test \"$LOOPFLEET_CHECK_WRAPPER\" = 1"]}]}"#);

        let (_ctx, mut cancel) = watch::channel(false);
        let mut taps = mpsc::channel(1).1;
        run_loop(&adapter, &git, &cfg, &mut cancel, &mut taps, &mut |_, _| {}).await;

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
        let mut taps = mpsc::channel(1).1;
        ctx.send(true).unwrap();
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut taps, &mut |_, _| {}).await;

        assert_eq!(outcome.state, RunState::Stopped);
        assert!(outcome.iterations.is_empty());
        assert_eq!(
            adapter.call.load(Ordering::SeqCst),
            0,
            "no pass should spawn"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_requested_mid_run_ends_after_the_current_pass() {
        let git = GitActor::spawn();
        // Never completes on its own; a stop is requested during pass 1, so the
        // run stops after that pass's snapshot rather than exhausting all 5.
        let (cfg, _repo, _root, _prog) = setup("loop-stop-mid", 5, &git).await;
        let adapter = ScriptedAdapter::new(cfg.progress_path.clone(), None);

        let (ctx, mut cancel) = watch::channel(false);
        let mut taps = mpsc::channel(1).1;
        let outcome = run_loop(&adapter, &git, &cfg, &mut cancel, &mut taps, &mut |n, _| {
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
