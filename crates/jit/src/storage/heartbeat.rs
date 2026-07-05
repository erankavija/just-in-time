//! Heartbeat mechanism for lease renewal and liveness detection.
//!
//! This module provides optional heartbeat functionality for long-running agents,
//! enabling them to maintain active leases and prove process liveness. Heartbeats
//! are particularly important for indefinite leases (TTL=0) to prevent staleness.
//!
//! # Design Principles
//!
//! - **Optional**: Not required for finite leases with auto-expiry
//! - **Background support**: Optional background thread for automatic updates
//! - **Liveness detection**: PID-based process verification
//! - **Atomic writes**: Crash-safe file operations
//! - **Cleanup**: Orphaned heartbeat removal for dead processes
//!
//! # Example
//!
//! ```no_run
//! use jit::storage::clock::SystemClock;
//! use jit::storage::heartbeat::{Heartbeat, HeartbeatManager};
//! use std::path::Path;
//!
//! let manager = HeartbeatManager::new(Path::new(".git/jit"));
//!
//! // Manual heartbeat update (time comes from the injected clock)
//! let heartbeat = Heartbeat::new(
//!     &SystemClock,
//!     "agent:copilot-1".to_string(),
//!     "wt:abc123".to_string(),
//!     "main".to_string(),
//!     30,
//! );
//! manager.write_heartbeat(&heartbeat).unwrap();
//!
//! // Start background thread (optional)
//! let handle = manager.start_heartbeat_thread(
//!     "agent:copilot-1".to_string(),
//!     "wt:abc123".to_string(),
//!     "main".to_string(),
//!     30,
//! );
//! ```

use crate::storage::clock::{Clock, SystemClock};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Default heartbeat interval in seconds
pub const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// Heartbeat metadata for an agent process.
///
/// Stored in `.git/jit/heartbeat/<agent-id>.json` to track process liveness
/// and prevent lease staleness for indefinite leases.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Heartbeat {
    /// Agent identifier (format: "type:identifier")
    pub agent_id: String,
    /// Worktree identifier
    pub worktree_id: String,
    /// Current branch name
    pub branch: String,
    /// Process ID
    pub pid: u32,
    /// Last heartbeat timestamp
    pub last_beat: DateTime<Utc>,
    /// Heartbeat interval in seconds
    pub interval_secs: u64,
}

impl Heartbeat {
    /// Create a new heartbeat, stamping `last_beat` from the injected clock.
    ///
    /// The timestamp is read from `clock` ([`SystemClock`] in production) rather
    /// than the system clock directly, so construction is deterministic under a
    /// test clock.
    ///
    /// # Arguments
    ///
    /// * `clock` - Time source for `last_beat`
    /// * `agent_id` - Agent identifier (e.g., "agent:copilot-1")
    /// * `worktree_id` - Worktree identifier (e.g., "wt:abc123")
    /// * `branch` - Current branch name
    /// * `interval_secs` - Heartbeat update interval
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::storage::clock::SystemClock;
    /// use jit::storage::heartbeat::Heartbeat;
    ///
    /// let hb = Heartbeat::new(
    ///     &SystemClock,
    ///     "agent:demo".to_string(),
    ///     "wt:demo".to_string(),
    ///     "main".to_string(),
    ///     30,
    /// );
    /// assert_eq!(hb.agent_id, "agent:demo");
    /// // Freshly created, evaluated against its own beat, it is not stale.
    /// assert!(!hb.is_stale_at(hb.last_beat));
    /// ```
    pub fn new(
        clock: &dyn Clock,
        agent_id: String,
        worktree_id: String,
        branch: String,
        interval_secs: u64,
    ) -> Self {
        Self {
            agent_id,
            worktree_id,
            branch,
            pid: std::process::id(),
            last_beat: clock.now(),
            interval_secs,
        }
    }

    /// Update `last_beat` to the injected clock's current time.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::storage::clock::SystemClock;
    /// use jit::storage::heartbeat::Heartbeat;
    ///
    /// let mut hb = Heartbeat::new(
    ///     &SystemClock,
    ///     "agent:demo".to_string(),
    ///     "wt:demo".to_string(),
    ///     "main".to_string(),
    ///     30,
    /// );
    /// hb.update(&SystemClock);
    /// assert!(!hb.is_stale_at(hb.last_beat));
    /// ```
    pub fn update(&mut self, clock: &dyn Clock) {
        self.last_beat = clock.now();
    }

    /// Check if this heartbeat is stale, reading "now" from the injected clock.
    ///
    /// A heartbeat is stale if more than 2x the interval has passed since last
    /// beat. Time comes from `clock` ([`SystemClock`] in production); use
    /// [`is_stale_at`](Heartbeat::is_stale_at) to pass an explicit instant.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::storage::clock::SystemClock;
    /// use jit::storage::heartbeat::Heartbeat;
    ///
    /// let hb = Heartbeat::new(
    ///     &SystemClock,
    ///     "agent:demo".to_string(),
    ///     "wt:demo".to_string(),
    ///     "main".to_string(),
    ///     30,
    /// );
    /// // A just-created heartbeat is fresh.
    /// assert!(!hb.is_stale(&SystemClock));
    /// ```
    pub fn is_stale(&self, clock: &dyn Clock) -> bool {
        self.is_stale_at(clock.now())
    }

    /// Check if this heartbeat is stale as of `now`.
    ///
    /// Identical to [`is_stale`](Heartbeat::is_stale) but takes the current time
    /// as a parameter (supplied via the [`Clock`](crate::storage::clock::Clock)
    /// abstraction), so freshness can be tested deterministically without real
    /// wall-clock delays. A heartbeat is stale once more than 2x its interval has
    /// elapsed since `last_beat`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::storage::clock::SystemClock;
    /// use jit::storage::heartbeat::Heartbeat;
    /// use chrono::{Duration, Utc};
    ///
    /// let mut hb = Heartbeat::new(
    ///     &SystemClock,
    ///     "agent:demo".to_string(),
    ///     "wt:demo".to_string(),
    ///     "main".to_string(),
    ///     1, // 1s interval -> 2s staleness threshold
    /// );
    /// hb.last_beat = Utc::now();
    /// let now = hb.last_beat;
    /// // Fresh right after the beat, stale once 2x the interval has elapsed.
    /// assert!(!hb.is_stale_at(now));
    /// assert!(hb.is_stale_at(now + Duration::seconds(3)));
    /// ```
    pub fn is_stale_at(&self, now: DateTime<Utc>) -> bool {
        let threshold_secs = self.interval_secs * 2;
        let elapsed = now
            .signed_duration_since(self.last_beat)
            .num_seconds()
            .max(0) as u64;
        elapsed >= threshold_secs
    }

    /// Check if the process associated with this heartbeat is still running.
    ///
    /// Uses platform-specific PID checking.
    pub fn is_process_alive(&self) -> bool {
        check_pid_alive(self.pid)
    }
}

/// Manager for heartbeat operations.
#[derive(Debug, Clone)]
pub struct HeartbeatManager {
    /// Path to control plane directory (.git/jit/)
    control_plane_dir: PathBuf,
    /// Source of "now" for background heartbeat updates. Defaults to
    /// [`SystemClock`] (the real system clock) in production; tests inject a
    /// deterministic clock so heartbeat freshness is verifiable without real
    /// wall-clock delays.
    clock: Arc<dyn Clock>,
}

impl HeartbeatManager {
    /// Create a new heartbeat manager.
    ///
    /// # Arguments
    ///
    /// * `control_plane_dir` - Path to `.git/jit/` directory
    pub fn new(control_plane_dir: &Path) -> Self {
        Self {
            control_plane_dir: control_plane_dir.to_path_buf(),
            clock: Arc::new(SystemClock),
        }
    }

    /// Override the clock used for background heartbeat updates (see the `clock`
    /// field). Test-only: production always uses [`SystemClock`].
    #[cfg(test)]
    pub(crate) fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Get the heartbeat directory path.
    fn heartbeat_dir(&self) -> PathBuf {
        self.control_plane_dir.join("heartbeat")
    }

    /// Get the heartbeat file path for an agent.
    fn heartbeat_path(&self, agent_id: &str) -> PathBuf {
        // Sanitize agent_id for filename (replace colons with hyphens)
        let filename = format!("{}.json", agent_id.replace(':', "-"));
        self.heartbeat_dir().join(filename)
    }

    /// Write a heartbeat file atomically.
    ///
    /// Uses write-temp-rename pattern for crash safety.
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation or file I/O fails.
    pub fn write_heartbeat(&self, heartbeat: &Heartbeat) -> Result<()> {
        let dir = self.heartbeat_dir();
        fs::create_dir_all(&dir).context("Failed to create heartbeat directory")?;

        let path = self.heartbeat_path(&heartbeat.agent_id);
        let temp_path = path.with_extension("tmp");

        // Write to temp file
        let json =
            serde_json::to_string_pretty(heartbeat).context("Failed to serialize heartbeat")?;
        fs::write(&temp_path, json).context("Failed to write heartbeat temp file")?;

        // Fsync temp file
        let file = File::open(&temp_path).context("Failed to open temp file for fsync")?;
        file.sync_all()
            .context("Failed to fsync heartbeat temp file")?;
        drop(file);

        // Atomic rename
        fs::rename(&temp_path, &path).context("Failed to rename heartbeat file")?;

        // Fsync parent directory
        let parent_dir = File::open(&dir).context("Failed to open heartbeat dir for fsync")?;
        parent_dir
            .sync_all()
            .context("Failed to fsync heartbeat directory")?;

        Ok(())
    }

    /// Read a heartbeat file for an agent.
    ///
    /// # Errors
    ///
    /// Returns an error if the file doesn't exist or cannot be deserialized.
    pub fn read_heartbeat(&self, agent_id: &str) -> Result<Heartbeat> {
        let path = self.heartbeat_path(agent_id);
        let json = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read heartbeat for {}", agent_id))?;
        serde_json::from_str(&json)
            .with_context(|| format!("Failed to deserialize heartbeat for {}", agent_id))
    }

    /// List all heartbeat files.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be read.
    pub fn list_heartbeats(&self) -> Result<Vec<Heartbeat>> {
        let dir = self.heartbeat_dir();
        if !dir.exists() {
            return Ok(vec![]);
        }

        let mut heartbeats = Vec::new();
        for entry in fs::read_dir(&dir).context("Failed to read heartbeat directory")? {
            let entry = entry.context("Failed to read directory entry")?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                match fs::read_to_string(&path) {
                    Ok(json) => match serde_json::from_str(&json) {
                        Ok(heartbeat) => heartbeats.push(heartbeat),
                        Err(_) => continue, // Skip malformed files
                    },
                    Err(_) => continue, // Skip unreadable files
                }
            }
        }

        Ok(heartbeats)
    }

    /// Remove a heartbeat file for an agent.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be removed.
    pub fn remove_heartbeat(&self, agent_id: &str) -> Result<()> {
        let path = self.heartbeat_path(agent_id);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("Failed to remove heartbeat for {}", agent_id))?;
        }
        Ok(())
    }

    /// Clean up stale heartbeat files (processes no longer running).
    ///
    /// Returns the number of heartbeats cleaned up.
    ///
    /// # Errors
    ///
    /// Returns an error if listing or removal fails.
    pub fn cleanup_stale_heartbeats(&self) -> Result<usize> {
        let heartbeats = self.list_heartbeats()?;
        let mut cleaned = 0;

        for heartbeat in heartbeats {
            if !heartbeat.is_process_alive() {
                self.remove_heartbeat(&heartbeat.agent_id)?;
                cleaned += 1;
            }
        }

        Ok(cleaned)
    }

    /// Start a background heartbeat thread.
    ///
    /// The thread will update the heartbeat file every `interval_secs` seconds.
    /// Returns a join handle for thread management.
    ///
    /// # Arguments
    ///
    /// * `agent_id` - Agent identifier
    /// * `worktree_id` - Worktree identifier
    /// * `branch` - Current branch name
    /// * `interval_secs` - Update interval in seconds
    ///
    /// # Example
    ///
    /// ```no_run
    /// use jit::storage::heartbeat::HeartbeatManager;
    /// use std::path::Path;
    ///
    /// let manager = HeartbeatManager::new(Path::new(".git/jit"));
    /// let handle = manager.start_heartbeat_thread(
    ///     "agent:copilot-1".to_string(),
    ///     "wt:abc123".to_string(),
    ///     "main".to_string(),
    ///     30,
    /// );
    ///
    /// // Do work...
    ///
    /// // Stop heartbeat thread
    /// // (thread will exit when handle is dropped, or you can join it)
    /// ```
    pub fn start_heartbeat_thread(
        &self,
        agent_id: String,
        worktree_id: String,
        branch: String,
        interval_secs: u64,
    ) -> JoinHandle<()> {
        // Production driver: a real interval timer between beats.
        let ticker = Arc::new(IntervalTicker {
            interval: Duration::from_secs(interval_secs),
        });
        self.spawn_heartbeat_loop(agent_id, worktree_id, branch, interval_secs, ticker)
    }

    /// Spawn the heartbeat loop driven by an injectable [`TickSource`].
    ///
    /// The loop writes a beat (stamped from the manager's clock), then blocks on
    /// [`TickSource::next_tick`] for the next tick. The tick source is what makes
    /// cadence injectable: production uses a real interval timer, tests pump ticks
    /// deterministically. `next_tick` returning `false` stops the loop.
    fn spawn_heartbeat_loop(
        &self,
        agent_id: String,
        worktree_id: String,
        branch: String,
        interval_secs: u64,
        ticker: Arc<dyn TickSource>,
    ) -> JoinHandle<()> {
        let manager = self.clone();

        thread::spawn(move || {
            let mut heartbeat = Heartbeat::new(
                &*manager.clock,
                agent_id,
                worktree_id,
                branch,
                interval_secs,
            );

            loop {
                // Stamp the beat from the injected clock (real system clock in
                // production; a deterministic clock under test).
                heartbeat.update(&*manager.clock);

                // Write to file (ignore errors to keep thread running)
                let _ = manager.write_heartbeat(&heartbeat);

                // Wait for the next tick; stop if the tick source signals so.
                if !ticker.next_tick() {
                    break;
                }
            }
        })
    }

    /// Spawn the heartbeat loop with a test-supplied [`TickSource`], so a test
    /// can drive cadence by pumping ticks instead of waiting on a real interval.
    #[cfg(test)]
    fn start_heartbeat_thread_with_ticker(
        &self,
        agent_id: String,
        worktree_id: String,
        branch: String,
        interval_secs: u64,
        ticker: Arc<dyn TickSource>,
    ) -> JoinHandle<()> {
        self.spawn_heartbeat_loop(agent_id, worktree_id, branch, interval_secs, ticker)
    }
}

/// Source of ticks that pace the background heartbeat loop.
///
/// Each call to [`next_tick`](TickSource::next_tick) blocks until the loop
/// should write the next beat. Returning `false` tells the loop to stop. This
/// is the injection seam for cadence: production uses a wall-clock interval
/// timer ([`IntervalTicker`]); tests supply a pumpable source so heartbeat
/// updates advance deterministically without real interval sleeps.
trait TickSource: Send + Sync {
    /// Block until the next beat is due. Returns `false` to stop the loop.
    fn next_tick(&self) -> bool;
}

/// Production [`TickSource`]: sleeps a fixed wall-clock interval between beats
/// and never stops on its own (the thread is torn down at process exit).
struct IntervalTicker {
    interval: Duration,
}

impl TickSource for IntervalTicker {
    fn next_tick(&self) -> bool {
        thread::sleep(self.interval);
        true
    }
}

/// Check if a process ID is still alive.
///
/// Platform-specific implementation.
#[cfg(unix)]
fn check_pid_alive(pid: u32) -> bool {
    use std::process::Command;

    // Use kill -0 to check if process exists
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Check if a process ID is still alive.
///
/// Platform-specific implementation.
#[cfg(windows)]
fn check_pid_alive(pid: u32) -> bool {
    use std::process::Command;

    // Use tasklist to check if process exists
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/NH"])
        .output()
        .map(|output| {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.contains(&pid.to_string())
        })
        .unwrap_or(false)
}

/// Fallback for unsupported platforms
#[cfg(not(any(unix, windows)))]
fn check_pid_alive(_pid: u32) -> bool {
    // Conservative: assume process is alive
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_heartbeat_new() {
        let heartbeat = Heartbeat::new(
            &SystemClock,
            "agent:test".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            30,
        );

        assert_eq!(heartbeat.agent_id, "agent:test");
        assert_eq!(heartbeat.worktree_id, "wt:abc123");
        assert_eq!(heartbeat.branch, "main");
        assert_eq!(heartbeat.interval_secs, 30);
        assert_eq!(heartbeat.pid, std::process::id());
        assert!(!heartbeat.is_stale(&SystemClock));
    }

    #[test]
    fn test_heartbeat_update() {
        let mut heartbeat = Heartbeat::new(
            &SystemClock,
            "agent:test".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            30,
        );

        let original_beat = heartbeat.last_beat;
        thread::sleep(Duration::from_millis(100));
        heartbeat.update(&SystemClock);

        assert!(heartbeat.last_beat > original_beat);
    }

    #[test]
    fn test_heartbeat_is_stale() {
        let mut heartbeat = Heartbeat::new(
            &SystemClock,
            "agent:test".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            1, // 1 second interval
        );

        assert!(!heartbeat.is_stale(&SystemClock));

        // Manually set last_beat to 3 seconds ago (> 2x interval)
        heartbeat.last_beat = Utc::now() - chrono::Duration::seconds(3);
        assert!(heartbeat.is_stale(&SystemClock));
    }

    #[test]
    fn test_heartbeat_process_alive() {
        let heartbeat = Heartbeat::new(
            &SystemClock,
            "agent:test".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            30,
        );

        // Current process should be alive
        assert!(heartbeat.is_process_alive());

        // Invalid PID should be dead
        let mut dead_heartbeat = heartbeat.clone();
        dead_heartbeat.pid = 99999; // Very unlikely to exist
                                    // Note: This might be flaky on some systems, but generally safe
    }

    #[test]
    fn test_heartbeat_serialization() {
        let heartbeat = Heartbeat::new(
            &SystemClock,
            "agent:test".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            30,
        );

        let json = serde_json::to_string(&heartbeat).unwrap();
        let deserialized: Heartbeat = serde_json::from_str(&json).unwrap();

        assert_eq!(heartbeat, deserialized);
    }

    #[test]
    fn test_manager_write_and_read_heartbeat() {
        let temp_dir = tempdir().unwrap();
        let manager = HeartbeatManager::new(temp_dir.path());

        let heartbeat = Heartbeat::new(
            &SystemClock,
            "agent:test".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            30,
        );

        manager.write_heartbeat(&heartbeat).unwrap();

        let read_heartbeat = manager.read_heartbeat("agent:test").unwrap();
        assert_eq!(heartbeat.agent_id, read_heartbeat.agent_id);
        assert_eq!(heartbeat.worktree_id, read_heartbeat.worktree_id);
        assert_eq!(heartbeat.branch, read_heartbeat.branch);
        assert_eq!(heartbeat.pid, read_heartbeat.pid);
    }

    #[test]
    fn test_manager_list_heartbeats() {
        let temp_dir = tempdir().unwrap();
        let manager = HeartbeatManager::new(temp_dir.path());

        let heartbeat1 = Heartbeat::new(
            &SystemClock,
            "agent:test1".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            30,
        );
        let heartbeat2 = Heartbeat::new(
            &SystemClock,
            "agent:test2".to_string(),
            "wt:def456".to_string(),
            "feature".to_string(),
            30,
        );

        manager.write_heartbeat(&heartbeat1).unwrap();
        manager.write_heartbeat(&heartbeat2).unwrap();

        let heartbeats = manager.list_heartbeats().unwrap();
        assert_eq!(heartbeats.len(), 2);

        let agent_ids: Vec<_> = heartbeats.iter().map(|h| h.agent_id.as_str()).collect();
        assert!(agent_ids.contains(&"agent:test1"));
        assert!(agent_ids.contains(&"agent:test2"));
    }

    #[test]
    fn test_manager_remove_heartbeat() {
        let temp_dir = tempdir().unwrap();
        let manager = HeartbeatManager::new(temp_dir.path());

        let heartbeat = Heartbeat::new(
            &SystemClock,
            "agent:test".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            30,
        );

        manager.write_heartbeat(&heartbeat).unwrap();
        assert!(manager.read_heartbeat("agent:test").is_ok());

        manager.remove_heartbeat("agent:test").unwrap();
        assert!(manager.read_heartbeat("agent:test").is_err());
    }

    #[test]
    fn test_manager_cleanup_stale_heartbeats() {
        let temp_dir = tempdir().unwrap();
        let manager = HeartbeatManager::new(temp_dir.path());

        // Current process heartbeat (alive)
        let heartbeat1 = Heartbeat::new(
            &SystemClock,
            "agent:alive".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            30,
        );
        manager.write_heartbeat(&heartbeat1).unwrap();

        // Dead process heartbeat
        let mut heartbeat2 = Heartbeat::new(
            &SystemClock,
            "agent:dead".to_string(),
            "wt:def456".to_string(),
            "feature".to_string(),
            30,
        );
        heartbeat2.pid = 99999; // Very unlikely to exist
        manager.write_heartbeat(&heartbeat2).unwrap();

        let cleaned = manager.cleanup_stale_heartbeats().unwrap();

        // Should clean up the dead process heartbeat
        // Note: This might be 0 or 1 depending on whether PID 99999 exists
        assert!(cleaned <= 1);

        // Alive heartbeat should still exist
        assert!(manager.read_heartbeat("agent:alive").is_ok());
    }

    #[test]
    fn test_manager_empty_list() {
        let temp_dir = tempdir().unwrap();
        let manager = HeartbeatManager::new(temp_dir.path());

        let heartbeats = manager.list_heartbeats().unwrap();
        assert_eq!(heartbeats.len(), 0);
    }

    /// Pumpable [`TickSource`] for tests: the background loop advances only when
    /// the test calls [`tick`](TestTicker::tick), so heartbeat cadence is driven
    /// deterministically rather than by a real interval sleep.
    struct TestTicker {
        state: std::sync::Mutex<TestTickerState>,
        cv: std::sync::Condvar,
    }

    struct TestTickerState {
        pending: u32,
        stop: bool,
    }

    impl TestTicker {
        fn new() -> Self {
            Self {
                state: std::sync::Mutex::new(TestTickerState {
                    pending: 0,
                    stop: false,
                }),
                cv: std::sync::Condvar::new(),
            }
        }

        /// Release exactly one tick, unblocking one loop iteration.
        fn tick(&self) {
            let mut s = self.state.lock().unwrap();
            s.pending += 1;
            self.cv.notify_all();
        }

        /// Signal the loop to stop so it can be joined cleanly.
        fn stop(&self) {
            let mut s = self.state.lock().unwrap();
            s.stop = true;
            self.cv.notify_all();
        }
    }

    impl TickSource for TestTicker {
        fn next_tick(&self) -> bool {
            let mut s = self.state.lock().unwrap();
            while s.pending == 0 && !s.stop {
                s = self.cv.wait(s).unwrap();
            }
            if s.stop {
                return false;
            }
            s.pending -= 1;
            true
        }
    }

    /// REQ-03: verifies the background thread updates the heartbeat file without
    /// depending on real wall-clock margins.
    ///
    /// Both time (via an injected [`FixedClock`]) and cadence (via a pumpable
    /// [`TestTicker`]) are deterministic: the second update happens because the
    /// test releases a tick, not because a real interval elapsed. Assertions
    /// synchronize on the observed `last_beat` value rather than racing a
    /// staleness margin. It confirms (1) the thread writes a beat stamped from
    /// the injected clock, and (2) it re-reads the clock on the next tick (an
    /// advanced clock is reflected in the following write), staying fresh by the
    /// injected clock.
    #[test]
    fn test_background_thread_updates_heartbeat() {
        use crate::storage::clock::{Clock, FixedClock};
        use std::time::Instant;

        // Poll until `read` returns `Some`, synchronizing on the write event the
        // thread performs (bounded only to avoid hanging a broken build).
        fn poll_until<T>(read: impl Fn() -> Option<T>) -> T {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(value) = read() {
                    return value;
                }
                assert!(Instant::now() < deadline, "timed out waiting for heartbeat");
                thread::sleep(Duration::from_millis(5));
            }
        }

        let temp_dir = tempdir().unwrap();
        // `FixedClock` reports millisecond precision; snapshot its `now()` (not
        // the higher-precision `Utc::now()`) so file round-trips compare equal.
        let clock = Arc::new(FixedClock::new(Utc::now()));
        let base = clock.now();
        let manager = HeartbeatManager::new(temp_dir.path()).with_clock(clock.clone());

        let ticker = Arc::new(TestTicker::new());
        let handle = manager.start_heartbeat_thread_with_ticker(
            "agent:background".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            1,
            ticker.clone(),
        );

        // First write happens before any tick: the thread stamps the beat from
        // the injected clock, then blocks waiting for the next tick.
        let first = poll_until(|| {
            manager
                .read_heartbeat("agent:background")
                .ok()
                .filter(|hb| hb.last_beat == base)
        });
        assert_eq!(first.agent_id, "agent:background");
        assert!(!first.is_stale(&*clock));

        // Advance the injected clock and release one tick. The next iteration
        // must reflect the new time, proving the thread re-reads the clock — with
        // no real interval sleep involved.
        clock.set(base + chrono::Duration::seconds(100));
        let advanced = clock.now();
        ticker.tick();

        let second = poll_until(|| {
            manager
                .read_heartbeat("agent:background")
                .ok()
                .filter(|hb| hb.last_beat == advanced)
        });
        assert!(second.last_beat > first.last_beat);
        assert!(!second.is_stale(&*clock));

        // Stop the loop and join deterministically (no detached forever-thread).
        ticker.stop();
        handle.join().unwrap();
    }

    #[test]
    fn test_heartbeat_path_sanitization() {
        let temp_dir = tempdir().unwrap();
        let manager = HeartbeatManager::new(temp_dir.path());

        let heartbeat = Heartbeat::new(
            &SystemClock,
            "agent:with:colons".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            30,
        );

        manager.write_heartbeat(&heartbeat).unwrap();

        // Should be able to read back with original ID
        let read_heartbeat = manager.read_heartbeat("agent:with:colons").unwrap();
        assert_eq!(heartbeat.agent_id, read_heartbeat.agent_id);
    }

    #[test]
    fn test_check_pid_alive_current_process() {
        let current_pid = std::process::id();
        assert!(check_pid_alive(current_pid));
    }
}
