//! Stale lock cleanup for recovery
//!
//! Detects and removes locks from dead processes using PID-based detection.
//! Only activates when worktree/claim features are in use - zero overhead for single developers.

use crate::storage::StorageWarning;
use anyhow::Result;
use std::path::Path;

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
/// # Returns
///
/// The non-fatal [`StorageWarning`]s observed during cleanup (locks removed,
/// long-lived locks left in place). The caller's output layer decides whether
/// and how to render them; this function never writes to stderr itself.
///
/// # Errors
///
/// Returns error if lock directory cannot be read
pub fn cleanup_stale_locks(lock_dir: &Path) -> Result<Vec<StorageWarning>> {
    use crate::storage::lock::LockMetadata;
    use chrono::Utc;
    use fs4::fs_std::FileExt;
    use std::fs::{self, OpenOptions};

    let mut warnings = Vec::new();

    if !lock_dir.exists() {
        return Ok(warnings);
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
                    warnings.push(StorageWarning::StaleLockRemoved { path: path.clone() });
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
                            if !crate::process::is_alive(meta.pid) {
                                warnings.push(StorageWarning::LockFromDeadProcess {
                                    pid: meta.pid,
                                    path: path.clone(),
                                });
                                // Force remove (process dead, lock should be stale)
                                let _ = fs::remove_file(&path);
                                let _ = fs::remove_file(&metadata_path);
                                continue;
                            }

                            // Check age (1 hour TTL for locks)
                            let age = Utc::now().signed_duration_since(meta.created_at);
                            if age.num_seconds() > 3600 {
                                warnings.push(StorageWarning::LockVeryOld {
                                    path: path.clone(),
                                    age_secs: age.num_seconds(),
                                    pid: meta.pid,
                                    agent_id: meta.agent_id.clone(),
                                });
                                // Don't auto-remove if process exists, require manual intervention
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_cleanup_empty_directory() {
        let temp_dir = TempDir::new().unwrap();
        let lock_dir = temp_dir.path().join("locks");
        std::fs::create_dir(&lock_dir).unwrap();

        let warnings = cleanup_stale_locks(&lock_dir).unwrap();
        // Should succeed with no warnings
        assert!(warnings.is_empty(), "Empty directory yields no warnings");
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

        let warnings = cleanup_stale_locks(&lock_dir).unwrap();

        // Lock should be removed because it's not actually held
        assert!(!lock_path.exists(), "Lock file should be removed");
        assert!(!meta_path.exists(), "Metadata file should be removed");
        assert_eq!(
            warnings,
            vec![StorageWarning::StaleLockRemoved {
                path: lock_path.clone()
            }],
            "Removing an unheld lock must surface a StaleLockRemoved warning"
        );
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

        let warnings = cleanup_stale_locks(&lock_dir).unwrap();

        // Lock should be removed despite being held (dead process)
        drop(lock_file);
        assert!(!lock_path.exists(), "Lock file should be removed");
        assert!(!meta_path.exists(), "Metadata file should be removed");
        assert_eq!(
            warnings,
            vec![StorageWarning::LockFromDeadProcess {
                pid: u32::MAX - 1,
                path: lock_path.clone()
            }],
            "Removing a dead-process lock must surface a LockFromDeadProcess warning"
        );
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

        let warnings = cleanup_stale_locks(&lock_dir).unwrap();

        // Lock should be preserved (held by live process)
        assert!(lock_path.exists(), "Lock file should still exist");
        assert!(meta_path.exists(), "Metadata file should still exist");
        assert!(
            warnings.is_empty(),
            "A fresh, live-process lock must produce no warnings"
        );

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

        // Should not panic, and must surface the long-lived lock as a warning.
        let warnings = cleanup_stale_locks(&lock_dir).unwrap();

        // Lock should still exist (live process, even if old)
        assert!(lock_path.exists(), "Lock file should still exist");
        assert!(meta_path.exists(), "Metadata file should still exist");
        assert!(
            matches!(
                warnings.as_slice(),
                [StorageWarning::LockVeryOld { pid, .. }] if *pid == std::process::id()
            ),
            "An old, live-process lock must surface a LockVeryOld warning, got {warnings:?}"
        );

        drop(lock_file);
    }
}
