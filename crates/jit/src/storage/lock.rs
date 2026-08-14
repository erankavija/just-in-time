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

/// The access a caller was waiting for when its lock wait expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    /// Sole access, held by one caller at a time.
    Exclusive,
    /// Read access, held by any number of callers at once.
    Shared,
}

impl std::fmt::Display for LockMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Exclusive => "exclusive",
            Self::Shared => "shared",
        })
    }
}

/// A lock wait that expired before the lock became available.
///
/// This says the caller was still queued when its wait ran out. It is not the
/// lock holder's answer, and it does not mean the resource was refused: a caller
/// that waits again may well be admitted. Code that must tell those apart —
/// "contention I have not cleared yet" from "the operation was rejected" —
/// matches on this type through [`is_lock_timeout`] rather than reading the
/// message, so the distinction survives any rewording.
#[derive(Debug, thiserror::Error)]
#[error("Lock timeout: could not acquire {mode} lock on {} after {waited:?}", path.display())]
pub struct LockTimeout {
    /// The access that was being waited for.
    pub mode: LockMode,
    /// The lock file the caller was queued on.
    pub path: PathBuf,
    /// The wait that expired.
    pub waited: Duration,
}

/// True when `error` is a lock wait that expired rather than an outcome the
/// locked operation decided.
pub fn is_lock_timeout(error: &anyhow::Error) -> bool {
    error.downcast_ref::<LockTimeout>().is_some()
}

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
                        return Err(LockTimeout {
                            mode: LockMode::Exclusive,
                            path: path.to_path_buf(),
                            waited: self.timeout,
                        }
                        .into());
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
                        return Err(LockTimeout {
                            mode: LockMode::Shared,
                            path: path.to_path_buf(),
                            waited: self.timeout,
                        }
                        .into());
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
                        return Err(LockTimeout {
                            mode: LockMode::Exclusive,
                            path: path.to_path_buf(),
                            waited: self.timeout,
                        }
                        .into());
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
    use crate::storage::contention_probe::{admitted_when_reached, Contenders};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
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

    /// While one caller holds the exclusive lock, every other caller that asks
    /// for it is refused.
    ///
    /// The test thread takes the lock itself, so the hold is read off the
    /// lock's own answer rather than assumed of a thread the host may not have
    /// scheduled. It releases only once every contender has recorded its
    /// refusal, so the refusals cannot invert into grants by the hold ending
    /// before a contender asked.
    /// Shorter than any hold these tests take, so a contender that meets the
    /// lock held is refused it and says so.
    const EXPIRING_LOCK_WAIT: Duration = Duration::from_millis(1);

    #[test]
    fn test_lock_exclusive_refuses_every_contender_that_asks_while_the_lock_is_held() {
        const CONTENDER_COUNT: usize = 4;

        let temp_dir = TempDir::new().unwrap();
        let file_path = Arc::new(temp_dir.path().join("test.lock"));
        let contenders = Contenders::new();

        let held = FileLocker::new(EXPIRING_LOCK_WAIT)
            .try_lock_exclusive(&file_path)
            .unwrap()
            .expect("no contender has started yet, so the lock is free");

        let asking = (0..CONTENDER_COUNT)
            .map(|_| {
                let file_path = Arc::clone(&file_path);
                let contenders = Arc::clone(&contenders);
                thread::spawn(move || {
                    // The hold outlives this wait by construction, so the
                    // wait's length decides how soon this contender reports,
                    // not what it reports.
                    let outcome = FileLocker::new(EXPIRING_LOCK_WAIT).lock_exclusive(&file_path);
                    if outcome.as_ref().err().is_some_and(is_lock_timeout) {
                        contenders.record_refusal();
                    } else {
                        // Anything that is not an expired wait is the lock
                        // deciding: this contender reached the critical
                        // section. Recorded so a lock that lets it through
                        // while the hold is live is reported by the wait below
                        // rather than waited out.
                        contenders.record_admission();
                    }
                    outcome.map(drop)
                })
            })
            .collect::<Vec<_>>();

        contenders.await_refusals(CONTENDER_COUNT);
        drop(held);

        let outcomes = asking
            .into_iter()
            .map(|contender| contender.join().unwrap())
            .collect::<Vec<_>>();

        assert!(
            outcomes
                .iter()
                .all(|outcome| outcome.as_ref().err().is_some_and(is_lock_timeout)),
            "every contender that asked while the lock was held was refused it, \
             and reports an expired wait rather than an outcome of its own: \
             {outcomes:?}"
        );
        assert!(
            FileLocker::new(EXPIRING_LOCK_WAIT)
                .try_lock_exclusive(&file_path)
                .unwrap()
                .is_some(),
            "the same lock is granted once the hold is released, so the hold is \
             what refused the contenders"
        );
    }

    /// Contenders that keep asking until the lock lets them through never hold
    /// it together: the number inside the critical section at once peaks at
    /// one.
    ///
    /// Every fact the verdict rests on is one the callers observed of
    /// themselves. An expired wait is put back to the lock rather than read as
    /// its answer, so a contender the host was slow to schedule changes how
    /// many times it asks and nothing about the outcome.
    #[test]
    fn test_lock_exclusive_grants_the_lock_to_one_contender_at_a_time() {
        const CONTENDER_COUNT: usize = 8;
        let temp_dir = TempDir::new().unwrap();
        let file_path = Arc::new(temp_dir.path().join("test.lock"));
        let contenders = Contenders::new();
        // Contenders inside the critical section, and the most ever seen there
        // together. A peak above one is two callers writing at once.
        let holding = Arc::new(AtomicUsize::new(0));
        let peak_holding = Arc::new(AtomicUsize::new(0));

        // Held until every contender has been refused it, so all of them are
        // queued before any is admitted. Without that, a host free to run the
        // contenders one after another would hold the peak at one whether the
        // lock excludes or not, and the assertion below would pass a lock that
        // does not.
        let held = FileLocker::new(EXPIRING_LOCK_WAIT)
            .try_lock_exclusive(&file_path)
            .unwrap()
            .expect("no contender has started yet, so the lock is free");

        let asking = (0..CONTENDER_COUNT)
            .map(|_| {
                let file_path = Arc::clone(&file_path);
                let contenders = Arc::clone(&contenders);
                let holding = Arc::clone(&holding);
                let peak_holding = Arc::clone(&peak_holding);
                thread::spawn(move || {
                    let locker = FileLocker::new(EXPIRING_LOCK_WAIT);
                    let guard = admitted_when_reached(
                        &contenders,
                        "the exclusive file lock",
                        || locker.lock_exclusive(&file_path),
                        is_lock_timeout,
                    )
                    .expect("an expired wait is asked again, so the lock answers every contender");
                    let now_holding = holding.fetch_add(1, Ordering::SeqCst) + 1;
                    peak_holding.fetch_max(now_holding, Ordering::SeqCst);
                    // The exclusion itself, asked of the lock while this
                    // contender holds it. A peak of one cannot establish it —
                    // a host free to run admitted contenders one after another
                    // observes a peak of one whether the lock excludes or not —
                    // so the holder asks directly instead, and gets an answer
                    // that does not depend on anyone else being scheduled.
                    let while_held = FileLocker::new(EXPIRING_LOCK_WAIT)
                        .try_lock_exclusive(&file_path)
                        .expect("asking whether the lock is free is not an error");
                    assert!(
                        while_held.is_none(),
                        "the lock this contender holds is granted to a second \
                         caller, so it excludes nobody"
                    );
                    holding.fetch_sub(1, Ordering::SeqCst);
                    drop(guard);
                })
            })
            .collect::<Vec<_>>();

        contenders.await_refusals(CONTENDER_COUNT);
        drop(held);

        asking
            .into_iter()
            .for_each(|contender| contender.join().unwrap());

        assert_eq!(
            peak_holding.load(Ordering::SeqCst),
            1,
            "the exclusive lock is held by one contender at a time"
        );
        assert_eq!(
            holding.load(Ordering::SeqCst),
            0,
            "every contender left the critical section it entered"
        );
        assert_eq!(
            contenders.admitted_count(),
            CONTENDER_COUNT,
            "the lock admitted every contender"
        );
    }

    /// Releasing the exclusive hold before the final acquisition proves the
    /// earlier refusals came from that hold rather than from the lock file.
    #[test]
    fn test_lock_shared_is_refused_while_the_exclusive_lock_is_held() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.lock");
        let locker = FileLocker::new(EXPIRING_LOCK_WAIT);

        let held = locker
            .try_lock_exclusive(&file_path)
            .unwrap()
            .expect("the exclusive lock must report that it was acquired");

        let shared_try = locker.try_lock_shared(&file_path).unwrap();
        assert!(
            shared_try.is_none(),
            "a shared lock request is refused while the exclusive lock is held"
        );

        let shared_wait = locker.lock_shared(&file_path);
        assert!(
            shared_wait.as_ref().err().is_some_and(is_lock_timeout),
            "a shared lock wait expires while the exclusive lock is held"
        );

        drop(held);

        assert!(
            locker.try_lock_shared(&file_path).unwrap().is_some(),
            "a shared lock succeeds after the exclusive lock is released"
        );
    }

    /// Shared holds coexist: while one is live the lock grants a second, and
    /// the exclusive lock is refused until the last of them is released.
    ///
    /// Every fact is asked of the lock by a caller that already holds it, so a
    /// host that runs the requests one after another proves the property as
    /// well as one that interleaves them. The exclusive requests are what
    /// separate a shared lock from no lock at all: an implementation that
    /// granted every request would satisfy the two shared grants alone.
    #[test]
    fn test_lock_shared_is_granted_to_a_second_caller_while_a_shared_hold_is_live() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.lock");
        let locker = FileLocker::new(EXPIRING_LOCK_WAIT);

        let first = locker
            .try_lock_shared(&file_path)
            .unwrap()
            .expect("nobody holds the lock yet, so the shared lock is free");
        let second = locker
            .try_lock_shared(&file_path)
            .unwrap()
            .expect("a live shared hold does not exclude a second shared caller");

        assert!(
            locker.try_lock_exclusive(&file_path).unwrap().is_none(),
            "the exclusive lock is granted while two shared holds are live, so \
             those holds exclude nobody"
        );

        drop(first);
        assert!(
            locker.try_lock_exclusive(&file_path).unwrap().is_none(),
            "the exclusive lock is granted while one shared hold remains, so the \
             refusal above came from the released hold alone"
        );

        drop(second);
        assert!(
            locker.try_lock_shared(&file_path).unwrap().is_some(),
            "the shared lock is refused after every hold on it was released"
        );
        assert!(
            locker.try_lock_exclusive(&file_path).unwrap().is_some(),
            "the exclusive lock is still refused after the last shared hold was \
             released, so something other than those holds refuses it"
        );
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
