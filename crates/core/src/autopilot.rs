//! Auto-merge gating: the pure decision behind "merge this run's branch
//! automatically once it completes" (see [`Settings::auto_merge_enabled`] in
//! `loopfleet-store`).
//!
//! [`should_auto_merge`] is deliberately a plain function over plain inputs —
//! no store or git access — so the driving code (wherever a run's state,
//! acceptance, or snapshot changes) can call it inline and the rule itself
//! stays independently testable.
//!
//! [`Settings::auto_merge_enabled`]: loopfleet_store::Settings::auto_merge_enabled

use loopfleet_store::Settings;

use crate::supervisor::RunState;
use crate::task_status::TaskStatus;
use crate::TaskView;

/// The next task autopilot should launch: the first task in document order
/// whose derived status is [`TaskStatus::NotStarted`].
pub fn next_task(tasks: &[TaskView]) -> Option<&TaskView> {
    tasks.iter().find(|t| t.status == TaskStatus::NotStarted)
}

/// Why auto-advance refuses to chain the plan's next task after an auto-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoAdvanceBlockedReason {
    /// The concurrency cap (M6 settings) is already met by active runs, so
    /// queuing another launch would just wait behind it — the delay is more
    /// useful spent on a fresh check once a slot frees up than on a launch
    /// that's guaranteed to be rejected when it fires.
    ConcurrencyCapReached,
    /// The plan already has an auto-advance launch scheduled (e.g. one from a
    /// task that finished moments earlier); advancing again would queue a
    /// second one on top of it.
    LaunchAlreadyPending,
}

/// Decide whether auto-advance may chain the plan's next task right now,
/// given the fleet's current load and this plan's own pending schedule.
///
/// Checks run in that order, so the first one that fails is the reason
/// reported. Returns `None` when neither condition blocks — the caller then
/// looks up the next task itself via [`next_task`].
pub fn should_auto_advance(
    active_runs: u32,
    concurrency_cap: u32,
    has_pending_auto_advance_launch: bool,
) -> Option<AutoAdvanceBlockedReason> {
    if concurrency_cap > 0 && active_runs >= concurrency_cap {
        return Some(AutoAdvanceBlockedReason::ConcurrencyCapReached);
    }
    if has_pending_auto_advance_launch {
        return Some(AutoAdvanceBlockedReason::LaunchAlreadyPending);
    }
    None
}

/// Why auto-merge does not arm for a run right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoMergeBlockedReason {
    /// The user has turned auto-merge off in settings.
    Disabled,
    /// The run has not reached [`RunState::Completed`] yet.
    RunNotCompleted,
    /// The run was already accepted (e.g. "used" by hand while it was still
    /// running) — there is nothing left for auto-merge to do.
    AlreadyAccepted,
    /// The run produced no shadow snapshot, so there is nothing to merge.
    NoSnapshot,
    /// The repo has a merge already in progress (conflicts left for the user
    /// to resolve by hand); auto-merge must not run on top of that.
    MergeInProgress,
}

/// The outcome of [`should_auto_merge`]: either the countdown should arm, or a
/// stated reason it should not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoMergeDecision {
    /// Arm the auto-merge countdown, waiting this many seconds before merging
    /// (giving the user a window to cancel).
    Arm { delay_seconds: u32 },
    /// Do not arm auto-merge, for the given reason.
    Blocked(AutoMergeBlockedReason),
}

/// Decide whether a run's branch should auto-merge, given its lifecycle
/// state, acceptance, snapshot presence, and the repo's own merge state.
///
/// All conditions must hold: auto-merge enabled in `settings`, the run
/// [`Completed`](RunState::Completed), not already `accepted`, `has_snapshot`,
/// and no `merge_in_progress` in the target repo. Checks run in that order, so
/// the first one that fails is the reason reported — callers showing a single
/// blocked reason get the most actionable one first (there is no point
/// telling the user about a repo-side conflict if auto-merge is off).
pub fn should_auto_merge(
    run_state: RunState,
    accepted: bool,
    has_snapshot: bool,
    merge_in_progress: bool,
    settings: &Settings,
) -> AutoMergeDecision {
    if !settings.auto_merge_enabled {
        return AutoMergeDecision::Blocked(AutoMergeBlockedReason::Disabled);
    }
    if run_state != RunState::Completed {
        return AutoMergeDecision::Blocked(AutoMergeBlockedReason::RunNotCompleted);
    }
    if accepted {
        return AutoMergeDecision::Blocked(AutoMergeBlockedReason::AlreadyAccepted);
    }
    if !has_snapshot {
        return AutoMergeDecision::Blocked(AutoMergeBlockedReason::NoSnapshot);
    }
    if merge_in_progress {
        return AutoMergeDecision::Blocked(AutoMergeBlockedReason::MergeInProgress);
    }
    AutoMergeDecision::Arm {
        delay_seconds: settings.auto_merge_countdown_seconds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(auto_merge_enabled: bool, auto_merge_countdown_seconds: u32) -> Settings {
        Settings {
            auto_merge_enabled,
            auto_merge_countdown_seconds,
            ..Settings::default()
        }
    }

    #[test]
    fn arms_when_all_conditions_hold() {
        let s = settings(true, 10);
        assert_eq!(
            should_auto_merge(RunState::Completed, false, true, false, &s),
            AutoMergeDecision::Arm { delay_seconds: 10 }
        );
    }

    #[test]
    fn blocked_when_disabled() {
        let s = settings(false, 10);
        assert_eq!(
            should_auto_merge(RunState::Completed, false, true, false, &s),
            AutoMergeDecision::Blocked(AutoMergeBlockedReason::Disabled)
        );
    }

    #[test]
    fn blocked_when_run_not_completed() {
        let s = settings(true, 10);
        assert_eq!(
            should_auto_merge(RunState::Running, false, true, false, &s),
            AutoMergeDecision::Blocked(AutoMergeBlockedReason::RunNotCompleted)
        );
    }

    #[test]
    fn blocked_when_already_accepted() {
        let s = settings(true, 10);
        assert_eq!(
            should_auto_merge(RunState::Completed, true, true, false, &s),
            AutoMergeDecision::Blocked(AutoMergeBlockedReason::AlreadyAccepted)
        );
    }

    #[test]
    fn blocked_when_no_snapshot() {
        let s = settings(true, 10);
        assert_eq!(
            should_auto_merge(RunState::Completed, false, false, false, &s),
            AutoMergeDecision::Blocked(AutoMergeBlockedReason::NoSnapshot)
        );
    }

    #[test]
    fn blocked_when_merge_in_progress() {
        let s = settings(true, 10);
        assert_eq!(
            should_auto_merge(RunState::Completed, false, true, true, &s),
            AutoMergeDecision::Blocked(AutoMergeBlockedReason::MergeInProgress)
        );
    }

    #[test]
    fn disabled_takes_priority_over_other_reasons() {
        let s = settings(false, 10);
        assert_eq!(
            should_auto_merge(RunState::Running, true, false, true, &s),
            AutoMergeDecision::Blocked(AutoMergeBlockedReason::Disabled)
        );
    }

    #[test]
    fn arm_carries_configured_delay() {
        let s = settings(true, 42);
        assert_eq!(
            should_auto_merge(RunState::Completed, false, true, false, &s),
            AutoMergeDecision::Arm { delay_seconds: 42 }
        );
    }

    fn task(anchor: &str, status: TaskStatus) -> TaskView {
        TaskView {
            anchor: anchor.into(),
            line_hint: 0,
            text: anchor.into(),
            checked: false,
            status,
            run_count: 0,
        }
    }

    #[test]
    fn next_task_finds_first_not_started_in_document_order() {
        let tasks = vec![
            task("alpha", TaskStatus::Accepted),
            task("beta", TaskStatus::InProgress),
            task("gamma", TaskStatus::NotStarted),
            task("delta", TaskStatus::NotStarted),
        ];
        assert_eq!(next_task(&tasks).unwrap().anchor, "gamma");
    }

    #[test]
    fn next_task_none_when_no_task_is_not_started() {
        let tasks = vec![
            task("alpha", TaskStatus::Accepted),
            task("beta", TaskStatus::CompletedUnaccepted),
        ];
        assert!(next_task(&tasks).is_none());
    }

    #[test]
    fn next_task_none_for_empty_tasks() {
        assert!(next_task(&[]).is_none());
    }

    #[test]
    fn auto_advance_allowed_under_cap_with_no_pending_launch() {
        assert_eq!(should_auto_advance(1, 3, false), None);
    }

    #[test]
    fn auto_advance_allowed_when_cap_disabled() {
        assert_eq!(should_auto_advance(10, 0, false), None);
    }

    #[test]
    fn auto_advance_blocked_when_cap_reached() {
        assert_eq!(
            should_auto_advance(3, 3, false),
            Some(AutoAdvanceBlockedReason::ConcurrencyCapReached)
        );
    }

    #[test]
    fn auto_advance_blocked_when_over_cap() {
        assert_eq!(
            should_auto_advance(5, 3, false),
            Some(AutoAdvanceBlockedReason::ConcurrencyCapReached)
        );
    }

    #[test]
    fn auto_advance_blocked_when_launch_already_pending() {
        assert_eq!(
            should_auto_advance(0, 3, true),
            Some(AutoAdvanceBlockedReason::LaunchAlreadyPending)
        );
    }

    #[test]
    fn cap_reached_takes_priority_over_pending_launch() {
        assert_eq!(
            should_auto_advance(3, 3, true),
            Some(AutoAdvanceBlockedReason::ConcurrencyCapReached)
        );
    }
}
