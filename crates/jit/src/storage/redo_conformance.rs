//! The redo record's interruption matrix, driven over every backend that
//! applies one.
//!
//! The matrix is the record mechanism's own behaviour, so it is proven once,
//! here, against the mechanism: one driver, generic over [`RecordBackend`], and
//! one injector per point that both backends are handed, because both step the
//! same [`RedoPoint`] values in the same order. A backend's own suite states
//! only what that backend adds (`@/inv/orthogonal-tests`).

use crate::repository_state::{FileMode, RepositoryEntry, VirtualPath};
use crate::storage::capability_fs::{CapabilityRoots, DataRoot};
use crate::storage::memory::test_fixtures as memory_fixtures;
use crate::storage::redo_apply::test_fixtures::{
    arbitrary_record, data_root, directory_vocabulary_paths, path_vocabulary, record_relative_path,
    record_scratch_relative_path, repository, seed_repository, snapshot, InterruptAt,
    RecordedPoints, SnapshotEntry, CAPTURED_LOG,
};
use crate::storage::{
    commit_redo_record, EventAppend, InMemoryStorage, MemoryRedoApplier, NoRedoInterruptions,
    PutFile, RedoApplier, RedoApplyError, RedoInterruptionInjector, RedoPoint, RedoRecord,
};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use std::collections::{BTreeMap, HashSet};
use std::mem::discriminant;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

/// The state a record is applied over: pre-existing targets its operations can
/// collide with, and whether the event log carries residue beyond the offset the
/// record captured.
///
/// Every occupant it seeds is a regular file. What an unsupported occupant
/// waiting at a target raises is a property of that target rather than of the
/// record, and the backends necessarily differ on it — `ELOOP` on the filesystem
/// against `Unaddressable` where there are no symlinks to model — so it stays
/// with each backend.
#[derive(Debug, Clone, Default)]
struct InitialState {
    occupied: Vec<VirtualPath>,
    residue: bool,
}

fn arbitrary_initial_state() -> impl Strategy<Value = InitialState> {
    (
        prop::collection::vec(path_vocabulary(), 0..3),
        any::<bool>(),
    )
        .prop_map(|(occupied, residue)| InitialState { occupied, residue })
}

/// What the matrix needs of a backend: seeded state, the commit point, one
/// interruptible application, and a reading of what a record published.
trait RecordBackend {
    /// What a record publishes, without the record itself, so two applications
    /// are compared on what they published rather than on what is left to
    /// replay.
    type Published: Clone + std::fmt::Debug + PartialEq;

    fn seeded(initial: &InitialState) -> Self;
    fn commit(&self, record: &RedoRecord);
    fn apply(
        &self,
        injector: Arc<dyn RedoInterruptionInjector>,
        record: &RedoRecord,
    ) -> Result<(), RedoApplyError>;
    fn published(&self) -> Self::Published;
    /// Whether a record is still waiting for an application to carry it forward.
    fn holds_record(&self) -> bool;
}

struct FileBackend {
    worktree: TempDir,
    roots: CapabilityRoots,
}

impl FileBackend {
    /// Where a commit leaves the record, so a caller can read it back or cut it
    /// down to the residue an interrupted commit leaves.
    fn record_path(&self) -> PathBuf {
        self.worktree.path().join(record_relative_path())
    }
}

impl RecordBackend for FileBackend {
    type Published = BTreeMap<PathBuf, SnapshotEntry>;

    fn seeded(initial: &InitialState) -> Self {
        let (worktree, roots) = repository();
        seed_repository(&worktree, &initial.occupied, initial.residue);
        Self { worktree, roots }
    }

    fn commit(&self, record: &RedoRecord) {
        commit_redo_record(&DataRoot::open(&data_root(&self.worktree)).unwrap(), record).unwrap();
    }

    fn apply(
        &self,
        injector: Arc<dyn RedoInterruptionInjector>,
        record: &RedoRecord,
    ) -> Result<(), RedoApplyError> {
        RedoApplier::new(injector).apply(&self.roots, record)
    }

    fn published(&self) -> Self::Published {
        let mut entries = snapshot(self.worktree.path());
        entries.remove(&record_relative_path());
        entries.remove(&record_scratch_relative_path());
        entries
    }

    fn holds_record(&self) -> bool {
        self.record_path().exists()
    }
}

struct MemoryBackend(InMemoryStorage);

impl RecordBackend for MemoryBackend {
    type Published = (BTreeMap<VirtualPath, RepositoryEntry>, bool);

    fn seeded(initial: &InitialState) -> Self {
        Self(memory_fixtures::seeded(&initial.occupied, initial.residue))
    }

    fn commit(&self, record: &RedoRecord) {
        self.0.commit_record(record).unwrap();
    }

    fn apply(
        &self,
        injector: Arc<dyn RedoInterruptionInjector>,
        record: &RedoRecord,
    ) -> Result<(), RedoApplyError> {
        MemoryRedoApplier::new(injector).apply(&self.0, record)
    }

    fn published(&self) -> Self::Published {
        let state = memory_fixtures::semantic_state(&self.0);
        (state.0, state.1)
    }

    fn holds_record(&self) -> bool {
        memory_fixtures::semantic_state(&self.0).2.is_some()
    }
}

/// What an interruption at one point leaves behind, before anything is replayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interrupted {
    /// No live write has run: the repository reads back as the complete old
    /// state, with the record still there to carry the mutation forward.
    NothingPublished,
    /// Operations may be half applied, and the record is still there.
    PartlyPublished,
    /// Every operation is applied and the record is consumed.
    Published,
}

/// What an interruption at `point` leaves.
///
/// Exhaustive over [`RedoPoint`] with no wildcard arm: a point added to the enum
/// fails the build here until this suite classifies it, so the matrix's coverage
/// is the enum's rather than a list kept by hand.
fn classify(point: &RedoPoint) -> Interrupted {
    match point {
        RedoPoint::BeforeApply => Interrupted::NothingPublished,
        RedoPoint::BeforeMakeDirectory { .. }
        | RedoPoint::BeforePut { .. }
        | RedoPoint::BeforeDelete { .. }
        | RedoPoint::BeforeEventAppend
        | RedoPoint::BeforeRecordRemoval => Interrupted::PartlyPublished,
        RedoPoint::AfterApply => Interrupted::Published,
    }
}

/// One value of every point. [`classify`] fails the build on a point nobody
/// classified; this list fails
/// [`test_the_matrix_drives_an_interruption_at_every_point_on_both_backends`] on
/// a point nobody drives.
fn point_samples() -> [RedoPoint; 7] {
    [
        RedoPoint::BeforeApply,
        RedoPoint::BeforeMakeDirectory { index: 0 },
        RedoPoint::BeforePut { index: 0 },
        RedoPoint::BeforeDelete { index: 0 },
        RedoPoint::BeforeEventAppend,
        RedoPoint::BeforeRecordRemoval,
        RedoPoint::AfterApply,
    ]
}

/// A record reaching every point of the pass: a made directory, two puts, a
/// delete, and an append over disjoint vocabulary paths.
fn full_record() -> RedoRecord {
    let published = |path| PutFile {
        path,
        bytes: b"published".to_vec(),
        mode: FileMode::Regular,
    };
    let [directory, _] = directory_vocabulary_paths();
    RedoRecord::new(
        vec![directory],
        vec![
            published(VirtualPath::data("index.json").unwrap()),
            published(VirtualPath::worktree("notes.md").unwrap()),
        ],
        vec![VirtualPath::worktree("dev/plan.md").unwrap()],
        Some(EventAppend {
            offset: CAPTURED_LOG.len() as u64,
            bytes: b"{\"tag\":\"issue_claimed\"}\n".to_vec(),
        }),
    )
    .unwrap()
}

/// A backend seeded with `initial`, holding `record` as its committed mutation.
fn committed<B: RecordBackend>(initial: &InitialState, record: &RedoRecord) -> B {
    let backend = B::seeded(initial);
    backend.commit(record);
    backend
}

/// One uninterrupted application over a fresh backend: the complete new state
/// every interrupted application must converge on, and the points its pass
/// checked.
fn applied_cleanly<B: RecordBackend>(
    initial: &InitialState,
    record: &RedoRecord,
) -> (B::Published, Vec<RedoPoint>) {
    let backend = committed::<B>(initial, record);
    let recorded = Arc::new(RecordedPoints::default());
    let injector: Arc<dyn RedoInterruptionInjector> = recorded.clone();
    backend.apply(injector, record).unwrap();
    (backend.published(), recorded.points())
}

fn uninterrupted<B: RecordBackend>(backend: &B, record: &RedoRecord) {
    backend
        .apply(Arc::new(NoRedoInterruptions), record)
        .unwrap();
}

/// Whether the pass stopped at the point the injector named, rather than running
/// past it or failing for another reason.
fn interrupted_at<T>(outcome: &Result<T, RedoApplyError>, point: &RedoPoint) -> bool {
    matches!(outcome, Err(RedoApplyError::Interrupted { point: stopped, .. }) if stopped == point)
}

/// Nothing a record names reaches the repository before the pass's first live
/// write: the record's own commit publishes no entry, and an interruption at a
/// point classified [`Interrupted::NothingPublished`] leaves the state the
/// mutation started from, with the record there to carry it forward.
fn nothing_published<B: RecordBackend>(
    initial: &InitialState,
    record: &RedoRecord,
    point: &RedoPoint,
    injector: Arc<dyn RedoInterruptionInjector>,
) -> Result<(), TestCaseError> {
    let backend = B::seeded(initial);
    let old = backend.published();
    backend.commit(record);
    prop_assert_eq!(
        backend.published(),
        old.clone(),
        "the record's own commit published a live write"
    );

    let outcome = backend.apply(injector, record);

    prop_assert!(
        interrupted_at(&outcome, point),
        "the pass ran past the interruption at {:?}",
        point
    );
    prop_assert_eq!(
        backend.published(),
        old,
        "a live write ran before the pass reached its first one, interrupted at {:?}",
        point
    );
    prop_assert!(
        backend.holds_record(),
        "the record that carries the mutation forward is gone, interrupted at {:?}",
        point
    );
    Ok(())
}

/// An application interrupted at `point`, then replayed twice: the leftover
/// record reaches the complete new state and holds it.
fn replay_converges<B: RecordBackend>(
    initial: &InitialState,
    record: &RedoRecord,
    point: &RedoPoint,
    injector: Arc<dyn RedoInterruptionInjector>,
    converged: &B::Published,
) -> Result<(), TestCaseError> {
    let backend = committed::<B>(initial, record);

    let outcome = backend.apply(injector, record);

    prop_assert!(
        interrupted_at(&outcome, point),
        "the pass ran past the interruption at {:?}",
        point
    );
    prop_assert_eq!(
        backend.holds_record(),
        classify(point) != Interrupted::Published,
        "the record left at {:?} is not the one its point classifies",
        point
    );

    uninterrupted(&backend, record);
    prop_assert_eq!(
        &backend.published(),
        converged,
        "re-application after {:?} left a different repository",
        point
    );

    uninterrupted(&backend, record);
    prop_assert_eq!(
        &backend.published(),
        converged,
        "a second application moved the repository the first one converged on, after {:?}",
        point
    );
    Ok(())
}

/// Every point the enum names is reached and interrupted, on both backends, so
/// the matrix's coverage is the enum's rather than one generated record's shape.
#[test]
fn test_the_matrix_drives_an_interruption_at_every_point_on_both_backends() {
    let initial = InitialState::default();
    let record = full_record();
    let (_, driven) = applied_cleanly::<MemoryBackend>(&initial, &record);

    assert_eq!(
        driven.iter().map(discriminant).collect::<HashSet<_>>(),
        point_samples()
            .iter()
            .map(discriminant)
            .collect::<HashSet<_>>(),
        "the driven record never reaches a point the enum names"
    );
    driven.iter().for_each(|point| {
        let injector: Arc<dyn RedoInterruptionInjector> =
            Arc::new(InterruptAt::point(point.clone()));
        let file = committed::<FileBackend>(&initial, &record);
        let memory = committed::<MemoryBackend>(&initial, &record);

        assert!(
            interrupted_at(&file.apply(injector.clone(), &record), point),
            "the filesystem pass ran past {point:?}"
        );
        assert!(
            interrupted_at(&memory.apply(injector, &record), point),
            "the in-memory pass ran past {point:?}"
        );
    });
}

/// A made directory something other than a directory occupies is a typed
/// refusal on both backends, rather than a silent success that leaves the
/// record's target un-made. The generated matrix never produces this case: its
/// directory vocabulary is disjoint from the paths it seeds occupants at,
/// because every other occupant is a convergence case rather than a refusal.
#[test]
fn test_a_non_directory_occupying_a_made_directory_is_refused_on_both_backends() {
    let [directory, _] = directory_vocabulary_paths();
    let initial = InitialState {
        occupied: vec![directory.clone()],
        residue: false,
    };
    let record = RedoRecord::new(vec![directory], Vec::new(), Vec::new(), None).unwrap();
    let file = committed::<FileBackend>(&initial, &record);
    let memory = committed::<MemoryBackend>(&initial, &record);

    let outcomes = [
        file.apply(Arc::new(NoRedoInterruptions), &record),
        memory.apply(Arc::new(NoRedoInterruptions), &record),
    ];

    outcomes.iter().for_each(|outcome| {
        assert!(
            matches!(outcome, Err(RedoApplyError::OccupiedDirectory { .. })),
            "a record whose directory a regular file occupies was applied: {outcome:?}"
        );
    });
}

proptest! {
    // Each case applies the record once per point of its pass, on both backends,
    // each application over its own repository, so the case count stays low.
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// The record's commit is the mutation's commit point, so an interruption
    /// anywhere inside it leaves the repository the mutation started from: the
    /// mutation is lost whole rather than in part.
    ///
    /// The commit writes the record's bytes at their final path, so a crash
    /// leaves any proper prefix of them; the commit runs here and its output is
    /// then cut back to that prefix, which is the same live state, because what
    /// the commit does after a given byte writes no content anywhere else. That
    /// a cut-back residue is no record a later session carries forward is
    /// [`RedoRecord::parse`]'s property, proven over the same prefixes in
    /// `redo_record`.
    ///
    /// The in-memory commit installs the record in one assignment and has no
    /// interior to interrupt, so this window is the filesystem backend's alone.
    #[test]
    fn prop_an_interruption_during_the_records_commit_leaves_the_complete_old_state(
        record in arbitrary_record(),
        initial in arbitrary_initial_state(),
    ) {
        let backend = FileBackend::seeded(&initial);
        let old = backend.published();

        backend.commit(&record);

        let complete = std::fs::read(backend.record_path()).unwrap();
        for written in 0..complete.len() {
            std::fs::write(backend.record_path(), &complete[..written]).unwrap();
            prop_assert_eq!(
                backend.published(),
                old.clone(),
                "a commit interrupted after {} of {} bytes reached the repository",
                written,
                complete.len()
            );
        }
    }

    /// The mutation is lost whole rather than in part while the pass has written
    /// nothing: durability of the record publishes no entry of its own, and the
    /// points classified before the first live write leave the old state.
    #[test]
    fn prop_nothing_is_published_before_the_pass_reaches_its_first_live_write(
        record in arbitrary_record(),
        initial in arbitrary_initial_state(),
    ) {
        let (_, driven) = applied_cleanly::<MemoryBackend>(&initial, &record);

        for point in driven.iter().filter(|point| classify(point) == Interrupted::NothingPublished) {
            let injector: Arc<dyn RedoInterruptionInjector> =
                Arc::new(InterruptAt::point(point.clone()));
            nothing_published::<FileBackend>(&initial, &record, point, injector.clone())?;
            nothing_published::<MemoryBackend>(&initial, &record, point, injector)?;
        }
    }

    /// An interruption at any point of the pass is recovered by applying the
    /// leftover record forward, whichever backend ran it, and a further
    /// application leaves the state it reached.
    #[test]
    fn prop_re_application_after_an_interruption_at_any_point_reaches_and_holds_the_new_state(
        record in arbitrary_record(),
        initial in arbitrary_initial_state(),
    ) {
        let (file_converged, file_points) = applied_cleanly::<FileBackend>(&initial, &record);
        let (memory_converged, memory_points) = applied_cleanly::<MemoryBackend>(&initial, &record);
        // One injector drives both backends, which means an interruption only
        // while their passes check the same points in the same order.
        prop_assert_eq!(
            &file_points,
            &memory_points,
            "the backends' passes check different points, so one injected interruption means two things"
        );

        for point in &file_points {
            let injector: Arc<dyn RedoInterruptionInjector> =
                Arc::new(InterruptAt::point(point.clone()));
            replay_converges::<FileBackend>(&initial, &record, point, injector.clone(), &file_converged)?;
            replay_converges::<MemoryBackend>(&initial, &record, point, injector, &memory_converged)?;
        }
    }
}
