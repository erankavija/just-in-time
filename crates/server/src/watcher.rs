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
use jit::repository_state::VirtualPath;
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
/// graph: issue records ([`VirtualPath::ISSUES`]), the index, the gate registry,
/// and gate-run results ([`VirtualPath::GATE_RUNS`] and everything below it).
///
/// Atomic-write temp files and the high-frequency [`VirtualPath::EVENTS`],
/// [`VirtualPath::SERVER_PID`] and [`VirtualPath::SERVER_LOG`] files never
/// refresh the graph.
pub fn is_graph_relevant(event: &notify::Event) -> bool {
    event.paths.iter().any(|path| {
        let filename = path.file_name().and_then(|f| f.to_str());

        // Skip atomic-write temp files.
        if filename.is_some_and(|f| f.ends_with(".tmp")) {
            return false;
        }

        let names = |known: &VirtualPath| filename.is_some() && known.file_name() == filename;

        // Ignore known high-frequency noise files.
        if [
            VirtualPath::EVENTS,
            VirtualPath::SERVER_PID,
            VirtualPath::SERVER_LOG,
        ]
        .iter()
        .any(names)
        {
            return false;
        }

        // Gate run results (any file under the gate-runs directory) update the
        // gate status shown on issue nodes.
        let in_directory = |dir: &VirtualPath| {
            path.components()
                .any(|c| dir.file_name().is_some_and(|n| c.as_os_str() == n))
        };
        if in_directory(&VirtualPath::GATE_RUNS) {
            return true;
        }

        // Issue records.
        let in_issues = path
            .parent()
            .and_then(|p| p.file_name())
            .is_some_and(|n| VirtualPath::ISSUES.file_name().is_some_and(|i| n == i));
        if in_issues && filename.is_some_and(|f| f.ends_with(".json")) {
            return true;
        }

        // Top-level registry files.
        [VirtualPath::INDEX, VirtualPath::GATES].iter().any(names)
    })
}

/// Start watching the given data directory for changes.
///
/// Returns a `ChangeTracker` and keeps the watcher alive. The caller must
/// keep the returned `RecommendedWatcher` alive for watching to continue.
/// A background Tokio task handles debouncing filesystem events (~200ms).
pub fn start_watching(data_dir: impl AsRef<Path>) -> Result<(ChangeTracker, RecommendedWatcher)> {
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

    watcher.watch(data_dir.as_ref(), RecursiveMode::Recursive)?;

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
        let layout = jit::test_utils::repository_layout_for_data_root(tmp.path());
        std::fs::write(layout.resolve(&VirtualPath::INDEX).unwrap(), "{}").unwrap();

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
        let data_dir = tmp.path();
        let layout = jit::test_utils::repository_layout_for_data_root(data_dir);

        let (tracker, _watcher) = start_watching(data_dir).unwrap();

        // Write an audit-log entry (should NOT trigger a version bump).
        std::fs::write(layout.resolve(&VirtualPath::EVENTS).unwrap(), "{}\n").unwrap();

        // Wait longer than the debounce window
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(
            tracker.current_version(),
            0,
            "audit-log writes must not trigger a version bump"
        );
    }

    #[tokio::test]
    async fn test_debounce_coalesces_burst_writes() {
        let tmp = tempfile::tempdir().unwrap();
        // Create the issues subdirectory *before* start_watching so notify's
        // recursive inotify registration covers it. Creating it after has
        // proven racy under parallel test load (the first file writes can
        // land before inotify sees the new subdir).
        let layout = jit::test_utils::repository_layout_for_data_root(tmp.path());
        let issues_dir = layout.resolve(&VirtualPath::ISSUES).unwrap();
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

    fn make_event(paths: &[impl AsRef<Path>]) -> notify::Event {
        notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: paths
                .iter()
                .map(|path| path.as_ref().to_path_buf())
                .collect(),
            attrs: Default::default(),
        }
    }

    fn data_path(path: &VirtualPath) -> std::path::PathBuf {
        let temp = tempfile::tempdir().unwrap();
        let layout = jit::test_utils::repository_layout_for_data_root(temp.path());
        layout.resolve(path).unwrap()
    }

    fn make_data_event(path: &VirtualPath) -> notify::Event {
        make_event(&[data_path(path)])
    }

    #[test]
    fn test_relevant_issue_json() {
        let e = make_event(&[data_path(&VirtualPath::ISSUES).join("abc123.json")]);
        assert!(is_graph_relevant(&e));
    }

    #[test]
    fn test_relevant_index_json() {
        assert!(is_graph_relevant(&make_data_event(&VirtualPath::INDEX)));
    }

    #[test]
    fn test_relevant_gates_toml() {
        assert!(is_graph_relevant(&make_data_event(&VirtualPath::GATES)));
    }

    #[test]
    fn test_relevant_gate_run_result() {
        let e = make_data_event(&VirtualPath::gate_run_record(
            &jit::test_utils::issue_id(1),
            &jit::test_utils::gate_run_id(1),
        ));
        assert!(is_graph_relevant(&e));
    }

    #[test]
    fn test_irrelevant_events_jsonl() {
        assert!(!is_graph_relevant(&make_data_event(&VirtualPath::EVENTS)));
    }

    #[test]
    fn test_irrelevant_unlisted_data_file() {
        assert!(!is_graph_relevant(&make_data_event(&VirtualPath::CONFIG)));
    }

    #[test]
    fn test_irrelevant_server_pid() {
        assert!(!is_graph_relevant(&make_data_event(
            &VirtualPath::SERVER_PID
        )));
    }

    #[test]
    fn test_irrelevant_server_log() {
        assert!(!is_graph_relevant(&make_data_event(
            &VirtualPath::SERVER_LOG
        )));
    }

    #[test]
    fn test_irrelevant_tmp_file() {
        let e = make_event(&[data_path(&VirtualPath::ISSUES).join("abc123.tmp")]);
        assert!(!is_graph_relevant(&e));
    }
}
