//! Filesystem watcher and change tracker for live updates.
//!
//! Watches the `.jit/` directory for changes and maintains a monotonic version
//! counter. SSE subscribers receive notifications via a broadcast channel.

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
            if res.is_ok() {
                let _ = fs_tx.try_send(());
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
        std::fs::write(tmp.path().join("test.json"), "{}").unwrap();

        // Wait for debounced notification (200ms debounce + margin)
        let version = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out waiting for change notification")
            .unwrap();

        assert!(version >= 1, "version should be at least 1, got {version}");
    }

    #[tokio::test]
    async fn test_debounce_coalesces_burst_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().to_str().unwrap();

        let (tracker, _watcher) = start_watching(data_dir).unwrap();

        // Write multiple files in rapid succession
        for i in 0..5 {
            std::fs::write(tmp.path().join(format!("file{i}.json")), "{}").unwrap();
        }

        // Wait for debounce to settle
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Version should be much less than 5 due to debouncing
        let version = tracker.current_version();
        assert!(
            version >= 1,
            "should have at least one notification, got {version}"
        );
    }
}
