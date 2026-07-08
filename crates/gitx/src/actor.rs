//! The single serialized git actor for mutating ops (PRD "Git layer").
//!
//! All mutating git operations — worktree add/remove, orphan pruning, shadow
//! snapshots — funnel through one worker thread, so concurrent runs never
//! collide on git lockfiles (index.lock, refs, worktree metadata). `git2` reads
//! (diff, status, log) stay concurrent and never go through the actor.
//!
//! The public surface is typed: one async method per mutating op in this crate,
//! nothing else. That keeps the actor honest — a caller cannot sneak an
//! arbitrary git mutation around the serialization point.

use std::path::PathBuf;

use tokio::sync::{mpsc, oneshot};

use crate::merge::{self, MergeError, MergeResult};
use crate::shadow::{self, Snapshot, SnapshotError};
use crate::worktree::{self, Worktree, WorktreeError};

/// A queued mutation: runs on the actor thread, delivers its result through the
/// oneshot captured inside.
type Job = Box<dyn FnOnce() + Send>;

/// Handle to the serialized git worker. Cheap to clone; all clones feed the same
/// single worker thread. The thread exits when the last handle is dropped.
#[derive(Clone)]
pub struct GitActor {
    tx: mpsc::Sender<Job>,
}

impl GitActor {
    /// Start the actor: one worker thread draining a bounded queue of mutations.
    /// The bounded channel is the backpressure — many concurrent runs queue up
    /// rather than piling into an unbounded buffer.
    pub fn spawn() -> Self {
        let (tx, mut rx) = mpsc::channel::<Job>(64);
        std::thread::spawn(move || {
            // blocking_recv keeps the synchronous git CLI calls off the async
            // runtime's worker threads (same model as the store's event-log
            // writer). Returns None when every GitActor clone is dropped.
            while let Some(job) = rx.blocking_recv() {
                job();
            }
        });
        Self { tx }
    }

    /// Create a run worktree (`worktree::add`) through the actor.
    pub async fn worktree_add(
        &self,
        repo: PathBuf,
        worktrees_root: PathBuf,
        run_id: String,
    ) -> Result<Worktree, WorktreeError> {
        self.exec(move || worktree::add(&repo, &worktrees_root, &run_id))
            .await
    }

    /// Remove a run worktree (`worktree::remove`) through the actor.
    pub async fn worktree_remove(&self, repo: PathBuf, path: PathBuf) -> Result<(), WorktreeError> {
        self.exec(move || worktree::remove(&repo, &path)).await
    }

    /// Prune stale worktree metadata (`worktree::cleanup_orphans`) through the
    /// actor. Startup-time.
    pub async fn cleanup_orphans(&self, repo: PathBuf) -> Result<usize, WorktreeError> {
        self.exec(move || worktree::cleanup_orphans(&repo)).await
    }

    /// Snapshot a worktree to its iteration shadow ref (`shadow::snapshot`)
    /// through the actor.
    pub async fn snapshot(
        &self,
        repo: PathBuf,
        worktree: PathBuf,
        run_id: String,
        iter: u32,
    ) -> Result<Snapshot, SnapshotError> {
        self.exec(move || shadow::snapshot(&repo, &worktree, &run_id, iter))
            .await
    }

    /// Merge a run's final commit into `target_branch` ("use this run",
    /// `merge::merge_run`) through the actor. `scratch_root` roots the throwaway
    /// worktree used when the target already exists.
    pub async fn merge_run(
        &self,
        repo: PathBuf,
        source_rev: String,
        target_branch: String,
        scratch_root: PathBuf,
    ) -> Result<MergeResult, MergeError> {
        self.exec(move || merge::merge_run(&repo, &source_rev, &target_branch, &scratch_root))
            .await
    }

    /// Run one mutation on the actor thread and await its result. Private: the
    /// typed methods above are the only doorway to the serialization point.
    async fn exec<T, F>(&self, f: F) -> T
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (done, result) = oneshot::channel();
        self.tx
            .send(Box::new(move || {
                // Receiver dropped (caller gave up) is fine; the mutation still
                // completed on the actor thread.
                let _ = done.send(f());
            }))
            .await
            .expect("git actor thread is alive for the app's lifetime");
        result
            .await
            .expect("git actor delivers a result for every job it accepts")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

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
        run(&["config", "commit.gpgsign", "false"]);
        std::fs::write(p.join("README.md"), "hi\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
        dir
    }

    /// The full mutating lifecycle works through the actor: add a worktree,
    /// snapshot an edit, remove the worktree.
    #[tokio::test(flavor = "multi_thread")]
    async fn lifecycle_through_the_actor() {
        let repo = repo_with_commit();
        let root = tempfile::tempdir().unwrap();
        let actor = GitActor::spawn();

        let wt = actor
            .worktree_add(repo.path().into(), root.path().into(), "actor-1".into())
            .await
            .unwrap();
        assert_eq!(wt.branch, "agent/actor-1");

        std::fs::write(wt.path.join("new.txt"), "x\n").unwrap();
        let snap = actor
            .snapshot(repo.path().into(), wt.path.clone(), "actor-1".into(), 1)
            .await
            .unwrap();
        assert_eq!(snap.git_ref, "refs/agentapp/run-actor-1/iter-1");

        actor
            .worktree_remove(repo.path().into(), wt.path.clone())
            .await
            .unwrap();
        assert!(!wt.path.exists());
        assert_eq!(actor.cleanup_orphans(repo.path().into()).await.unwrap(), 0);
    }

    /// Concurrent submitters never overlap on the actor thread: each job flips
    /// a shared "busy" flag and fails if it was already set. With 8 tasks
    /// hammering the actor, any parallel execution trips the flag.
    #[tokio::test(flavor = "multi_thread")]
    async fn mutations_are_serialized() {
        let actor = GitActor::spawn();
        let busy = Arc::new(AtomicBool::new(false));

        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let actor = actor.clone();
                let busy = Arc::clone(&busy);
                tokio::spawn(async move {
                    actor
                        .exec(move || {
                            assert!(!busy.swap(true, Ordering::SeqCst), "jobs overlapped");
                            std::thread::sleep(std::time::Duration::from_millis(5));
                            busy.store(false, Ordering::SeqCst);
                        })
                        .await;
                })
            })
            .collect();
        for t in tasks {
            t.await.unwrap();
        }
    }

    /// Two runs snapshotting the same repo concurrently — the lockfile-collision
    /// scenario the actor exists for — both succeed.
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_runs_share_one_repo_safely() {
        let repo = repo_with_commit();
        let root = tempfile::tempdir().unwrap();
        let actor = GitActor::spawn();

        let a = actor
            .worktree_add(repo.path().into(), root.path().into(), "actor-a".into())
            .await
            .unwrap();
        let b = actor
            .worktree_add(repo.path().into(), root.path().into(), "actor-b".into())
            .await
            .unwrap();
        std::fs::write(a.path.join("a.txt"), "a\n").unwrap();
        std::fs::write(b.path.join("b.txt"), "b\n").unwrap();

        let (sa, sb) = tokio::join!(
            actor.snapshot(repo.path().into(), a.path.clone(), "actor-a".into(), 1),
            actor.snapshot(repo.path().into(), b.path.clone(), "actor-b".into(), 1),
        );
        let (sa, sb) = (sa.unwrap(), sb.unwrap());
        assert_ne!(sa.commit, sb.commit);
        assert_eq!(sa.git_ref, "refs/agentapp/run-actor-a/iter-1");
        assert_eq!(sb.git_ref, "refs/agentapp/run-actor-b/iter-1");
    }
}
