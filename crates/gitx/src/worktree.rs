//! Per-run git worktree management, shelling out to the `git` CLI.
//!
//! libgit2's worktree support is patchy and the agents expect the CLI's exact
//! semantics, so create/remove/list/prune all go through `git worktree`
//! (PRD "Git layer"). Reads elsewhere still use `git2`; this module only owns
//! the mutating worktree lifecycle. In M3 these calls are funneled through the
//! single serialized git actor so concurrent runs never collide on lockfiles —
//! this module stays actor-agnostic and just builds/runs the commands.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Branch prefix for run worktrees: `agent/<run-id>` (PRD "Git layer").
pub const BRANCH_PREFIX: &str = "agent/";

/// The branch name a run's worktree lives on.
pub fn branch_for(run_id: &str) -> String {
    format!("{BRANCH_PREFIX}{run_id}")
}

/// A worktree owned by a run: its checkout path and the `agent/<run-id>` branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
}

/// Failure running a `git worktree` command.
#[derive(Debug)]
pub enum WorktreeError {
    /// The `git` process could not be spawned or its output read.
    Io(std::io::Error),
    /// `git` ran but exited non-zero; carries the (trimmed) stderr.
    Git(String),
}

impl std::fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorktreeError::Io(e) => write!(f, "git worktree: {e}"),
            WorktreeError::Git(msg) => write!(f, "git worktree failed: {msg}"),
        }
    }
}

impl std::error::Error for WorktreeError {}

impl From<std::io::Error> for WorktreeError {
    fn from(e: std::io::Error) -> Self {
        WorktreeError::Io(e)
    }
}

type Result<T> = std::result::Result<T, WorktreeError>;

/// Run `git -C <repo> <args...>`, returning trimmed stdout or the stderr on
/// non-zero exit.
fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        Err(WorktreeError::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

/// Create a worktree for `run_id` at `<worktrees_root>/<run_id>`, on a fresh
/// branch `agent/<run-id>` cut from `repo`'s current HEAD.
///
/// `git worktree add -b <branch> <path>` requires `repo` to have at least one
/// commit (a HEAD to branch from). `worktrees_root` should be an app-managed
/// location outside the repo; its parent is created if missing.
pub fn add(repo: &Path, worktrees_root: &Path, run_id: &str) -> Result<Worktree> {
    let branch = branch_for(run_id);
    let path = worktrees_root.join(run_id);
    std::fs::create_dir_all(worktrees_root)?;
    git(
        repo,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            &path.to_string_lossy(),
        ],
    )?;
    // git records the canonical path (on macOS /var -> /private/var); canonicalize
    // ours too so it matches what `list` reports and what's on disk.
    let path = std::fs::canonicalize(&path)?;
    Ok(Worktree { path, branch })
}

/// Remove a worktree. Uses `--force` because agents leave dirty/untracked trees
/// (commits are app-owned via shadow refs, so the checkout is expected to be
/// dirty at teardown). The `agent/<run-id>` branch and any shadow refs are left
/// intact so the run's diff history survives (PRD: "keep all shadow refs").
pub fn remove(repo: &Path, path: &Path) -> Result<()> {
    git(
        repo,
        &["worktree", "remove", "--force", &path.to_string_lossy()],
    )?;
    Ok(())
}

/// List this app's run worktrees (those on an `agent/` branch), parsed from
/// `git worktree list --porcelain`. The repo's own main worktree and any
/// unrelated ones are filtered out.
pub fn list(repo: &Path) -> Result<Vec<Worktree>> {
    let porcelain = git(repo, &["worktree", "list", "--porcelain"])?;
    let mut out = Vec::new();
    let mut cur_path: Option<PathBuf> = None;
    for line in porcelain.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            cur_path = Some(PathBuf::from(p));
        } else if let Some(b) = line.strip_prefix("branch ") {
            let branch = b.strip_prefix("refs/heads/").unwrap_or(b).to_string();
            if branch.starts_with(BRANCH_PREFIX) {
                if let Some(path) = cur_path.take() {
                    out.push(Worktree { path, branch });
                }
            }
        } else if line.is_empty() {
            cur_path = None;
        }
    }
    Ok(out)
}

/// Prune stale worktree metadata for run worktrees whose directory has gone
/// missing — e.g. a crash mid-run left the administrative entry behind. This is
/// the startup orphan cleanup; `git worktree prune` is git's own mechanism for
/// exactly this. Branches and shadow refs are untouched. Returns the number of
/// entries pruned.
pub fn cleanup_orphans(repo: &Path) -> Result<usize> {
    let before = git(repo, &["worktree", "list", "--porcelain"])?;
    let before_count = before.lines().filter(|l| l.starts_with("worktree ")).count();
    git(repo, &["worktree", "prune"])?;
    let after = git(repo, &["worktree", "list", "--porcelain"])?;
    let after_count = after.lines().filter(|l| l.starts_with("worktree ")).count();
    Ok(before_count.saturating_sub(after_count))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repo with one commit, so `worktree add -b` has a HEAD to branch from.
    fn repo_with_commit() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let run = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(p).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t.test"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(p.join("README.md"), "hi\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
        dir
    }

    #[test]
    fn branch_naming() {
        assert_eq!(branch_for("abc-123"), "agent/abc-123");
    }

    #[test]
    fn add_creates_worktree_on_agent_branch() {
        let repo = repo_with_commit();
        let root = tempfile::tempdir().unwrap();
        let wt = add(repo.path(), root.path(), "run-1").unwrap();

        assert_eq!(wt.branch, "agent/run-1");
        assert_eq!(wt.path, root.path().canonicalize().unwrap().join("run-1"));
        assert!(wt.path.join("README.md").is_file(), "checkout populated");

        let listed = list(repo.path()).unwrap();
        assert_eq!(listed, vec![wt]);
    }

    #[test]
    fn remove_deletes_dirty_worktree_but_keeps_branch() {
        let repo = repo_with_commit();
        let root = tempfile::tempdir().unwrap();
        let wt = add(repo.path(), root.path(), "run-2").unwrap();

        // Simulate an agent leaving the tree dirty + untracked files.
        std::fs::write(wt.path.join("README.md"), "changed\n").unwrap();
        std::fs::write(wt.path.join("scratch.txt"), "junk\n").unwrap();

        remove(repo.path(), &wt.path).unwrap();
        assert!(!wt.path.exists(), "worktree dir removed");
        assert!(list(repo.path()).unwrap().is_empty(), "no worktree left");

        // Branch survives for "use this run" / diff history.
        let branches = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["branch", "--list", "agent/run-2"])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&branches.stdout).contains("agent/run-2"),
            "agent branch kept after worktree removal"
        );
    }

    #[test]
    fn list_ignores_non_agent_worktrees() {
        let repo = repo_with_commit();
        let root = tempfile::tempdir().unwrap();

        // A worktree on a non-agent branch must not appear.
        let other = root.path().join("other");
        git(
            repo.path(),
            &["worktree", "add", "-b", "feature/x", &other.to_string_lossy()],
        )
        .unwrap();

        let agent = add(repo.path(), root.path(), "run-3").unwrap();
        let listed = list(repo.path()).unwrap();
        assert_eq!(listed, vec![agent]);
    }

    #[test]
    fn cleanup_prunes_orphaned_worktree() {
        let repo = repo_with_commit();
        let root = tempfile::tempdir().unwrap();
        let wt = add(repo.path(), root.path(), "run-4").unwrap();

        // Crash simulation: the checkout dir vanishes but git's metadata remains.
        std::fs::remove_dir_all(&wt.path).unwrap();
        assert_eq!(list(repo.path()).unwrap().len(), 1, "stale entry present");

        let pruned = cleanup_orphans(repo.path()).unwrap();
        assert_eq!(pruned, 1);
        assert!(list(repo.path()).unwrap().is_empty(), "orphan pruned");
    }

    #[test]
    fn cleanup_noop_when_nothing_orphaned() {
        let repo = repo_with_commit();
        let root = tempfile::tempdir().unwrap();
        add(repo.path(), root.path(), "run-5").unwrap();

        assert_eq!(cleanup_orphans(repo.path()).unwrap(), 0);
        assert_eq!(list(repo.path()).unwrap().len(), 1, "live worktree kept");
    }
}
