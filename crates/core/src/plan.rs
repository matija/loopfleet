//! Plan parsing (PRD "Plans"): deterministic, no inference.
//!
//! A plan is a markdown file — either `PRD.md` at the repo root (the zero-config
//! convention) or a `.md` file under a `plans/` folder. The parser extracts a
//! title (first H1) and the task list (markdown checkboxes). Each task carries a
//! `{ normalized_text, line_hint }` anchor whose **identity is the normalized
//! bold span** — the imperative that names the task, with trailing rationale
//! prose excluded so rewording it cannot orphan a run's binding. The line is a
//! hint/tiebreaker, never the key (PRD: Plans, Data model).
//!
//! The authored `checked` state is the "implemented" baseline for derived
//! `TaskStatus` (a pre-checked task reads as `Accepted`); it is never a live
//! progress signal. Live per-task state is derived from run records elsewhere,
//! not read from the file. A checked task stays runnable — launching is never
//! gated by it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A parsed plan: its title and ordered task list. Free-form prose is left in
/// the source file (the plan view renders the raw markdown), so it is not
/// modelled here — only the task list is load-bearing for run binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedPlan {
    /// The first level-1 heading (`# …`), if any.
    pub title: Option<String>,
    /// Tasks in document order.
    pub tasks: Vec<ParsedTask>,
}

/// One checkbox task. `text` is the authored display text; `checked` is the
/// authored "implemented" baseline; `anchor` is the stable identity used to
/// bind runs to this task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedTask {
    pub anchor: TaskAnchor,
    pub text: String,
    pub checked: bool,
}

/// A task's identity within a plan. `normalized_text` is the key; `line_hint`
/// (1-based) is a tiebreaker for locating the task in the file, not the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskAnchor {
    pub normalized_text: String,
    pub line_hint: u32,
}

/// Which convention locates the plan file(s) for a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanConvention {
    /// `PRD.md` at the repo root.
    Prd,
    /// `.md` files under a `plans/` folder, one plan per file.
    Folder,
}

impl PlanConvention {
    /// Map the persisted `plan_convention` token (`"prd"` | `"folder"`).
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "prd" => Some(PlanConvention::Prd),
            "folder" => Some(PlanConvention::Folder),
            _ => None,
        }
    }
}

/// Locate the plan file(s) for a repo under the given convention.
///
/// - `Prd`: `<repo>/PRD.md` if it exists (0 or 1 path).
/// - `Folder`: every `*.md` under `<repo>/plans/`, sorted by path for a stable
///   order. A missing folder yields an empty list, not an error.
pub fn discover_plans(repo: &Path, convention: PlanConvention) -> io::Result<Vec<PathBuf>> {
    match convention {
        PlanConvention::Prd => {
            let path = repo.join("PRD.md");
            Ok(if path.is_file() {
                vec![path]
            } else {
                Vec::new()
            })
        }
        PlanConvention::Folder => {
            let dir = repo.join("plans");
            let mut out = Vec::new();
            if dir.is_dir() {
                for entry in fs::read_dir(&dir)? {
                    let path = entry?.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("md") {
                        out.push(path);
                    }
                }
            }
            out.sort();
            Ok(out)
        }
    }
}

/// Read and parse a plan file.
pub fn parse_plan_file(path: &Path) -> io::Result<ParsedPlan> {
    Ok(parse_plan(&fs::read_to_string(path)?))
}

/// Parse plan markdown. Deterministic and side-effect free: the same input
/// always yields the same tasks in the same order.
pub fn parse_plan(content: &str) -> ParsedPlan {
    let lines: Vec<&str> = content.lines().collect();
    let mut tasks = Vec::new();
    let mut title = None;
    let mut in_fence = false;

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        // Toggle on fenced code blocks so a checkbox shown inside an example
        // (```- [ ] …```) is never mistaken for a real task.
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            i += 1;
            continue;
        }
        if in_fence {
            i += 1;
            continue;
        }
        if title.is_none() {
            if let Some(rest) = trimmed.strip_prefix("# ") {
                title = Some(rest.trim().to_string());
            }
        }
        if let Some((checked, first)) = parse_checkbox(line) {
            let line_hint = (i + 1) as u32;
            let mut text = first.to_string();
            // A wrapped list item continues on indented lines with no marker
            // of their own; join them back into one task rather than
            // truncating the task at its first physical line.
            let mut j = i + 1;
            while j < lines.len() {
                let cont = lines[j];
                if cont.trim().is_empty() {
                    break;
                }
                if !cont.starts_with(' ') && !cont.starts_with('\t') {
                    break;
                }
                let cont_trimmed = cont.trim();
                if cont_trimmed.starts_with("```")
                    || cont_trimmed.starts_with("~~~")
                    || cont_trimmed.starts_with('#')
                    || is_list_marker(cont_trimmed)
                {
                    break;
                }
                text.push(' ');
                text.push_str(cont_trimmed);
                j += 1;
            }
            tasks.push(ParsedTask {
                anchor: TaskAnchor {
                    normalized_text: anchor_for(&text),
                    // 1-based: humans and editors count lines from 1.
                    line_hint,
                },
                text,
                checked,
            });
            i = j;
            continue;
        }
        i += 1;
    }

    ParsedPlan { title, tasks }
}

/// Whether a trimmed line starts a new markdown list item (checkbox or
/// plain), which ends the previous item's continuation.
fn is_list_marker(trimmed: &str) -> bool {
    trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("+ ")
        || trimmed == "-"
        || trimmed == "*"
        || trimmed == "+"
}

/// Recognize a markdown checkbox list item: an optional-indent list marker
/// (`-`/`*`/`+`), a `[ ]`/`[x]`/`[X]` box, then non-empty text. Returns
/// `(checked, text)` where `text` is trimmed. Anything else is `None`.
fn parse_checkbox(line: &str) -> Option<(bool, &str)> {
    let body = line.trim_start();
    let after_marker = body
        .strip_prefix("- ")
        .or_else(|| body.strip_prefix("* "))
        .or_else(|| body.strip_prefix("+ "))?
        .trim_start();

    let inner = after_marker.strip_prefix('[')?;
    let state = inner.chars().next()?;
    let after_box = inner[state.len_utf8()..].strip_prefix(']')?;

    let checked = match state {
        ' ' => false,
        'x' | 'X' => true,
        _ => return None,
    };

    let text = after_box.trim();
    if text.is_empty() {
        return None;
    }
    Some((checked, text))
}

/// The identity-bearing slice of a task's text.
///
/// Tasks are authored as a bolded imperative followed by unbolded rationale
/// prose (`- [ ] **Do the thing.** Because reasons.`). Only the bold span names
/// the task; the rationale is commentary the author rewords freely. Taking the
/// bold span alone as the identity keeps a run bound to its task across
/// rationale edits, which would otherwise silently orphan it.
///
/// Falls back to the whole text when there is no leading bold span (a plain
/// task line) or when the span is empty.
fn identity_source(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("**") else {
        return text;
    };
    let Some(end) = rest.find("**") else {
        return text;
    };
    let inner = rest[..end].trim();
    if inner.is_empty() {
        text
    } else {
        inner
    }
}

/// Collapse text to its comparable form: trim, collapse internal whitespace to
/// single spaces, lowercase. Resilient to whitespace/case edits.
fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A task's stable identity, derived from its authored text: the normalized
/// bold span if there is one, else the normalized whole text.
///
/// **Changing this function re-keys every task**, unbinding the runs already
/// stored against the old form. [`legacy_anchor_for`] exists so previously
/// stored anchors stay resolvable; keep it in step.
pub fn anchor_for(text: &str) -> String {
    normalize(identity_source(text))
}

/// The whole-text anchor form, kept for resolving anchors stored before the
/// identity was narrowed to the bold span. Not used for new bindings — see
/// `task_binding`, which matches stored anchors against this and against its
/// prefixes (older anchors held only a task's first physical line).
pub fn legacy_anchor_for(text: &str) -> String {
    normalize(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_checkboxes_with_mixed_states() {
        let plan = parse_plan("- [ ] alpha\n- [x] beta\n* [X] gamma\n");
        assert_eq!(plan.tasks.len(), 3);
        assert_eq!(plan.tasks[0].text, "alpha");
        assert!(!plan.tasks[0].checked);
        assert!(plan.tasks[1].checked);
        assert_eq!(plan.tasks[1].text, "beta");
        // `*` marker and uppercase X both recognized.
        assert!(plan.tasks[2].checked);
        assert_eq!(plan.tasks[2].text, "gamma");
    }

    #[test]
    fn extracts_first_h1_as_title_ignoring_h2() {
        let plan = parse_plan("## sub\n# The Title\n# Second H1\n- [ ] t\n");
        assert_eq!(plan.title.as_deref(), Some("The Title"));
    }

    #[test]
    fn no_title_when_no_h1() {
        assert_eq!(parse_plan("## only h2\n- [ ] t\n").title, None);
    }

    #[test]
    fn anchor_normalizes_and_records_line_hint() {
        // Extra spacing and mixed case collapse to a stable identity; the line
        // hint is the 1-based file line.
        let plan = parse_plan("intro\n\n-   [ ]   Do   The   Thing  \n");
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].anchor.normalized_text, "do the thing");
        assert_eq!(plan.tasks[0].anchor.line_hint, 3);
        // Display text keeps its own casing, only outer-trimmed.
        assert_eq!(plan.tasks[0].text, "Do   The   Thing");
    }

    #[test]
    fn ignores_checkboxes_inside_code_fences() {
        let plan = parse_plan("- [ ] real\n```\n- [ ] fake\n```\n- [x] also real\n");
        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[0].text, "real");
        assert_eq!(plan.tasks[1].text, "also real");
    }

    #[test]
    fn ignores_non_checkbox_list_items_and_empty_boxes() {
        let plan = parse_plan("- plain bullet\n- [] no space\n- [ ]   \n- [ ] good\n");
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].text, "good");
    }

    #[test]
    fn parses_indented_checkboxes() {
        let plan = parse_plan("  - [ ] nested\n");
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].text, "nested");
    }

    #[test]
    fn anchor_is_the_bold_span_not_the_rationale() {
        // The bold imperative names the task; the prose after it is commentary.
        let plan = parse_plan("- [ ] **Do the thing.** Because reasons.\n");
        assert_eq!(plan.tasks[0].anchor.normalized_text, "do the thing.");
        // Display text still carries the whole authored block.
        assert_eq!(plan.tasks[0].text, "**Do the thing.** Because reasons.");
    }

    #[test]
    fn anchor_survives_a_rationale_reword() {
        // The edit that used to orphan a run's binding is now a no-op on identity.
        let before = parse_plan("- [ ] **Do the thing.** Because reasons.\n");
        let after = parse_plan("- [ ] **Do the thing.** Rewritten justification.\n");
        assert_eq!(
            before.tasks[0].anchor.normalized_text,
            after.tasks[0].anchor.normalized_text
        );
    }

    #[test]
    fn anchor_spans_a_wrapped_bold_imperative() {
        let plan = parse_plan(
            "- [ ] **Extend the thing and\n  the other thing with a\n  new field.**\n  Rationale here.\n",
        );
        assert_eq!(
            plan.tasks[0].anchor.normalized_text,
            "extend the thing and the other thing with a new field."
        );
    }

    #[test]
    fn anchor_falls_back_to_whole_text_without_a_bold_span() {
        let plan = parse_plan("- [ ] plain task, no bold\n");
        assert_eq!(plan.tasks[0].anchor.normalized_text, "plain task, no bold");
        // An unterminated or empty span is not an identity either.
        assert_eq!(
            parse_plan("- [ ] **unterminated\n").tasks[0].anchor.normalized_text,
            "**unterminated"
        );
        assert_eq!(
            parse_plan("- [ ] **** just stars\n").tasks[0].anchor.normalized_text,
            "**** just stars"
        );
    }

    #[test]
    fn anchor_ignores_bold_that_does_not_lead() {
        // Only a leading span is the imperative; mid-text emphasis is prose.
        let plan = parse_plan("- [ ] Do the **thing** now.\n");
        assert_eq!(plan.tasks[0].anchor.normalized_text, "do the **thing** now.");
    }

    #[test]
    fn legacy_anchor_keeps_the_whole_text_form() {
        // What older runs were stored against, so they stay resolvable.
        assert_eq!(
            legacy_anchor_for("**Do the thing.** Because reasons."),
            "**do the thing.** because reasons."
        );
    }

    #[test]
    fn joins_wrapped_checkbox_continuation_lines() {
        let plan = parse_plan(
            "- [ ] **Extend the thing and\n  the other thing with a\n  new field.**\n  Rationale here.\n\n**Done when.** stuff\n",
        );
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(
            plan.tasks[0].text,
            "**Extend the thing and the other thing with a new field.** Rationale here."
        );
        assert_eq!(plan.tasks[0].anchor.line_hint, 1);
    }

    #[test]
    fn continuation_stops_at_next_list_item_or_unindented_line() {
        let plan = parse_plan("- [ ] first\n  more first\n- [ ] second\nunindented\n");
        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[0].text, "first more first");
        assert_eq!(plan.tasks[1].text, "second");
    }

    #[test]
    fn discover_prd_finds_root_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(discover_plans(dir.path(), PlanConvention::Prd)
            .unwrap()
            .is_empty());
        fs::write(dir.path().join("PRD.md"), "# P\n- [ ] t\n").unwrap();
        let found = discover_plans(dir.path(), PlanConvention::Prd).unwrap();
        assert_eq!(found, vec![dir.path().join("PRD.md")]);
    }

    #[test]
    fn discover_folder_finds_sorted_md_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let plans = dir.path().join("plans");
        fs::create_dir(&plans).unwrap();
        fs::write(plans.join("b.md"), "").unwrap();
        fs::write(plans.join("a.md"), "").unwrap();
        fs::write(plans.join("notes.txt"), "").unwrap();
        let found = discover_plans(dir.path(), PlanConvention::Folder).unwrap();
        assert_eq!(found, vec![plans.join("a.md"), plans.join("b.md")]);
    }

    #[test]
    fn discover_folder_missing_dir_is_empty_not_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(discover_plans(dir.path(), PlanConvention::Folder)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn convention_from_token() {
        assert_eq!(PlanConvention::from_token("prd"), Some(PlanConvention::Prd));
        assert_eq!(
            PlanConvention::from_token("folder"),
            Some(PlanConvention::Folder)
        );
        assert_eq!(PlanConvention::from_token("nope"), None);
    }

    #[test]
    fn parses_an_authored_prd() {
        // Real-world round-trip against a checked-in copy of an authored PRD:
        // the parser must survive contact with a real plan, not just snippets.
        let prd = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample-prd.md");
        let plan = parse_plan_file(Path::new(prd)).unwrap();
        assert_eq!(
            plan.title.as_deref(),
            Some("PRD: Workbench UI — a panel-style workbench surface for Loopfleet")
        );
        // Only the four items under "Tasks" — the bulleted non-goals carry no
        // checkbox and the fenced example checkbox is not a task.
        assert_eq!(plan.tasks.len(), 4);
        assert!(plan
            .tasks
            .iter()
            .all(|t| !t.anchor.normalized_text.is_empty()));
        assert_eq!(
            plan.tasks.iter().filter(|t| t.checked).count(),
            2,
            "the authored checked baseline is preserved"
        );
        // Wrapped list items are joined into one line of display text.
        assert!(plan.tasks[0].text.starts_with(
            "**View model.** A single `View` union (`overview | plan | task | run | compare`)"
        ));
        assert_eq!(plan.tasks[0].anchor.normalized_text, "view model.");
    }
}
