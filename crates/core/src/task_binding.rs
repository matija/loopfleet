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
//! # Position is admissible per plan, not per anchor
//!
//! Proximity only means anything if the file is still recognizably the plan the
//! runs were stored against. It is not, if the file was replaced wholesale —
//! which is what a plan file at a fixed conventional path (`PRD.md`) undergoes
//! every time one plan is finished and the next is written in its place. The
//! plan id is derived from that path, so the new plan inherits the old one's
//! entire run history; the task rows of every plan that came before it are still
//! stored, still carrying the lines they sat on; and none of the new tasks
//! matches any stored anchor by text. Position alone then binds old runs to
//! whichever unrelated new task happens to sit within the window, and a plan
//! nobody has started reads as almost entirely accepted.
//!
//! So position is gated on evidence that the file is the same plan: at least one
//! anchor *a run was stored against* still matching a task by text. That is what
//! [`PlanBinding`] is for — the decision is made once from the plan's run
//! anchors, and every run is then resolved under it.
//!
//! The evidence has to be the run anchors specifically, not every anchor in the
//! task table. Task rows are re-synced from the current file before any run is
//! resolved, so the table always contains the new plan's own tasks and would
//! always vouch for it. The runs are the only record of what the file used to
//! say.
//!
//! The gate costs the case where every task in a plan was reworded in one edit,
//! which is indistinguishable from a replaced plan and now strands its runs
//! rather than guessing at them. That is the trade this module already declares
//! elsewhere: losing a binding is recoverable and visible, a wrong one is
//! neither.

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

/// One plan's tasks, together with the once-per-plan decision of whether the
/// positional signal may be used against them at all (see the module docs).
///
/// Build one per plan from the anchors its runs are stored against, then
/// resolve each run through it. Resolving anchors one at a time against a bare
/// task list cannot make this decision, which is why there is no such entry
/// point.
pub struct PlanBinding<'a> {
    tasks: &'a [ParsedTask],
    trust_position: bool,
}

impl<'a> PlanBinding<'a> {
    /// Decide whether this file is still the plan `run_anchors` were stored
    /// against — at least one of them still matching a task by text — and admit
    /// the positional signal only if it is.
    ///
    /// `run_anchors` is the `task_anchor` of every run on the plan, in any
    /// order; duplicates are harmless (one match is all the decision needs).
    /// Passing task-table anchors here instead would defeat the gate: those are
    /// re-synced from the file being judged.
    pub fn new<I, S>(tasks: &'a [ParsedTask], run_anchors: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let trust_position = run_anchors
            .into_iter()
            .any(|a| match_text(tasks, a.as_ref()).is_some());
        Self {
            tasks,
            trust_position,
        }
    }

    /// Whether the positional signal is admissible for this plan. False means
    /// no run's anchor matched any task by text, so the file is treated as a
    /// different plan and text is the only signal.
    pub fn trusts_position(&self) -> bool {
        self.trust_position
    }

    /// Resolve one stored anchor against the plan's freshly parsed tasks.
    ///
    /// `stored_line_hint` is the line recorded for this anchor's task row, if
    /// one is still stored — the only positional evidence a run carries. `None`
    /// skips the positional signal, as does a plan that failed the gate.
    pub fn resolve(
        &self,
        stored_anchor: &str,
        stored_line_hint: Option<u32>,
    ) -> Option<Resolution> {
        let by_position = if self.trust_position {
            stored_line_hint.and_then(|line| nearest_task(self.tasks, line))
        } else {
            None
        };
        let by_text = match_text(self.tasks, stored_anchor);

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
}

/// The text signal: the stored anchor as the task's current anchor, else as an
/// older derivation of its text. The two text arms in one place, so the gate and
/// the cascade cannot disagree about what "matches by text" means.
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
/// whole task list should prefer [`PlanBinding::resolve`], which also rejects
/// ambiguity.
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

    /// A binder whose gate is open: one of the plan's own anchors is stored, so
    /// the file is recognizably the same plan and position is admissible. The
    /// starting point for every test that is about the cascade rather than the
    /// gate.
    fn binder(tasks: &[ParsedTask]) -> PlanBinding<'_> {
        PlanBinding::new(tasks, ["add the widget to the registry."])
    }

    #[test]
    fn exact_anchor_binds() {
        let tasks = plan();
        let r = binder(&tasks).resolve("add the widget to the registry.", None).unwrap();
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
        let r = binder(&tasks).resolve(stored, None).unwrap();
        assert_eq!(r.task_index, 0);
        assert_eq!(r.kind, MatchKind::LegacyText);
    }

    #[test]
    fn first_physical_line_anchor_still_binds() {
        // What runs launched before wrapped lines were joined were stored
        // against: the bullet truncated at its first line.
        let tasks = plan();
        let r = binder(&tasks).resolve("**add the widget to", None).unwrap();
        assert_eq!(r.task_index, 0);
        assert_eq!(r.kind, MatchKind::LegacyText);
    }

    #[test]
    fn ambiguous_legacy_prefix_binds_to_nothing() {
        // A prefix shared by two tasks is not evidence for either.
        let tasks = parse_plan("- [ ] **Add the widget.**\n- [ ] **Add the gadget.**\n").tasks;
        assert!(binder(&tasks).resolve("**add the", None).is_none());
    }

    #[test]
    fn position_recovers_an_anchor_text_cannot_place() {
        // Task fully reworded: no text signal survives, but its line does.
        let tasks = plan();
        let r = binder(&tasks).resolve("nothing like any task here", Some(6)).unwrap();
        assert_eq!(r.task_index, 1);
        assert_eq!(r.kind, MatchKind::Position);
    }

    #[test]
    fn text_wins_over_position_and_reports_the_disagreement() {
        let tasks = plan();
        // Text says task 0; the stored line points at task 1.
        let r = binder(&tasks).resolve("add the widget to the registry.", Some(6)).unwrap();
        assert_eq!(r.task_index, 0);
        assert_eq!(r.kind, MatchKind::Exact);
        assert!(r.position_disagreed);
    }

    #[test]
    fn agreeing_signals_are_not_a_disagreement() {
        let tasks = plan();
        let r = binder(&tasks).resolve("add the widget to the registry.", Some(2)).unwrap();
        assert!(!r.position_disagreed);
    }

    #[test]
    fn no_signal_at_all_resolves_to_nothing() {
        let tasks = plan();
        assert!(binder(&tasks).resolve("unrelated anchor", None).is_none());
    }

    #[test]
    fn equidistant_tasks_are_no_positional_answer() {
        // Line 4 sits exactly between the two tasks' lines (2 and 6).
        let tasks = plan();
        assert!(binder(&tasks).resolve("unrelated anchor", Some(4)).is_none());
    }

    #[test]
    fn a_distant_line_is_not_positional_evidence() {
        // The task this run belonged to was deleted; the only survivor is far
        // away. Stranding the run beats attributing it to unrelated work.
        let tasks = plan();
        assert!(binder(&tasks).resolve("unrelated anchor", Some(400)).is_none());
    }

    #[test]
    fn empty_task_list_resolves_to_nothing() {
        assert!(PlanBinding::new(&[], ["anything"]).resolve("anything", Some(1)).is_none());
    }

    // --- the per-plan gate on position ---

    #[test]
    fn one_surviving_text_match_opens_the_gate_for_the_rest() {
        // A plan being edited: one task reworded, the other untouched. The
        // untouched anchor is the evidence that this is still the same plan, so
        // the reworded one is recovered by position.
        let tasks = plan();
        let b = PlanBinding::new(
            &tasks,
            ["add the widget to the registry.", "the old wording"],
        );
        assert!(b.trusts_position());
        let r = b.resolve("the old wording", Some(6)).unwrap();
        assert_eq!(r.task_index, 1);
        assert_eq!(r.kind, MatchKind::Position);
    }

    #[test]
    fn a_replaced_plan_closes_the_gate_and_strands_its_runs() {
        // The file at this path is a different plan: nothing stored matches any
        // task by text. Every stored anchor's line still lands inside the new
        // plan, and none of them may be used.
        let tasks = plan();
        let stored = ["a task from the previous prd", "another one"];
        let b = PlanBinding::new(&tasks, stored);
        assert!(!b.trusts_position());
        for anchor in stored {
            assert!(b.resolve(anchor, Some(2)).is_none());
            assert!(b.resolve(anchor, Some(6)).is_none());
        }
    }

    #[test]
    fn a_closed_gate_still_resolves_by_text() {
        // The gate only withdraws the positional signal. An anchor that does
        // match by text is unaffected — including the one that would have
        // opened the gate had it been passed to the constructor.
        let tasks = plan();
        let b = PlanBinding::new(&tasks, ["nothing matches this"]);
        assert!(!b.trusts_position());
        let r = b.resolve("add the widget to the registry.", Some(6)).unwrap();
        assert_eq!(r.task_index, 0);
        assert_eq!(r.kind, MatchKind::Exact);
        // Position is not consulted at all, so it cannot be said to disagree.
        assert!(!r.position_disagreed);
    }

    #[test]
    fn no_runs_at_all_closes_the_gate() {
        // A plan with no runs yet has nothing to vouch for the file's identity.
        // Nothing is resolved against it either, so this is a statement about
        // the default rather than a behaviour anyone observes.
        let tasks = plan();
        let b = PlanBinding::new(&tasks, Vec::<String>::new());
        assert!(!b.trusts_position());
    }

    #[test]
    fn the_gate_accepts_legacy_text_as_evidence() {
        // A plan whose anchors all drifted to an older derivation is still the
        // same plan — the legacy arm vouches for it just as the exact one does.
        let tasks = plan();
        let b = PlanBinding::new(&tasks, ["**add the widget to"]);
        assert!(b.trusts_position());
    }
}
