//! Plan and task persistence (PRD data model).
//!
//! Plans and their tasks are synced from the parsed plan file so a run can bind
//! to a stable `(plan_id, task_anchor)`. Per-task live state is DERIVED from run
//! records at read time (see `loopfleet_core::task_status`), never stored here;
//! `checked` is the authored "implemented" baseline (read as `Accepted` by
//! `derive_status`), not a live progress signal.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};

/// Deterministic plan id from its project id and plan file path. Re-syncing the
/// same file yields the same id, so runs launched earlier stay bound to it.
pub fn plan_id(project_id: &str, file_path: &str) -> String {
    format!("{project_id}::{file_path}")
}

/// Insert the plan row if absent (id is deterministic; the task list is the
/// mutable part and lives in `tasks`). Idempotent across re-syncs.
pub fn upsert_plan(
    conn: &Connection,
    id: &str,
    project_id: &str,
    file_path: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO plans (id, project_id, file_path) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET file_path = excluded.file_path",
        params![id, project_id, file_path],
    )?;
    Ok(())
}

/// The plan file path recorded for `plan_id`, or `None` if no such plan is
/// synced. Read-only lookup used to serve a plan's document without re-running
/// the overview sync.
pub fn plan_file_path(conn: &Connection, plan_id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT file_path FROM plans WHERE id = ?1",
        params![plan_id],
        |r| r.get(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

/// The project id owning `plan_id`, or `None` if no such plan is synced.
/// Read-only lookup used to resolve a plan reference back to its project,
/// mirroring `project_id_for_run`'s join-through-the-owner shape.
pub fn project_id_for_plan(conn: &Connection, plan_id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT project_id FROM plans WHERE id = ?1",
        params![plan_id],
        |r| r.get(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

/// Upsert one task by its anchor (the primary key), marking it present in the
/// file and its line usable as binding evidence.
///
/// Never deletes tasks that vanished from the file — a launched run may still
/// reference one via its FK. A vanished row is not harmless, though, which is
/// what `present`/`positional` are for: see [`mark_tasks_absent`] and
/// [`revoke_positional_recovery`], and call them around a sync rather than
/// relying on the upserts alone.
pub fn upsert_task(
    conn: &Connection,
    plan_id: &str,
    normalized_text: &str,
    line_hint: u32,
    text: &str,
    checked: bool,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO tasks (plan_id, normalized_text, line_hint, text, checked, present, positional)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, 1)
         ON CONFLICT(plan_id, normalized_text)
         DO UPDATE SET line_hint  = excluded.line_hint,
                       text       = excluded.text,
                       checked    = excluded.checked,
                       present    = 1,
                       positional = 1",
        params![plan_id, normalized_text, line_hint, text, checked as i64],
    )?;
    Ok(())
}

/// The anchors of every task that was in the plan file at the last sync.
///
/// Read *before* re-syncing: it is the only record of what the file used to say,
/// and the sync overwrites it. Comparing it with the freshly parsed anchors is
/// what tells a reworded task from a replaced plan.
pub fn present_task_anchors(conn: &Connection, plan_id: &str) -> rusqlite::Result<HashSet<String>> {
    let mut stmt =
        conn.prepare("SELECT normalized_text FROM tasks WHERE plan_id = ?1 AND present = 1")?;
    let rows = stmt.query_map(params![plan_id], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// Withdraw the positional signal from the plan's currently-present tasks,
/// permanently.
///
/// Called at the moment a sync finds that nothing in the file survived from the
/// last one: the plan at this path was replaced, so these tasks belong to a plan
/// that no longer exists and their lines are not evidence about the new one. The
/// flag is sticky by design — the alternative, re-deciding on every read, is
/// what let a single run launched against the new plan resurrect the old plan's
/// bindings.
pub fn revoke_positional_recovery(conn: &Connection, plan_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE tasks SET positional = 0 WHERE plan_id = ?1 AND present = 1",
        params![plan_id],
    )?;
    Ok(())
}

/// Mark every task in the plan absent, to be re-marked present by the upserts
/// that follow. Run once at the start of a sync so tasks that vanished from the
/// file are known to have vanished.
pub fn mark_tasks_absent(conn: &Connection, plan_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE tasks SET present = 0 WHERE plan_id = ?1",
        params![plan_id],
    )?;
    Ok(())
}

/// Every *positionally recoverable* anchor in a plan mapped to its last known
/// line.
///
/// Task rows outlive the file's current contents (`upsert_task` never deletes),
/// so this is the positional evidence a run carries about where its task used
/// to sit — the second signal `loopfleet_core::task_binding` resolves against
/// when a stored anchor no longer matches any parsed task's text.
///
/// Rows whose `positional` flag was revoked are omitted, so a run whose plan was
/// replaced gets no hint at all and cannot be bound by position. That filter is
/// the whole of the guard: `task_binding` needs no notion of it, because an
/// anchor with no hint already skips the positional signal.
pub fn task_line_hints(conn: &Connection, plan_id: &str) -> rusqlite::Result<HashMap<String, u32>> {
    let mut stmt = conn.prepare(
        "SELECT normalized_text, line_hint FROM tasks WHERE plan_id = ?1 AND positional = 1",
    )?;
    let rows = stmt.query_map(params![plan_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
    })?;
    rows.collect()
}

/// Move a plan to a new id and file path, carrying its tasks, runs, and
/// scheduled launches along with it, in one transaction.
///
/// `plans.id` has no `ON UPDATE CASCADE` and `store::open` turns `foreign_keys`
/// on, so updating the parent row first would fail FK checks against the
/// children still pointing at `old_id`. `PRAGMA defer_foreign_keys = ON` defers
/// FK enforcement to commit time (SQLite resets it automatically once the
/// transaction ends), letting the parent move before its children catch up.
/// Error carried by [`rekey_plan`] when the requested move is rejected.
///
/// Boxed into `rusqlite::Error::ToSqlConversionFailure` so the function can
/// stay on `rusqlite::Result` without a crate-wide custom error type.
#[derive(Debug)]
struct RekeyRejected(String);

impl std::fmt::Display for RekeyRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for RekeyRejected {}

fn rekey_rejected(msg: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(RekeyRejected(msg.into())))
}

fn plan_exists(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM plans WHERE id = ?1)",
        params![id],
        |r| r.get(0),
    )
}

pub fn rekey_plan(
    conn: &Connection,
    old_id: &str,
    new_id: &str,
    new_file_path: &str,
) -> rusqlite::Result<()> {
    conn.execute_batch("BEGIN")?;
    let result = (|| {
        // Validate before mutating anything: an unknown `old_id` or an
        // already-occupied `new_id` must reject cleanly rather than silently
        // no-op (unknown old_id) or merge two plans' tasks onto the same
        // `(plan_id, normalized_text)` primary key (occupied new_id).
        if !plan_exists(conn, old_id)? {
            return Err(rekey_rejected(format!("rekey_plan: no plan with id {old_id:?}")));
        }
        if old_id != new_id && plan_exists(conn, new_id)? {
            return Err(rekey_rejected(format!(
                "rekey_plan: a plan with id {new_id:?} already exists"
            )));
        }

        conn.execute_batch("PRAGMA defer_foreign_keys = ON")?;

        conn.execute(
            "UPDATE plans SET id = ?1, file_path = ?2 WHERE id = ?3",
            params![new_id, new_file_path, old_id],
        )?;
        conn.execute(
            "UPDATE tasks SET plan_id = ?1 WHERE plan_id = ?2",
            params![new_id, old_id],
        )?;
        conn.execute(
            "UPDATE runs SET plan_id = ?1 WHERE plan_id = ?2",
            params![new_id, old_id],
        )?;
        conn.execute(
            "UPDATE scheduled_launches SET plan_id = ?1 WHERE plan_id = ?2",
            params![new_id, old_id],
        )?;

        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(e) => {
            conn.execute_batch("ROLLBACK")?;
            Err(e)
        }
    }
}

/// One task row, read back by its `(plan_id, task_anchor)` key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    pub text: String,
    pub checked: bool,
}

/// Load one task's authored fields by its anchor, or `None` if no such task is
/// synced. Used by the report exporter, which needs a single task's text and
/// authored-checked state without re-parsing or re-syncing the plan file.
pub fn load_task(
    conn: &Connection,
    plan_id: &str,
    task_anchor: &str,
) -> rusqlite::Result<Option<TaskRow>> {
    conn.query_row(
        "SELECT text, checked FROM tasks WHERE plan_id = ?1 AND normalized_text = ?2",
        params![plan_id, task_anchor],
        |r| {
            Ok(TaskRow {
                text: r.get(0)?,
                checked: r.get::<_, i64>(1)? != 0,
            })
        },
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES (?1, ?2, 'prd')",
            params![id, format!("/repos/{id}")],
        )
        .unwrap();
    }

    #[test]
    fn plan_id_is_deterministic() {
        assert_eq!(plan_id("p1", "/r/PRD.md"), plan_id("p1", "/r/PRD.md"));
        assert_ne!(plan_id("p1", "/r/PRD.md"), plan_id("p2", "/r/PRD.md"));
    }

    #[test]
    fn upsert_plan_is_idempotent() {
        let conn = crate::open(":memory:").unwrap();
        project(&conn, "proj");
        let pid = plan_id("proj", "/repos/proj/PRD.md");
        upsert_plan(&conn, &pid, "proj", "/repos/proj/PRD.md").unwrap();
        upsert_plan(&conn, &pid, "proj", "/repos/proj/PRD.md").unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM plans", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn plan_file_path_returns_synced_path_or_none() {
        let conn = crate::open(":memory:").unwrap();
        project(&conn, "proj");
        let pid = plan_id("proj", "/repos/proj/PRD.md");
        upsert_plan(&conn, &pid, "proj", "/repos/proj/PRD.md").unwrap();

        assert_eq!(
            plan_file_path(&conn, &pid).unwrap().as_deref(),
            Some("/repos/proj/PRD.md")
        );
        assert_eq!(plan_file_path(&conn, "proj::missing.md").unwrap(), None);
    }

    #[test]
    fn upsert_task_inserts_then_updates_authored_state() {
        let conn = crate::open(":memory:").unwrap();
        project(&conn, "proj");
        let pid = plan_id("proj", "PRD.md");
        upsert_plan(&conn, &pid, "proj", "PRD.md").unwrap();

        upsert_task(&conn, &pid, "do the thing", 5, "Do the thing", false).unwrap();
        // Re-sync with an edited line hint and a now-authored-checked state.
        upsert_task(&conn, &pid, "do the thing", 7, "Do the thing", true).unwrap();

        let (line, checked): (u32, i64) = conn
            .query_row(
                "SELECT line_hint, checked FROM tasks WHERE plan_id=?1 AND normalized_text=?2",
                params![pid, "do the thing"],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(line, 7);
        assert_eq!(checked, 1);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "upsert must not duplicate the anchor");
    }

    #[test]
    fn rekey_plan_moves_id_and_carries_children() {
        let conn = crate::open(":memory:").unwrap();
        project(&conn, "proj");
        let old_id = plan_id("proj", "PRD.md");
        upsert_plan(&conn, &old_id, "proj", "PRD.md").unwrap();
        upsert_task(&conn, &old_id, "do the thing", 5, "Do the thing", false).unwrap();

        crate::insert_run(
            &conn,
            &crate::NewRun {
                id: "run-1".into(),
                plan_id: old_id.clone(),
                task_anchor: "do the thing".into(),
                agent: "claude".into(),
                model: None,
                worktree_path: "/wt".into(),
                branch: "b".into(),
                sb_profile: "p".into(),
                progress_path: "/progress".into(),
                max_iterations: 1,
                status: "queued".into(),
            },
        )
        .unwrap();

        crate::insert_scheduled_launch(
            &conn,
            &crate::NewScheduledLaunch {
                plan_id: old_id.clone(),
                task_anchor: "do the thing".into(),
                agent: "claude".into(),
                model: None,
                pass_count: 1,
                launch_at: 0,
                origin: "manual".into(),
            },
        )
        .unwrap();

        let new_id = plan_id("proj", "PLAN.md");
        rekey_plan(&conn, &old_id, &new_id, "PLAN.md").unwrap();

        assert_eq!(plan_file_path(&conn, &new_id).unwrap().as_deref(), Some("PLAN.md"));
        assert_eq!(plan_file_path(&conn, &old_id).unwrap(), None);

        let task_plan_id: String = conn
            .query_row(
                "SELECT plan_id FROM tasks WHERE normalized_text = 'do the thing'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(task_plan_id, new_id);

        let run_plan_id: String = conn
            .query_row("SELECT plan_id FROM runs WHERE id = 'run-1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(run_plan_id, new_id);

        let sl_plan_id: String = conn
            .query_row("SELECT plan_id FROM scheduled_launches", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sl_plan_id, new_id);
    }

    #[test]
    fn rekey_plan_rejects_unknown_old_id() {
        let conn = crate::open(":memory:").unwrap();
        project(&conn, "proj");
        let new_id = plan_id("proj", "PLAN.md");

        let err = rekey_plan(&conn, "proj::missing.md", &new_id, "PLAN.md");
        assert!(err.is_err());

        assert_eq!(plan_file_path(&conn, &new_id).unwrap(), None);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM plans", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn rekey_plan_rejects_occupied_new_id() {
        let conn = crate::open(":memory:").unwrap();
        project(&conn, "proj");
        let old_id = plan_id("proj", "PRD.md");
        let new_id = plan_id("proj", "PLAN.md");
        upsert_plan(&conn, &old_id, "proj", "PRD.md").unwrap();
        upsert_task(&conn, &old_id, "old task", 1, "Old task", false).unwrap();
        upsert_plan(&conn, &new_id, "proj", "PLAN.md").unwrap();
        upsert_task(&conn, &new_id, "new task", 1, "New task", false).unwrap();

        let err = rekey_plan(&conn, &old_id, &new_id, "PLAN.md");
        assert!(err.is_err());

        // Both plans and both tasks must be untouched, not merged.
        assert_eq!(plan_file_path(&conn, &old_id).unwrap().as_deref(), Some("PRD.md"));
        assert_eq!(plan_file_path(&conn, &new_id).unwrap().as_deref(), Some("PLAN.md"));
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2, "tasks from the two plans must not merge");
        let old_task_plan_id: String = conn
            .query_row(
                "SELECT plan_id FROM tasks WHERE normalized_text = 'old task'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_task_plan_id, old_id);
    }
}
