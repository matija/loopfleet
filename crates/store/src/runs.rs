//! Run and iteration persistence (PRD data model: `Run`, `Iteration`).
//!
//! A run binds to one task (Model B: one run, one task) via
//! `(plan_id, task_anchor)`. The plan view reads runs back to DERIVE each task's
//! live `TaskStatus`; acceptance is a separate flag, not a status.

use rusqlite::{params, Connection};
use std::time::{SystemTime, UNIX_EPOCH};

/// A run to persist at launch. Worktree/branch/profile/progress paths are
/// app-managed (the git actor and sandbox produce them); the run starts in
/// whatever `status` the supervisor sets (`queued` or `running`).
#[derive(Debug, Clone)]
pub struct NewRun {
    pub id: String,
    pub plan_id: String,
    pub task_anchor: String,
    pub agent: String,
    /// Model override for this run (e.g. Claude's "opus"/"sonnet", or a pinned
    /// version string). `None` means the agent's own default.
    pub model: Option<String>,
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
           (id, plan_id, task_anchor, agent, model, worktree_path, branch,
            sb_profile, progress_path, max_iterations, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            run.id,
            run.plan_id,
            run.task_anchor,
            run.agent,
            run.model,
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

/// Terminal statuses (PRD data model): once a run reaches one of these, it
/// stops making progress and `finished_at` is stamped.
const TERMINAL_STATUSES: &[&str] = &["completed", "failed", "stopped", "limit-reached"];

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Advance a run's persisted status (`runs.status`). The caller validates the
/// transition via `RunState` before calling. When `status` is terminal
/// (completed/failed/stopped/limit-reached), also stamps `finished_at` to now;
/// otherwise `finished_at` is left untouched.
pub fn update_run_status(conn: &Connection, run_id: &str, status: &str) -> rusqlite::Result<()> {
    if TERMINAL_STATUSES.contains(&status) {
        conn.execute(
            "UPDATE runs SET status = ?2, finished_at = ?3 WHERE id = ?1",
            params![run_id, status, now_millis()],
        )?;
    } else {
        conn.execute(
            "UPDATE runs SET status = ?2 WHERE id = ?1",
            params![run_id, status],
        )?;
    }
    Ok(())
}

/// Count runs currently active (`queued` or `running`) across all projects.
/// The supervisor compares this against the settings concurrency cap before
/// launching another run.
pub fn count_active_runs(conn: &Connection) -> rusqlite::Result<u32> {
    conn.query_row(
        "SELECT COUNT(*) FROM runs WHERE status IN ('queued', 'running')",
        [],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n as u32)
}

/// Crash recovery: mark every run left in a non-terminal state
/// (`queued`/`running`) as `failed`, returning the affected run ids. Called once
/// at startup — a run still marked in-flight was interrupted by a prior crash or
/// quit, and its background task and agent process are gone (runs don't survive
/// app restart in v1). Only `runs.status` is touched: iterations and the
/// app-owned shadow refs are left intact (PRD: "keep refs").
pub fn fail_interrupted_runs(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "UPDATE runs SET status = 'failed'
         WHERE status IN ('queued', 'running')
         RETURNING id",
    )?;
    let ids = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

/// Mark a run accepted ("use this run"). Acceptance is a separate flag from
/// status (PRD data model): `"Implemented" = a run you accepted`. Idempotent.
pub fn set_run_accepted(conn: &Connection, run_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE runs SET accepted = 1 WHERE id = ?1",
        params![run_id],
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
    /// The plan this run's task is bound to, so a caller can look the task's
    /// full authored text back up by `(plan_id, task_anchor)`.
    pub plan_id: String,
    pub task_anchor: String,
    pub agent: String,
    /// The model override this run was launched with, if any.
    pub model: Option<String>,
    pub status: String,
    pub max_iterations: u32,
    /// The parent repository where this run's shadow refs live.
    pub repo_path: String,
    /// The run's worktree checkout path, if it still has one (M3 reap deletes
    /// this without clearing the column, so callers check the filesystem too).
    pub worktree_path: Option<String>,
    /// Whether this run was accepted ("use this run") — a flag separate from
    /// `status`, so a completed run may or may not have been merged.
    pub accepted: bool,
}

/// Load one run's detail (with its parent repo path), or `None` if absent.
pub fn load_run(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<RunDetail>> {
    conn.query_row(
        "SELECT r.id, r.plan_id, r.task_anchor, r.agent, r.model, r.status, r.max_iterations,
                pr.repo_path, r.worktree_path, r.accepted
         FROM runs r
         JOIN plans pl ON r.plan_id = pl.id
         JOIN projects pr ON pl.project_id = pr.id
         WHERE r.id = ?1",
        [run_id],
        |r| {
            Ok(RunDetail {
                id: r.get(0)?,
                plan_id: r.get(1)?,
                task_anchor: r.get(2)?,
                agent: r.get(3)?,
                model: r.get(4)?,
                status: r.get(5)?,
                max_iterations: r.get(6)?,
                repo_path: r.get(7)?,
                worktree_path: r.get(8)?,
                accepted: r.get::<_, i64>(9)? != 0,
            })
        },
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

/// Record that a terminal run's on-disk footprint (worktree, sandbox profile,
/// progress dir) has been reaped, stamping `reaped_at` to now. Idempotent.
pub fn mark_run_reaped(conn: &Connection, run_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE runs SET reaped_at = ?2 WHERE id = ?1",
        params![run_id, now_millis()],
    )?;
    Ok(())
}

/// A terminal, not-yet-reaped run as seen by the worktree sweep: enough to
/// decide eligibility (`accepted`, `finished_at`, falling back to the
/// worktree directory's mtime when NULL) without a second round trip.
#[derive(Debug, Clone)]
pub struct SweepCandidate {
    pub id: String,
    pub accepted: bool,
    pub finished_at: Option<i64>,
    pub worktree_path: Option<String>,
}

/// Every terminal run that hasn't been reaped yet, for `sweep_worktrees` to
/// filter by retention. Reaped runs are excluded since reaping is idempotent
/// but pointless to repeat.
pub fn list_sweep_candidates(conn: &Connection) -> rusqlite::Result<Vec<SweepCandidate>> {
    let mut stmt = conn.prepare(
        "SELECT id, accepted, finished_at, worktree_path
         FROM runs
         WHERE status IN ('completed', 'failed', 'stopped', 'limit-reached')
           AND reaped_at IS NULL",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(SweepCandidate {
            id: r.get(0)?,
            accepted: r.get::<_, i64>(1)? != 0,
            finished_at: r.get(2)?,
            worktree_path: r.get(3)?,
        })
    })?;
    rows.collect()
}

/// Every run id on record, for `sweep_worktrees` to tell an orphan worktree
/// directory (no matching run row at all) apart from a known run's checkout.
pub fn all_run_ids(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM runs")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

/// The project a run belongs to (joined through plan → project), or `None` if
/// the run doesn't exist. Used to re-arm a persisted pending resume at startup,
/// which only carries the run id, not the project id `launch_run` needs.
pub fn project_id_for_run(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT pl.project_id
         FROM runs r
         JOIN plans pl ON r.plan_id = pl.id
         WHERE r.id = ?1",
        [run_id],
        |r| r.get(0),
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

/// The agent/model/pass-count a plan's most recent run was launched with, for
/// "Continue plan" to carry forward the plan's last-used launch preferences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastLaunchPrefs {
    pub agent: String,
    pub model: Option<String>,
    pub max_iterations: u32,
}

/// The launch prefs of the most recently inserted run bound to `plan_id`, or
/// `None` if the plan has never had a run. Ordered by `rowid` (insertion
/// order) rather than `id`, since run ids are random UUIDs.
pub fn latest_launch_prefs_for_plan(
    conn: &Connection,
    plan_id: &str,
) -> rusqlite::Result<Option<LastLaunchPrefs>> {
    conn.query_row(
        "SELECT agent, model, max_iterations FROM runs
         WHERE plan_id = ?1 ORDER BY rowid DESC LIMIT 1",
        [plan_id],
        |r| {
            Ok(LastLaunchPrefs {
                agent: r.get(0)?,
                model: r.get(1)?,
                max_iterations: r.get(2)?,
            })
        },
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
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

/// A run's id and worktree path, scoped to one project, for `remove_project`
/// to reap each run's worktree before the project row goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRun {
    pub id: String,
    pub worktree_path: Option<String>,
}

/// Every run bound to any plan in `project_id`, regardless of status —
/// `remove_project` reaps each one's worktree itself rather than relying on
/// the sweep, since the project (and its runs' rows) are about to be deleted
/// outright.
pub fn list_runs_for_project(
    conn: &Connection,
    project_id: &str,
) -> rusqlite::Result<Vec<ProjectRun>> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.worktree_path
         FROM runs r
         JOIN plans pl ON r.plan_id = pl.id
         WHERE pl.project_id = ?1",
    )?;
    let rows = stmt
        .query_map([project_id], |r| {
            Ok(ProjectRun {
                id: r.get(0)?,
                worktree_path: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Whether `project_id` has any run still `queued` or `running`. Removing a
/// project out from under an in-flight run would orphan its background task
/// and cancel channel, so callers refuse removal while this is true.
pub fn has_active_runs_for_project(conn: &Connection, project_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM runs r JOIN plans pl ON r.plan_id = pl.id
             WHERE pl.project_id = ?1 AND r.status IN ('queued', 'running')
         )",
        [project_id],
        |r| r.get::<_, bool>(0),
    )
}

/// Whether `plan_id` has any run still `queued` or `running`. Same guard as
/// [`has_active_runs_for_project`], scoped to a single plan.
pub fn has_active_runs_for_plan(conn: &Connection, plan_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM runs WHERE plan_id = ?1 AND status IN ('queued', 'running')
         )",
        [plan_id],
        |r| r.get::<_, bool>(0),
    )
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
            model: None,
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

        assert_eq!(
            list_runs_for_plan(&conn, &pid).unwrap()[0].status,
            "completed"
        );
        let iters = load_iterations(&conn, "r1").unwrap();
        assert_eq!(iters.len(), 2);
        assert_eq!(iters[0].n, 1);
        assert_eq!(
            iters[0].shadow_ref.as_deref(),
            Some("refs/agentapp/run-r1/iter-1")
        );
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
    fn project_id_for_run_joins_through_plan() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        insert_run(&conn, &new_run("r1", &pid, "task a", "running")).unwrap();

        assert_eq!(
            project_id_for_run(&conn, "r1").unwrap().as_deref(),
            Some("p")
        );
        assert!(project_id_for_run(&conn, "nope").unwrap().is_none());
    }

    #[test]
    fn accept_run_sets_the_flag() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        insert_run(&conn, &new_run("r1", &pid, "task a", "completed")).unwrap();
        assert!(!list_runs_for_plan(&conn, &pid).unwrap()[0].accepted);

        set_run_accepted(&conn, "r1").unwrap();
        assert!(list_runs_for_plan(&conn, &pid).unwrap()[0].accepted);
        // Idempotent.
        set_run_accepted(&conn, "r1").unwrap();
        assert!(list_runs_for_plan(&conn, &pid).unwrap()[0].accepted);
    }

    #[test]
    fn crash_recovery_fails_interrupted_runs_and_keeps_refs() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        // Two in-flight (queued/running) + one already terminal.
        insert_run(&conn, &new_run("r1", &pid, "task a", "running")).unwrap();
        insert_run(&conn, &new_run("r2", &pid, "task a", "queued")).unwrap();
        insert_run(&conn, &new_run("r3", &pid, "task a", "completed")).unwrap();
        // r1 produced a snapshot before the crash.
        insert_iteration(&conn, "r1", 1, "refs/agentapp/run-r1/iter-1", Some(4)).unwrap();

        let mut failed = fail_interrupted_runs(&conn).unwrap();
        failed.sort();
        assert_eq!(failed, vec!["r1".to_string(), "r2".to_string()]);

        // Both in-flight runs are now failed; the completed one is untouched.
        let runs = list_runs_for_plan(&conn, &pid).unwrap();
        let status = |id: &str| runs.iter().find(|r| r.id == id).unwrap().status.clone();
        assert_eq!(status("r1"), "failed");
        assert_eq!(status("r2"), "failed");
        assert_eq!(status("r3"), "completed");
        // The shadow-ref record survives — recovery keeps refs.
        assert_eq!(load_iterations(&conn, "r1").unwrap().len(), 1);

        // Idempotent: a second startup finds nothing to recover.
        assert!(fail_interrupted_runs(&conn).unwrap().is_empty());
    }

    fn finished_at(conn: &Connection, run_id: &str) -> Option<i64> {
        conn.query_row(
            "SELECT finished_at FROM runs WHERE id = ?1",
            [run_id],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn terminal_status_stamps_finished_at() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        for (id, status) in [
            ("r1", "completed"),
            ("r2", "failed"),
            ("r3", "stopped"),
            ("r4", "limit-reached"),
        ] {
            insert_run(&conn, &new_run(id, &pid, "task a", "running")).unwrap();
            assert!(finished_at(&conn, id).is_none());
            update_run_status(&conn, id, status).unwrap();
            assert!(
                finished_at(&conn, id).is_some(),
                "{status} should stamp finished_at"
            );
        }
    }

    #[test]
    fn non_terminal_status_leaves_finished_at_untouched() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        insert_run(&conn, &new_run("r1", &pid, "task a", "queued")).unwrap();

        update_run_status(&conn, "r1", "running").unwrap();
        assert!(finished_at(&conn, "r1").is_none());

        update_run_status(&conn, "r1", "completed").unwrap();
        let first = finished_at(&conn, "r1").unwrap();

        // A further non-terminal transition (shouldn't normally happen, but
        // guards the "leave untouched" behavior) doesn't clear or bump it.
        update_run_status(&conn, "r1", "running").unwrap();
        assert_eq!(finished_at(&conn, "r1"), Some(first));
    }

    /// Seed a second project ("p2") with its own plan/task, for tests that
    /// need to confirm project-scoped queries don't leak across projects.
    fn seed_second_project(conn: &Connection) -> String {
        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES ('p2','/r2','prd')",
            [],
        )
        .unwrap();
        let pid = crate::plan_id("p2", "PRD.md");
        crate::upsert_plan(conn, &pid, "p2", "PRD.md").unwrap();
        crate::upsert_task(conn, &pid, "task b", 1, "Task B", false).unwrap();
        pid
    }

    #[test]
    fn list_runs_for_project_is_scoped_and_ignores_status() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        let pid2 = seed_second_project(&conn);
        insert_run(&conn, &new_run("r1", &pid, "task a", "completed")).unwrap();
        insert_run(&conn, &new_run("r2", &pid, "task a", "running")).unwrap();
        insert_run(&conn, &new_run("r3", &pid2, "task b", "completed")).unwrap();

        let mut runs = list_runs_for_project(&conn, "p").unwrap();
        runs.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(
            runs,
            vec![
                ProjectRun {
                    id: "r1".into(),
                    worktree_path: Some("/wt".into())
                },
                ProjectRun {
                    id: "r2".into(),
                    worktree_path: Some("/wt".into())
                },
            ]
        );
    }

    #[test]
    fn has_active_runs_for_project_checks_queued_and_running_only() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        let pid2 = seed_second_project(&conn);
        insert_run(&conn, &new_run("r1", &pid, "task a", "completed")).unwrap();
        assert!(!has_active_runs_for_project(&conn, "p").unwrap());

        insert_run(&conn, &new_run("r2", &pid2, "task b", "queued")).unwrap();
        assert!(!has_active_runs_for_project(&conn, "p").unwrap());
        assert!(has_active_runs_for_project(&conn, "p2").unwrap());
    }

    #[test]
    fn has_active_runs_for_plan_checks_queued_and_running_only() {
        let conn = crate::open(":memory:").unwrap();
        let pid = seed(&conn);
        let pid2 = crate::plan_id("p", "OTHER.md");
        crate::upsert_plan(&conn, &pid2, "p", "OTHER.md").unwrap();
        crate::upsert_task(&conn, &pid2, "task b", 1, "Task B", false).unwrap();

        insert_run(&conn, &new_run("r1", &pid, "task a", "completed")).unwrap();
        assert!(!has_active_runs_for_plan(&conn, &pid).unwrap());

        insert_run(&conn, &new_run("r2", &pid2, "task b", "queued")).unwrap();
        assert!(!has_active_runs_for_plan(&conn, &pid).unwrap());
        assert!(has_active_runs_for_plan(&conn, &pid2).unwrap());
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
