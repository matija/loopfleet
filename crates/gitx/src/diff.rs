//! Read-only diff service over the app-owned shadow refs.
//!
//! Three views the UI needs (PRD "Git layer" / "Compare view"):
//!   * iteration diff   — what a single iteration changed (iter-(n-1)..iter-n)
//!   * run cumulative    — everything a run produced (base HEAD..final iter)
//!   * run-vs-run        — two runs' final refs side by side (the compare view)
//!
//! All three reduce to diffing two tree-ish revisions, so the core is one
//! `diff_refs` and the named views are thin wrappers that resolve the right refs.
//! Uses `git2` (reads stay on `git2`; mutations shell out — PRD "Git layer");
//! reads are concurrent, so this never funnels through the M3 git actor.

use git2::{Delta, DiffFormat, DiffFindOptions, Repository, Tree};

use crate::shadow::shadow_ref;

/// How a file changed between two revisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    /// Typechange, conflicted, or anything else git reports.
    Other,
}

/// One file's change in a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// The file's path on the "to" side (the delete's old path when deleted).
    pub path: String,
    /// The source path for a rename/copy; `None` otherwise.
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    pub insertions: usize,
    pub deletions: usize,
}

/// A diff between two revisions: a per-file summary plus the full unified patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffResult {
    /// One entry per changed file (renames detected).
    pub files: Vec<FileChange>,
    /// The complete unified-diff text, for the diff viewer.
    pub patch: String,
}

/// Failure reading a diff.
#[derive(Debug)]
pub enum DiffError {
    Git(git2::Error),
}

impl std::fmt::Display for DiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffError::Git(e) => write!(f, "git diff: {e}"),
        }
    }
}

impl std::error::Error for DiffError {}

impl From<git2::Error> for DiffError {
    fn from(e: git2::Error) -> Self {
        DiffError::Git(e)
    }
}

type Result<T> = std::result::Result<T, DiffError>;

/// Diff what iteration `iter` of `run_id` changed: its shadow commit against its
/// parent (the previous iteration, or the base HEAD for iter 1 — the shadow refs
/// form a chain, so the commit's first parent is exactly the prior state).
pub fn iteration_diff(repo: &Repository, run_id: &str, iter: u32) -> Result<DiffResult> {
    let commit = repo
        .revparse_single(&shadow_ref(run_id, iter))?
        .peel_to_commit()?;
    let to = commit.tree()?;
    let from = match commit.parent(0) {
        Ok(parent) => Some(parent.tree()?),
        Err(_) => None,
    };
    build(repo, from.as_ref(), Some(&to))
}

/// Convenience wrapper over [`iteration_diff`] that opens `repo_path` itself, so
/// callers (the timeline) that hold only a path need no `git2` dependency.
pub fn iteration_diff_at(repo_path: &std::path::Path, run_id: &str, iter: u32) -> Result<DiffResult> {
    let repo = Repository::open(repo_path)?;
    iteration_diff(&repo, run_id, iter)
}

/// Diff everything run `run_id` produced: the base HEAD the run was cut from
/// (iter-1's parent) against `final_iter`'s shadow commit.
pub fn run_cumulative_diff(repo: &Repository, run_id: &str, final_iter: u32) -> Result<DiffResult> {
    let first = repo
        .revparse_single(&shadow_ref(run_id, 1))?
        .peel_to_commit()?;
    let from = match first.parent(0) {
        Ok(parent) => Some(parent.tree()?),
        Err(_) => None,
    };
    let to = repo
        .revparse_single(&shadow_ref(run_id, final_iter))?
        .peel_to_commit()?
        .tree()?;
    build(repo, from.as_ref(), Some(&to))
}

/// Diff two arbitrary revisions (the compare view: two runs' final shadow refs).
/// Each rev is anything `git rev-parse` accepts (a shadow ref, a sha, a branch).
pub fn diff_refs(repo: &Repository, from_rev: &str, to_rev: &str) -> Result<DiffResult> {
    let from = repo.revparse_single(from_rev)?.peel_to_tree()?;
    let to = repo.revparse_single(to_rev)?.peel_to_tree()?;
    build(repo, Some(&from), Some(&to))
}

/// Build the diff between two optional trees (a `None` side = the empty tree, so
/// an initial snapshot with no parent shows every file as added).
fn build(repo: &Repository, from: Option<&Tree>, to: Option<&Tree>) -> Result<DiffResult> {
    let mut diff = repo.diff_tree_to_tree(from, to, None)?;
    // Report renames/copies as such instead of an add+delete pair.
    diff.find_similar(Some(DiffFindOptions::new().renames(true).copies(true)))?;

    let mut files = Vec::new();
    for (idx, delta) in diff.deltas().enumerate() {
        let status = map_status(delta.status());
        let new_path = path_of(delta.new_file().path());
        let old_path = path_of(delta.old_file().path());
        // A deleted file has no new-side path; show its old path instead.
        let path = new_path.clone().unwrap_or_else(|| old_path.clone().unwrap_or_default());
        let old_path = match status {
            ChangeStatus::Renamed | ChangeStatus::Copied => old_path,
            _ => None,
        };
        let (insertions, deletions) = match git2::Patch::from_diff(&diff, idx)? {
            Some(patch) => {
                let (_, ins, del) = patch.line_stats()?;
                (ins, del)
            }
            None => (0, 0),
        };
        files.push(FileChange {
            path,
            old_path,
            status,
            insertions,
            deletions,
        });
    }

    let mut patch = String::new();
    diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
        if matches!(line.origin(), '+' | '-' | ' ') {
            patch.push(line.origin());
        }
        patch.push_str(&String::from_utf8_lossy(line.content()));
        true
    })?;

    Ok(DiffResult { files, patch })
}

fn map_status(d: Delta) -> ChangeStatus {
    match d {
        Delta::Added => ChangeStatus::Added,
        Delta::Deleted => ChangeStatus::Deleted,
        Delta::Modified => ChangeStatus::Modified,
        Delta::Renamed => ChangeStatus::Renamed,
        Delta::Copied => ChangeStatus::Copied,
        _ => ChangeStatus::Other,
    }
}

fn path_of(p: Option<&std::path::Path>) -> Option<String> {
    p.and_then(|p| p.to_str()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shadow::snapshot;
    use std::path::Path;
    use std::process::Command;

    /// A repo with one commit plus a run worktree cut from it.
    fn repo_with_worktree(run_id: &str) -> (tempfile::TempDir, tempfile::TempDir, crate::worktree::Worktree) {
        let repo = tempfile::tempdir().unwrap();
        let p = repo.path();
        let run = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(p).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t.test"]);
        run(&["config", "user.name", "t"]);
        run(&["config", "commit.gpgsign", "false"]);
        std::fs::write(p.join("README.md"), "hi\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);

        let root = tempfile::tempdir().unwrap();
        let wt = crate::worktree::add(p, root.path(), run_id).unwrap();
        (repo, root, wt)
    }

    fn open(repo: &Path) -> Repository {
        Repository::open(repo).unwrap()
    }

    #[test]
    fn iteration_diff_first_iter_is_vs_base() {
        let (repo, _root, wt) = repo_with_worktree("diff-r1");
        std::fs::write(wt.path.join("README.md"), "changed\n").unwrap();
        std::fs::write(wt.path.join("new.txt"), "fresh\n").unwrap();
        snapshot(repo.path(), &wt.path, "diff-r1", 1).unwrap();

        let d = iteration_diff(&open(repo.path()), "diff-r1", 1).unwrap();

        let readme = d.files.iter().find(|f| f.path == "README.md").unwrap();
        assert_eq!(readme.status, ChangeStatus::Modified);
        let new = d.files.iter().find(|f| f.path == "new.txt").unwrap();
        assert_eq!(new.status, ChangeStatus::Added);
        assert_eq!(new.insertions, 1);
        assert!(d.patch.contains("+changed"), "patch: {}", d.patch);
        assert!(d.patch.contains("+fresh"));
    }

    #[test]
    fn iteration_diff_later_iter_is_vs_previous() {
        let (repo, _root, wt) = repo_with_worktree("diff-r2");
        std::fs::write(wt.path.join("a.txt"), "one\n").unwrap();
        snapshot(repo.path(), &wt.path, "diff-r2", 1).unwrap();
        std::fs::write(wt.path.join("a.txt"), "two\n").unwrap();
        std::fs::write(wt.path.join("b.txt"), "b\n").unwrap();
        snapshot(repo.path(), &wt.path, "diff-r2", 2).unwrap();

        let d = iteration_diff(&open(repo.path()), "diff-r2", 2).unwrap();

        // iter-2's diff shows only what iter-2 changed, not iter-1's a.txt add.
        let a = d.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!(a.status, ChangeStatus::Modified);
        assert!(d.files.iter().any(|f| f.path == "b.txt" && f.status == ChangeStatus::Added));
        assert!(d.patch.contains("-one"));
        assert!(d.patch.contains("+two"));
    }

    #[test]
    fn run_cumulative_spans_base_to_final() {
        let (repo, _root, wt) = repo_with_worktree("diff-r3");
        std::fs::write(wt.path.join("a.txt"), "one\n").unwrap();
        snapshot(repo.path(), &wt.path, "diff-r3", 1).unwrap();
        std::fs::write(wt.path.join("a.txt"), "two\n").unwrap();
        snapshot(repo.path(), &wt.path, "diff-r3", 2).unwrap();

        let d = run_cumulative_diff(&open(repo.path()), "diff-r3", 2).unwrap();

        // Cumulative: a.txt is a NEW file relative to base (added across the run),
        // ending at its final "two" content.
        let a = d.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!(a.status, ChangeStatus::Added);
        assert!(d.patch.contains("+two"));
        assert!(!d.patch.contains("+one"), "cumulative shows final state only: {}", d.patch);
    }

    #[test]
    fn diff_refs_compares_two_runs() {
        let (repo, _root, wt1) = repo_with_worktree("run-a");
        std::fs::write(wt1.path.join("f.txt"), "alpha\n").unwrap();
        let sa = snapshot(repo.path(), &wt1.path, "run-a", 1).unwrap();

        // A second run in its own worktree off the same repo/base.
        let root2 = tempfile::tempdir().unwrap();
        let wt2 = crate::worktree::add(repo.path(), root2.path(), "run-b").unwrap();
        std::fs::write(wt2.path.join("f.txt"), "beta\n").unwrap();
        let sb = snapshot(repo.path(), &wt2.path, "run-b", 1).unwrap();

        let d = diff_refs(&open(repo.path()), &sa.git_ref, &sb.git_ref).unwrap();

        let f = d.files.iter().find(|f| f.path == "f.txt").unwrap();
        assert_eq!(f.status, ChangeStatus::Modified);
        assert!(d.patch.contains("-alpha"));
        assert!(d.patch.contains("+beta"));
    }

    #[test]
    fn detects_deletion_and_rename() {
        let (repo, _root, wt) = repo_with_worktree("diff-r4");
        // iter-1: add a file with enough content to be rename-matched.
        std::fs::write(wt.path.join("orig.txt"), "line1\nline2\nline3\n").unwrap();
        snapshot(repo.path(), &wt.path, "diff-r4", 1).unwrap();
        // iter-2: rename it (same content) and delete the base README.
        std::fs::rename(wt.path.join("orig.txt"), wt.path.join("renamed.txt")).unwrap();
        std::fs::remove_file(wt.path.join("README.md")).unwrap();
        snapshot(repo.path(), &wt.path, "diff-r4", 2).unwrap();

        let d = iteration_diff(&open(repo.path()), "diff-r4", 2).unwrap();

        let renamed = d.files.iter().find(|f| f.status == ChangeStatus::Renamed).unwrap();
        assert_eq!(renamed.path, "renamed.txt");
        assert_eq!(renamed.old_path.as_deref(), Some("orig.txt"));
        assert!(d.files.iter().any(|f| f.path == "README.md" && f.status == ChangeStatus::Deleted));
    }
}
