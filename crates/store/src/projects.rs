//! Project persistence (PRD data model: `Project { id, repo_path, plan_convention }`).

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// A registered project: a git repo the app supervises runs against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    /// Absolute, canonicalized repo path. Unique per project.
    pub repo_path: String,
    /// Plan convention: `"prd"` (PRD.md at root) or `"folder"` (plans/ dir).
    pub plan_convention: String,
}

/// Insert a project. Errors on a duplicate `repo_path` (the UNIQUE constraint),
/// which the caller maps to an "already registered" condition.
pub fn insert_project(conn: &Connection, project: &Project) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO projects (id, repo_path, plan_convention) VALUES (?1, ?2, ?3)",
        (&project.id, &project.repo_path, &project.plan_convention),
    )?;
    Ok(())
}

/// All registered projects, ordered by repo path for a stable listing.
pub fn list_projects(conn: &Connection) -> rusqlite::Result<Vec<Project>> {
    let mut stmt = conn
        .prepare("SELECT id, repo_path, plan_convention FROM projects ORDER BY repo_path")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Project {
                id: r.get(0)?,
                repo_path: r.get(1)?,
                plan_convention: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, path: &str) -> Project {
        Project {
            id: id.into(),
            repo_path: path.into(),
            plan_convention: "prd".into(),
        }
    }

    #[test]
    fn insert_then_list_roundtrips() {
        let conn = crate::open(":memory:").unwrap();
        insert_project(&conn, &project("p1", "/repos/b")).unwrap();
        insert_project(&conn, &project("p2", "/repos/a")).unwrap();
        let got = list_projects(&conn).unwrap();
        // Ordered by repo_path.
        assert_eq!(got, vec![project("p2", "/repos/a"), project("p1", "/repos/b")]);
    }

    #[test]
    fn duplicate_repo_path_is_rejected() {
        let conn = crate::open(":memory:").unwrap();
        insert_project(&conn, &project("p1", "/repos/a")).unwrap();
        let err = insert_project(&conn, &project("p2", "/repos/a"));
        assert!(err.is_err(), "duplicate repo_path must violate UNIQUE");
    }
}
