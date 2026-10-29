pub const SUMMARY_MARKER: &str = "SUMMARY:";

pub fn summary_from_contents(contents: &str) -> Option<String> {
    contents
        .lines()
        .filter_map(|l| l.trim().strip_prefix(SUMMARY_MARKER))
        .map(str::trim)
        .rfind(|s| !s.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_is_read_from_the_marker_line() {
        assert_eq!(
            summary_from_contents("did work\nSUMMARY: added the parser\n").as_deref(),
            Some("added the parser")
        );
        assert_eq!(
            summary_from_contents("   SUMMARY:   added the parser   ").as_deref(),
            Some("added the parser")
        );
    }

    #[test]
    fn last_summary_wins() {
        let contents = "pass 1\nSUMMARY: first cut\npass 2\nSUMMARY: final cut\n";
        assert_eq!(summary_from_contents(contents).as_deref(), Some("final cut"));
    }

    #[test]
    fn absent_or_empty_summary_is_none() {
        assert_eq!(summary_from_contents(""), None);
        assert_eq!(summary_from_contents("nothing here yet\n"), None);
        assert_eq!(summary_from_contents("SUMMARY:\n"), None);
        assert_eq!(summary_from_contents("SUMMARY:    \n"), None);
        assert_eq!(summary_from_contents("I will write SUMMARY: something later\n"), None);
    }

    #[test]
    fn a_bare_marker_does_not_shadow_an_earlier_summary() {
        assert_eq!(
            summary_from_contents("SUMMARY: real work\nSUMMARY:\n").as_deref(),
            Some("real work")
        );
    }
}
