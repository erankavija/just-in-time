//! The repository-wide write lock: the OUTERMOST lock on every mutating storage
//! path.
//!
//! [`RepoWriteLock`] lives next to the data it guards — `.jit/.repo-write.lock`
//! in the storage root — so it is present whether or not the working tree is a
//! git repository (`@/charter/D-4`). Every write path in
//! [`IssueStore`](crate::storage::IssueStore) takes it before any finer lock
//! (`.index.lock`, the per-issue lock, `.gates.lock`, `.events.lock`), and a
//! multi-write sequence such as `jit apply` holds ONE guard across its whole
//! read-validate-write-rollback window. A concurrent writer therefore observes
//! the sequence's start or its end, never a midpoint, and a compensating
//! rollback can only undo writes the sequence itself made.
//!
//! # Reentrancy
//!
//! A sequence holding the guard calls straight into the storage writes, which
//! take the same lock again. `flock(2)` is per-open-file-description, so a second
//! exclusive acquisition from the same process on a fresh descriptor would block
//! forever. The lock therefore tracks its owning thread and nests: the outermost
//! acquisition takes the file lock, inner ones bump a depth counter, and the file
//! lock drops when the outermost guard drops. Threads of the same process
//! serialize on the in-process state before ever reaching the file lock; separate
//! processes (and separate [`RepoWriteLock`] instances over one root) serialize on
//! the file lock itself.
//!
//! # Lock order
//!
//! `repo-write` → (`.index.lock` → per-issue `.lock`) | `.gates.lock` |
//! `.events.lock`. No path acquires the repo-write lock while holding an inner
//! lock, and read paths never take it, so no cycle exists.

use super::lock::{FileLocker, LockGuard};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::ThreadId;
use std::time::Duration;

/// File name of the repository write lock inside the storage root.
///
/// Machine-local coordination state, matched by the `.jit/**/*.lock` gitignore
/// entry alongside `.index.lock` and friends.
pub const REPO_WRITE_LOCK_FILE: &str = ".repo-write.lock";

/// Who currently holds the lock, how deep, and the file lock they hold.
#[derive(Debug, Default)]
struct LockState {
    /// The thread inside the outermost acquisition, if any.
    owner: Option<ThreadId>,
    /// Nesting depth of `owner`'s acquisitions; `0` exactly when `owner` is `None`.
    depth: usize,
    /// The cross-process file lock, held for the whole outermost acquisition.
    /// `None` for a process-local lock (a storage backend with no files).
    file_guard: Option<LockGuard>,
}

/// A reentrant, repository-wide write lock shared by every mutating path of one
/// storage backend.
///
/// Always held behind an [`Arc`]: cloning a storage backend must share the lock,
/// otherwise two clones in one thread would deadlock against each other's file
/// lock. Construct with [`RepoWriteLock::for_storage_root`] (file-backed,
/// cross-process) or [`RepoWriteLock::in_process`] (thread-only, for backends
/// with no on-disk state).
#[derive(Debug)]
pub struct RepoWriteLock {
    /// Lock file and its acquisition timeout; `None` for a process-local lock.
    backing: Option<(PathBuf, FileLocker)>,
    state: Mutex<LockState>,
    /// Signalled when the outermost guard drops and `owner` becomes `None`.
    released: Condvar,
}

impl RepoWriteLock {
    /// A file-backed lock at `<storage_root>/.repo-write.lock`, serializing
    /// writers across processes as well as threads.
    ///
    /// `timeout` bounds how long an acquisition waits for the file lock. The lock
    /// file (and the storage root) are created on first acquisition, not here.
    pub fn for_storage_root<P: AsRef<Path>>(storage_root: P, timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            backing: Some((
                storage_root.as_ref().join(REPO_WRITE_LOCK_FILE),
                FileLocker::new(timeout),
            )),
            state: Mutex::new(LockState::default()),
            released: Condvar::new(),
        })
    }

    /// A process-local lock, serializing only the threads sharing this instance.
    ///
    /// For backends whose state lives in memory: there is no file to guard and no
    /// other process to guard it against, but apply-style sequences must still
    /// exclude concurrent writers within the process.
    pub fn in_process() -> Arc<Self> {
        Arc::new(Self {
            backing: None,
            state: Mutex::new(LockState::default()),
            released: Condvar::new(),
        })
    }

    /// Path of the lock file, or `None` for a process-local lock.
    pub fn path(&self) -> Option<&Path> {
        self.backing.as_ref().map(|(path, _)| path.as_path())
    }

    /// Acquire the lock, blocking until it is free, and hold it until the returned
    /// guard drops.
    ///
    /// Reentrant: a thread already inside an acquisition takes the lock again
    /// without touching the file lock, so a storage write nested inside a
    /// multi-write sequence cannot self-deadlock.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage root cannot be created or the file lock
    /// cannot be acquired within the configured timeout.
    pub fn acquire(self: &Arc<Self>) -> Result<RepoWriteGuard> {
        let me = std::thread::current().id();
        let mut state = self.lock_state();

        if state.owner == Some(me) {
            state.depth += 1;
            drop(state);
            return Ok(RepoWriteGuard {
                lock: Arc::clone(self),
            });
        }

        // Another thread of this process owns the lock: wait for it to release
        // rather than contending on the file lock, so ownership stays single.
        while state.owner.is_some() {
            state = self
                .released
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }

        // Nobody owns the lock, so nobody will call `release` while we hold the
        // state mutex across the (possibly blocking) file-lock acquisition.
        if let Some((path, locker)) = &self.backing {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("Failed to create storage root {}", parent.display())
                })?;
            }
            state.file_guard = Some(locker.lock_exclusive(path)?);
        }
        state.owner = Some(me);
        state.depth = 1;
        drop(state);

        Ok(RepoWriteGuard {
            lock: Arc::clone(self),
        })
    }

    /// Drop one nesting level, releasing the file lock at depth zero.
    fn release(&self) {
        let mut state = self.lock_state();
        state.depth = state.depth.saturating_sub(1);
        if state.depth == 0 {
            state.owner = None;
            state.file_guard = None; // drops the flock
            drop(state);
            self.released.notify_all();
        }
    }

    /// The state mutex, recovering from a panic in a lock holder: the guard is
    /// released on unwind either way, and refusing to lock afterwards would wedge
    /// every writer in the process.
    fn lock_state(&self) -> MutexGuard<'_, LockState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// RAII guard for one acquisition of a [`RepoWriteLock`].
///
/// The lock is released when the outermost guard drops, including on unwind.
#[derive(Debug)]
#[must_use = "the repository write lock is released as soon as the guard drops"]
pub struct RepoWriteGuard {
    lock: Arc<RepoWriteLock>,
}

impl Drop for RepoWriteGuard {
    fn drop(&mut self) {
        self.lock.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Barrier;
    use tempfile::TempDir;

    #[test]
    fn test_nested_acquisition_on_one_thread_does_not_deadlock() {
        let temp = TempDir::new().unwrap();
        let lock = RepoWriteLock::for_storage_root(temp.path(), Duration::from_millis(200));

        let outer = lock.acquire().unwrap();
        let inner = lock.acquire().unwrap();
        let innermost = lock.acquire().unwrap();
        drop(innermost);
        drop(inner);
        drop(outer);

        // Fully released: a fresh instance over the same root can take it.
        let other = RepoWriteLock::for_storage_root(temp.path(), Duration::from_millis(200));
        let _guard = other.acquire().unwrap();
    }

    #[test]
    fn test_file_lock_is_held_until_the_outermost_guard_drops() {
        let temp = TempDir::new().unwrap();
        let lock = RepoWriteLock::for_storage_root(temp.path(), Duration::from_millis(100));
        let other = RepoWriteLock::for_storage_root(temp.path(), Duration::from_millis(100));

        let outer = lock.acquire().unwrap();
        let inner = lock.acquire().unwrap();
        drop(inner);

        // Inner release must NOT free the file lock for a separate instance.
        assert!(
            other.acquire().is_err(),
            "the file lock must survive an inner guard drop"
        );

        drop(outer);
        assert!(other.acquire().is_ok(), "outermost drop releases the lock");
    }

    #[test]
    fn test_second_thread_waits_for_the_holder() {
        let temp = TempDir::new().unwrap();
        let lock = RepoWriteLock::for_storage_root(temp.path(), Duration::from_secs(5));

        let started = Arc::new(Barrier::new(2));
        let holder_done = Arc::new(AtomicBool::new(false));

        let guard = lock.acquire().unwrap();

        let waiter = {
            let lock = Arc::clone(&lock);
            let started = Arc::clone(&started);
            let holder_done = Arc::clone(&holder_done);
            std::thread::spawn(move || {
                started.wait();
                let _g = lock.acquire().unwrap();
                // The holder released before this acquisition returned.
                assert!(
                    holder_done.load(Ordering::SeqCst),
                    "a second thread must not enter while the lock is held"
                );
            })
        };

        started.wait();
        std::thread::sleep(Duration::from_millis(150));
        holder_done.store(true, Ordering::SeqCst);
        drop(guard);

        waiter.join().unwrap();
    }

    #[test]
    fn test_in_process_lock_needs_no_files() {
        let lock = RepoWriteLock::in_process();
        assert!(lock.path().is_none());
        let outer = lock.acquire().unwrap();
        let inner = lock.acquire().unwrap();
        drop(inner);
        drop(outer);
        let _again = lock.acquire().unwrap();
    }

    #[test]
    fn test_lock_released_when_holder_panics() {
        let temp = TempDir::new().unwrap();
        let lock = RepoWriteLock::for_storage_root(temp.path(), Duration::from_millis(500));

        let panicking = {
            let lock = Arc::clone(&lock);
            std::thread::spawn(move || {
                let _guard = lock.acquire().unwrap();
                panic!("holder panics with the guard alive");
            })
        };
        assert!(panicking.join().is_err());

        // Unwinding dropped the guard; the lock is usable again.
        let _guard = lock.acquire().unwrap();
    }
}
