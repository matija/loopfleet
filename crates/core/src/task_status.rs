//! Derived per-task state (PRD "Plans" / "Data model": `TaskStatus`).
//!
//! Per-task live state is **derived from run records plus the authored
//! `checked` box**, never stored as truth. The plan view computes it fresh per
//! read. An authored `- [x]` reads as the author's "this is implemented" and
//! seeds `Accepted` as the lowest-precedence baseline — any real run outranks
//! it (a completed-but-unaccepted rerun still surfaces as `CompletedUnaccepted`
//! for review, an active run shows `InProgress`). A checked task stays runnable:
//! launching against it is never gated.

use serde::Serialize;

use crate::RunState;

/// The four derived task states. Serialized in kebab-case for the UI
/// (`not-started`, `in-progress`, `completed-unaccepted`, `accepted`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    /// No run has made progress: zero runs, or only failed/stopped ones. The
    /// four-state model has no dedicated "attempted/failed" state, so a task
    /// whose only runs failed reads as not-started (nothing running, completed,
    /// or accepted).
    NotStarted,
    /// A run is queued or running against this task.
    InProgress,
    /// At least one run completed the task, but none is accepted yet — the
    /// review/compare queue the UI surfaces loudly.
    CompletedUnaccepted,
    /// A completed run was accepted via "use this run". "Implemented".
    Accepted,
}

/// One run's bearing on its task's status: its lifecycle state and whether it
/// was accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskRun {
    pub state: RunState,
    pub accepted: bool,
}

/// Derive a task's status from the runs bound to it plus the authored
/// `checked` box. Precedence (highest first): any accepted → `Accepted`; any
/// completed → `CompletedUnaccepted`; any queued or running → `InProgress`;
/// an authored `- [x]` with no outranking run → `Accepted`; otherwise
/// `NotStarted`. So the checkmark is the "implemented" baseline, and any real
/// run overrides it — a rerun that completes surfaces for review instead of
/// silently reading as done.
pub fn derive_status(runs: &[TaskRun], checked: bool) -> TaskStatus {
    if runs.iter().any(|r| r.accepted) {
        TaskStatus::Accepted
    } else if runs.iter().any(|r| r.state == RunState::Completed) {
        TaskStatus::CompletedUnaccepted
    } else if runs
        .iter()
        .any(|r| matches!(r.state, RunState::Queued | RunState::Running))
    {
        TaskStatus::InProgress
    } else if checked {
        TaskStatus::Accepted
    } else {
        TaskStatus::NotStarted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(state: RunState, accepted: bool) -> TaskRun {
        TaskRun { state, accepted }
    }

    #[test]
    fn no_runs_is_not_started() {
        assert_eq!(derive_status(&[], false), TaskStatus::NotStarted);
    }

    #[test]
    fn only_failed_or_stopped_is_not_started() {
        assert_eq!(
            derive_status(&[run(RunState::Failed, false), run(RunState::Stopped, false)], false),
            TaskStatus::NotStarted
        );
    }

    #[test]
    fn queued_or_running_is_in_progress() {
        assert_eq!(
            derive_status(&[run(RunState::Failed, false), run(RunState::Running, false)], false),
            TaskStatus::InProgress
        );
        assert_eq!(
            derive_status(&[run(RunState::Queued, false)], false),
            TaskStatus::InProgress
        );
    }

    #[test]
    fn completed_outranks_in_progress() {
        // A finished run plus a fresh re-run: the completed one still surfaces
        // this task into the review queue.
        assert_eq!(
            derive_status(
                &[
                    run(RunState::Completed, false),
                    run(RunState::Running, false),
                ],
                false,
            ),
            TaskStatus::CompletedUnaccepted
        );
    }

    #[test]
    fn accepted_outranks_everything() {
        assert_eq!(
            derive_status(
                &[
                    run(RunState::Completed, true),
                    run(RunState::Completed, false),
                    run(RunState::Running, false),
                ],
                false,
            ),
            TaskStatus::Accepted
        );
    }

    #[test]
    fn authored_checked_is_accepted_baseline() {
        // An authored `- [x]` with no runs reads as implemented.
        assert_eq!(derive_status(&[], true), TaskStatus::Accepted);
        // Failed/stopped runs don't un-implement an authored-done task.
        assert_eq!(
            derive_status(&[run(RunState::Failed, false)], true),
            TaskStatus::Accepted
        );
    }

    #[test]
    fn checked_does_not_mask_a_completed_rerun() {
        // A checked task that is rerun and completes still surfaces for review —
        // the checkmark is the baseline, not a live truth that hides new runs.
        assert_eq!(
            derive_status(&[run(RunState::Completed, false)], true),
            TaskStatus::CompletedUnaccepted
        );
        // An active rerun shows in-progress even on a checked task.
        assert_eq!(
            derive_status(&[run(RunState::Running, false)], true),
            TaskStatus::InProgress
        );
    }

    #[test]
    fn serializes_kebab_case_for_the_ui() {
        let json = serde_json::to_string(&TaskStatus::CompletedUnaccepted).unwrap();
        assert_eq!(json, "\"completed-unaccepted\"");
    }
}
