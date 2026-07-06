//! App-owned shadow-ref snapshots of run worktrees.
//!
//! After each iteration the app (trusted, unsandboxed) snapshots the worktree's
//! full state to `refs/agentapp/run-<id>/iter-<n>`. The agent never runs
//! `git commit` and never gets `.git` write (PRD "Git layer"): commits are
//! app-owned, giving cheap, real, diffable history that never touches the user's
//! branches. Shells out to `git` (a mutation, consistent with the worktree
//! module) using a throwaway index so the worktree's own index stays untouched.
//! In M3 these calls funnel through the single serialized git actor; this module
//! stays actor-agnostic and just builds/runs the commands.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Fixed identity for app-owned shadow commits. The commit belongs to the app,
/// not the agent or the user, so it uses a stable synthetic identity rather than
/// the repo's `user.*` config (which may be unset — `commit-tree` would fail).
const COMMIT_NAME: &str = "loopfleet";
const COMMIT_EMAIL: &str = "loopfleet@localhost";

/// The shadow ref for iteration `iter` of `run_id`:
/// `refs/agentapp/run-<id>/iter-<n>` (PRD "Git layer").
pub fn shadow_ref(run_id: &str, iter: u32) -> String {
    format!("refs/agentapp/run-{run_id}/iter-{iter}")
}

/// The shadow snapshot produced for one iteration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The ref the snapshot was written to (`refs/agentapp/run-<id>/iter-<n>`).
    pub git_ref: String,
    /// The commit the ref now points at.
    pub commit: String,
}

/// Failure running a `git` command while snapshotting.
#[derive(Debug)]
pub enum SnapshotError {
    /// The `git` process could not be spawned or its output read.
    Io(std::io::Error),
    /// `git` ran but exited non-zero; carries the (trimmed) stderr.
    Git(String),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::Io(e) => write!(f, "git snapshot: {e}"),
            SnapshotError::Git(msg) => write!(f, "git snapshot failed: {msg}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<std::io::Error> for SnapshotError {
    fn from(e: std::io::Error) -> Self {
        SnapshotError::Io(e)
    }
}

type Result<T> = std::result::Result<T, SnapshotError>;

/// Snapshot the full state of `worktree` to `refs/agentapp/run-<id>/iter-<n>`.
///
/// Stages the entire worktree (tracked + untracked, respecting `.gitignore`)
/// into a throwaway index so the worktree's own index is never disturbed, writes
/// that tree, commits it under the app identity, and points the shadow ref at the
/// new commit. The commit is parented on the previous iteration's shadow commit
/// (falling back to the worktree's base HEAD) so the refs form a readable chain.
/// The agent branch and any user branches are never touched.
pub fn snapshot(repo: &Path, worktree: &Path, run_id: &str, iter: u32) -> Result<Snapshot> {
    // A throwaway index, keyed uniquely so serialized snapshots never collide,
    // so the worktree's real index is left exactly as the agent left it.
    let tmp_index = temp_index_path(run_id, iter);
    let _ = std::fs::remove_file(&tmp_index);
    let index_env: [(&str, &OsStr); 1] = [("GIT_INDEX_FILE", tmp_index.as_os_str())];
    // `add -A` from an empty index stages every file on disk; it honors
    // .gitignore, so build artifacts are not snapshotted. Objects land in the
    // shared object database.
    run_git(worktree, &["add", "-A"], &index_env)?;
    let tree = run_git(worktree, &["write-tree"], &index_env)?;
    let _ = std::fs::remove_file(&tmp_index);

    let parent = resolve_parent(repo, worktree, run_id, iter)?;
    let msg = format!("run {run_id} iter {iter}");
    let ident: [(&str, &OsStr); 4] = [
        ("GIT_AUTHOR_NAME", OsStr::new(COMMIT_NAME)),
        ("GIT_AUTHOR_EMAIL", OsStr::new(COMMIT_EMAIL)),
        ("GIT_COMMITTER_NAME", OsStr::new(COMMIT_NAME)),
        ("GIT_COMMITTER_EMAIL", OsStr::new(COMMIT_EMAIL)),
    ];
    let commit = run_git(
        repo,
        &["commit-tree", &tree, "-p", &parent, "-m", &msg],
        &ident,
    )?;

    let git_ref = shadow_ref(run_id, iter);
    run_git(repo, &["update-ref", &git_ref, &commit], &[])?;
    Ok(Snapshot { git_ref, commit })
}

/// Parent for iteration `iter`: the previous iteration's shadow commit if it
/// exists, else the worktree's base HEAD (the agent branch tip — the agent never
/// commits, so this is the commit the worktree was cut from).
fn resolve_parent(repo: &Path, worktree: &Path, run_id: &str, iter: u32) -> Result<String> {
    if iter > 1 {
        if let Some(sha) = rev_parse_opt(repo, &shadow_ref(run_id, iter - 1))? {
            return Ok(sha);
        }
    }
    run_git(worktree, &["rev-parse", "HEAD"], &[])
}

/// Resolve `rev` to a commit sha, or `None` if it does not exist. `--verify
/// --quiet` exits non-zero with no output for a missing ref, which is not an
/// error here (the first iteration has no predecessor).
fn rev_parse_opt(dir: &Path, rev: &str) -> Result<Option<String>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--verify", "--quiet", rev])
        .output()?;
    if out.status.success() {
        Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_string()))
    } else {
        Ok(None)
    }
}

/// Run `git -C <dir> <args...>` with extra env vars, returning trimmed stdout or
/// the stderr on non-zero exit.
fn run_git(dir: &Path, args: &[&str], envs: &[(&str, &OsStr)]) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        Err(SnapshotError::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

/// A unique throwaway-index path in the system temp dir. Keyed by pid + run + n;
/// the git actor serializes snapshots, so this only needs to avoid clashing with
/// unrelated processes and prior iterations.
fn temp_index_path(run_id: &str, iter: u32) -> PathBuf {
    std::env::temp_dir().join(format!(
        "loopfleet-index-{}-{run_id}-{iter}",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repo with one commit, plus a run worktree cut from it.
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
        // Don't inherit the user's global commit.gpgsign — tests must not depend
        // on a gpg agent being available.
        run(&["config", "commit.gpgsign", "false"]);
        std::fs::write(p.join("README.md"), "hi\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);

        let root = tempfile::tempdir().unwrap();
        let wt = crate::worktree::add(p, root.path(), run_id).unwrap();
        (repo, root, wt)
    }

    #[test]
    fn shadow_ref_naming() {
        assert_eq!(shadow_ref("abc-123", 2), "refs/agentapp/run-abc-123/iter-2");
    }

    #[test]
    fn snapshot_captures_worktree_changes() {
        let (repo, _root, wt) = repo_with_worktree("run-1");
        // The agent edits a tracked file and drops an untracked one.
        std::fs::write(wt.path.join("README.md"), "changed\n").unwrap();
        std::fs::write(wt.path.join("new.txt"), "fresh\n").unwrap();

        let snap = snapshot(repo.path(), &wt.path, "run-1", 1).unwrap();
        assert_eq!(snap.git_ref, "refs/agentapp/run-run-1/iter-1");

        // The ref resolves to the commit, and the tree reflects disk exactly.
        let resolved = run_git(repo.path(), &["rev-parse", &snap.git_ref], &[]).unwrap();
        assert_eq!(resolved, snap.commit);
        let readme = run_git(repo.path(), &["show", &format!("{}:README.md", snap.commit)], &[]).unwrap();
        assert_eq!(readme, "changed");
        let new = run_git(repo.path(), &["show", &format!("{}:new.txt", snap.commit)], &[]).unwrap();
        assert_eq!(new, "fresh");
    }

    #[test]
    fn first_snapshot_parents_on_base_and_leaves_agent_branch_untouched() {
        let (repo, _root, wt) = repo_with_worktree("run-2");
        let base = run_git(&wt.path, &["rev-parse", "HEAD"], &[]).unwrap();
        std::fs::write(wt.path.join("new.txt"), "x\n").unwrap();

        let snap = snapshot(repo.path(), &wt.path, "run-2", 1).unwrap();

        let parent = run_git(repo.path(), &["rev-parse", &format!("{}^", snap.commit)], &[]).unwrap();
        assert_eq!(parent, base, "iter-1 parented on the base commit");
        // The app-owned commit never advances the agent branch or user branches.
        let branch_tip = run_git(repo.path(), &["rev-parse", "agent/run-2"], &[]).unwrap();
        assert_eq!(branch_tip, base, "agent branch tip unchanged");
        assert_ne!(branch_tip, snap.commit);
    }

    #[test]
    fn snapshots_chain_across_iterations() {
        let (repo, _root, wt) = repo_with_worktree("run-3");
        std::fs::write(wt.path.join("a.txt"), "one\n").unwrap();
        let s1 = snapshot(repo.path(), &wt.path, "run-3", 1).unwrap();
        std::fs::write(wt.path.join("a.txt"), "two\n").unwrap();
        let s2 = snapshot(repo.path(), &wt.path, "run-3", 2).unwrap();

        let s2_parent = run_git(repo.path(), &["rev-parse", &format!("{}^", s2.commit)], &[]).unwrap();
        assert_eq!(s2_parent, s1.commit, "iter-2 chains onto iter-1");

        // Iteration diff between the two refs shows the second edit.
        let diff = run_git(
            repo.path(),
            &["diff", "--name-only", &s1.commit, &s2.commit],
            &[],
        )
        .unwrap();
        assert_eq!(diff, "a.txt");
    }

    #[test]
    fn snapshot_leaves_worktree_index_untouched() {
        let (repo, _root, wt) = repo_with_worktree("run-4");
        std::fs::write(wt.path.join("README.md"), "changed\n").unwrap();
        std::fs::write(wt.path.join("untracked.txt"), "u\n").unwrap();

        snapshot(repo.path(), &wt.path, "run-4", 1).unwrap();

        // The worktree's real index is unchanged: README is still unstaged (` M`)
        // and the new file is still untracked (`??`) — the snapshot used a
        // throwaway index.
        let status = run_git(&wt.path, &["status", "--porcelain"], &[]).unwrap();
        assert!(status.contains(" M README.md"), "README unstaged: {status:?}");
        assert!(status.contains("?? untracked.txt"), "file untracked: {status:?}");
    }

    #[test]
    fn snapshot_respects_gitignore() {
        let (repo, _root, wt) = repo_with_worktree("run-5");
        std::fs::write(wt.path.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(wt.path.join("ignored.txt"), "secret\n").unwrap();
        std::fs::write(wt.path.join("kept.txt"), "ok\n").unwrap();

        let snap = snapshot(repo.path(), &wt.path, "run-5", 1).unwrap();

        let tree = run_git(repo.path(), &["ls-tree", "--name-only", &snap.commit], &[]).unwrap();
        assert!(tree.contains("kept.txt"), "tracked file present: {tree:?}");
        assert!(!tree.contains("ignored.txt"), "gitignored file excluded: {tree:?}");
    }
}
