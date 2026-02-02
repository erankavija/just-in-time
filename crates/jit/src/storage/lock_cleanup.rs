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
/// Reads lock metadata files (.lock.meta) and checks if the PID still exists.
/// Removes locks from dead processes.
///
/// # Arguments
///
/// * `lock_dir` - Directory containing lock files
///
/// # Errors
///
/// Returns error if lock directory cannot be read
pub fn cleanup_stale_locks(lock_dir: &Path) -> Result<Vec<String>> {
    use crate::storage::lock::LockMetadata;
    use std::fs;

    let mut removed = Vec::new();

    if !lock_dir.exists() {
        return Ok(removed);
    }

    // Iterate through lock files
    for entry in fs::read_dir(lock_dir)? {
        let entry = entry?;
        let path = entry.path();

        // Only process .lock files (not .lock.meta)
        if path.extension().is_none_or(|ext| ext != "lock") {
            continue;
        }

        // Read metadata
        let meta_path = path.with_extension("lock.meta");
        let metadata = match fs::read_to_string(&meta_path) {
            Ok(content) => match serde_json::from_str::<LockMetadata>(&content) {
                Ok(meta) => meta,
                Err(_) => {
                    // Can't parse metadata, assume stale and remove
                    let _ = fs::remove_file(&path);
                    let _ = fs::remove_file(&meta_path);
                    removed.push(path.to_string_lossy().to_string());
                    continue;
                }
            },
            Err(_) => {
                // No metadata, assume stale
                let _ = fs::remove_file(&path);
                removed.push(path.to_string_lossy().to_string());
                continue;
            }
        };

        // Check if process still exists
        if !process_exists(metadata.pid) {
            // Process is dead, remove lock and metadata
            let _ = fs::remove_file(&path);
            let _ = fs::remove_file(&meta_path);
            removed.push(path.to_string_lossy().to_string());
        }
        // If process exists, preserve the lock
    }

    Ok(removed)
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
        assert!(
            !process_exists(invalid_pid),
            "Invalid PID should not exist"
        );
    }

    #[test]
    fn test_cleanup_empty_directory() {
        let temp_dir = TempDir::new().unwrap();
        let lock_dir = temp_dir.path().join("locks");
        std::fs::create_dir(&lock_dir).unwrap();

        let removed = cleanup_stale_locks(&lock_dir).unwrap();
        assert_eq!(removed.len(), 0, "No locks to clean up");
    }

    #[test]
    fn test_cleanup_removes_locks_from_dead_process() {
        let temp_dir = TempDir::new().unwrap();
        let lock_dir = temp_dir.path().join("locks");
        std::fs::create_dir(&lock_dir).unwrap();

        // Create a lock file with metadata for a dead process
        let lock_path = lock_dir.join("test.lock");
        std::fs::write(&lock_path, "").unwrap();

        let meta_path = lock_dir.join("test.lock.meta");
        let metadata = crate::storage::lock::LockMetadata {
            pid: u32::MAX - 1, // Invalid PID
            agent_id: "agent:dead".to_string(),
            created_at: chrono::Utc::now(),
            last_updated: chrono::Utc::now(),
        };
        std::fs::write(
            &meta_path,
            serde_json::to_string_pretty(&metadata).unwrap(),
        )
        .unwrap();

        let removed = cleanup_stale_locks(&lock_dir).unwrap();

        assert_eq!(removed.len(), 1, "Should remove one stale lock");
        assert!(!lock_path.exists(), "Lock file should be removed");
        assert!(!meta_path.exists(), "Metadata file should be removed");
    }

    #[test]
    fn test_cleanup_preserves_locks_from_live_processes() {
        let temp_dir = TempDir::new().unwrap();
        let lock_dir = temp_dir.path().join("locks");
        std::fs::create_dir(&lock_dir).unwrap();

        // Create a lock file with metadata for current process
        let lock_path = lock_dir.join("test.lock");
        std::fs::write(&lock_path, "").unwrap();

        let meta_path = lock_dir.join("test.lock.meta");
        let metadata = crate::storage::lock::LockMetadata {
            pid: std::process::id(), // Current process
            agent_id: "agent:live".to_string(),
            created_at: chrono::Utc::now(),
            last_updated: chrono::Utc::now(),
        };
        std::fs::write(
            &meta_path,
            serde_json::to_string_pretty(&metadata).unwrap(),
        )
        .unwrap();

        let removed = cleanup_stale_locks(&lock_dir).unwrap();

        assert_eq!(removed.len(), 0, "Should not remove locks from live process");
        assert!(lock_path.exists(), "Lock file should still exist");
        assert!(meta_path.exists(), "Metadata file should still exist");
    }
}
