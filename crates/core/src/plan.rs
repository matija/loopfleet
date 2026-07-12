//! Plan parsing (PRD "Plans"): deterministic, no inference.
//!
//! A plan is a markdown file — either `PRD.md` at the repo root (the zero-config
//! convention) or a `.md` file under a `plans/` folder. The parser extracts a
//! title (first H1) and the task list (markdown checkboxes). Each task carries a
//! `{ normalized_text, line_hint }` anchor whose **identity is the normalized
//! text**; the line is a hint/tiebreaker, never the key (PRD: Plans, Data model).
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
            Ok(if path.is_file() { vec![path] } else { Vec::new() })
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
    let mut tasks = Vec::new();
    let mut title = None;
    let mut in_fence = false;

    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        // Toggle on fenced code blocks so a checkbox shown inside an example
        // (```- [ ] …```) is never mistaken for a real task.
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if title.is_none() {
            if let Some(rest) = trimmed.strip_prefix("# ") {
                title = Some(rest.trim().to_string());
            }
        }
        if let Some((checked, text)) = parse_checkbox(line) {
            tasks.push(ParsedTask {
                anchor: TaskAnchor {
                    normalized_text: normalize(text),
                    // 1-based: humans and editors count lines from 1.
                    line_hint: (i + 1) as u32,
                },
                text: text.to_string(),
                checked,
            });
        }
    }

    ParsedPlan { title, tasks }
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

/// Normalize task text into its stable identity: trim, collapse internal
/// whitespace to single spaces, and lowercase. Resilient to whitespace/case
/// edits so a run's binding survives cosmetic changes to the task line.
fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
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
    fn parses_the_repo_prd() {
        // Real-world round-trip against this repo's own PRD.md: the parser must
        // survive contact with the actual authored plan. Asserts structure that
        // holds across PRD edits, not specific task text.
        let prd = concat!(env!("CARGO_MANIFEST_DIR"), "/../../PRD.md");
        let plan = parse_plan_file(Path::new(prd)).unwrap();
        assert!(plan.title.as_deref().unwrap().contains("Workbench UI"));
        assert!(!plan.tasks.is_empty());
        assert!(plan.tasks.iter().any(|t| t.checked));
        assert!(plan.tasks.iter().any(|t| !t.checked));
    }
}
