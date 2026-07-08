//! The run timeline (M4): a run's iterations as rows, each with the normalized
//! events that occurred during it and the diff that iteration produced.
//!
//! This is the read side of the timeline UI. A run's events are stored as one
//! flat, `seq`-ordered log; each iteration records the `seq` of its last event
//! (`event_log_offset`), so this module partitions the log back into per-pass
//! groups. The per-iteration diff comes from the app-owned shadow refs via the
//! `git2` diff service. The app is read-only here — nothing is mutated.

use loopfleet_gitx::{iteration_diff_at, ChangeStatus, DiffResult};
use loopfleet_store::Connection;
use serde::Serialize;

/// A whole run's timeline: its metadata plus one row per iteration.
#[derive(Debug, Serialize)]
pub struct RunTimeline {
    pub run_id: String,
    pub agent: String,
    pub status: String,
    pub task_anchor: String,
    pub max_iterations: u32,
    pub iterations: Vec<IterationView>,
}

/// One iteration row: its snapshot ref, the events that occurred during it, and
/// the diff it produced (`None` if the shadow ref is missing or unreadable).
#[derive(Debug, Serialize)]
pub struct IterationView {
    pub n: u32,
    pub shadow_ref: Option<String>,
    pub events: Vec<TimelineEvent>,
    pub diff: Option<DiffView>,
}

/// One normalized event with its log position and timestamp. `event` is the
/// stored `{"kind":…}` payload, passed through as-is for the UI.
#[derive(Debug, Serialize)]
pub struct TimelineEvent {
    pub seq: i64,
    pub ts: i64,
    pub event: serde_json::Value,
}

/// An iteration's diff: a per-file summary plus the full unified patch.
#[derive(Debug, Serialize)]
pub struct DiffView {
    pub files: Vec<FileChangeView>,
    pub patch: String,
}

/// One file's change in an iteration diff.
#[derive(Debug, Serialize)]
pub struct FileChangeView {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub insertions: usize,
    pub deletions: usize,
}

/// Why a run timeline could not be built.
#[derive(Debug)]
pub enum TimelineError {
    /// No run with that id.
    NotFound(String),
    /// Reading the run, its iterations, or its events failed.
    Store(rusqlite::Error),
}

impl std::fmt::Display for TimelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TimelineError::NotFound(id) => write!(f, "no run: {id}"),
            TimelineError::Store(e) => write!(f, "reading timeline: {e}"),
        }
    }
}

impl std::error::Error for TimelineError {}

impl From<rusqlite::Error> for TimelineError {
    fn from(e: rusqlite::Error) -> Self {
        TimelineError::Store(e)
    }
}

/// Assemble the timeline for `run_id`: its metadata, its iterations in pass
/// order, the events grouped into each iteration, and each iteration's diff.
pub fn run_timeline(conn: &Connection, run_id: &str) -> Result<RunTimeline, TimelineError> {
    let run = loopfleet_store::load_run(conn, run_id)?
        .ok_or_else(|| TimelineError::NotFound(run_id.to_string()))?;
    let iterations = loopfleet_store::load_iterations(conn, run_id)?;
    let events = loopfleet_store::load_events(conn, run_id)?;

    // Partition the flat event log into per-iteration groups by `event_log_offset`
    // (each iteration owns events up to and including its offset). The last
    // iteration sweeps anything trailing so no event is dropped from the view.
    let mut events = events.into_iter().peekable();
    let last = iterations.len().saturating_sub(1);
    let repo = std::path::Path::new(&run.repo_path);

    let mut views = Vec::with_capacity(iterations.len());
    for (idx, it) in iterations.iter().enumerate() {
        let mut group = Vec::new();
        while let Some(e) = events.peek() {
            let within = it.event_log_offset.is_some_and(|off| e.seq <= off);
            if within || idx == last {
                let e = events.next().unwrap();
                group.push(TimelineEvent {
                    seq: e.seq,
                    ts: e.ts,
                    event: serde_json::from_str(&e.event_json)
                        .unwrap_or(serde_json::Value::String(e.event_json)),
                });
            } else {
                break;
            }
        }

        let diff = iteration_diff_at(repo, run_id, it.n).ok().map(to_diff_view);
        views.push(IterationView {
            n: it.n,
            shadow_ref: it.shadow_ref.clone(),
            events: group,
            diff,
        });
    }

    Ok(RunTimeline {
        run_id: run.id,
        agent: run.agent,
        status: run.status,
        task_anchor: run.task_anchor,
        max_iterations: run.max_iterations,
        iterations: views,
    })
}

/// Map a gitx [`DiffResult`] into the serializable [`DiffView`] the UI consumes.
/// Shared with the compare view (which shows each run's cumulative diff).
pub(crate) fn to_diff_view(d: DiffResult) -> DiffView {
    DiffView {
        files: d
            .files
            .into_iter()
            .map(|f| FileChangeView {
                path: f.path,
                old_path: f.old_path,
                status: status_token(f.status).to_string(),
                insertions: f.insertions,
                deletions: f.deletions,
            })
            .collect(),
        patch: d.patch,
    }
}

fn status_token(s: ChangeStatus) -> &'static str {
    match s {
        ChangeStatus::Added => "added",
        ChangeStatus::Modified => "modified",
        ChangeStatus::Deleted => "deleted",
        ChangeStatus::Renamed => "renamed",
        ChangeStatus::Copied => "copied",
        ChangeStatus::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loopfleet_store::NewRun;

    /// Seed a project (repo_path = a non-git temp dir so diffs resolve to `None`),
    /// a plan, one task, and a run bound to it.
    fn seed(conn: &Connection, repo_path: &str) {
        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES ('p', ?1, 'prd')",
            [repo_path],
        )
        .unwrap();
        let pid = loopfleet_store::plan_id("p", "PRD.md");
        loopfleet_store::upsert_plan(conn, &pid, "p", "PRD.md").unwrap();
        loopfleet_store::upsert_task(conn, &pid, "task a", 1, "Task A", false).unwrap();
        loopfleet_store::insert_run(
            conn,
            &NewRun {
                id: "r1".into(),
                plan_id: pid,
                task_anchor: "task a".into(),
                agent: "claude".into(),
                worktree_path: "/wt".into(),
                branch: "agent/r1".into(),
                sb_profile: "/p.sb".into(),
                progress_path: "/prog.md".into(),
                max_iterations: 3,
                status: "completed".into(),
            },
        )
        .unwrap();
    }

    #[test]
    fn groups_events_by_iteration_offset() {
        let dir = tempfile::tempdir().unwrap(); // not a git repo → diffs are None
        let conn = loopfleet_store::open(":memory:").unwrap();
        seed(&conn, &dir.path().to_string_lossy());

        // Five events on the run; seqs 1..=5 in a fresh db.
        for kind in ["turn_started", "assistant_text", "ended", "turn_started", "ended"] {
            loopfleet_store::insert_event(&conn, "r1", &format!("{{\"kind\":\"{kind}\"}}")).unwrap();
        }
        // Iteration 1 owns events up to seq 2; iteration 2 the rest.
        loopfleet_store::insert_iteration(&conn, "r1", 1, "refs/agentapp/run-r1/iter-1", Some(2))
            .unwrap();
        loopfleet_store::insert_iteration(&conn, "r1", 2, "refs/agentapp/run-r1/iter-2", Some(5))
            .unwrap();

        let tl = run_timeline(&conn, "r1").unwrap();
        assert_eq!(tl.agent, "claude");
        assert_eq!(tl.task_anchor, "task a");
        assert_eq!(tl.iterations.len(), 2);

        assert_eq!(tl.iterations[0].n, 1);
        assert_eq!(tl.iterations[0].events.len(), 2);
        assert_eq!(tl.iterations[0].events[0].seq, 1);
        assert!(tl.iterations[0].diff.is_none()); // repo is not a git repo

        assert_eq!(tl.iterations[1].events.len(), 3);
        assert_eq!(tl.iterations[1].events[2].seq, 5);
        // The passed-through payload keeps its {"kind":…} shape.
        assert_eq!(tl.iterations[1].events[2].event["kind"], "ended");
    }

    #[test]
    fn trailing_events_land_on_the_last_iteration() {
        let dir = tempfile::tempdir().unwrap();
        let conn = loopfleet_store::open(":memory:").unwrap();
        seed(&conn, &dir.path().to_string_lossy());

        for _ in 0..3 {
            loopfleet_store::insert_event(&conn, "r1", "{\"kind\":\"ended\"}").unwrap();
        }
        // Only one iteration, offset short of the last event: it must still sweep
        // every remaining event (nothing is dropped from the view).
        loopfleet_store::insert_iteration(&conn, "r1", 1, "refs/agentapp/run-r1/iter-1", Some(1))
            .unwrap();

        let tl = run_timeline(&conn, "r1").unwrap();
        assert_eq!(tl.iterations.len(), 1);
        assert_eq!(tl.iterations[0].events.len(), 3);
    }

    #[test]
    fn unknown_run_is_not_found() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        assert!(matches!(
            run_timeline(&conn, "ghost"),
            Err(TimelineError::NotFound(_))
        ));
    }
}
