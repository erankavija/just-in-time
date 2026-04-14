//! Filesystem watcher and change tracker for live updates.
//!
//! Watches the `.jit/` directory for changes and maintains a monotonic version
//! counter. SSE subscribers receive notifications via a broadcast channel.
//!
//! Only graph-relevant file changes trigger a version bump. High-frequency
//! noise files (audit log, leases, temp files, server metadata) are ignored
//! so that the web UI graph is not constantly re-rendered during active work.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::broadcast;

/// Tracks changes to the `.jit/` directory and notifies subscribers.
///
/// Each detected change increments a monotonic version counter and broadcasts
/// the new version to all SSE subscribers.
#[derive(Clone)]
pub struct ChangeTracker {
    version: Arc<AtomicU64>,
    tx: broadcast::Sender<u64>,
}

impl ChangeTracker {
    /// Create a new ChangeTracker with the given broadcast capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            version: Arc::new(AtomicU64::new(0)),
            tx,
        }
    }

    /// Get the current version number.
    pub fn current_version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    /// Subscribe to change notifications. Returns a receiver that yields
    /// new version numbers on each change.
    pub fn subscribe(&self) -> broadcast::Receiver<u64> {
        self.tx.subscribe()
    }

    /// Bump the version and broadcast to subscribers. Returns the new version.
    pub fn notify_change(&self) -> u64 {
        let new_version = self.version.fetch_add(1, Ordering::Relaxed) + 1;
        // Ignore send errors — no subscribers is fine
        let _ = self.tx.send(new_version);
        new_version
    }
}

const DEBOUNCE_MS: u64 = 200;

/// Returns `true` if the notify event affects files that are displayed in the
/// graph — issue records, the index, gate registry, and gate run results.
///
/// High-frequency noise files that never affect the rendered graph are
/// explicitly excluded:
///
/// * `events.jsonl` — append-only audit log, written on every command
/// * `claims.jsonl` / `claims/` — lease coordination heartbeats
/// * `server.pid.json`, `server.log` — server lifecycle metadata
/// * `*.tmp` — atomic-write intermediaries (written then immediately renamed)
///
/// Paths that *do* trigger a refresh:
/// * `issues/*.json` — issue state, labels, gates, dependencies
/// * `index.json` — issue added or removed
/// * `gates.json` — gate registry changed
/// * `gate-runs/**` — automated gate execution results
pub fn is_graph_relevant(event: &notify::Event) -> bool {
    event.paths.iter().any(|path| {
        let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");

        // Skip atomic-write temp files.
        if filename.ends_with(".tmp") {
            return false;
        }

        // Ignore known high-frequency noise files.
        if matches!(
            filename,
            "events.jsonl" | "claims.jsonl" | "server.pid.json" | "server.log"
        ) {
            return false;
        }

        // Ignore the claims/ lease directory.
        if path.components().any(|c| c.as_os_str() == "claims") {
            return false;
        }

        // Gate run results (any file under gate-runs/) are graph-relevant —
        // they update gate status shown on issue nodes.
        if path.components().any(|c| c.as_os_str() == "gate-runs") {
            return true;
        }

        // Issue records.
        let parent_name = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|f| f.to_str())
            .unwrap_or("");
        if parent_name == "issues" && filename.ends_with(".json") {
            return true;
        }

        // Top-level registry files.
        matches!(filename, "index.json" | "gates.json")
    })
}

/// Start watching the given data directory for changes.
///
/// Returns a `ChangeTracker` and keeps the watcher alive. The caller must
/// keep the returned `RecommendedWatcher` alive for watching to continue.
/// A background Tokio task handles debouncing filesystem events (~200ms).
pub fn start_watching(data_dir: &str) -> Result<(ChangeTracker, RecommendedWatcher)> {
    let tracker = ChangeTracker::new(64);

    // Use a tokio mpsc channel — the sender is Send and can be used from notify's thread
    let (fs_tx, mut fs_rx) = tokio::sync::mpsc::channel::<()>(128);

    let mut watcher = notify::recommended_watcher(
        move |res: std::result::Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                if is_graph_relevant(&event) {
                    let _ = fs_tx.try_send(());
                }
            }
        },
    )?;

    watcher.watch(Path::new(data_dir), RecursiveMode::Recursive)?;

    // Spawn a Tokio task that drains the channel with debouncing
    let debounce_tracker = tracker.clone();
    tokio::spawn(async move {
        while fs_rx.recv().await.is_some() {
            // Got an event — debounce: sleep then drain any queued events
            tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;
            while fs_rx.try_recv().is_ok() {}
            debounce_tracker.notify_change();
        }
    });

    Ok((tracker, watcher))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_version_is_zero() {
        let tracker = ChangeTracker::new(16);
        assert_eq!(tracker.current_version(), 0);
    }

    #[test]
    fn test_notify_change_increments_version() {
        let tracker = ChangeTracker::new(16);
        assert_eq!(tracker.notify_change(), 1);
        assert_eq!(tracker.notify_change(), 2);
        assert_eq!(tracker.current_version(), 2);
    }

    #[tokio::test]
    async fn test_subscriber_receives_version() {
        let tracker = ChangeTracker::new(16);
        let mut rx = tracker.subscribe();

        tracker.notify_change();
        let version = rx.recv().await.unwrap();
        assert_eq!(version, 1);
    }

    #[tokio::test]
    async fn test_multiple_subscribers_receive_same_version() {
        let tracker = ChangeTracker::new(16);
        let mut rx1 = tracker.subscribe();
        let mut rx2 = tracker.subscribe();

        tracker.notify_change();
        assert_eq!(rx1.recv().await.unwrap(), 1);
        assert_eq!(rx2.recv().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_clone_shares_state() {
        let tracker = ChangeTracker::new(16);
        let clone = tracker.clone();

        tracker.notify_change();
        assert_eq!(clone.current_version(), 1);

        clone.notify_change();
        assert_eq!(tracker.current_version(), 2);
    }

    #[tokio::test]
    async fn test_start_watching_detects_file_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().to_str().unwrap();

        let (tracker, _watcher) = start_watching(data_dir).unwrap();
        let mut rx = tracker.subscribe();

        // Write a file into the watched directory
        std::fs::write(tmp.path().join("index.json"), "{}").unwrap();

        // Wait for debounced notification (200ms debounce + margin)
        let version = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out waiting for change notification")
            .unwrap();

        assert!(version >= 1, "version should be at least 1, got {version}");
    }

    #[tokio::test]
    async fn test_start_watching_ignores_events_jsonl() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().to_str().unwrap();

        let (tracker, _watcher) = start_watching(data_dir).unwrap();

        // Write an events.jsonl entry (should NOT trigger a version bump)
        std::fs::write(tmp.path().join("events.jsonl"), "{}\n").unwrap();

        // Wait longer than the debounce window
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(
            tracker.current_version(),
            0,
            "events.jsonl write must not trigger a version bump"
        );
    }

    #[tokio::test]
    async fn test_debounce_coalesces_burst_writes() {
        let tmp = tempfile::tempdir().unwrap();
        // Create the issues subdirectory *before* start_watching so notify's
        // recursive inotify registration covers it. Creating it after has
        // proven racy under parallel test load (the first file writes can
        // land before inotify sees the new subdir).
        let issues_dir = tmp.path().join("issues");
        std::fs::create_dir_all(&issues_dir).unwrap();
        let data_dir = tmp.path().to_str().unwrap();

        let (tracker, _watcher) = start_watching(data_dir).unwrap();

        // Write multiple relevant files in rapid succession.
        for i in 0..5 {
            std::fs::write(issues_dir.join(format!("{i}.json")), "{}").unwrap();
        }

        // Poll for up to ~5s instead of a fixed sleep — the notify backend
        // can take noticeably longer under parallel test load.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if tracker.current_version() >= 1 {
                break;
            }
            if std::time::Instant::now() >= deadline {
                panic!(
                    "should have at least one notification within 5s, got {}",
                    tracker.current_version()
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    // --- is_graph_relevant unit tests ---

    fn make_event(paths: &[&str]) -> notify::Event {
        use std::path::PathBuf;
        notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: paths.iter().map(|p| PathBuf::from(*p)).collect(),
            attrs: Default::default(),
        }
    }

    #[test]
    fn test_relevant_issue_json() {
        let e = make_event(&["/repo/.jit/issues/abc123.json"]);
        assert!(is_graph_relevant(&e));
    }

    #[test]
    fn test_relevant_index_json() {
        let e = make_event(&["/repo/.jit/index.json"]);
        assert!(is_graph_relevant(&e));
    }

    #[test]
    fn test_relevant_gates_json() {
        let e = make_event(&["/repo/.jit/gates.json"]);
        assert!(is_graph_relevant(&e));
    }

    #[test]
    fn test_relevant_gate_run_result() {
        let e = make_event(&["/repo/.jit/gate-runs/run-123/result.json"]);
        assert!(is_graph_relevant(&e));
    }

    #[test]
    fn test_irrelevant_events_jsonl() {
        let e = make_event(&["/repo/.jit/events.jsonl"]);
        assert!(!is_graph_relevant(&e));
    }

    #[test]
    fn test_irrelevant_claims_jsonl() {
        let e = make_event(&["/repo/.jit/claims.jsonl"]);
        assert!(!is_graph_relevant(&e));
    }

    #[test]
    fn test_irrelevant_claims_dir() {
        let e = make_event(&["/repo/.jit/claims/lease-abc.json"]);
        assert!(!is_graph_relevant(&e));
    }

    #[test]
    fn test_irrelevant_server_pid() {
        let e = make_event(&["/repo/.jit/server.pid.json"]);
        assert!(!is_graph_relevant(&e));
    }

    #[test]
    fn test_irrelevant_server_log() {
        let e = make_event(&["/repo/.jit/server.log"]);
        assert!(!is_graph_relevant(&e));
    }

    #[test]
    fn test_irrelevant_tmp_file() {
        let e = make_event(&["/repo/.jit/issues/abc123.tmp"]);
        assert!(!is_graph_relevant(&e));
    }
}
