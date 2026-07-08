//! loopfleet gitx: the single serialized git actor for mutating ops (worktree
//! add/remove, shadow commits, ref updates) plus `git2`-backed reads (diff,
//! status, log). Mutations funnel through [`GitActor`] (M3) so concurrent runs
//! never collide on git lockfiles; reads stay concurrent.

use std::path::Path;

pub mod actor;
pub mod diff;
pub mod merge;
pub mod shadow;
pub mod status;
pub mod worktree;
pub use actor::GitActor;
pub use diff::{
    diff_refs, iteration_diff, iteration_diff_at, run_cumulative_diff, run_cumulative_diff_at,
    ChangeStatus, DiffError, DiffResult, FileChange,
};
pub use merge::{merge_run, MergeError, MergeResult};
pub use shadow::{shadow_ref, Snapshot, SnapshotError};
pub use status::worktree_changes;
pub use worktree::{Worktree, WorktreeError};

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
