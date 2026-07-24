//! File locking for multi-agent safety
//!
//! This module provides cross-platform file locking using advisory locks
//! to prevent race conditions when multiple processes access `.jit/` concurrently.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use fs4::fs_std::FileExt as Fs4FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Metadata for lock diagnostics
///
/// Written alongside lock files to enable diagnosis of stuck processes.
/// Only used when claim coordination is active.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockMetadata {
    /// Process ID of lock holder
    pub pid: u32,
    /// Agent identifier (e.g., "agent:copilot-1")
    pub agent_id: String,
    /// When the lock was acquired
    pub created_at: DateTime<Utc>,
    /// Last time the lock was updated
    pub last_updated: DateTime<Utc>,
}

/// Lock guard that automatically releases the lock when dropped (RAII pattern)
///
/// Optionally tracks lock metadata for diagnostics when claim coordination is active.
#[derive(Debug)]
pub struct LockGuard {
    file: File,
    #[allow(dead_code)]
    path: PathBuf,
    /// Path to metadata file (if tracking enabled)
    meta_path: Option<PathBuf>,
}

impl LockGuard {
    fn new(file: File, path: PathBuf) -> Self {
        Self {
            file,
            path,
            meta_path: None,
        }
    }

    fn new_with_metadata(file: File, path: PathBuf, meta_path: PathBuf) -> Self {
        Self {
            file,
            path,
            meta_path: Some(meta_path),
        }
    }

    /// Get the path of the locked file
    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // fs4 automatically unlocks on file close (drop)
        let _ = Fs4FileExt::unlock(&self.file);

        // Clean up metadata file if it exists
        if let Some(ref meta_path) = self.meta_path {
            let _ = std::fs::remove_file(meta_path);
        }
    }
}

/// File locking abstraction for cross-platform safety
///
/// Uses advisory file locks (flock on Unix, LockFileEx on Windows) to coordinate
/// access between multiple processes. Locks are automatically released when the
/// `LockGuard` is dropped, ensuring cleanup even on panic.
#[derive(Debug, Clone)]
pub struct FileLocker {
    timeout: Duration,
}

impl FileLocker {
    /// Create a new FileLocker with the specified timeout
    ///
    /// # Arguments
    ///
    /// * `timeout` - Maximum time to wait for lock acquisition
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Acquire an exclusive (write) lock on the file
    ///
    /// This will block until the lock is acquired or the timeout expires.
    /// Only one process can hold an exclusive lock at a time, and exclusive
    /// locks block both readers and writers.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file cannot be opened
    /// - The lock cannot be acquired within the timeout
    pub fn lock_exclusive(&self, path: &Path) -> Result<LockGuard> {
        let file = self.open_or_create(path)?;

        // Try to acquire lock with polling and timeout
        let start = std::time::Instant::now();
        let poll_interval = Duration::from_millis(crate::runtime_defaults::LOCK_POLL_INTERVAL_MS);

        loop {
            match Fs4FileExt::try_lock_exclusive(&file) {
                Ok(true) => {
                    return Ok(LockGuard::new(file, path.to_path_buf()));
                }
                Ok(false) => {
                    if start.elapsed() >= self.timeout {
                        anyhow::bail!(
                            "Lock timeout: could not acquire exclusive lock on {} after {:?}",
                            path.display(),
                            self.timeout
                        );
                    }
                    std::thread::sleep(poll_interval);
                }
                Err(e) => {
                    anyhow::bail!("IO error while trying to lock {}: {}", path.display(), e);
                }
            }
        }
    }

    /// Acquire a shared (read) lock on the file
    ///
    /// Multiple processes can hold shared locks simultaneously, but shared
    /// locks block writers (exclusive locks).
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file cannot be opened
    /// - The lock cannot be acquired within the timeout
    pub fn lock_shared(&self, path: &Path) -> Result<LockGuard> {
        let file = self.open_or_create(path)?;

        let start = std::time::Instant::now();
        let poll_interval = Duration::from_millis(crate::runtime_defaults::LOCK_POLL_INTERVAL_MS);

        loop {
            match Fs4FileExt::try_lock_shared(&file) {
                Ok(true) => {
                    return Ok(LockGuard::new(file, path.to_path_buf()));
                }
                Ok(false) => {
                    if start.elapsed() >= self.timeout {
                        anyhow::bail!(
                            "Lock timeout: could not acquire shared lock on {} after {:?}",
                            path.display(),
                            self.timeout
                        );
                    }
                    std::thread::sleep(poll_interval);
                }
                Err(e) => {
                    anyhow::bail!("IO error while trying to lock {}: {}", path.display(), e);
                }
            }
        }
    }

    /// Try to acquire an exclusive lock without blocking
    ///
    /// Returns `Ok(Some(guard))` if the lock was acquired immediately,
    /// `Ok(None)` if the lock is held by another process.
    ///
    /// # Errors
    ///
    /// Returns an error only if the file cannot be opened.
    #[allow(dead_code)]
    pub fn try_lock_exclusive(&self, path: &Path) -> Result<Option<LockGuard>> {
        let file = self.open_or_create(path)?;

        match Fs4FileExt::try_lock_exclusive(&file) {
            Ok(true) => Ok(Some(LockGuard::new(file, path.to_path_buf()))),
            Ok(false) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Try to acquire a shared lock without blocking
    ///
    /// Returns `Ok(Some(guard))` if the lock was acquired immediately,
    /// `Ok(None)` if an exclusive lock is held by another process.
    ///
    /// # Errors
    ///
    /// Returns an error only if the file cannot be opened.
    #[allow(dead_code)]
    pub fn try_lock_shared(&self, path: &Path) -> Result<Option<LockGuard>> {
        let file = self.open_or_create(path)?;

        match Fs4FileExt::try_lock_shared(&file) {
            Ok(true) => Ok(Some(LockGuard::new(file, path.to_path_buf()))),
            Ok(false) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Acquire an exclusive lock with metadata tracking
    ///
    /// Writes a `.lock.meta` file alongside the lock file containing PID,
    /// agent ID, and timestamps for diagnostics. Metadata is automatically
    /// cleaned up when the lock is released.
    ///
    /// **Note:** Only use this when claim coordination is active. Regular
    /// locking without metadata is faster and sufficient for single-agent use.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the lock file
    /// * `agent_id` - Agent identifier (e.g., "agent:copilot-1")
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file cannot be opened
    /// - The lock cannot be acquired within the timeout
    /// - Metadata file cannot be written
    pub fn lock_exclusive_with_metadata(&self, path: &Path, agent_id: &str) -> Result<LockGuard> {
        let file = self.open_or_create(path)?;

        // Try to acquire lock with polling and timeout
        let start = std::time::Instant::now();
        let poll_interval = Duration::from_millis(crate::runtime_defaults::LOCK_POLL_INTERVAL_MS);

        loop {
            match Fs4FileExt::try_lock_exclusive(&file) {
                Ok(true) => {
                    // Lock acquired - write metadata
                    let meta_path = path.with_extension("lock.meta");
                    let metadata = LockMetadata {
                        pid: std::process::id(),
                        agent_id: agent_id.to_string(),
                        created_at: Utc::now(),
                        last_updated: Utc::now(),
                    };

                    std::fs::write(
                        &meta_path,
                        serde_json::to_string_pretty(&metadata)
                            .context("Failed to serialize lock metadata")?,
                    )
                    .with_context(|| {
                        format!("Failed to write lock metadata: {}", meta_path.display())
                    })?;

                    return Ok(LockGuard::new_with_metadata(
                        file,
                        path.to_path_buf(),
                        meta_path,
                    ));
                }
                Ok(false) => {
                    if start.elapsed() >= self.timeout {
                        anyhow::bail!(
                            "Lock timeout: could not acquire exclusive lock on {} after {:?}",
                            path.display(),
                            self.timeout
                        );
                    }
                    std::thread::sleep(poll_interval);
                }
                Err(e) => {
                    anyhow::bail!("IO error while trying to lock {}: {}", path.display(), e);
                }
            }
        }
    }

    /// Open file for locking, creating it if it doesn't exist
    fn open_or_create(&self, path: &Path) -> Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .with_context(|| format!("Failed to open file for locking: {}", path.display()))
    }
}

/// Remove orphaned per-issue read-lock sidecar files from an issues directory.
///
/// `load_issue` historically took a shared lock on a `<id>.lock` sidecar next to
/// every issue file it read. That lock was redundant — issue JSON is published by
/// atomic temp-file-plus-rename, so a reader can never observe a torn file, and
/// writers serialize on `.repo-write.lock` — yet it created one zero-byte sidecar
/// per issue ever read, growing without bound. The read lock is gone; this removes
/// the inert leftovers it left behind.
///
/// Only empty `<uuid>.lock` regular files directly inside `issues_dir` are
/// removed, so unrelated, non-empty, and symlink files are never touched. The
/// retained fixed locks (`.repo-write`, `.index`, `.gates`, `.events`, the
/// bootstrap lock, and the claims lock) live outside this directory and are
/// unaffected. Removal is best-effort cleanup rather than a correctness
/// mechanism: a file that cannot be removed is skipped instead of raising an
/// error.
///
/// Returns the number of sidecar files removed.
///
/// # Errors
///
/// Returns an error only if `issues_dir` exists but cannot be enumerated.
pub(crate) fn remove_orphaned_issue_read_sidecars(issues_dir: &Path) -> Result<usize> {
    use std::fs;

    if !issues_dir.exists() {
        return Ok(0);
    }

    let removed = fs::read_dir(issues_dir)
        .with_context(|| {
            format!(
                "Failed to read issues directory for sidecar cleanup: {}",
                issues_dir.display()
            )
        })?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "lock"))
        .filter(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| uuid::Uuid::parse_str(stem).is_ok())
        })
        .filter(|path| {
            fs::symlink_metadata(path)
                .is_ok_and(|meta| meta.file_type().is_file() && meta.len() == 0)
        })
        .filter(|path| fs::remove_file(path).is_ok())
        .count();

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread;
    use tempfile::TempDir;

    #[test]
    fn test_exclusive_lock_acquired() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.lock");

        let locker = FileLocker::new(Duration::from_millis(100));
        let guard = locker.lock_exclusive(&file_path).unwrap();

        assert_eq!(guard.path(), file_path);
    }

    #[test]
    fn test_shared_lock_acquired() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.lock");

        let locker = FileLocker::new(Duration::from_millis(100));
        let _guard = locker.lock_shared(&file_path).unwrap();
    }

    #[test]
    fn test_try_lock_non_blocking() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.lock");

        let locker = FileLocker::new(Duration::from_millis(100));

        // First try should succeed
        let guard = locker.try_lock_exclusive(&file_path).unwrap();
        assert!(guard.is_some());

        // Second try should fail (lock held)
        let result = locker.try_lock_exclusive(&file_path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_lock_released_on_drop() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.lock");

        let locker = FileLocker::new(Duration::from_millis(100));

        {
            let _guard = locker.lock_exclusive(&file_path).unwrap();
            // Lock held here
        } // Lock dropped and released

        // Should be able to acquire lock again
        let _guard2 = locker.lock_exclusive(&file_path).unwrap();
    }

    #[test]
    fn test_exclusive_lock_prevents_concurrent_writes() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.lock");

        // Thread 1: Acquire lock and hold it
        let path1 = Arc::new(file_path.clone());
        let acquired = Arc::new(Mutex::new(false));
        let acquired1 = Arc::clone(&acquired);

        let handle1 = thread::spawn(move || {
            let locker = FileLocker::new(Duration::from_millis(500));
            let _guard = locker.lock_exclusive(&path1).unwrap();
            *acquired1.lock().unwrap() = true;
            thread::sleep(Duration::from_millis(200));
            // Lock held until guard drops
        });

        // Wait for first thread to acquire lock
        thread::sleep(Duration::from_millis(50));
        assert!(
            *acquired.lock().unwrap(),
            "First thread should have acquired lock"
        );

        // Thread 2: Try to acquire same lock with short timeout (should fail)
        let path2 = file_path.clone();
        let handle2 = thread::spawn(move || {
            let locker = FileLocker::new(Duration::from_millis(50));
            // Should timeout since lock is held
            locker.lock_exclusive(&path2)
        });

        handle1.join().unwrap();
        let result2 = handle2.join().unwrap();

        // Second thread should have timed out
        assert!(
            result2.is_err(),
            "Second thread should timeout waiting for lock"
        );
        assert!(
            result2.unwrap_err().to_string().contains("Lock timeout"),
            "Error should mention lock timeout"
        );
    }

    #[test]
    fn test_shared_locks_allow_concurrent_reads() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.lock");

        let path = Arc::new(file_path);
        let barrier = Arc::new(Barrier::new(3));
        let success_count = Arc::new(Mutex::new(0));

        let handles: Vec<_> = (0..3)
            .map(|_| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                let success_count = Arc::clone(&success_count);

                thread::spawn(move || {
                    barrier.wait();

                    let locker = FileLocker::new(Duration::from_millis(500));
                    if let Ok(_guard) = locker.lock_shared(&path) {
                        thread::sleep(Duration::from_millis(100));
                        *success_count.lock().unwrap() += 1;
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // All threads should acquire shared lock
        let count = *success_count.lock().unwrap();
        assert_eq!(count, 3, "All threads should acquire shared lock");
    }

    #[test]
    fn test_lock_with_metadata_writes_meta_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.lock");

        let locker = FileLocker::new(Duration::from_millis(100));
        let _guard = locker
            .lock_exclusive_with_metadata(&file_path, "agent:test-1")
            .unwrap();

        // Metadata file should exist
        let meta_path = file_path.with_extension("lock.meta");
        assert!(meta_path.exists(), "Metadata file should exist");

        // Verify metadata content
        let content = std::fs::read_to_string(&meta_path).unwrap();
        let metadata: LockMetadata = serde_json::from_str(&content).unwrap();

        assert_eq!(metadata.agent_id, "agent:test-1");
        assert_eq!(metadata.pid, std::process::id());
    }

    #[test]
    fn test_remove_orphaned_issue_read_sidecars_removes_empty_lock_files_and_counts_them() {
        let temp_dir = TempDir::new().unwrap();
        let issues_dir = temp_dir.path().join("issues");
        std::fs::create_dir(&issues_dir).unwrap();

        // Two zero-byte per-issue sidecars, as the retired read lock created them.
        let sidecar_a = issues_dir.join(format!("{}.lock", uuid::Uuid::new_v4()));
        let sidecar_b = issues_dir.join(format!("{}.lock", uuid::Uuid::new_v4()));
        std::fs::write(&sidecar_a, "").unwrap();
        std::fs::write(&sidecar_b, "").unwrap();

        let removed = remove_orphaned_issue_read_sidecars(&issues_dir).unwrap();

        assert_eq!(
            removed, 2,
            "Both empty sidecars should be removed and counted"
        );
        assert!(!sidecar_a.exists(), "Empty sidecar should be gone");
        assert!(!sidecar_b.exists(), "Empty sidecar should be gone");
    }

    #[test]
    fn test_remove_orphaned_issue_read_sidecars_preserves_issue_json_and_nonempty_files() {
        let temp_dir = TempDir::new().unwrap();
        let issues_dir = temp_dir.path().join("issues");
        std::fs::create_dir(&issues_dir).unwrap();

        // Issue payloads must survive; only empty `.lock` sidecars are cleaned.
        let issue_json = issues_dir.join("issue-a.json");
        std::fs::write(&issue_json, "{}").unwrap();
        // A UUID-shaped lock carrying content is not one of the inert sidecars.
        let nonempty_lock = issues_dir.join(format!("{}.lock", uuid::Uuid::new_v4()));
        std::fs::write(&nonempty_lock, "not-empty").unwrap();
        // An empty lock without an issue UUID is unrelated and must also survive.
        let unrelated_lock = issues_dir.join("held.lock");
        std::fs::write(&unrelated_lock, "").unwrap();
        let empty_sidecar = issues_dir.join(format!("{}.lock", uuid::Uuid::new_v4()));
        std::fs::write(&empty_sidecar, "").unwrap();

        let removed = remove_orphaned_issue_read_sidecars(&issues_dir).unwrap();

        assert_eq!(
            removed, 1,
            "Only the single empty sidecar should be removed"
        );
        assert!(issue_json.exists(), "Issue JSON must be preserved");
        assert!(
            nonempty_lock.exists(),
            "Non-empty .lock file must be preserved"
        );
        assert!(
            unrelated_lock.exists(),
            "Empty non-issue .lock file must be preserved"
        );
        assert!(!empty_sidecar.exists(), "Empty sidecar should be removed");
    }

    #[test]
    fn test_remove_orphaned_issue_read_sidecars_tolerates_absent_directory() {
        let temp_dir = TempDir::new().unwrap();
        let missing = temp_dir.path().join("issues");

        let removed = remove_orphaned_issue_read_sidecars(&missing).unwrap();

        assert_eq!(removed, 0, "An absent issues directory removes nothing");
    }

    #[test]
    fn test_lock_with_metadata_cleans_up_on_drop() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.lock");
        let meta_path = file_path.with_extension("lock.meta");

        let locker = FileLocker::new(Duration::from_millis(100));

        {
            let _guard = locker
                .lock_exclusive_with_metadata(&file_path, "agent:test-1")
                .unwrap();
            assert!(meta_path.exists(), "Metadata should exist while locked");
        } // Lock dropped

        // Metadata should be cleaned up
        assert!(
            !meta_path.exists(),
            "Metadata file should be removed on lock release"
        );
    }
}
