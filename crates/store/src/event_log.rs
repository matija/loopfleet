//! The single-writer event log.
//!
//! Every normalized event — from any run, any adapter, plus the app-sourced
//! `FileChanged` lane — converges on one bounded channel drained by one writer
//! task that owns the SQLite connection. SQLite is single-writer, so this is the
//! only place rows are appended to `events`; concurrent producers never contend
//! for the write lock.
//!
//! The channel is bounded on purpose: **the bound is the backpressure.** A slow
//! writer (or a burst of events) stalls the producers at `send().await` rather
//! than growing an unbounded in-memory buffer.
//!
//! The store stays agnostic to the event type: it persists the already-encoded
//! `{"kind":…}` JSON that `NormalizedEvent` serializes to, so nothing here
//! depends on the `core` enum (which itself depends on the store). Producers
//! serialize before sending. `FileChanged` is pushed here by the app's worktree
//! watcher (M2/M3), never by an adapter — it enters through the same [`Sender`]
//! as adapter-sourced events, so the merge point is this channel and the
//! ordering it imposes.

use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// One event bound for the log: the owning run/session id
/// (`events.run_or_session_id`) plus the event's encoded `{"kind":…}` JSON.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub owner_id: String,
    pub event_json: String,
}

/// A clonable handle for pushing events onto the log. Cloned once per producer
/// (one forwarding task per run, plus the app's worktree watcher). The writer
/// stops when the last sender is dropped.
pub type Sender = mpsc::Sender<LogEntry>;

/// An event read back out of the log, with its append order (`seq`) and
/// timestamp (unix millis). `event_json` is the stored `{"kind":…}` payload;
/// callers deserialize it back to a `NormalizedEvent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    pub seq: i64,
    pub event_json: String,
    pub ts: i64,
}

/// A running single-writer event log: a bounded channel plus the writer task
/// that owns the connection and drains it.
pub struct EventLog {
    tx: Sender,
    writer: JoinHandle<rusqlite::Result<usize>>,
}

impl EventLog {
    /// Start the writer. It takes ownership of `conn` and appends every received
    /// [`LogEntry`] to `events` until all senders drop. `capacity` bounds the
    /// channel — the backpressure. The write runs on a blocking task so SQLite
    /// never blocks the async runtime's worker threads.
    pub fn spawn(conn: Connection, capacity: usize) -> EventLog {
        let (tx, mut rx) = mpsc::channel::<LogEntry>(capacity);
        let writer = tokio::task::spawn_blocking(move || -> rusqlite::Result<usize> {
            let mut stmt = conn.prepare(
                "INSERT INTO events (run_or_session_id, normalized_event_json, ts)
                 VALUES (?1, ?2, ?3)",
            )?;
            let mut written = 0usize;
            while let Some(entry) = rx.blocking_recv() {
                stmt.execute(params![entry.owner_id, entry.event_json, now_millis()])?;
                written += 1;
            }
            Ok(written)
        });
        EventLog { tx, writer }
    }

    /// A sender for pushing events onto the log. Clone one per producer.
    pub fn sender(&self) -> Sender {
        self.tx.clone()
    }

    /// Drop this log's own sender and wait for the writer to finish draining
    /// whatever remains once every other sender has also dropped. Returns the
    /// number of events written, or the first write error that stopped it.
    pub async fn shutdown(self) -> rusqlite::Result<usize> {
        let EventLog { tx, writer } = self;
        drop(tx);
        writer.await.expect("event-log writer task panicked")
    }
}

/// Read a run's or session's events back in append order. Used by the timeline
/// (M4) and by tests to prove what the writer stored comes back intact.
pub fn load_events(conn: &Connection, owner_id: &str) -> rusqlite::Result<Vec<StoredEvent>> {
    let mut stmt = conn.prepare(
        "SELECT seq, normalized_event_json, ts FROM events
         WHERE run_or_session_id = ?1 ORDER BY seq",
    )?;
    let rows = stmt.query_map([owner_id], |row| {
        Ok(StoredEvent {
            seq: row.get(0)?,
            event_json: row.get(1)?,
            ts: row.get(2)?,
        })
    })?;
    rows.collect()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open;

    /// A representative run's worth of encoded events, in the same `{"kind":…}`
    /// shape `NormalizedEvent` serializes to and the store persists.
    fn sample_run() -> Vec<String> {
        [
            r#"{"kind":"turn_started"}"#,
            r#"{"kind":"assistant_text","text":"hi"}"#,
            r#"{"kind":"tool_call","call_id":"c1","name":"read","input_excerpt":"{}"}"#,
            r#"{"kind":"tool_result","call_id":"c1","ok":true,"output_excerpt":"ok"}"#,
            r#"{"kind":"turn_completed","usage":{"input_tokens":3,"output_tokens":4}}"#,
            r#"{"kind":"ended"}"#,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    async fn send_all(tx: &Sender, owner: &str, events: &[String]) {
        for event_json in events {
            tx.send(LogEntry {
                owner_id: owner.into(),
                event_json: event_json.clone(),
            })
            .await
            .expect("writer is alive");
        }
    }

    /// End-to-end against a file-backed DB: written events read back byte-for-byte
    /// and in append order, across the writer/reader connection boundary.
    #[tokio::test]
    async fn round_trips_through_sqlite() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("log.db");
        let events = sample_run();

        let log = EventLog::spawn(open(&db).unwrap(), 8);
        send_all(&log.sender(), "run-1", &events).await;
        assert_eq!(log.shutdown().await.unwrap(), events.len());

        let reader = open(&db).unwrap();
        let stored = load_events(&reader, "run-1").unwrap();
        let got: Vec<String> = stored.iter().map(|s| s.event_json.clone()).collect();
        assert_eq!(got, events);
        // seq is strictly increasing in insertion order.
        assert!(stored.windows(2).all(|w| w[0].seq < w[1].seq));
    }

    /// The app-sourced `FileChanged` lane flows through the same writer channel
    /// as adapter-sourced events and is stored no differently — the channel is
    /// the merge point for both lanes.
    #[tokio::test]
    async fn file_changed_flows_through_the_same_channel() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("log.db");
        let log = EventLog::spawn(open(&db).unwrap(), 8);

        let adapter_tx = log.sender(); // stands in for an adapter forwarding task
        let app_tx = log.sender(); // stands in for the worktree watcher
        adapter_tx
            .send(LogEntry {
                owner_id: "run-1".into(),
                event_json: r#"{"kind":"turn_started"}"#.into(),
            })
            .await
            .unwrap();
        app_tx
            .send(LogEntry {
                owner_id: "run-1".into(),
                event_json: r#"{"kind":"file_changed","path":"src/lib.rs"}"#.into(),
            })
            .await
            .unwrap();
        drop(adapter_tx);
        drop(app_tx);

        assert_eq!(log.shutdown().await.unwrap(), 2);
        let reader = open(&db).unwrap();
        let stored = load_events(&reader, "run-1").unwrap();
        assert_eq!(
            stored.iter().map(|s| s.event_json.as_str()).collect::<Vec<_>>(),
            vec![
                r#"{"kind":"turn_started"}"#,
                r#"{"kind":"file_changed","path":"src/lib.rs"}"#,
            ]
        );
    }

    /// Events from concurrent runs share one writer but stay separable by owner.
    #[tokio::test]
    async fn separates_events_by_owner() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("log.db");
        let log = EventLog::spawn(open(&db).unwrap(), 8);

        let a = log.sender();
        let b = log.sender();
        send_all(
            &a,
            "run-a",
            &[
                r#"{"kind":"turn_started"}"#.into(),
                r#"{"kind":"ended"}"#.into(),
            ],
        )
        .await;
        send_all(&b, "run-b", &[r#"{"kind":"turn_started"}"#.into()]).await;
        drop(a);
        drop(b);
        log.shutdown().await.unwrap();

        let reader = open(&db).unwrap();
        assert_eq!(load_events(&reader, "run-a").unwrap().len(), 2);
        assert_eq!(load_events(&reader, "run-b").unwrap().len(), 1);
    }
}
