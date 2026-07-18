//! App settings (PRD M6): global defaults the launch UI reads and the
//! concurrency cap the supervisor enforces, plus per-project sandbox write
//! overrides.
//!
//! Global settings live in a key/value `settings` table; a missing key falls
//! back to [`Settings::default`], so the table only records what the user has
//! explicitly changed. Per-project overrides live on the `projects` row.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// Global app defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Pre-selected agent in the launch affordance.
    pub default_agent: String,
    /// Pre-selected iteration count in the launch affordance.
    pub default_iterations: u32,
    /// Max simultaneously active (queued/running) runs. A launch past this is
    /// rejected. `0` means no cap.
    pub concurrency_cap: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            default_agent: "claude".into(),
            default_iterations: 1,
            concurrency_cap: 3,
        }
    }
}

/// Load the saved settings, filling any unset key from [`Settings::default`].
pub fn load_settings(conn: &Connection) -> rusqlite::Result<Settings> {
    let mut s = Settings::default();
    if let Some(v) = get(conn, "default_agent")? {
        s.default_agent = v;
    }
    if let Some(v) = get(conn, "default_iterations")?.and_then(|v| v.parse().ok()) {
        s.default_iterations = v;
    }
    if let Some(v) = get(conn, "concurrency_cap")?.and_then(|v| v.parse().ok()) {
        s.concurrency_cap = v;
    }
    Ok(s)
}

/// Persist every settings field (upsert per key).
pub fn save_settings(conn: &Connection, s: &Settings) -> rusqlite::Result<()> {
    set(conn, "default_agent", &s.default_agent)?;
    set(conn, "default_iterations", &s.default_iterations.to_string())?;
    set(conn, "concurrency_cap", &s.concurrency_cap.to_string())?;
    Ok(())
}

fn get(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
}

fn set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// A project's per-project sandbox write overrides, as a list of paths (one per
/// stored line; blank lines dropped).
pub fn project_sandbox_writes(conn: &Connection, project_id: &str) -> rusqlite::Result<Vec<String>> {
    let raw: String = conn.query_row(
        "SELECT sandbox_extra_writes FROM projects WHERE id = ?1",
        [project_id],
        |r| r.get(0),
    )?;
    Ok(raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// Replace a project's sandbox write overrides (stored newline-separated).
pub fn set_project_sandbox_writes(
    conn: &Connection,
    project_id: &str,
    paths: &[String],
) -> rusqlite::Result<()> {
    let joined = paths
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    conn.execute(
        "UPDATE projects SET sandbox_extra_writes = ?2 WHERE id = ?1",
        params![project_id, joined],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_project(conn: &Connection) {
        conn.execute(
            "INSERT INTO projects (id, repo_path, plan_convention) VALUES ('p','/r','prd')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn load_returns_defaults_when_unset() {
        let conn = crate::open(":memory:").unwrap();
        assert_eq!(load_settings(&conn).unwrap(), Settings::default());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let conn = crate::open(":memory:").unwrap();
        let s = Settings {
            default_agent: "pi".into(),
            default_iterations: 10,
            concurrency_cap: 1,
        };
        save_settings(&conn, &s).unwrap();
        assert_eq!(load_settings(&conn).unwrap(), s);
        // Re-saving overwrites rather than duplicating keys.
        save_settings(&conn, &Settings::default()).unwrap();
        assert_eq!(load_settings(&conn).unwrap(), Settings::default());
    }

    #[test]
    fn project_writes_default_empty_and_roundtrip() {
        let conn = crate::open(":memory:").unwrap();
        seed_project(&conn);
        assert!(project_sandbox_writes(&conn, "p").unwrap().is_empty());

        set_project_sandbox_writes(&conn, "p", &["/opt/data".into(), "  ".into(), "/var/x".into()])
            .unwrap();
        // Blank entries dropped.
        assert_eq!(
            project_sandbox_writes(&conn, "p").unwrap(),
            vec!["/opt/data".to_string(), "/var/x".to_string()]
        );
    }
}
