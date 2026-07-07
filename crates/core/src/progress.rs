//! The per-run progress-file completion protocol (M3).
//!
//! Every run is bound to one task and writes its progress to an external,
//! app-managed file (outside the repo, keyed by run-id). "Done" is uniform
//! across all agents: the agent writes a line containing exactly the
//! [`COMPLETION_MARKER`] to that file (PRD open-question 2). The app is
//! read-only on the file — it only watches for the marker.
//!
//! This module is the single place that knows the marker's shape. Both the
//! [`run_loop`](crate::run_loop) (which checks at each pass boundary) and the
//! live [`watch_for_completion`] watcher consume it, so the loop and the run UI
//! can never disagree on what "complete" means.
//!
//! Detection is by polling, not fs-events: the progress file lives outside the
//! repo, polling is deterministic and needs no watcher dependency, and the
//! marker is append-only so there is nothing to miss between polls.

use std::path::Path;
use std::time::Duration;

use tokio::sync::oneshot;

/// The machine-readable marker the agent writes to signal its bound task is
/// fully done. It must appear on its own (trimmed) line.
pub const COMPLETION_MARKER: &str = "STATUS: COMPLETE";

/// Whether `contents` carries the completion marker on its own (trimmed) line.
/// Prose that merely mentions the marker mid-line does not trip this.
pub fn contents_mark_complete(contents: &str) -> bool {
    contents.lines().any(|l| l.trim() == COMPLETION_MARKER)
}

/// Read `path` and test for the marker. A missing or unreadable file is treated
/// as "not complete" — the agent simply has not written it yet.
pub fn file_marks_complete(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|c| contents_mark_complete(&c))
        .unwrap_or(false)
}

/// Watch `path` for the completion marker, polling every `interval`.
///
/// Resolves to `true` when the marker appears (checked once up front, then on
/// each poll), or `false` if `cancel` fires first — the run ended for another
/// reason (failed, stopped, iterations exhausted), so the app stops watching.
/// The caller cancels by sending on or dropping the paired [`oneshot::Sender`].
pub async fn watch_for_completion(
    path: &Path,
    interval: Duration,
    mut cancel: oneshot::Receiver<()>,
) -> bool {
    // Fast path: the marker may already be present (e.g. the watcher started
    // after the agent wrote it).
    if file_marks_complete(path) {
        return true;
    }
    loop {
        tokio::select! {
            // Sender sent () or was dropped — either way, stop watching.
            _ = &mut cancel => return false,
            _ = tokio::time::sleep(interval) => {
                if file_marks_complete(path) {
                    return true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn marker_on_its_own_line_is_complete() {
        assert!(contents_mark_complete("did work\nSTATUS: COMPLETE\n"));
        // Leading/trailing whitespace on the line is tolerated.
        assert!(contents_mark_complete("  STATUS: COMPLETE  "));
    }

    #[test]
    fn prose_mentioning_the_marker_is_not_complete() {
        assert!(!contents_mark_complete(
            "I will write STATUS: COMPLETE when done\n"
        ));
        assert!(!contents_mark_complete("nothing here yet\n"));
        assert!(!contents_mark_complete(""));
    }

    #[test]
    fn missing_file_is_not_complete() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!file_marks_complete(&dir.path().join("nope.md")));
    }

    #[test]
    fn present_file_with_marker_is_complete() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("progress.md");
        std::fs::write(&p, "pass 1 did work\nSTATUS: COMPLETE\n").unwrap();
        assert!(file_marks_complete(&p));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn returns_true_when_marker_already_present() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("progress.md");
        std::fs::write(&p, "STATUS: COMPLETE\n").unwrap();
        let (_tx, rx) = oneshot::channel();
        assert!(watch_for_completion(&p, Duration::from_millis(5), rx).await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn detects_marker_appearing_after_start() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("progress.md");
        std::fs::write(&p, "pass 1 did work\n").unwrap();

        let writer_path = p.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&writer_path)
                .unwrap();
            writeln!(f, "STATUS: COMPLETE").unwrap();
        });

        let (_tx, rx) = oneshot::channel();
        assert!(watch_for_completion(&p, Duration::from_millis(5), rx).await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_stops_watching_without_completion() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("progress.md");
        std::fs::write(&p, "pass 1 did work\n").unwrap();

        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let _ = tx.send(());
        });

        // Marker never written; cancel fires → false.
        assert!(!watch_for_completion(&p, Duration::from_millis(5), rx).await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dropping_the_sender_cancels() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("progress.md");
        std::fs::write(&p, "pass 1 did work\n").unwrap();

        let (tx, rx) = oneshot::channel();
        drop(tx); // sender gone → receiver resolves → watch returns false
        assert!(!watch_for_completion(&p, Duration::from_millis(5), rx).await);
    }
}
