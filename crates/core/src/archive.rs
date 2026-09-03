//! Names an archived plan file.
//!
//! Archiving a finished plan moves it out of the way under a name a human can
//! read a year later. The convention is the one the seventeen names already in
//! `prds/` follow — `quiet-cockpit`, `worktree-retention`,
//! `release-profiles-compositing-budget`, `ship-forget-nothing-cleanup`: a
//! two-to-five-word kebab slug taken from the plan's own title, with no date
//! and no sequence number, since the plan's git history already records when
//! it landed and a number says nothing about what the plan was for.
//!
//! [`proposed_archive_name`] is pure: it proposes, and the caller decides
//! whether to write. The `taken` list is what keeps two plans with the same
//! title from colliding.
//!
//! [`archive_plan_preview`] is the read-only counterpart a confirmation dialog
//! calls before any of that happens: it gathers the proposal plus the counts
//! the dialog states as consequences, mirroring `project_removal_preview`.

use std::fs;
use std::path::Path;

use loopfleet_store::Connection;
use serde::Serialize;

use crate::plan::parse_plan;

/// Words that carry no meaning in a two-to-five-word slug, dropped so the
/// budget is spent on the words that identify the plan.
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "of", "for", "to", "in", "on", "with",
];

/// The most words a slug keeps. Past five the name stops being scannable and
/// starts being a sentence.
const MAX_WORDS: usize = 5;

/// The most characters a slug keeps, before `.md`. Cuts land on a word
/// boundary so the tail is a whole word rather than a fragment.
const MAX_CHARS: usize = 48;

/// The slug for a plan whose title gives back no usable name: missing (no
/// H1 — legal input, since the parser allows it), blank, made entirely of
/// punctuation, made entirely of [`STOPWORDS`], or a single word too long to
/// slug into anything but a truncated fragment.
const FALLBACK: &str = "plan";

/// The file name (with `.md`) an archived plan titled `title` should take,
/// given the names already `taken` in the archive.
///
/// The title is lowercased, stripped of a leading `PRD:` / `PRD -` prefix (the
/// plan file announces itself as a PRD; the archive folder already says that),
/// split on runs of non-alphanumerics, filtered of [`STOPWORDS`], and cut to
/// [`MAX_WORDS`] words and [`MAX_CHARS`] characters. A title with no title at
/// all (no H1, which the plan parser allows), or nothing left once
/// punctuation and stopwords are stripped, or nothing but a single word too
/// long to cut at a word boundary, has no name to give — it archives as
/// [`FALLBACK`] rather than as an empty, hyphen-only, or unreadably truncated
/// name.
///
/// When the result is already `taken`, a `-2`, `-3`, … discriminator is
/// appended until the name is free — matched case-insensitively, since the
/// archive may live on a case-insensitive filesystem where `Quiet-Cockpit.md`
/// and `quiet-cockpit.md` are the same file. The discriminator is applied
/// after the length trim, and the slug gives back the characters it needs, so
/// even a disambiguated name stays within [`MAX_CHARS`]. The returned name is
/// therefore one the caller can accept as it stands.
pub fn proposed_archive_name(title: Option<&str>, taken: &[String]) -> String {
    let slug = slugify(title.unwrap_or_default());
    let mut candidate = format!("{slug}.md");

    let mut n = 1;
    while is_taken(&candidate, taken) {
        n += 1;
        candidate = format!("{}.md", with_discriminator(&slug, n));
    }
    candidate
}

/// `slug` carrying a `-n` discriminator, cut back far enough that the whole
/// stem still fits [`MAX_CHARS`]. The discriminator is the part that makes the
/// name free, so the slug yields to it rather than the other way round.
fn with_discriminator(slug: &str, n: usize) -> String {
    let suffix = format!("-{n}");
    let budget = MAX_CHARS.saturating_sub(suffix.chars().count());
    format!("{}{suffix}", truncate_at_word(slug, budget))
}

/// The kebab slug for `title`, or [`FALLBACK`] when nothing survives.
fn slugify(title: &str) -> String {
    let lowered = title.to_lowercase();
    let stripped = strip_prd_prefix(lowered.trim());

    let words: Vec<&str> = stripped
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    let kept: Vec<&str> = words
        .iter()
        .copied()
        .filter(|w| !STOPWORDS.contains(w))
        .collect();

    // No title, all punctuation, or all stopwords: nothing survives to name
    // the plan with.
    if kept.is_empty() {
        return FALLBACK.to_string();
    }
    // A single word too long to cut at a word boundary would truncate to an
    // arbitrary, unreadable fragment — fall back instead of publishing that.
    if kept.len() == 1 && kept[0].chars().count() > MAX_CHARS {
        return FALLBACK.to_string();
    }

    let joined = kept
        .into_iter()
        .take(MAX_WORDS)
        .collect::<Vec<_>>()
        .join("-");

    truncate_at_word(&joined, MAX_CHARS)
}

/// Strips a leading `prd:` / `prd -` (or bare `prd `) prefix from an already
/// lowercased title, returning the rest trimmed.
fn strip_prd_prefix(title: &str) -> &str {
    let Some(rest) = title.strip_prefix("prd") else {
        return title;
    };
    let trimmed = rest.trim_start();
    match trimmed
        .strip_prefix(':')
        .or_else(|| trimmed.strip_prefix('-'))
    {
        Some(after) => after.trim_start(),
        // A bare `PRD ` prefix, but not a title that merely starts with the
        // letters (`prdx`, or `prd` alone, which would strip to nothing).
        None if rest.starts_with(char::is_whitespace) && !trimmed.is_empty() => trimmed,
        None => title,
    }
}

/// Cuts `slug` to at most `max_chars`, backing up to the preceding hyphen so
/// the cut lands between words rather than inside one. A first word longer
/// than the budget is cut hard — there is no boundary to back up to.
fn truncate_at_word(slug: &str, max_chars: usize) -> String {
    let Some((limit, _)) = slug.char_indices().nth(max_chars) else {
        return slug.to_string();
    };
    let head = &slug[..limit];
    match head.rfind('-') {
        Some(cut) => head[..cut].to_string(),
        None => head.to_string(),
    }
}

/// Whether `candidate` collides with an already-archived name, compared
/// case-insensitively.
fn is_taken(candidate: &str, taken: &[String]) -> bool {
    taken.iter().any(|name| name.to_lowercase() == candidate)
}

/// Validates `name` as an archive file name: a `.md` file made of lowercase
/// alphanumerics and single interior hyphens, nothing else.
///
/// The name is user-editable and ends up in a filesystem path, so it is
/// untrusted input — this is the check both the archive write path and the
/// frontend's name field run before letting the name through. Each rejection
/// names the specific thing wrong with the input, since a generic "invalid
/// name" leaves the user guessing which character to fix.
pub fn valid_archive_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name is empty".to_string());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("name contains a path separator".to_string());
    }
    if name.contains("..") {
        return Err("name contains \"..\"".to_string());
    }
    let Some(stem) = name.strip_suffix(".md") else {
        return Err("name must end in \".md\"".to_string());
    };
    if stem.is_empty() {
        return Err("name is empty".to_string());
    }
    if stem.starts_with('-') {
        return Err("name starts with a hyphen".to_string());
    }
    if stem.ends_with('-') {
        return Err("name ends with a hyphen".to_string());
    }
    if stem.contains("--") {
        return Err("name contains consecutive hyphens".to_string());
    }
    if let Some(c) = stem
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
    {
        return Err(format!("name contains an invalid character: {c:?}"));
    }
    Ok(())
}

/// What an archive confirmation dialog needs to state the consequences of
/// archiving a plan: its title, current path, the name it would be given, the
/// directory it would land in, and how much it would carry along.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArchivePlanPreview {
    pub title: Option<String>,
    pub file_path: String,
    /// What [`proposed_archive_name`] gives back for this title, checked
    /// against `destination_dir`'s current `.md` entries.
    pub proposed_file_name: String,
    pub destination_dir: String,
    pub destination_exists: bool,
    pub task_count: usize,
    pub run_count: usize,
}

/// Why an archive preview could not be built.
#[derive(Debug)]
pub enum ArchivePreviewError {
    /// No plan is synced under this id.
    UnknownPlan(String),
    /// Reading the plan file, or listing the destination directory, failed.
    Io(std::io::Error),
    /// Reading the plan's stored state failed.
    Store(rusqlite::Error),
}

impl std::fmt::Display for ArchivePreviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchivePreviewError::UnknownPlan(id) => write!(f, "unknown plan: {id}"),
            ArchivePreviewError::Io(e) => write!(f, "reading plan: {e}"),
            ArchivePreviewError::Store(e) => write!(f, "reading plan store state: {e}"),
        }
    }
}

impl std::error::Error for ArchivePreviewError {}

/// Gather what an archive confirmation dialog shows for `plan_id`, read-only:
/// the plan's title (re-parsed from its current file, not the last sync) and
/// current path, the name [`proposed_archive_name`] would give it — checked
/// against the `.md` files already sitting in the destination directory — that
/// directory's path and whether it exists yet, and the plan's task and run
/// counts.
///
/// The destination directory is `prds/` alongside the plan file, the
/// convention [`proposed_archive_name`]'s doc names. A directory that doesn't
/// exist yet has no entries to collide with, so `taken` is empty rather than
/// an error — the write path this previews is what creates it.
pub fn archive_plan_preview(
    conn: &Connection,
    plan_id: &str,
) -> Result<ArchivePlanPreview, ArchivePreviewError> {
    let file_path = loopfleet_store::plan_file_path(conn, plan_id)
        .map_err(ArchivePreviewError::Store)?
        .ok_or_else(|| ArchivePreviewError::UnknownPlan(plan_id.to_string()))?;

    let markdown = fs::read_to_string(&file_path).map_err(ArchivePreviewError::Io)?;
    let title = parse_plan(&markdown).title;

    let destination_dir = Path::new(&file_path)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("prds");
    let destination_exists = destination_dir.is_dir();

    let taken: Vec<String> = if destination_exists {
        fs::read_dir(&destination_dir)
            .map_err(ArchivePreviewError::Io)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".md"))
            .collect()
    } else {
        Vec::new()
    };

    let proposed_file_name = proposed_archive_name(title.as_deref(), &taken);

    let task_count = loopfleet_store::task_count_for_plan(conn, plan_id)
        .map_err(ArchivePreviewError::Store)?;
    let run_count = loopfleet_store::run_count_for_plan(conn, plan_id)
        .map_err(ArchivePreviewError::Store)?;

    Ok(ArchivePlanPreview {
        title,
        file_path,
        proposed_file_name,
        destination_dir: destination_dir.to_string_lossy().into_owned(),
        destination_exists,
        task_count,
        run_count,
    })
}

/// Why an archive write was rejected.
#[derive(Debug)]
pub enum ArchivePlanError {
    /// The proposed file name failed [`valid_archive_name`].
    InvalidName(String),
    /// No plan is synced under this id.
    UnknownPlan(String),
    /// The plan still has a run `queued` or `running` — archiving would move
    /// the file out from under it.
    ActiveRuns(String),
    /// A file already sits at the destination path; archiving never
    /// overwrites.
    DestinationExists(String),
    /// The plan's file does not resolve to a path inside its project's repo.
    OutsideRepo(String),
    /// Creating `prds/`, moving the file, or re-reading its parent failed.
    Io(std::io::Error),
    /// Reading or writing the plan's stored state failed.
    Store(rusqlite::Error),
}

impl std::fmt::Display for ArchivePlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchivePlanError::InvalidName(msg) => write!(f, "invalid archive name: {msg}"),
            ArchivePlanError::UnknownPlan(id) => write!(f, "unknown plan: {id}"),
            ArchivePlanError::ActiveRuns(id) => {
                write!(f, "plan {id} has runs still queued or running")
            }
            ArchivePlanError::DestinationExists(path) => {
                write!(f, "archive destination already exists: {path}")
            }
            ArchivePlanError::OutsideRepo(path) => {
                write!(f, "plan file is outside its project's repo: {path}")
            }
            ArchivePlanError::Io(e) => write!(f, "moving plan file: {e}"),
            ArchivePlanError::Store(e) => write!(f, "updating plan store state: {e}"),
        }
    }
}

impl std::error::Error for ArchivePlanError {}

/// Move `plan_id`'s file into `<repo>/prds/<file_name>` and re-key the plan's
/// store row to the new path, returning the absolute archived path.
///
/// `file_name` is validated with [`valid_archive_name`] before anything else
/// runs, since it is untrusted input that ends up in a filesystem path — it
/// crosses the IPC boundary from an editable field, so a validated name is
/// not enough on its own: the destination is always built by joining that
/// name onto `<repo_path>/prds`, taken from the project's own row, never from
/// a directory the caller supplies. The plan's current file path is checked
/// against that same `repo_path` before anything moves, so a plan whose
/// stored path has drifted outside its project's repo is refused rather than
/// moved further astray. A plan with a run still `queued` or `running` is
/// rejected outright — the file must not move out from under an in-flight
/// run — and so is a destination that already exists, since archiving never
/// overwrites.
///
/// The file is renamed *before* the store is re-keyed: a failed rename
/// leaves the store pointing at a file that is still exactly where it was,
/// consistent with disk. A failed re-key after a successful rename leaves a
/// moved file the store doesn't know about yet, which the next overview sync
/// simply fails to discover — recoverable, unlike the reverse order, where a
/// re-key ahead of a failed rename would leave the store pointing at a path
/// nothing lives at.
pub fn archive_plan(
    conn: &Connection,
    plan_id: &str,
    file_name: &str,
) -> Result<String, ArchivePlanError> {
    valid_archive_name(file_name).map_err(ArchivePlanError::InvalidName)?;

    let file_path = loopfleet_store::plan_file_path(conn, plan_id)
        .map_err(ArchivePlanError::Store)?
        .ok_or_else(|| ArchivePlanError::UnknownPlan(plan_id.to_string()))?;

    if loopfleet_store::has_active_runs_for_plan(conn, plan_id).map_err(ArchivePlanError::Store)? {
        return Err(ArchivePlanError::ActiveRuns(plan_id.to_string()));
    }

    let project_id = loopfleet_store::project_id_for_plan(conn, plan_id)
        .map_err(ArchivePlanError::Store)?
        .ok_or_else(|| ArchivePlanError::UnknownPlan(plan_id.to_string()))?;

    let repo_path = loopfleet_store::repo_path_for_project(conn, &project_id)
        .map_err(ArchivePlanError::Store)?
        .ok_or_else(|| ArchivePlanError::UnknownPlan(plan_id.to_string()))?;

    let source = Path::new(&file_path);
    let repo = Path::new(&repo_path);
    if !source.starts_with(repo) {
        return Err(ArchivePlanError::OutsideRepo(file_path));
    }

    let destination_dir = repo.join("prds");
    let destination = destination_dir.join(file_name);

    if destination.exists() {
        return Err(ArchivePlanError::DestinationExists(
            destination.to_string_lossy().into_owned(),
        ));
    }

    fs::create_dir_all(&destination_dir).map_err(ArchivePlanError::Io)?;
    fs::rename(source, &destination).map_err(ArchivePlanError::Io)?;

    let new_path = destination.to_string_lossy().into_owned();
    let new_id = loopfleet_store::plan_id(&project_id, &new_path);
    loopfleet_store::rekey_plan(conn, plan_id, &new_id, &new_path)
        .map_err(ArchivePlanError::Store)?;

    Ok(new_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(title: &str) -> String {
        proposed_archive_name(Some(title), &[])
    }

    #[test]
    fn matches_the_names_already_in_prds() {
        assert_eq!(name("The Quiet Cockpit"), "quiet-cockpit.md");
        assert_eq!(name("Worktree retention"), "worktree-retention.md");
        assert_eq!(
            name("Release profiles: compositing budget"),
            "release-profiles-compositing-budget.md"
        );
        assert_eq!(
            name("Ship and forget nothing: cleanup"),
            "ship-forget-nothing-cleanup.md"
        );
    }

    #[test]
    fn strips_a_leading_prd_prefix() {
        assert_eq!(name("PRD: Quiet Cockpit"), "quiet-cockpit.md");
        assert_eq!(name("PRD - Quiet Cockpit"), "quiet-cockpit.md");
        assert_eq!(name("PRD Quiet Cockpit"), "quiet-cockpit.md");
        assert_eq!(name("prd:quiet cockpit"), "quiet-cockpit.md");
    }

    #[test]
    fn keeps_a_title_that_merely_starts_with_the_letters() {
        assert_eq!(name("PRDs we have loved"), "prds-we-have-loved.md");
    }

    #[test]
    fn drops_stopwords() {
        assert_eq!(
            name("A plan for the review of an inbox"),
            "plan-review-inbox.md"
        );
    }

    #[test]
    fn falls_back_when_the_title_is_entirely_stopwords() {
        assert_eq!(name("The and of"), "plan.md");
    }

    #[test]
    fn collapses_runs_of_non_alphanumerics_to_one_hyphen() {
        assert_eq!(name("  Quiet -- cockpit!! (v2)  "), "quiet-cockpit-v2.md");
        assert_eq!(name("quiet_cockpit"), "quiet-cockpit.md");
    }

    #[test]
    fn keeps_at_most_five_words() {
        assert_eq!(
            name("one two three four five six seven"),
            "one-two-three-four-five.md"
        );
    }

    #[test]
    fn trims_to_forty_eight_characters_at_a_word_boundary() {
        let out = name("supervisor reconciliation reattachment persistence budget");
        let slug = out.strip_suffix(".md").unwrap();
        assert!(slug.chars().count() <= MAX_CHARS, "{out:?}");
        assert_eq!(out, "supervisor-reconciliation-reattachment.md");
    }

    #[test]
    fn falls_back_when_the_title_is_one_word_too_long_to_cut_at_a_boundary() {
        let title = "a".repeat(60);
        assert_eq!(name(&title), "plan.md");
    }

    #[test]
    fn falls_back_when_there_is_no_title() {
        assert_eq!(proposed_archive_name(None, &[]), "plan.md");
        assert_eq!(name("   "), "plan.md");
        assert_eq!(name("!!! ???"), "plan.md");
    }

    #[test]
    fn a_titleless_plan_disambiguates_like_any_other_fallback() {
        let taken = vec!["plan.md".to_string()];
        assert_eq!(proposed_archive_name(None, &taken), "plan-2.md");

        let taken = vec!["plan.md".to_string(), "plan-2.md".to_string()];
        assert_eq!(proposed_archive_name(None, &taken), "plan-3.md");
    }

    #[test]
    fn disambiguates_against_taken_names() {
        let taken = vec!["quiet-cockpit.md".to_string()];
        assert_eq!(
            proposed_archive_name(Some("Quiet Cockpit"), &taken),
            "quiet-cockpit-2.md"
        );

        let taken = vec![
            "quiet-cockpit.md".to_string(),
            "quiet-cockpit-2.md".to_string(),
        ];
        assert_eq!(
            proposed_archive_name(Some("Quiet Cockpit"), &taken),
            "quiet-cockpit-3.md"
        );
    }

    #[test]
    fn collisions_are_case_insensitive() {
        let taken = vec!["Quiet-Cockpit.md".to_string()];
        assert_eq!(
            proposed_archive_name(Some("Quiet Cockpit"), &taken),
            "quiet-cockpit-2.md"
        );
    }

    #[test]
    fn counts_up_until_the_name_is_free() {
        let taken: Vec<String> = [
            "quiet-cockpit.md",
            "quiet-cockpit-2.md",
            "quiet-cockpit-3.md",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            proposed_archive_name(Some("Quiet Cockpit"), &taken),
            "quiet-cockpit-4.md"
        );
    }

    #[test]
    fn a_discriminated_name_still_fits_the_character_cap() {
        let title = "Reconciliation supervisor reattachment budgeting";
        assert_eq!(name(title).chars().count(), MAX_CHARS + ".md".len());

        let taken = vec![name(title)];
        let out = proposed_archive_name(Some(title), &taken);
        // The slug gives back the two characters the `-2` needs, and gives
        // them back a whole word at a time.
        assert_eq!(out, "reconciliation-supervisor-reattachment-2.md");
        assert!(out.strip_suffix(".md").unwrap().chars().count() <= MAX_CHARS);
    }

    #[test]
    fn an_overlong_word_falls_back_and_still_disambiguates() {
        let title = "a".repeat(60);
        let taken = vec![name(&title)];
        assert_eq!(
            proposed_archive_name(Some(&title), &taken),
            "plan-2.md"
        );
    }

    #[test]
    fn keeps_counting_when_the_trimmed_name_is_taken_too() {
        let title = "Reconciliation supervisor reattachment budgeting";
        let taken = vec![
            name(title),
            "reconciliation-supervisor-reattachment-2.md".to_string(),
        ];
        assert_eq!(
            proposed_archive_name(Some(title), &taken),
            "reconciliation-supervisor-reattachment-3.md"
        );
    }

    #[test]
    fn unrelated_taken_names_are_ignored() {
        let taken = vec!["worktree-retention.md".to_string()];
        assert_eq!(
            proposed_archive_name(Some("Quiet Cockpit"), &taken),
            "quiet-cockpit.md"
        );
    }

    #[test]
    fn accepts_names_produced_by_proposed_archive_name() {
        assert!(valid_archive_name("quiet-cockpit.md").is_ok());
        assert!(valid_archive_name("plan.md").is_ok());
        assert!(valid_archive_name("plan-2.md").is_ok());
        assert!(valid_archive_name("a.md").is_ok());
        assert!(valid_archive_name("a1-b2.md").is_ok());
    }

    #[test]
    fn rejects_the_empty_string() {
        assert_eq!(valid_archive_name(""), Err("name is empty".to_string()));
    }

    #[test]
    fn rejects_a_path_separator() {
        assert_eq!(
            valid_archive_name("sub/dir.md"),
            Err("name contains a path separator".to_string())
        );
        assert_eq!(
            valid_archive_name("sub\\dir.md"),
            Err("name contains a path separator".to_string())
        );
    }

    #[test]
    fn rejects_dot_dot() {
        assert_eq!(
            valid_archive_name("../etc.md"),
            Err("name contains a path separator".to_string())
        );
        assert_eq!(
            valid_archive_name("..md"),
            Err("name contains \"..\"".to_string())
        );
    }

    #[test]
    fn rejects_a_missing_or_wrong_extension() {
        assert_eq!(
            valid_archive_name("plan"),
            Err("name must end in \".md\"".to_string())
        );
        assert_eq!(
            valid_archive_name("plan.txt"),
            Err("name must end in \".md\"".to_string())
        );
        assert_eq!(
            valid_archive_name(".md"),
            Err("name is empty".to_string())
        );
    }

    #[test]
    fn rejects_a_leading_or_trailing_hyphen() {
        assert_eq!(
            valid_archive_name("-plan.md"),
            Err("name starts with a hyphen".to_string())
        );
        assert_eq!(
            valid_archive_name("plan-.md"),
            Err("name ends with a hyphen".to_string())
        );
    }

    #[test]
    fn rejects_consecutive_hyphens() {
        assert_eq!(
            valid_archive_name("quiet--cockpit.md"),
            Err("name contains consecutive hyphens".to_string())
        );
    }

    #[test]
    fn rejects_uppercase_and_other_invalid_characters() {
        assert_eq!(
            valid_archive_name("Quiet-Cockpit.md"),
            Err("name contains an invalid character: 'Q'".to_string())
        );
        assert_eq!(
            valid_archive_name("quiet_cockpit.md"),
            Err("name contains an invalid character: '_'".to_string())
        );
        assert_eq!(
            valid_archive_name("quiet cockpit.md"),
            Err("name contains an invalid character: ' '".to_string())
        );
    }

    /// A project whose repo dir holds a PRD.md, registered in the store and
    /// synced so `archive_plan_preview` has a plan id to look up.
    fn synced_plan(conn: &Connection, prd: &str) -> (String, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("PRD.md"), prd).unwrap();
        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES ('proj', ?1, 'prd')",
            [dir.path().to_string_lossy().into_owned()],
        )
        .unwrap();

        let file_path = dir.path().join("PRD.md").to_string_lossy().into_owned();
        let pid = loopfleet_store::plan_id("proj", &file_path);
        loopfleet_store::upsert_plan(conn, &pid, "proj", &file_path).unwrap();
        (pid, dir)
    }

    #[test]
    fn preview_reports_title_path_and_counts() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let (pid, _dir) = synced_plan(&conn, "# Quiet Cockpit\n\n- [ ] do the thing\n");
        loopfleet_store::upsert_task(&conn, &pid, "do the thing", 3, "Do the thing", false)
            .unwrap();

        let preview = archive_plan_preview(&conn, &pid).unwrap();
        assert_eq!(preview.title.as_deref(), Some("Quiet Cockpit"));
        assert_eq!(preview.proposed_file_name, "quiet-cockpit.md");
        assert!(!preview.destination_exists);
        assert_eq!(preview.task_count, 1);
        assert_eq!(preview.run_count, 0);
        assert!(preview.destination_dir.ends_with("prds"));
    }

    #[test]
    fn preview_disambiguates_against_existing_archive_entries() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let (pid, dir) = synced_plan(&conn, "# Quiet Cockpit\n");
        std::fs::create_dir(dir.path().join("prds")).unwrap();
        std::fs::write(dir.path().join("prds/quiet-cockpit.md"), "").unwrap();

        let preview = archive_plan_preview(&conn, &pid).unwrap();
        assert!(preview.destination_exists);
        assert_eq!(preview.proposed_file_name, "quiet-cockpit-2.md");
    }

    #[test]
    fn preview_counts_runs_bound_to_the_plan() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let (pid, _dir) = synced_plan(&conn, "# Quiet Cockpit\n\n- [ ] do the thing\n");
        loopfleet_store::upsert_task(&conn, &pid, "do the thing", 3, "Do the thing", false)
            .unwrap();
        loopfleet_store::insert_run(
            &conn,
            &loopfleet_store::NewRun {
                id: "run-1".into(),
                plan_id: pid.clone(),
                task_anchor: "do the thing".into(),
                agent: "claude".into(),
                model: None,
                worktree_path: "/wt".into(),
                branch: "agent/x".into(),
                sb_profile: "/p.sb".into(),
                progress_path: "/prog.md".into(),
                max_iterations: 1,
                status: "completed".into(),
            },
        )
        .unwrap();

        let preview = archive_plan_preview(&conn, &pid).unwrap();
        assert_eq!(preview.run_count, 1);
    }

    #[test]
    fn preview_rejects_an_unknown_plan() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        assert!(matches!(
            archive_plan_preview(&conn, "nope"),
            Err(ArchivePreviewError::UnknownPlan(_))
        ));
    }

    #[test]
    fn archive_plan_moves_the_file_and_rekeys_the_store() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let (pid, dir) = synced_plan(&conn, "# Quiet Cockpit\n\n- [ ] do the thing\n");
        loopfleet_store::upsert_task(&conn, &pid, "do the thing", 3, "Do the thing", false)
            .unwrap();

        let archived_path = archive_plan(&conn, &pid, "quiet-cockpit.md").unwrap();

        let expected = dir.path().join("prds/quiet-cockpit.md");
        assert_eq!(archived_path, expected.to_string_lossy());
        assert!(expected.is_file());
        assert!(!dir.path().join("PRD.md").exists());

        let new_id = loopfleet_store::plan_id("proj", &archived_path);
        assert_eq!(
            loopfleet_store::plan_file_path(&conn, &new_id).unwrap(),
            Some(archived_path)
        );
        assert_eq!(loopfleet_store::plan_file_path(&conn, &pid).unwrap(), None);
        assert_eq!(
            loopfleet_store::task_count_for_plan(&conn, &new_id).unwrap(),
            1
        );
    }

    #[test]
    fn archive_plan_creates_the_prds_dir_when_absent() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let (pid, dir) = synced_plan(&conn, "# Quiet Cockpit\n");
        assert!(!dir.path().join("prds").exists());

        archive_plan(&conn, &pid, "quiet-cockpit.md").unwrap();

        assert!(dir.path().join("prds").is_dir());
    }

    #[test]
    fn archive_plan_rejects_an_invalid_name() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let (pid, _dir) = synced_plan(&conn, "# Quiet Cockpit\n");

        let err = archive_plan(&conn, &pid, "Quiet Cockpit.md").unwrap_err();
        assert!(matches!(err, ArchivePlanError::InvalidName(_)));
    }

    #[test]
    fn archive_plan_rejects_an_unknown_plan() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let err = archive_plan(&conn, "nope", "quiet-cockpit.md").unwrap_err();
        assert!(matches!(err, ArchivePlanError::UnknownPlan(_)));
    }

    #[test]
    fn archive_plan_rejects_a_plan_with_active_runs() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let (pid, _dir) = synced_plan(&conn, "# Quiet Cockpit\n\n- [ ] do the thing\n");
        loopfleet_store::upsert_task(&conn, &pid, "do the thing", 3, "Do the thing", false)
            .unwrap();
        loopfleet_store::insert_run(
            &conn,
            &loopfleet_store::NewRun {
                id: "run-1".into(),
                plan_id: pid.clone(),
                task_anchor: "do the thing".into(),
                agent: "claude".into(),
                model: None,
                worktree_path: "/wt".into(),
                branch: "agent/x".into(),
                sb_profile: "/p.sb".into(),
                progress_path: "/prog.md".into(),
                max_iterations: 1,
                status: "running".into(),
            },
        )
        .unwrap();

        let err = archive_plan(&conn, &pid, "quiet-cockpit.md").unwrap_err();
        assert!(matches!(err, ArchivePlanError::ActiveRuns(_)));
        // Nothing moved.
        assert!(loopfleet_store::plan_file_path(&conn, &pid)
            .unwrap()
            .is_some());
    }

    #[test]
    fn archive_plan_rejects_an_existing_destination() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let (pid, dir) = synced_plan(&conn, "# Quiet Cockpit\n");
        std::fs::create_dir(dir.path().join("prds")).unwrap();
        std::fs::write(dir.path().join("prds/quiet-cockpit.md"), "existing").unwrap();

        let err = archive_plan(&conn, &pid, "quiet-cockpit.md").unwrap_err();
        assert!(matches!(err, ArchivePlanError::DestinationExists(_)));
        // Source untouched, destination untouched.
        assert!(dir.path().join("PRD.md").exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("prds/quiet-cockpit.md")).unwrap(),
            "existing"
        );
    }

    #[test]
    fn archive_plan_rejects_a_plan_file_outside_its_project_repo() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let repo_dir = tempfile::tempdir().unwrap();
        let outside_dir = tempfile::tempdir().unwrap();

        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES ('proj', ?1, 'prd')",
            [repo_dir.path().to_string_lossy().into_owned()],
        )
        .unwrap();

        // A plan whose stored file path lives outside the project's repo_path
        // — e.g. drifted after the project was re-pointed elsewhere.
        let outside_file = outside_dir.path().join("PRD.md");
        std::fs::write(&outside_file, "# Quiet Cockpit\n").unwrap();
        let file_path = outside_file.to_string_lossy().into_owned();
        let pid = loopfleet_store::plan_id("proj", &file_path);
        loopfleet_store::upsert_plan(&conn, &pid, "proj", &file_path).unwrap();

        let err = archive_plan(&conn, &pid, "quiet-cockpit.md").unwrap_err();
        assert!(matches!(err, ArchivePlanError::OutsideRepo(_)));
        // Nothing moved.
        assert!(outside_file.exists());
        assert!(!repo_dir.path().join("prds").exists());
    }

    #[test]
    fn archive_plan_destination_is_built_from_repo_path_not_the_file_path() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        let repo_dir = tempfile::tempdir().unwrap();

        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES ('proj', ?1, 'prd')",
            [repo_dir.path().to_string_lossy().into_owned()],
        )
        .unwrap();

        // The plan file lives in a subdirectory of the repo, not the repo
        // root — the destination must still be <repo_path>/prds/<name>, not
        // <plan's own parent dir>/prds/<name>.
        let sub_dir = repo_dir.path().join("nested");
        std::fs::create_dir(&sub_dir).unwrap();
        let file_path = sub_dir.join("PRD.md").to_string_lossy().into_owned();
        std::fs::write(&file_path, "# Quiet Cockpit\n").unwrap();
        let pid = loopfleet_store::plan_id("proj", &file_path);
        loopfleet_store::upsert_plan(&conn, &pid, "proj", &file_path).unwrap();

        let archived_path = archive_plan(&conn, &pid, "quiet-cockpit.md").unwrap();

        let expected = repo_dir.path().join("prds/quiet-cockpit.md");
        assert_eq!(archived_path, expected.to_string_lossy());
        assert!(expected.is_file());
    }
}
