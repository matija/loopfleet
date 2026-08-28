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

/// Subjects longer than this are cut, with the overflow pushed into the body,
/// matching the conventional git commit subject length.
const MAX_SUBJECT_CHARS: usize = 72;

/// Build the squash-commit message for `run_id`, run by `agent` against
/// `task_text`, summarized by `summary`, having taken `pass_count` passes.
///
/// The subject text falls back through `summary`, then the first line of
/// `task_text`, then `Apply loopfleet run <short-id>` — whichever is first to
/// give something non-empty — since a run can finish without the agent ever
/// producing a usable summary. Whatever is chosen is trimmed to a single line
/// (a summary or task first line can itself carry embedded newlines) before
/// the agent/pass-count parenthetical is appended and the whole subject is
/// capped at [`MAX_SUBJECT_CHARS`]; anything past the cap is pushed into the
/// body rather than silently dropped. `task_text` is reproduced verbatim as
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
    let chosen = first_line(summary)
        .or_else(|| first_line(task_text).map(|line| strip_plan_syntax(&line)))
        .unwrap_or_else(|| format!("Apply loopfleet run {}", short_id(run_id)));

    let suffix = if pass_count > 1 {
        format!(" ({agent}, {pass_count} passes)")
    } else {
        format!(" ({agent})")
    };

    let (subject, overflow) = cap_subject(&format!("{chosen}{suffix}"), MAX_SUBJECT_CHARS);

    let mut body = String::new();
    if let Some(overflow) = overflow {
        body.push_str(&overflow);
        body.push_str("\n\n");
    }
    body.push_str("Task: ");
    body.push_str(task_text.trim());

    format!("{subject}\n\n{body}\n\nloopfleet-run: {run_id}")
}

/// The first non-blank line of `text`, trimmed; `None` when `text` has no
/// non-whitespace content at all.
fn first_line(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.lines().next().unwrap_or(trimmed).trim().to_string())
}

/// Strip Markdown plan syntax from a task-derived subject line: a leading
/// bullet marker (`-`/`*`/`+`), a leading `[ ]`/`[x]`/`[X]` checkbox, and
/// `**` wrapping the whole line. A raw plan line still carries the structure
/// the plan file needs (list marker, checkbox state, emphasis); a commit
/// subject just needs the sentence.
fn strip_plan_syntax(text: &str) -> String {
    let after_bullet = text
        .strip_prefix("- ")
        .or_else(|| text.strip_prefix("* "))
        .or_else(|| text.strip_prefix("+ "))
        .map(|rest| rest.trim_start())
        .unwrap_or(text);

    let after_checkbox = strip_checkbox_marker(after_bullet).unwrap_or(after_bullet);

    let unwrapped = after_checkbox
        .strip_prefix("**")
        .and_then(|rest| rest.strip_suffix("**"))
        .filter(|inner| !inner.is_empty())
        .unwrap_or(after_checkbox);

    unwrapped.trim().to_string()
}

/// Strips a leading `[ ]`/`[x]`/`[X]` checkbox marker, returning the
/// remaining text trimmed of the space that follows it. `None` when `text`
/// doesn't start with a recognized checkbox.
fn strip_checkbox_marker(text: &str) -> Option<&str> {
    let inner = text.strip_prefix('[')?;
    let state = inner.chars().next()?;
    let after_box = inner[state.len_utf8()..].strip_prefix(']')?;
    matches!(state, ' ' | 'x' | 'X').then(|| after_box.trim_start())
}

/// The first 8 characters of `run_id` (or the whole id if it is shorter),
/// matching the short-sha convention used elsewhere for identifying a run.
fn short_id(run_id: &str) -> &str {
    match run_id.char_indices().nth(8) {
        Some((i, _)) => &run_id[..i],
        None => run_id,
    }
}

/// Splits `subject` at `max_chars`, returning the (possibly untouched) head
/// and, when a cut was made, the trimmed remainder to fold into the body.
/// The cut backs up to the nearest preceding space so a word (e.g. the
/// `(agent)` parenthetical) is never split across the subject and body.
fn cap_subject(subject: &str, max_chars: usize) -> (String, Option<String>) {
    match subject.char_indices().nth(max_chars) {
        Some((limit, _)) => {
            let cut = subject[..limit].rfind(' ').unwrap_or(limit);
            let (head, tail) = subject.split_at(cut);
            (head.to_string(), Some(tail.trim().to_string()))
        }
        None => (subject.to_string(), None),
    }
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

    #[test]
    fn falls_back_to_first_line_of_task_when_summary_is_blank() {
        let msg = compose_commit_message("", "Build the widget\nwith extra care", "abc123", "claude", 1);
        assert_eq!(
            msg,
            "Build the widget (claude)\n\nTask: Build the widget\nwith extra care\n\nloopfleet-run: abc123"
        );
    }

    #[test]
    fn falls_back_to_run_id_when_summary_and_task_are_blank() {
        let msg = compose_commit_message("   ", "   ", "abc1234567890", "claude", 1);
        assert_eq!(
            msg,
            "Apply loopfleet run abc12345 (claude)\n\nTask: \n\nloopfleet-run: abc1234567890"
        );
    }

    #[test]
    fn chosen_subject_is_trimmed_to_a_single_line() {
        let msg = compose_commit_message("Add the widget\nand more", "Build the widget", "abc123", "claude", 1);
        assert_eq!(
            msg,
            "Add the widget (claude)\n\nTask: Build the widget\n\nloopfleet-run: abc123"
        );
    }

    #[test]
    fn strips_plan_syntax_from_task_derived_subject() {
        let msg = compose_commit_message("", "- [ ] **Build the widget**", "abc123", "claude", 1);
        assert_eq!(
            msg,
            "Build the widget (claude)\n\nTask: - [ ] **Build the widget**\n\nloopfleet-run: abc123"
        );
    }

    #[test]
    fn strips_bare_bullet_and_checked_box_from_task_derived_subject() {
        let msg = compose_commit_message("", "* [x] Build the widget", "abc123", "claude", 1);
        assert_eq!(
            msg,
            "Build the widget (claude)\n\nTask: * [x] Build the widget\n\nloopfleet-run: abc123"
        );
    }

    #[test]
    fn overlong_subject_is_capped_with_remainder_in_the_body() {
        let summary = "This summary is deliberately long enough to overflow the seventy two character subject cap";
        let full_subject = format!("{summary} (claude)");
        let limit = full_subject.char_indices().nth(72).unwrap().0;
        let cut = full_subject[..limit].rfind(' ').unwrap();
        let (expected_subject, expected_overflow) = full_subject.split_at(cut);

        let msg = compose_commit_message(summary, "Build the widget", "abc123", "claude", 1);

        assert_eq!(
            msg,
            format!(
                "{expected_subject}\n\n{}\n\nTask: Build the widget\n\nloopfleet-run: abc123",
                expected_overflow.trim()
            )
        );
    }

    #[test]
    fn cap_does_not_split_the_agent_parenthetical() {
        // Regression: a subject landing right around the 72-char cap used to
        // slice mid-word, e.g. "...(cla" / "ude)...".
        let summary = "Add pure should_auto_merge decision in crates/core/src/autopilot.rs";
        let msg = compose_commit_message(summary, "Build the widget", "abc123", "claude", 1);
        let subject = msg.lines().next().unwrap();

        assert!(
            !subject.contains("(cla"),
            "subject split mid-word: {subject:?}"
        );
        assert!(subject.ends_with("(claude)") || !subject.contains('('));
    }
}
