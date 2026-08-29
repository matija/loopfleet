//! loopfleet store: SQLite persistence (runs, iterations, events, refs) with a
//! single-writer event log. This module owns the schema and its migrations; the
//! normalized event enum and higher-level queries land in later milestones.

pub use rusqlite::Connection;

mod event_log;
pub use event_log::{
    insert_event, load_events, EventLog, LogEntry, Sender as EventSender, StoredEvent,
};

mod projects;
pub use projects::{
    delete_project, insert_project, list_projects, project_removal_preview, DeleteProjectSummary,
    Project, ProjectRemovalPreview,
};

mod settings;
pub use settings::{
    load_settings, project_sandbox_writes, save_settings, set_project_sandbox_writes, Settings,
};

mod plans;
pub use plans::{
    load_task, mark_tasks_absent, plan_file_path, plan_id, present_task_anchors,
    project_id_for_plan, revoke_positional_recovery, task_line_hints, upsert_plan, upsert_task,
    TaskRow,
};

mod runs;
pub use runs::{
    all_run_ids, count_active_runs, fail_interrupted_runs, has_active_runs_for_project,
    insert_iteration, insert_run, latest_launch_prefs_for_plan, list_runs_for_plan,
    list_runs_for_project, list_sweep_candidates, load_iterations, load_run, mark_run_reaped,
    project_id_for_run, set_run_accepted, update_run_status, IterationRow, LastLaunchPrefs,
    NewRun, ProjectRun, RunDetail, RunSummary, SweepCandidate,
};

mod agent_usage;
pub use agent_usage::{list_agent_usage, load_agent_usage, record_agent_usage, AgentUsage};

mod pending_resumes;
pub use pending_resumes::{
    delete_pending_resume, has_pending_resume, insert_pending_resume, list_pending_resumes,
    list_pending_resumes_for_project, NewPendingResume, PendingResume,
};

mod scheduled_launches;
pub use scheduled_launches::{
    delete_scheduled_launch, insert_scheduled_launch, list_scheduled_launches,
    list_scheduled_launches_for_project, reschedule_launch, NewScheduledLaunch, ScheduledLaunch,
};

/// Ordered list of migrations. Each entry is `(name, sql)`; names double as the
/// applied-migrations key, so they must be unique and never reordered. Add new
/// migrations by appending — never editing an already-shipped entry.
const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("migrations/0001_init.sql")),
    ("0002_settings", include_str!("migrations/0002_settings.sql")),
    (
        "0003_pending_resumes",
        include_str!("migrations/0003_pending_resumes.sql"),
    ),
    (
        "0004_run_model",
        include_str!("migrations/0004_run_model.sql"),
    ),
    (
        "0004_run_finished_at",
        include_str!("migrations/0004_run_finished_at.sql"),
    ),
    (
        "0005_run_reaped_at",
        include_str!("migrations/0005_run_reaped_at.sql"),
    ),
    (
        "0006_agent_usage",
        include_str!("migrations/0006_agent_usage.sql"),
    ),
    (
        "0007_scheduled_launches",
        include_str!("migrations/0007_scheduled_launches.sql"),
    ),
    (
        "0008_scheduled_launch_reschedule",
        include_str!("migrations/0008_scheduled_launch_reschedule.sql"),
    ),
    (
        "0009_scheduled_launch_origin",
        include_str!("migrations/0009_scheduled_launch_origin.sql"),
    ),
    (
        "0010_task_presence",
        include_str!("migrations/0010_task_presence.sql"),
    ),
];

/// Open a SQLite database at `path`, enable foreign keys, and apply all pending
/// migrations. Use `":memory:"` for an ephemeral database (tests).
pub fn open<P: AsRef<std::path::Path>>(path: P) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate(&conn)?;
    Ok(conn)
}

/// Apply every migration not yet recorded in `schema_migrations`, in order, each
/// in its own transaction. Idempotent: already-applied migrations are skipped.
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             name       TEXT PRIMARY KEY,
             applied_at INTEGER NOT NULL DEFAULT (unixepoch())
         )",
    )?;

    for (name, sql) in MIGRATIONS {
        let already: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE name = ?1)",
            [name],
            |row| row.get(0),
        )?;
        if already {
            continue;
        }
        conn.execute_batch("BEGIN")?;
        match conn
            .execute_batch(sql)
            .and_then(|_| conn.execute("INSERT INTO schema_migrations (name) VALUES (?1)", [name]))
        {
            Ok(_) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                conn.execute_batch("ROLLBACK")?;
                return Err(e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rows
    }

    #[test]
    fn open_applies_full_schema() {
        let conn = open(":memory:").unwrap();
        let t = tables(&conn);
        for expected in [
            "projects",
            "plans",
            "tasks",
            "runs",
            "iterations",
            "sessions",
            "events",
            "pending_resumes",
            "agent_usage",
            "scheduled_launches",
            "schema_migrations",
        ] {
            assert!(t.contains(&expected.to_string()), "missing table {expected}");
        }
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = open(":memory:").unwrap();
        // Re-running must not error or re-apply.
        migrate(&conn).unwrap();
        let applied: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(applied, MIGRATIONS.len() as i64);
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let conn = open(":memory:").unwrap();
        // A plan referencing a non-existent project must be rejected.
        let err = conn.execute(
            "INSERT INTO plans (id, project_id, file_path) VALUES ('p1', 'nope', 'PRD.md')",
            [],
        );
        assert!(err.is_err(), "foreign key violation should be rejected");
    }

    #[test]
    fn events_seq_autoincrements() {
        let conn = open(":memory:").unwrap();
        conn.execute(
            "INSERT INTO events (run_or_session_id, normalized_event_json, ts)
             VALUES ('r1', '{}', 1), ('r1', '{}', 2)",
            [],
        )
        .unwrap();
        let seqs: Vec<i64> = conn
            .prepare("SELECT seq FROM events ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(seqs, vec![1, 2]);
    }
}
