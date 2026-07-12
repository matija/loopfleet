//! Plan and task persistence (PRD data model).
//!
//! Plans and their tasks are synced from the parsed plan file so a run can bind
//! to a stable `(plan_id, task_anchor)`. Per-task live state is DERIVED from run
//! records at read time (see `loopfleet_core::task_status`), never stored here;
//! `checked` is the authored "implemented" baseline (read as `Accepted` by
//! `derive_status`), not a live progress signal.

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

/// Upsert one task by its anchor (the primary key). Deliberately never deletes
/// tasks that vanished from the file: a launched run may still reference one via
/// its FK, and the plan view derives which tasks to show from the freshly parsed
/// file anyway, so a stale row is harmless.
pub fn upsert_task(
    conn: &Connection,
    plan_id: &str,
    normalized_text: &str,
    line_hint: u32,
    text: &str,
    checked: bool,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO tasks (plan_id, normalized_text, line_hint, text, checked)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(plan_id, normalized_text)
         DO UPDATE SET line_hint = excluded.line_hint,
                       text      = excluded.text,
                       checked   = excluded.checked",
        params![plan_id, normalized_text, line_hint, text, checked as i64],
    )?;
    Ok(())
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
}
