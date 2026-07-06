//! loopfleet gitx: the single serialized git actor for mutating ops (worktree
//! add/remove, shadow commits, ref updates) plus `git2`-backed reads (diff,
//! status, log). Worktree/shadow-ref ops land in M2; this module currently
//! provides the read-only repo validation used by project registration (M0).

use std::path::Path;

/// True if `path` is (or is contained by) a git repository. Uses `git2` in
/// read-only mode; opens the repo at `path`, so both a working directory with a
/// `.git` and a bare repository count. Never mutates anything.
pub fn is_git_repo<P: AsRef<Path>>(path: P) -> bool {
    git2::Repository::open(path.as_ref()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_repo_is_recognized() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        assert!(is_git_repo(dir.path()));
    }

    #[test]
    fn plain_directory_is_not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_git_repo(dir.path()));
    }

    #[test]
    fn missing_path_is_not_a_repo() {
        assert!(!is_git_repo("/nonexistent/path/loopfleet-test"));
    }
}
