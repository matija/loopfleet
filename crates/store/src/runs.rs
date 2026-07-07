//! Run and iteration persistence (PRD data model: `Run`, `Iteration`).
//!
//! A run binds to one task (Model B: one run, one task) via
//! `(plan_id, task_anchor)`. The plan view reads runs back to DERIVE each task's
//! live `TaskStatus`; acceptance is a separate flag, not a status.

use rusqlite::{params, Connection};

/// A run to persist at launch. Worktree/branch/profile/progress paths are
/// app-managed (the git actor and sandbox produce them); the run starts in
/// whatever `status` the supervisor sets (`queued` or `running`).
#[derive(Debug, Clone)]
pub struct NewRun {
    pub id: String,
    pub plan_id: String,
    pub task_anchor: String,
    pub agent: String,
    pub worktree_path: String,
    pub branch: String,
    pub sb_profile: String,
    pub progress_path: String,
    pub max_iterations: u32,
    pub status: String,
}

/// Insert a launched run.
pub fn insert_run(conn: &Connection, run: &NewRun) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO runs
           (id, plan_id, task_anchor, agent, worktree_path, branch,
            sb_profile, progress_path, max_iterations, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            run.id,
            run.plan_id,
            run.task_anchor,
            run.agent,
            run.worktree_path,
            run.branch,
            run.sb_profile,
            run.progress_path,
            run.max_iterations,
            run.status,
        ],
    )?;
    Ok(())
}

/// Advance a run's persisted status (`runs.status`). The caller validates the
/// transition via `RunState` before calling.
pub fn update_run_status(conn: &Connection, run_id: &str, status: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE runs SET status = ?2 WHERE id = ?1",
        params![run_id, status],
    )?;
    Ok(())
}

/// Record one iteration's app-owned shadow-ref snapshot. `event_log_offset` is
/// the `seq` of this iteration's last event, so the timeline can partition a
/// run's flat event log back into per-iteration groups (`None` if unknown).
pub fn insert_iteration(
    conn: &Connection,
    run_id: &str,
    n: u32,
    shadow_ref: &str,
    event_log_offset: Option<i64>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO iterations (run_id, n, shadow_ref, event_log_offset)
         VALUES (?1, ?2, ?3, ?4)",
        params![run_id, n, shadow_ref, event_log_offset],
    )?;
    Ok(())
}

/// One iteration row, read back for the run timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IterationRow {
    pub n: u32,
    pub shadow_ref: Option<String>,
    /// The `seq` of this iteration's last event (its upper event boundary).
    pub event_log_offset: Option<i64>,
}

/// A run's iterations in pass order.
pub fn load_iterations(conn: &Connection, run_id: &str) -> rusqlite::Result<Vec<IterationRow>> {
    let mut stmt = conn.prepare(
        "SELECT n, shadow_ref, event_log_offset FROM iterations
         WHERE run_id = ?1 ORDER BY n",
    )?;
    let rows = stmt
        .query_map([run_id], |r| {
            Ok(IterationRow {
                n: r.get(0)?,
                shadow_ref: r.get(1)?,
                event_log_offset: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// One run with the parent repo it belongs to (joined through plan → project),
/// for the timeline view (which diffs the run's shadow refs in that repo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDetail {
    pub id: String,
    pub task_anchor: String,
    pub agent: String,
    pub status: String,
    pub max_iterations: u32,
    /// The parent repository where this run's shadow refs live.
    pub repo_path: String,
}

/// Load one run's detail (with its parent repo path), or `None` if absent.
pub fn load_run(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<RunDetail>> {
    conn.query_row(
        "SELECT r.id, r.task_anchor, r.agent, r.status, r.max_iterations, pr.repo_path
         FROM runs r
         JOIN plans pl ON r.plan_id = pl.id
         JOIN projects pr ON pl.project_id = pr.id
         WHERE r.id = ?1",
        [run_id],
        |r| {
            Ok(RunDetail {
                id: r.get(0)?,
                task_anchor: r.get(1)?,
                agent: r.get(2)?,
                status: r.get(3)?,
                max_iterations: r.get(4)?,
                repo_path: r.get(5)?,
            })
        },
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

/// A run's bearing on its task's derived status: just its `status` token and
/// acceptance flag, keyed by the task it is bound to.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RunSummary {
    pub id: String,
    pub task_anchor: String,
    pub status: String,
    pub accepted: bool,
}

/// Every run bound to any task in `plan_id`. The plan view groups these by
/// `task_anchor` and derives each task's `TaskStatus`.
pub fn list_runs_for_plan(conn: &Connection, plan_id: &str) -> rusqlite::Result<Vec<RunSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, task_anchor, status, accepted FROM runs
         WHERE plan_id = ?1 ORDER BY id",
    )?;
    let rows = stmt
        .query_map([plan_id], |r| {
            Ok(RunSummary {
                id: r.get(0)?,
                task_anchor: r.get(1)?,
                status: r.get(2)?,
                accepted: r.get::<_, i64>(3)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seed a project, plan, and one task so a run's FK `(plan_id, task_anchor)`
    /// resolves.
    fn seed(conn: &Connection) -> String {
        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES ('p','/r','prd')",
            [],
        )
        .unwrap();
        let pid = crate::plan_id("p", "PRD.md");
        crate::upsert_plan(conn, &pid, "p", "PRD.md").unwrap();
        crate::upsert_task(conn, &pid, "task a", 1, "Task A", false).unwrap();
        pid
    }

    fn new_run(id: &str, pid: &str, anchor: &str, status: &str) -> NewRun {
        NewRun {
            id: id.into(),
            plan_id: pid.into(),
            task_anchor: anchor.into(),
            agent: "claude".into(),
            worktree_path: "/wt".into(),
            branch: format!("agent/{id}"),
            sb_profile: "/prof.sb".into(),
            progress_path: "/prog/progress.md".into(),
            max_iterations: 5,
            status: status.into(),
        }
    }

    #[test]
    fn insert_then_list_by_plan() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        insert_run(&conn, &new_run("r1", &pid, "task a", "running")).unwrap();
        let runs = list_runs_for_plan(&conn, &pid).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].task_anchor, "task a");
        assert_eq!(runs[0].status, "running");
        assert!(!runs[0].accepted);
    }

    #[test]
    fn update_status_and_record_iterations() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        insert_run(&conn, &new_run("r1", &pid, "task a", "running")).unwrap();

        insert_iteration(&conn, "r1", 1, "refs/agentapp/run-r1/iter-1", Some(4)).unwrap();
        insert_iteration(&conn, "r1", 2, "refs/agentapp/run-r1/iter-2", Some(9)).unwrap();
        update_run_status(&conn, "r1", "completed").unwrap();

        assert_eq!(list_runs_for_plan(&conn, &pid).unwrap()[0].status, "completed");
        let iters = load_iterations(&conn, "r1").unwrap();
        assert_eq!(iters.len(), 2);
        assert_eq!(iters[0].n, 1);
        assert_eq!(iters[0].shadow_ref.as_deref(), Some("refs/agentapp/run-r1/iter-1"));
        assert_eq!(iters[1].event_log_offset, Some(9));
    }

    #[test]
    fn load_run_joins_repo_path() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        insert_run(&conn, &new_run("r1", &pid, "task a", "running")).unwrap();

        let detail = load_run(&conn, "r1").unwrap().unwrap();
        assert_eq!(detail.id, "r1");
        assert_eq!(detail.task_anchor, "task a");
        assert_eq!(detail.agent, "claude");
        assert_eq!(detail.repo_path, "/r");
        assert_eq!(detail.max_iterations, 5);
        assert!(load_run(&conn, "nope").unwrap().is_none());
    }

    #[test]
    fn run_requires_an_existing_task() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        // No task "ghost" exists → the FK (plan_id, task_anchor) is violated.
        let err = insert_run(&conn, &new_run("r1", &pid, "ghost", "running"));
        assert!(err.is_err(), "run must bind to a real task");
    }
}
