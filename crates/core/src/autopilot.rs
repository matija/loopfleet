//! Auto-merge gating: the pure decision behind "merge this run's branch
//! automatically once it's accepted" (see [`Settings::auto_merge_enabled`] in
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

/// Why auto-merge does not arm for a run right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoMergeBlockedReason {
    /// The user has turned auto-merge off in settings.
    Disabled,
    /// The run has not reached [`RunState::Completed`] yet.
    RunNotCompleted,
    /// The run has not been accepted.
    NotAccepted,
    /// The run produced no shadow snapshot, so there is nothing to merge.
    NoSnapshot,
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
/// state, acceptance, and whether it produced a snapshot to merge.
///
/// All four conditions must hold: auto-merge enabled in `settings`, the run
/// [`Completed`](RunState::Completed), `accepted`, and `has_snapshot`. Checks
/// run in that order, so the first one that fails is the reason reported —
/// callers showing a single blocked reason get the most actionable one first
/// (there is no point telling the user "not accepted" if auto-merge is off).
pub fn should_auto_merge(
    run_state: RunState,
    accepted: bool,
    has_snapshot: bool,
    settings: &Settings,
) -> AutoMergeDecision {
    if !settings.auto_merge_enabled {
        return AutoMergeDecision::Blocked(AutoMergeBlockedReason::Disabled);
    }
    if run_state != RunState::Completed {
        return AutoMergeDecision::Blocked(AutoMergeBlockedReason::RunNotCompleted);
    }
    if !accepted {
        return AutoMergeDecision::Blocked(AutoMergeBlockedReason::NotAccepted);
    }
    if !has_snapshot {
        return AutoMergeDecision::Blocked(AutoMergeBlockedReason::NoSnapshot);
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
            should_auto_merge(RunState::Completed, true, true, &s),
            AutoMergeDecision::Arm { delay_seconds: 10 }
        );
    }

    #[test]
    fn blocked_when_disabled() {
        let s = settings(false, 10);
        assert_eq!(
            should_auto_merge(RunState::Completed, true, true, &s),
            AutoMergeDecision::Blocked(AutoMergeBlockedReason::Disabled)
        );
    }

    #[test]
    fn blocked_when_run_not_completed() {
        let s = settings(true, 10);
        assert_eq!(
            should_auto_merge(RunState::Running, true, true, &s),
            AutoMergeDecision::Blocked(AutoMergeBlockedReason::RunNotCompleted)
        );
    }

    #[test]
    fn blocked_when_not_accepted() {
        let s = settings(true, 10);
        assert_eq!(
            should_auto_merge(RunState::Completed, false, true, &s),
            AutoMergeDecision::Blocked(AutoMergeBlockedReason::NotAccepted)
        );
    }

    #[test]
    fn blocked_when_no_snapshot() {
        let s = settings(true, 10);
        assert_eq!(
            should_auto_merge(RunState::Completed, true, false, &s),
            AutoMergeDecision::Blocked(AutoMergeBlockedReason::NoSnapshot)
        );
    }

    #[test]
    fn disabled_takes_priority_over_other_reasons() {
        let s = settings(false, 10);
        assert_eq!(
            should_auto_merge(RunState::Running, false, false, &s),
            AutoMergeDecision::Blocked(AutoMergeBlockedReason::Disabled)
        );
    }

    #[test]
    fn arm_carries_configured_delay() {
        let s = settings(true, 42);
        assert_eq!(
            should_auto_merge(RunState::Completed, true, true, &s),
            AutoMergeDecision::Arm { delay_seconds: 42 }
        );
    }
}
