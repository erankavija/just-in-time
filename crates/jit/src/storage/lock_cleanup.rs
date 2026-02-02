//! Stale lock cleanup for recovery
//!
//! Detects and removes locks from dead processes using PID-based detection.
//! Only activates when worktree/claim features are in use - zero overhead for single developers.

use anyhow::Result;
use std::path::Path;

/// Check if a process with the given PID exists
///
/// Uses platform-specific methods:
/// - Unix: signal 0 (doesn't send signal, just checks if process exists)
/// - Windows: OpenProcess with PROCESS_QUERY_INFORMATION
#[cfg(unix)]
pub fn process_exists(pid: u32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    // Signal 0 (None) doesn't send a signal but checks if process exists
    kill(Pid::from_raw(pid as i32), None).is_ok()
}

#[cfg(windows)]
pub fn process_exists(_pid: u32) -> bool {
    // TODO: Implement Windows process checking
    // For now, assume process exists (safe default)
    true
}

#[cfg(not(any(unix, windows)))]
pub fn process_exists(_pid: u32) -> bool {
    // Unknown platform - assume process exists (safe default)
    true
}

/// Clean up stale lock files from dead processes
///
/// Uses two-stage detection:
/// 1. Try to acquire lock non-blocking - if successful, lock was stale
/// 2. If lock is held, check metadata:
///    - If process is dead, force remove lock
///    - If process is alive but lock is > 1 hour old, log error
///
/// # Arguments
///
/// * `lock_dir` - Directory containing lock files
///
/// # Errors
///
/// Returns error if lock directory cannot be read
pub fn cleanup_stale_locks(lock_dir: &Path) -> Result<()> {
    use crate::storage::lock::LockMetadata;
    use chrono::Utc;
    use fs4::fs_std::FileExt;
    use std::fs::{self, OpenOptions};

    if !lock_dir.exists() {
        return Ok(());
    }

    // Iterate through lock files
    for entry in fs::read_dir(lock_dir)? {
        let entry = entry?;
        let path = entry.path();

        // Skip metadata files
        if path.extension().is_none_or(|e| e == "meta") {
            continue;
        }

        // Only process .lock files
        if path.extension().is_none_or(|ext| ext != "lock") {
            continue;
        }

        let metadata_path = path.with_extension("lock.meta");

        // Try to acquire lock non-blocking
        if let Ok(file) = OpenOptions::new().write(true).open(&path) {
            match file.try_lock_exclusive() {
                Ok(true) => {
                    // Lock acquired => it was stale
                    eprintln!("Warning: Removed stale lock: {}", path.display());
                    // Lock automatically released when file dropped
                    drop(file);
                    let _ = fs::remove_file(&path);
                    let _ = fs::remove_file(&metadata_path);
                }
                Ok(false) | Err(_) => {
                    // Lock held, check metadata
                    if let Ok(meta_json) = fs::read_to_string(&metadata_path) {
                        if let Ok(meta) = serde_json::from_str::<LockMetadata>(&meta_json) {
                            // Check if process still exists
                            if !process_exists(meta.pid) {
                                eprintln!(
                                    "Warning: Removing lock from dead process {}: {}",
                                    meta.pid,
                                    path.display()
                                );
                                // Force remove (process dead, lock should be stale)
                                let _ = fs::remove_file(&path);
                                let _ = fs::remove_file(&metadata_path);
                                continue;
                            }

                            // Check age (1 hour TTL for locks)
                            let age = Utc::now().signed_duration_since(meta.created_at);
                            if age.num_seconds() > 3600 {
                                eprintln!(
                                    "Error: Lock very old: {} ({}s, pid={}, agent={})",
                                    path.display(),
                                    age.num_seconds(),
                                    meta.pid,
                                    meta.agent_id
                                );
                                // Don't auto-remove if process exists, require manual intervention
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_process_exists_for_current_process() {
        let pid = std::process::id();
        assert!(process_exists(pid), "Current process should exist");
    }

    #[test]
    fn test_process_exists_for_invalid_pid() {
        // Use a very high PID that is unlikely to exist
        let invalid_pid = u32::MAX - 1;
        assert!(!process_exists(invalid_pid), "Invalid PID should not exist");
    }

    #[test]
    fn test_cleanup_empty_directory() {
        let temp_dir = TempDir::new().unwrap();
        let lock_dir = temp_dir.path().join("locks");
        std::fs::create_dir(&lock_dir).unwrap();

        cleanup_stale_locks(&lock_dir).unwrap();
        // Should succeed with no errors
    }

    #[test]
    fn test_cleanup_removes_unheld_locks() {
        let temp_dir = TempDir::new().unwrap();
        let lock_dir = temp_dir.path().join("locks");
        std::fs::create_dir(&lock_dir).unwrap();

        // Create a lock file that isn't held
        let lock_path = lock_dir.join("test.lock");
        std::fs::write(&lock_path, "").unwrap();

        let meta_path = lock_dir.join("test.lock.meta");
        let metadata = crate::storage::lock::LockMetadata {
            pid: std::process::id(),
            agent_id: "agent:test".to_string(),
            created_at: chrono::Utc::now(),
            last_updated: chrono::Utc::now(),
        };
        std::fs::write(&meta_path, serde_json::to_string_pretty(&metadata).unwrap()).unwrap();

        cleanup_stale_locks(&lock_dir).unwrap();

        // Lock should be removed because it's not actually held
        assert!(!lock_path.exists(), "Lock file should be removed");
        assert!(!meta_path.exists(), "Metadata file should be removed");
    }

    #[test]
    fn test_cleanup_removes_locks_from_dead_process() {
        use fs4::fs_std::FileExt;
        use std::fs::OpenOptions;

        let temp_dir = TempDir::new().unwrap();
        let lock_dir = temp_dir.path().join("locks");
        std::fs::create_dir(&lock_dir).unwrap();

        // Create a lock file with metadata for a dead process, and hold it
        let lock_path = lock_dir.join("test.lock");
        let lock_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&lock_path)
            .unwrap();
        lock_file.lock_exclusive().unwrap();

        let meta_path = lock_dir.join("test.lock.meta");
        let metadata = crate::storage::lock::LockMetadata {
            pid: u32::MAX - 1, // Invalid PID
            agent_id: "agent:dead".to_string(),
            created_at: chrono::Utc::now(),
            last_updated: chrono::Utc::now(),
        };
        std::fs::write(&meta_path, serde_json::to_string_pretty(&metadata).unwrap()).unwrap();

        cleanup_stale_locks(&lock_dir).unwrap();

        // Lock should be removed despite being held (dead process)
        drop(lock_file);
        assert!(!lock_path.exists(), "Lock file should be removed");
        assert!(!meta_path.exists(), "Metadata file should be removed");
    }

    #[test]
    fn test_cleanup_preserves_locks_from_live_processes() {
        use fs4::fs_std::FileExt;
        use std::fs::OpenOptions;

        let temp_dir = TempDir::new().unwrap();
        let lock_dir = temp_dir.path().join("locks");
        std::fs::create_dir(&lock_dir).unwrap();

        // Create a lock file with metadata for current process and hold it
        let lock_path = lock_dir.join("test.lock");
        let lock_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&lock_path)
            .unwrap();
        lock_file.lock_exclusive().unwrap();

        let meta_path = lock_dir.join("test.lock.meta");
        let metadata = crate::storage::lock::LockMetadata {
            pid: std::process::id(), // Current process
            agent_id: "agent:live".to_string(),
            created_at: chrono::Utc::now(),
            last_updated: chrono::Utc::now(),
        };
        std::fs::write(&meta_path, serde_json::to_string_pretty(&metadata).unwrap()).unwrap();

        cleanup_stale_locks(&lock_dir).unwrap();

        // Lock should be preserved (held by live process)
        assert!(lock_path.exists(), "Lock file should still exist");
        assert!(meta_path.exists(), "Metadata file should still exist");

        drop(lock_file);
    }

    #[test]
    fn test_cleanup_detects_old_locks() {
        use chrono::Duration;
        use fs4::fs_std::FileExt;
        use std::fs::OpenOptions;

        let temp_dir = TempDir::new().unwrap();
        let lock_dir = temp_dir.path().join("locks");
        std::fs::create_dir(&lock_dir).unwrap();

        // Create a lock file with old timestamp
        let lock_path = lock_dir.join("test.lock");
        let lock_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&lock_path)
            .unwrap();
        lock_file.lock_exclusive().unwrap();

        let meta_path = lock_dir.join("test.lock.meta");
        let old_time = chrono::Utc::now() - Duration::hours(2); // 2 hours old
        let metadata = crate::storage::lock::LockMetadata {
            pid: std::process::id(),
            agent_id: "agent:live".to_string(),
            created_at: old_time,
            last_updated: old_time,
        };
        std::fs::write(&meta_path, serde_json::to_string_pretty(&metadata).unwrap()).unwrap();

        // Should not panic, just log error (can't verify logging in test)
        cleanup_stale_locks(&lock_dir).unwrap();

        // Lock should still exist (live process, even if old)
        assert!(lock_path.exists(), "Lock file should still exist");
        assert!(meta_path.exists(), "Metadata file should still exist");

        drop(lock_file);
    }
}
