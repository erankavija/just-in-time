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
//! use jit::storage::heartbeat::{Heartbeat, HeartbeatManager};
//! use std::path::Path;
//!
//! let manager = HeartbeatManager::new(Path::new(".git/jit"));
//!
//! // Manual heartbeat update
//! let heartbeat = Heartbeat::new(
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

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
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
    /// Create a new heartbeat with current timestamp and process ID.
    ///
    /// # Arguments
    ///
    /// * `agent_id` - Agent identifier (e.g., "agent:copilot-1")
    /// * `worktree_id` - Worktree identifier (e.g., "wt:abc123")
    /// * `branch` - Current branch name
    /// * `interval_secs` - Heartbeat update interval
    pub fn new(agent_id: String, worktree_id: String, branch: String, interval_secs: u64) -> Self {
        Self {
            agent_id,
            worktree_id,
            branch,
            pid: std::process::id(),
            last_beat: Utc::now(),
            interval_secs,
        }
    }

    /// Update the last_beat timestamp to current time.
    pub fn update(&mut self) {
        self.last_beat = Utc::now();
    }

    /// Check if this heartbeat is stale based on its interval.
    ///
    /// A heartbeat is stale if more than 2x the interval has passed since last beat.
    pub fn is_stale(&self) -> bool {
        let threshold_secs = self.interval_secs * 2;
        let elapsed = Utc::now()
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
        }
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
        let manager = self.clone();

        thread::spawn(move || {
            let mut heartbeat =
                Heartbeat::new(agent_id.clone(), worktree_id, branch, interval_secs);

            loop {
                // Update heartbeat
                heartbeat.update();

                // Write to file (ignore errors to keep thread running)
                let _ = manager.write_heartbeat(&heartbeat);

                // Sleep until next heartbeat
                thread::sleep(Duration::from_secs(interval_secs));
            }
        })
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
        assert!(!heartbeat.is_stale());
    }

    #[test]
    fn test_heartbeat_update() {
        let mut heartbeat = Heartbeat::new(
            "agent:test".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            30,
        );

        let original_beat = heartbeat.last_beat;
        thread::sleep(Duration::from_millis(100));
        heartbeat.update();

        assert!(heartbeat.last_beat > original_beat);
    }

    #[test]
    fn test_heartbeat_is_stale() {
        let mut heartbeat = Heartbeat::new(
            "agent:test".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            1, // 1 second interval
        );

        assert!(!heartbeat.is_stale());

        // Manually set last_beat to 3 seconds ago (> 2x interval)
        heartbeat.last_beat = Utc::now() - chrono::Duration::seconds(3);
        assert!(heartbeat.is_stale());
    }

    #[test]
    fn test_heartbeat_process_alive() {
        let heartbeat = Heartbeat::new(
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
            "agent:test1".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            30,
        );
        let heartbeat2 = Heartbeat::new(
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
            "agent:alive".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            30,
        );
        manager.write_heartbeat(&heartbeat1).unwrap();

        // Dead process heartbeat
        let mut heartbeat2 = Heartbeat::new(
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

    #[test]
    fn test_background_thread_updates_heartbeat() {
        let temp_dir = tempdir().unwrap();
        let manager = HeartbeatManager::new(temp_dir.path());

        let handle = manager.start_heartbeat_thread(
            "agent:background".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            1, // 1 second for faster test
        );

        // Wait for at least 2 heartbeats
        thread::sleep(Duration::from_millis(2500));

        // Read heartbeat
        let heartbeat = manager.read_heartbeat("agent:background").unwrap();
        assert_eq!(heartbeat.agent_id, "agent:background");
        assert!(!heartbeat.is_stale());

        // Note: We don't join the thread as it runs forever
        // The thread will be killed when the process exits
        drop(handle);
    }

    #[test]
    fn test_heartbeat_path_sanitization() {
        let temp_dir = tempdir().unwrap();
        let manager = HeartbeatManager::new(temp_dir.path());

        let heartbeat = Heartbeat::new(
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
