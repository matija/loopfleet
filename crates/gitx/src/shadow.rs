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

#[cfg(test)]
use std::ffi::OsStr;
use std::path::Path;
#[cfg(test)]
use std::process::Command;

/// Fallback identity for shadow commits when the repo has no `user.*` configured.
/// Shadow commits are normally stamped with the user's own git identity (see
/// [`commit_identity`]) so a run's work is attributed to them once used, but a
/// fresh repo may have no identity set at all, and `commit-tree` needs one — this
/// synthetic identity keeps snapshotting working in that case.
const COMMIT_NAME: &str = "Loopfleet";
const COMMIT_EMAIL: &str = "loopfleet@tandoku.hr";

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
    let repository = git2::Repository::open(repo).map_err(git_error)?;
    let parent = if iter > 1 {
        repository.revparse_single(&shadow_ref(run_id, iter - 1)).ok()
            .and_then(|object| object.peel_to_commit().ok())
    } else { None };
    let parent = parent.or_else(|| repository.find_branch(&format!("agent/{run_id}"), git2::BranchType::Local)
        .ok().and_then(|branch| branch.get().peel_to_commit().ok()))
        .ok_or_else(|| SnapshotError::Git("missing run branch".into()))?;
    let signature = repository.signature().or_else(|_| git2::Signature::now(COMMIT_NAME, COMMIT_EMAIL)).map_err(git_error)?;
    repository.set_workdir(worktree, false).map_err(git_error)?;
    let mut index = git2::Index::new().map_err(git_error)?;
    repository.set_index(&mut index).map_err(git_error)?;
    index.add_all(["*"], git2::IndexAddOption::DEFAULT, None).map_err(git_error)?;
    let tree_id = index.write_tree_to(&repository).map_err(git_error)?;
    let tree = repository.find_tree(tree_id).map_err(git_error)?;
    let git_ref = shadow_ref(run_id, iter);
    let commit = repository.commit(Some(&git_ref), &signature, &signature,
        &format!("run {run_id} iter {iter}"), &tree, &[&parent]).map_err(git_error)?;
    Ok(Snapshot { git_ref, commit: commit.to_string() })
}

fn git_error(error: git2::Error) -> SnapshotError {
    SnapshotError::Git(error.to_string())
}

/// Run `git -C <dir> <args...>` with extra env vars, returning trimmed stdout or
/// the stderr on non-zero exit.
#[cfg(test)]
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
    fn snapshot_is_attributed_to_repo_identity() {
        // The repo configures a real user identity (t <t@t.test>); the shadow
        // commit must carry it, not the synthetic Loopfleet fallback, so a used
        // run lands in the user's history under their name.
        let (repo, _root, wt) = repo_with_worktree("run-6");
        std::fs::write(wt.path.join("a.txt"), "one\n").unwrap();
        let snap = snapshot(repo.path(), &wt.path, "run-6", 1).unwrap();

        let author = run_git(
            repo.path(),
            &["log", "-1", "--pretty=%an <%ae>", &snap.commit],
            &[],
        )
        .unwrap();
        assert_eq!(author, "t <t@t.test>");
        let committer = run_git(
            repo.path(),
            &["log", "-1", "--pretty=%cn <%ce>", &snap.commit],
            &[],
        )
        .unwrap();
        assert_eq!(committer, "t <t@t.test>");
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
