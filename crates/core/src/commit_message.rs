//! Composes the squash-commit message for a run: what "use this run" writes
//! when the agent left no commit message of its own to carry forward (see
//! `loopfleet_gitx::merge::squash_message`).
//!
//! The shape is subject, blank line, the bound task's own text as the body (so
//! the commit reads as "what the plan asked for"), blank line, a
//! `loopfleet-run:` trailer identifying the run that produced it — parallel to
//! `loopfleet_gitx::MERGE_COMMIT_TRAILER` but naming the run rather than
//! crediting an author, since that credit is a separate trailer added later by
//! the merge itself.

/// Build the squash-commit message for `run_id`, run by `agent` against
/// `task_text`, summarized by `summary`, having taken `pass_count` passes.
///
/// `summary` becomes the subject line as-is (the caller is responsible for
/// keeping it commit-subject-shaped); `task_text` is reproduced verbatim as
/// the body so the commit says what the plan asked for, not a paraphrase.
/// `pass_count` is folded into the subject as a parenthetical only when more
/// than one pass ran, since a single-pass run reads as unremarkable.
pub fn compose_commit_message(
    summary: &str,
    task_text: &str,
    run_id: &str,
    agent: &str,
    pass_count: u32,
) -> String {
    let summary = summary.trim();
    let subject = if pass_count > 1 {
        format!("{summary} ({agent}, {pass_count} passes)")
    } else {
        format!("{summary} ({agent})")
    };

    format!(
        "{subject}\n\nTask: {task}\n\nloopfleet-run: {run_id}",
        task = task_text.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_subject_body_and_trailer() {
        let msg = compose_commit_message("Add the widget", "Build the widget", "abc123", "claude", 1);
        assert_eq!(
            msg,
            "Add the widget (claude)\n\nTask: Build the widget\n\nloopfleet-run: abc123"
        );
    }

    #[test]
    fn multiple_passes_are_noted_in_the_subject() {
        let msg = compose_commit_message("Add the widget", "Build the widget", "abc123", "claude", 3);
        assert_eq!(
            msg,
            "Add the widget (claude, 3 passes)\n\nTask: Build the widget\n\nloopfleet-run: abc123"
        );
    }

    #[test]
    fn trims_surrounding_whitespace() {
        let msg = compose_commit_message("  Add the widget  ", "  Build the widget  ", "abc123", "claude", 1);
        assert_eq!(
            msg,
            "Add the widget (claude)\n\nTask: Build the widget\n\nloopfleet-run: abc123"
        );
    }
}
