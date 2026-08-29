//! Binding stored run anchors back to parsed tasks.
//!
//! A run records the `task_anchor` it was launched against; the plan view
//! re-derives anchors from the file on every read. When the two agree, binding
//! is exact. When they don't — because the anchor derivation changed, or the
//! author reworded the task — the run silently unbinds and its task reads
//! `NotStarted`, which is a *legal* state and so looks like nothing is wrong.
//! That failure mode is the reason this module exists.
//!
//! Resolution cascades over two independent signals, text and position, and
//! reports which one carried the match:
//!
//! 1. **Exact** — the stored anchor is the task's current anchor.
//! 2. **Legacy text** — the stored anchor is an older derivation of the same
//!    task's text: the whole-text form, or a prefix of it (anchors stored before
//!    wrapped lines were joined held only a task's first physical line). Must be
//!    unambiguous across the plan.
//! 3. **Position** — the anchor's last known line, via its still-stored task
//!    row, lands on exactly one nearby task. Bounded: a task whose text was
//!    rewritten wholesale stays roughly where it was, so proximity is evidence
//!    only at close range.
//!
//! Text wins where the two disagree; the disagreement is reported rather than
//! swallowed, because a plan whose runs resolve only by position is a plan whose
//! anchors have drifted.
//!
//! # Position is only offered where it is still evidence
//!
//! Proximity only means anything if the file is still recognizably the plan the
//! runs were stored against. It is not, if the file was replaced wholesale —
//! which is what a plan file at a fixed conventional path (`PRD.md`) undergoes
//! every time one plan is finished and the next is written in its place. The
//! plan id is derived from that path, so the new plan inherits the old one's
//! entire run history; the old task rows still carry the lines they sat on;
//! those lines still land inside the new file; and nothing matches by text.
//! Position alone then binds old runs to whichever unrelated new task falls
//! within the window, and a plan nobody has started reads as mostly accepted.
//!
//! This module does not try to detect that, because it cannot: at resolve time a
//! deleted task and a rewritten one leave exactly the same trace. The
//! distinction is drawn where it is observable — at sync time, by comparing the
//! file against what it said before — and recorded per task in the store. A task
//! whose plan was replaced stops being offered a `stored_line_hint` at all, so
//! the positional arm below simply never fires for it.
//!
//! That is why `stored_line_hint` is an `Option` the caller supplies rather than
//! something looked up here: whether a task's line is still evidence is a fact
//! about the plan's history, which the store owns. See
//! `loopfleet_store::task_line_hints`.

use crate::plan::{legacy_anchor_for, ParsedTask};

/// Which signal carried a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    /// The stored anchor is the task's current anchor.
    Exact,
    /// The stored anchor is an older derivation of this task's text.
    LegacyText,
    /// Text matched nothing; the anchor's last known line did.
    Position,
}

/// One run anchor resolved to a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    /// Index into the parsed task list.
    pub task_index: usize,
    pub kind: MatchKind,
    /// Position pointed at a different task than text did. Text was taken; this
    /// flags that the two signals no longer agree.
    pub position_disagreed: bool,
}

/// Resolve one stored anchor against a plan's freshly parsed tasks.
///
/// `stored_line_hint` is the line recorded for this anchor's task row, if one is
/// still stored *and still counts as evidence* — see the module docs. `None`
/// skips the positional signal entirely, which is how a run whose plan was
/// replaced is kept from binding to an unrelated task.
pub fn resolve(
    tasks: &[ParsedTask],
    stored_anchor: &str,
    stored_line_hint: Option<u32>,
) -> Option<Resolution> {
    let by_position = stored_line_hint.and_then(|line| nearest_task(tasks, line));
    let by_text = match_text(tasks, stored_anchor);

    if let Some((task_index, kind)) = by_text {
        return Some(Resolution {
            task_index,
            kind,
            position_disagreed: by_position.is_some_and(|p| p != task_index),
        });
    }
    by_position.map(|task_index| Resolution {
        task_index,
        kind: MatchKind::Position,
        position_disagreed: false,
    })
}

/// The text signal: the stored anchor as the task's current anchor, else as an
/// older derivation of its text. Both text arms in one place, so "matches by
/// text" has a single definition.
fn match_text(tasks: &[ParsedTask], anchor: &str) -> Option<(usize, MatchKind)> {
    match_exact(tasks, anchor)
        .map(|i| (i, MatchKind::Exact))
        .or_else(|| match_legacy_text(tasks, anchor).map(|i| (i, MatchKind::LegacyText)))
}

/// The stored anchor is the task's current anchor.
fn match_exact(tasks: &[ParsedTask], anchor: &str) -> Option<usize> {
    tasks
        .iter()
        .position(|t| t.anchor.normalized_text == anchor)
}

/// Whether `stored_anchor` is an older derivation of this task's text: the
/// whole-text form, or a prefix of it (anchors stored before wrapped lines were
/// joined held only a task's first physical line).
///
/// The single definition of "same task, older anchor" — callers that can see the
/// whole task list should prefer [`resolve`], which also rejects ambiguity.
pub fn is_legacy_form_of(stored_anchor: &str, task_text: &str) -> bool {
    !stored_anchor.is_empty() && legacy_anchor_for(task_text).starts_with(stored_anchor)
}

/// The stored anchor is an older derivation of exactly one task's text.
/// Ambiguity yields nothing — a guess here would attach a run's history to the
/// wrong task, which is worse than losing it.
fn match_legacy_text(tasks: &[ParsedTask], anchor: &str) -> Option<usize> {
    let mut found = None;
    for (i, t) in tasks.iter().enumerate() {
        if is_legacy_form_of(anchor, &t.text) {
            if found.is_some() {
                return None;
            }
            found = Some(i);
        }
    }
    found
}

/// How far a task may have moved and still be recognized by position alone.
///
/// Position is a weak signal: with no bound, the nearest task to a *deleted*
/// task's line is simply whichever one survived, and the run's history would be
/// misattributed to unrelated work. A rewritten task, or one displaced by an
/// edit just above it, stays within a few lines; past that, proximity is an
/// accident of ordering rather than evidence. Losing a binding is recoverable
/// and visible; a wrong one is neither.
const POSITION_WINDOW_LINES: u32 = 12;

/// The single task within [`POSITION_WINDOW_LINES`] of `line`. A tie between two
/// equally near tasks is no answer, for the same reason ambiguous text is no
/// answer.
fn nearest_task(tasks: &[ParsedTask], line: u32) -> Option<usize> {
    let distance = |t: &ParsedTask| t.anchor.line_hint.abs_diff(line);
    let best = tasks.iter().map(distance).min()?;
    if best > POSITION_WINDOW_LINES {
        return None;
    }
    let mut at_best = tasks
        .iter()
        .enumerate()
        .filter(|(_, t)| distance(t) == best);
    let (i, _) = at_best.next()?;
    if at_best.next().is_some() {
        return None;
    }
    Some(i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::parse_plan;

    /// Two wrapped tasks in the authored bold-imperative-plus-rationale shape.
    fn plan() -> Vec<ParsedTask> {
        parse_plan(
            "# P\n\
             - [ ] **Add the widget to\n  the registry.**\n  Rationale one.\n\n\
             - [ ] **Remove the gadget.**\n  Rationale two.\n",
        )
        .tasks
    }

    #[test]
    fn exact_anchor_binds() {
        let tasks = plan();
        let r = resolve(&tasks, "add the widget to the registry.", None).unwrap();
        assert_eq!(r.task_index, 0);
        assert_eq!(r.kind, MatchKind::Exact);
        assert!(!r.position_disagreed);
    }

    #[test]
    fn whole_text_anchor_still_binds() {
        // What runs launched between the wrapped-line fix and the narrowing
        // were stored against: bold markers, rationale and all.
        let tasks = plan();
        let stored = "**add the widget to the registry.** rationale one.";
        let r = resolve(&tasks, stored, None).unwrap();
        assert_eq!(r.task_index, 0);
        assert_eq!(r.kind, MatchKind::LegacyText);
    }

    #[test]
    fn first_physical_line_anchor_still_binds() {
        // What runs launched before wrapped lines were joined were stored
        // against: the bullet truncated at its first line.
        let tasks = plan();
        let r = resolve(&tasks, "**add the widget to", None).unwrap();
        assert_eq!(r.task_index, 0);
        assert_eq!(r.kind, MatchKind::LegacyText);
    }

    #[test]
    fn ambiguous_legacy_prefix_binds_to_nothing() {
        // A prefix shared by two tasks is not evidence for either.
        let tasks = parse_plan("- [ ] **Add the widget.**\n- [ ] **Add the gadget.**\n").tasks;
        assert!(resolve(&tasks, "**add the", None).is_none());
    }

    #[test]
    fn position_recovers_an_anchor_text_cannot_place() {
        // Task fully reworded: no text signal survives, but its line does.
        let tasks = plan();
        let r = resolve(&tasks, "nothing like any task here", Some(6)).unwrap();
        assert_eq!(r.task_index, 1);
        assert_eq!(r.kind, MatchKind::Position);
    }

    #[test]
    fn text_wins_over_position_and_reports_the_disagreement() {
        let tasks = plan();
        // Text says task 0; the stored line points at task 1.
        let r = resolve(&tasks, "add the widget to the registry.", Some(6)).unwrap();
        assert_eq!(r.task_index, 0);
        assert_eq!(r.kind, MatchKind::Exact);
        assert!(r.position_disagreed);
    }

    #[test]
    fn agreeing_signals_are_not_a_disagreement() {
        let tasks = plan();
        let r = resolve(&tasks, "add the widget to the registry.", Some(2)).unwrap();
        assert!(!r.position_disagreed);
    }

    #[test]
    fn no_signal_at_all_resolves_to_nothing() {
        let tasks = plan();
        assert!(resolve(&tasks, "unrelated anchor", None).is_none());
    }

    #[test]
    fn equidistant_tasks_are_no_positional_answer() {
        // Line 4 sits exactly between the two tasks' lines (2 and 6).
        let tasks = plan();
        assert!(resolve(&tasks, "unrelated anchor", Some(4)).is_none());
    }

    #[test]
    fn a_distant_line_is_not_positional_evidence() {
        // The task this run belonged to was deleted; the only survivor is far
        // away. Stranding the run beats attributing it to unrelated work.
        let tasks = plan();
        assert!(resolve(&tasks, "unrelated anchor", Some(400)).is_none());
    }

    #[test]
    fn empty_task_list_resolves_to_nothing() {
        assert!(resolve(&[], "anything", Some(1)).is_none());
    }
}
