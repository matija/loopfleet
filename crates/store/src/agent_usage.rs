//! Per-agent limit headroom: the most recent rate-limit observation for each
//! agent.
//!
//! Every `RateLimited` event the app sees carries a snapshot of that agent's
//! standing — when its limit resets and what it said about it. That snapshot is
//! per-agent state, not per-run state: a limit the `claude` adapter reported
//! during one run still applies to the next one. So it lives keyed by agent and
//! is overwritten on each observation; only the latest matters, and a caller
//! reads it to know an agent's headroom without replaying the event log.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// The latest rate-limit observation recorded for one agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentUsage {
    /// The agent this observation is about (e.g. `"claude"`).
    pub agent: String,
    /// ISO-8601 timestamp at which the agent said the limit resets, when it
    /// reported one. `None` means the agent gave no reset time — the limit is
    /// known to have been hit, but not when it lifts.
    pub reset_at: Option<String>,
    /// The agent's own description of the limit, when it gave one.
    pub message: Option<String>,
    /// When the app observed the event, unix millis.
    pub observed_at: i64,
}

/// Record an agent's latest rate-limit observation, replacing any previous one
/// for that agent. `observed_at` is unix millis; the caller supplies it so the
/// clock stays testable.
pub fn record_agent_usage(
    conn: &Connection,
    agent: &str,
    reset_at: Option<&str>,
    message: Option<&str>,
    observed_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO agent_usage (agent, reset_at, message, observed_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(agent) DO UPDATE SET
             reset_at    = excluded.reset_at,
             message     = excluded.message,
             observed_at = excluded.observed_at",
        params![agent, reset_at, message, observed_at],
    )?;
    Ok(())
}

/// The latest observation for `agent`, or `None` if the app has never seen that
/// agent hit a limit.
pub fn load_agent_usage(conn: &Connection, agent: &str) -> rusqlite::Result<Option<AgentUsage>> {
    conn.query_row(
        "SELECT agent, reset_at, message, observed_at FROM agent_usage WHERE agent = ?1",
        [agent],
        row_to_usage,
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

/// Every recorded agent observation, most recently observed first.
pub fn list_agent_usage(conn: &Connection) -> rusqlite::Result<Vec<AgentUsage>> {
    let mut stmt = conn.prepare(
        "SELECT agent, reset_at, message, observed_at
         FROM agent_usage ORDER BY observed_at DESC, agent",
    )?;
    let rows = stmt
        .query_map([], row_to_usage)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn row_to_usage(r: &rusqlite::Row<'_>) -> rusqlite::Result<AgentUsage> {
    Ok(AgentUsage {
        agent: r.get(0)?,
        reset_at: r.get(1)?,
        message: r.get(2)?,
        observed_at: r.get(3)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_is_none_before_any_observation() {
        let conn = crate::open(":memory:").unwrap();
        assert_eq!(load_agent_usage(&conn, "claude").unwrap(), None);
        assert!(list_agent_usage(&conn).unwrap().is_empty());
    }

    #[test]
    fn record_then_load_roundtrips() {
        let conn = crate::open(":memory:").unwrap();
        record_agent_usage(
            &conn,
            "claude",
            Some("2026-08-25T12:00:00Z"),
            Some("5-hour limit reached"),
            1_700_000_000_000,
        )
        .unwrap();

        assert_eq!(
            load_agent_usage(&conn, "claude").unwrap(),
            Some(AgentUsage {
                agent: "claude".into(),
                reset_at: Some("2026-08-25T12:00:00Z".into()),
                message: Some("5-hour limit reached".into()),
                observed_at: 1_700_000_000_000,
            })
        );
    }

    #[test]
    fn absent_snapshot_fields_roundtrip_as_none() {
        let conn = crate::open(":memory:").unwrap();
        record_agent_usage(&conn, "claude", None, None, 42).unwrap();

        let usage = load_agent_usage(&conn, "claude").unwrap().unwrap();
        assert_eq!(usage.reset_at, None);
        assert_eq!(usage.message, None);
        assert_eq!(usage.observed_at, 42);
    }

    #[test]
    fn a_later_observation_replaces_the_earlier_one() {
        let conn = crate::open(":memory:").unwrap();
        record_agent_usage(&conn, "claude", Some("t1"), Some("first"), 1_000).unwrap();
        record_agent_usage(&conn, "claude", Some("t2"), Some("second"), 2_000).unwrap();

        // One row per agent: the newest observation wins outright, including
        // clearing a field the agent no longer reports.
        assert_eq!(list_agent_usage(&conn).unwrap().len(), 1);
        assert_eq!(
            load_agent_usage(&conn, "claude").unwrap(),
            Some(AgentUsage {
                agent: "claude".into(),
                reset_at: Some("t2".into()),
                message: Some("second".into()),
                observed_at: 2_000,
            })
        );

        record_agent_usage(&conn, "claude", None, None, 3_000).unwrap();
        let usage = load_agent_usage(&conn, "claude").unwrap().unwrap();
        assert_eq!(usage.reset_at, None);
        assert_eq!(usage.message, None);
    }

    #[test]
    fn agents_are_tracked_independently() {
        let conn = crate::open(":memory:").unwrap();
        record_agent_usage(&conn, "claude", Some("t1"), None, 1_000).unwrap();
        record_agent_usage(&conn, "codex", Some("t2"), None, 2_000).unwrap();

        assert_eq!(
            load_agent_usage(&conn, "claude").unwrap().unwrap().reset_at,
            Some("t1".into())
        );
        assert_eq!(
            load_agent_usage(&conn, "codex").unwrap().unwrap().reset_at,
            Some("t2".into())
        );
        // Listed newest-observation first.
        let agents: Vec<String> = list_agent_usage(&conn)
            .unwrap()
            .into_iter()
            .map(|u| u.agent)
            .collect();
        assert_eq!(agents, vec!["codex".to_string(), "claude".to_string()]);
    }
}
