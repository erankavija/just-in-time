//! Multi-threaded contention tests for `open_mutation_session`.
//!
//! These tests drive one shared [`InMemoryStorage`] with real OS threads
//! through the public [`RepositoryStateStore`] trait — no failure injection.
//! Correctness is asserted as invariants over the race (mutual exclusion,
//! exhaustive success/conflict accounting) rather than a fixed interleaving,
//! so the tests stay deterministic under heavy parallel load.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

/// A structurally valid layout with no on-disk footprint: `InMemoryStorage`
/// never touches the filesystem through `open_mutation_session`, so the
/// paths only need to be absolute and distinct, not to exist.
fn labeled_layout(root: &std::path::Path, label: &str) -> RepositoryLayout {
    RepositoryLayout::new(
        RepositoryRootEvidence::new(
            root.join(format!("wt-{label}")),
            format!("contention-worktree:{label}"),
            true,
        ),
        RepositoryRootEvidence::new(
            root.join(format!("data-{label}")),
            format!("contention-data:{label}"),
            true,
        ),
    )
    .unwrap()
}

/// REQ-01: many threads race to open a mutation session for the SAME layout
/// against one shared store. Every attempt is a legitimate reentrant open
/// (`ActiveLayoutTracker::enter` admits repeated entry for an equal layout),
/// so every thread must observe `Ok`, and the invariant under test is that
/// the store still serializes them one at a time rather than letting two
/// sessions hold the repository write lock concurrently — the property the
/// whole capture/apply boundary depends on.
#[test]
fn test_open_mutation_session_concurrent_same_layout_serializes_over_shared_store() {
    const THREADS: usize = 8;

    let temp = TempDir::new().unwrap();
    let layout = labeled_layout(temp.path(), "shared");
    let storage = InMemoryStorage::new();

    let barrier = Arc::new(Barrier::new(THREADS));
    // Number of sessions currently holding the write lock, and the peak
    // ever observed. A max above 1 would mean two sessions were live at
    // once — exactly what the repository write lock must prevent.
    let concurrent_holders = Arc::new(AtomicUsize::new(0));
    let peak_holders = Arc::new(AtomicUsize::new(0));
    let successes = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let storage = storage.clone();
            let layout = layout.clone();
            let barrier = Arc::clone(&barrier);
            let concurrent_holders = Arc::clone(&concurrent_holders);
            let peak_holders = Arc::clone(&peak_holders);
            let successes = Arc::clone(&successes);
            thread::spawn(move || {
                barrier.wait();
                let session = storage
                    .open_mutation_session(layout)
                    .expect("same-layout reentry must never be rejected");
                let now_holding = concurrent_holders.fetch_add(1, Ordering::SeqCst) + 1;
                peak_holders.fetch_max(now_holding, Ordering::SeqCst);
                // Widen the window in which a broken mutual-exclusion
                // guarantee would actually overlap two holders.
                thread::sleep(Duration::from_millis(10));
                concurrent_holders.fetch_sub(1, Ordering::SeqCst);
                successes.fetch_add(1, Ordering::SeqCst);
                drop(session);
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(
        successes.load(Ordering::SeqCst),
        THREADS,
        "every reentrant same-layout open must succeed"
    );
    assert_eq!(
        peak_holders.load(Ordering::SeqCst),
        1,
        "at most one session may hold the write lock at a time"
    );
    assert_eq!(concurrent_holders.load(Ordering::SeqCst), 0);
}

/// REQ-02: while one session retains a layout, concurrent attempts to open a
/// session for a DIFFERENT layout must be rejected. This exercises
/// `ActiveLayoutTracker::enter`'s `RetryableConflict` path — surfaced only
/// through the public `RepositoryStateStore::open_mutation_session` trait
/// method — under real thread contention rather than a single sequential
/// call.
#[test]
fn test_open_mutation_session_active_layout_reentry_rejected_under_concurrency() {
    const THREADS: usize = 6;

    let temp = TempDir::new().unwrap();
    let held_layout = labeled_layout(temp.path(), "held");
    let other_layout = labeled_layout(temp.path(), "other");
    let storage = InMemoryStorage::new();

    // Retained for the whole race: every racer below observes this exact
    // layout as active, so the conflict outcome is deterministic regardless
    // of scheduling order.
    let held_session = storage
        .open_mutation_session(held_layout)
        .expect("first open of an unheld layout always succeeds");

    let barrier = Arc::new(Barrier::new(THREADS));
    let conflicts = Arc::new(AtomicUsize::new(0));
    let unexpected = Arc::new(Mutex::new(Vec::new()));

    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let storage = storage.clone();
            let other_layout = other_layout.clone();
            let barrier = Arc::clone(&barrier);
            let conflicts = Arc::clone(&conflicts);
            let unexpected = Arc::clone(&unexpected);
            thread::spawn(move || {
                barrier.wait();
                match storage.open_mutation_session(other_layout) {
                    Err(RepositoryStateStoreError::RetryableConflict { .. }) => {
                        conflicts.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok(_) => unexpected
                        .lock()
                        .unwrap()
                        .push("unexpected success while a different layout was held".to_string()),
                    Err(other) => unexpected.lock().unwrap().push(format!("{other:?}")),
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(
        conflicts.load(Ordering::SeqCst),
        THREADS,
        "every concurrent reentry under a different layout must be rejected"
    );
    assert!(
        unexpected.lock().unwrap().is_empty(),
        "no racer should see success or a non-conflict error while the layout is held"
    );

    drop(held_session);

    // The rejection is transient, not a stuck tracker: once the holder
    // releases, the previously conflicting layout opens cleanly.
    let released = storage.open_mutation_session(other_layout);
    assert!(
        released.is_ok(),
        "the tracker must clear once the holding session drops"
    );
}
