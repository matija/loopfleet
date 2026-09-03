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

/// The slug for a plan whose title is missing, blank, or made entirely of
/// characters that survive nothing.
const FALLBACK: &str = "untitled";

/// The file name (with `.md`) an archived plan titled `title` should take,
/// given the names already `taken` in the archive.
///
/// The title is lowercased, stripped of a leading `PRD:` / `PRD -` prefix (the
/// plan file announces itself as a PRD; the archive folder already says that),
/// split on runs of non-alphanumerics, filtered of [`STOPWORDS`], and cut to
/// [`MAX_WORDS`] words and [`MAX_CHARS`] characters. A title that is all
/// stopwords keeps them rather than archiving as [`FALLBACK`], since "the end"
/// is still a better name than "untitled".
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

    // Dropping the stopwords from a title that is nothing but stopwords would
    // leave no name at all, so in that case they are the name.
    let kept: Vec<&str> = words
        .iter()
        .copied()
        .filter(|w| !STOPWORDS.contains(w))
        .collect();
    let kept = if kept.is_empty() { words } else { kept };

    if kept.is_empty() {
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
    fn keeps_stopwords_when_they_are_the_whole_title() {
        assert_eq!(name("The and of"), "the-and-of.md");
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
    fn cuts_a_single_overlong_word_hard() {
        let title = "a".repeat(60);
        let out = name(&title);
        assert_eq!(out, format!("{}.md", "a".repeat(MAX_CHARS)));
    }

    #[test]
    fn falls_back_when_there_is_no_title() {
        assert_eq!(proposed_archive_name(None, &[]), "untitled.md");
        assert_eq!(name("   "), "untitled.md");
        assert_eq!(name("!!! ???"), "untitled.md");
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
    fn a_discriminated_overlong_word_is_cut_hard_to_fit() {
        let title = "a".repeat(60);
        let taken = vec![name(&title)];
        let out = proposed_archive_name(Some(&title), &taken);
        assert_eq!(out, format!("{}-2.md", "a".repeat(MAX_CHARS - 2)));
        assert_eq!(out.strip_suffix(".md").unwrap().chars().count(), MAX_CHARS);
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
}
