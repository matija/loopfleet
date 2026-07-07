//! Derived per-task state (PRD "Plans" / "Data model": `TaskStatus`).
//!
//! Per-task live state is **derived from run records**, never stored as truth.
//! The plan view computes it fresh from the runs bound to each task. `checked`
//! (authored input) plays no part — it only gates whether a task is launchable.

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

/// Derive a task's status from the runs bound to it. Precedence (highest first):
/// any accepted → `Accepted`; any completed → `CompletedUnaccepted`; any queued
/// or running → `InProgress`; otherwise `NotStarted`.
pub fn derive_status(runs: &[TaskRun]) -> TaskStatus {
    if runs.iter().any(|r| r.accepted) {
        TaskStatus::Accepted
    } else if runs.iter().any(|r| r.state == RunState::Completed) {
        TaskStatus::CompletedUnaccepted
    } else if runs
        .iter()
        .any(|r| matches!(r.state, RunState::Queued | RunState::Running))
    {
        TaskStatus::InProgress
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
        assert_eq!(derive_status(&[]), TaskStatus::NotStarted);
    }

    #[test]
    fn only_failed_or_stopped_is_not_started() {
        assert_eq!(
            derive_status(&[run(RunState::Failed, false), run(RunState::Stopped, false)]),
            TaskStatus::NotStarted
        );
    }

    #[test]
    fn queued_or_running_is_in_progress() {
        assert_eq!(
            derive_status(&[run(RunState::Failed, false), run(RunState::Running, false)]),
            TaskStatus::InProgress
        );
        assert_eq!(
            derive_status(&[run(RunState::Queued, false)]),
            TaskStatus::InProgress
        );
    }

    #[test]
    fn completed_outranks_in_progress() {
        // A finished run plus a fresh re-run: the completed one still surfaces
        // this task into the review queue.
        assert_eq!(
            derive_status(&[
                run(RunState::Completed, false),
                run(RunState::Running, false),
            ]),
            TaskStatus::CompletedUnaccepted
        );
    }

    #[test]
    fn accepted_outranks_everything() {
        assert_eq!(
            derive_status(&[
                run(RunState::Completed, true),
                run(RunState::Completed, false),
                run(RunState::Running, false),
            ]),
            TaskStatus::Accepted
        );
    }

    #[test]
    fn serializes_kebab_case_for_the_ui() {
        let json = serde_json::to_string(&TaskStatus::CompletedUnaccepted).unwrap();
        assert_eq!(json, "\"completed-unaccepted\"");
    }
}
