//! Applying a redo record to the repository's filesystem roots.
//!
//! One forward pass over the record's operations, then the record is removed.
//! Re-application is the same pass: a put overwrites, a delete tolerates
//! absence, the append rewrites its bytes at its offset, so a record applied
//! twice converges. Nothing publishes through this module yet.

use crate::repository_state::{FileMode, RepositoryRootClass, VirtualPath};
use crate::storage::atomic_write::sync_capability_directory;
use crate::storage::file_transaction::repository_unix_mode;
use crate::storage::redo_record::{
    EventAppend, PutFile, RedoRecord, RECORD_FILE_NAME, RECORD_SCRATCH_DIR,
};
use crate::storage::repository_state_store::{
    open_capability_parent, open_child_dir_nofollow, open_or_create_capability_parent,
    CapabilityRoots, RepositoryStateStoreError,
};
use cap_primitives::fs::FollowSymlinks;
use cap_std::fs::{Dir, File, OpenOptions};
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

/// One step of a record's application, named so an interruption matrix can stop
/// the pass at it.
///
/// A point names the state before its step runs, and both backends step these
/// in this order, so an injected interruption means the same thing on either.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RedoPoint {
    /// Every path resolved, before the first live write.
    BeforeApply,
    /// Before put `index` replaces its path.
    BeforePut { index: usize },
    /// Before delete `index` removes its path.
    BeforeDelete { index: usize },
    /// Before the appended bytes are written at their offset.
    BeforeEventAppend,
    /// Every operation applied, before the record is removed.
    BeforeRecordRemoval,
    /// The record is gone: the mutation is complete.
    AfterApply,
}

/// Test seam for interrupting one named step of a record's application.
pub trait RedoInterruptionInjector: Send + Sync {
    /// Interrupt the pass at one point, or let it proceed.
    fn check(&self, point: &RedoPoint) -> std::io::Result<()>;
}

/// Production injector: every point proceeds.
#[derive(Debug, Default)]
pub struct NoRedoInterruptions;

impl RedoInterruptionInjector for NoRedoInterruptions {
    fn check(&self, _point: &RedoPoint) -> std::io::Result<()> {
        Ok(())
    }
}

/// Failures of one record application.
#[derive(Debug, thiserror::Error)]
pub enum RedoApplyError {
    #[error("redo record addresses {path}, which the selected roots do not hold")]
    Unaddressable { path: String },
    #[error("redo application interrupted at {point:?}: {source}")]
    Interrupted {
        point: RedoPoint,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Roots(#[from] RepositoryStateStoreError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The filesystem backend's record application, holding the injector every
/// application it runs checks its points through.
///
/// The injector is the boundary's rather than a call's, so no caller can vary
/// interruption between two applications of the same applier.
pub struct RedoApplier {
    injector: Arc<dyn RedoInterruptionInjector>,
}

impl RedoApplier {
    /// Hold `injector` for every application this applier runs.
    pub fn new(injector: Arc<dyn RedoInterruptionInjector>) -> Self {
        Self { injector }
    }

    /// Apply `record` to `roots` and remove it, checking every [`RedoPoint`].
    ///
    /// Every path is resolved before the first live write, so a record naming a
    /// target outside the selected roots changes nothing.
    ///
    /// # Errors
    ///
    /// Returns [`RedoApplyError`] when a path is unaddressable, an injected
    /// point interrupts the pass, or the filesystem refuses an operation.
    pub fn apply(
        &self,
        roots: &CapabilityRoots,
        record: &RedoRecord,
    ) -> Result<(), RedoApplyError> {
        let injector = &*self.injector;
        let event_log = VirtualPath::EVENTS;
        record
            .puts
            .iter()
            .map(|put| &put.path)
            .chain(record.deletes.iter())
            .chain(record.event_append.iter().map(|_| &event_log))
            .try_for_each(|path| resolve(roots, path).map(|_| ()))?;
        check(injector, RedoPoint::BeforeApply)?;
        record
            .puts
            .iter()
            .enumerate()
            .try_for_each(|(index, put)| {
                check(injector, RedoPoint::BeforePut { index })?;
                put_file(roots, put)
            })?;
        record
            .deletes
            .iter()
            .enumerate()
            .try_for_each(|(index, path)| {
                check(injector, RedoPoint::BeforeDelete { index })?;
                delete_file(roots, path)
            })?;
        if let Some(append) = &record.event_append {
            check(injector, RedoPoint::BeforeEventAppend)?;
            append_events(roots, append)?;
        }
        check(injector, RedoPoint::BeforeRecordRemoval)?;
        remove_record(roots)?;
        check(injector, RedoPoint::AfterApply)
    }
}

fn check(injector: &dyn RedoInterruptionInjector, point: RedoPoint) -> Result<(), RedoApplyError> {
    injector
        .check(&point)
        .map_err(|source| RedoApplyError::Interrupted { point, source })
}

/// The root handle a path is addressed within, with its root-relative portion.
///
/// The parent chain is walked no-follow here rather than at the write, so a
/// symlinked ancestor — the one escape a canonical identity cannot carry — is
/// refused while the repository is still untouched.
fn resolve<'a>(
    roots: &'a CapabilityRoots,
    path: &'a VirtualPath,
) -> Result<(&'a Dir, &'a Path), RedoApplyError> {
    let root = roots
        .root(path.root_class())
        .ok_or_else(|| unaddressable(path))?;
    open_capability_parent(root, path.relative().as_path())?;
    Ok((root, path.relative().as_path()))
}

/// Replace a put's path with its bytes, creating the directories it needs.
///
/// Every directory the put creates is durable before the pass reaches
/// [`RedoPoint::BeforeRecordRemoval`], so a crash cannot drop an ancestor entry
/// while removing the record that would replay the put.
fn put_file(roots: &CapabilityRoots, put: &PutFile) -> Result<(), RedoApplyError> {
    let (root, relative) = resolve(roots, &put.path)?;
    let (parent, leaf) = open_or_create_capability_parent(root, relative)?;
    let staged = staged_name();
    write_staged(&parent, &staged, put)?;
    parent.rename(&staged, &parent, &leaf)?;
    sync_capability_directory(&parent).map_err(Into::into)
}

/// A stage name beside the target: random, so nothing can be waiting at it, and
/// `.tmp`, so a staging its apply never renamed is reclaimed by the orphan
/// sweep in [`super::temp_cleanup`].
fn staged_name() -> String {
    format!(".redo.{}.tmp", uuid::Uuid::new_v4().simple())
}

/// Write the staged bytes with the put's mode and return once they are durable.
///
/// The open creates the file or fails: an occupied stage name is never opened
/// through, so no symlink carries the bytes elsewhere and no occupant is
/// truncated (`@/inv/atomic-writes`).
fn write_staged(parent: &Dir, staged: &str, put: &PutFile) -> Result<(), RedoApplyError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let mut file = parent.open_with(staged, &options)?;
    file.write_all(&put.bytes)?;
    set_mode(&file, put.mode)?;
    file.sync_all().map_err(Into::into)
}

#[cfg(unix)]
fn set_mode(file: &File, mode: FileMode) -> Result<(), RedoApplyError> {
    use cap_std::fs::PermissionsExt;

    repository_unix_mode(mode)
        .map(|mode| file.set_permissions(cap_std::fs::Permissions::from_mode(mode)))
        .transpose()?;
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_file: &File, _mode: FileMode) -> Result<(), RedoApplyError> {
    Ok(())
}

/// Remove a deleted path, tolerating a target a previous application already
/// removed.
fn delete_file(roots: &CapabilityRoots, path: &VirtualPath) -> Result<(), RedoApplyError> {
    let (root, relative) = resolve(roots, path)?;
    let Some((parent, leaf)) = open_capability_parent(root, relative)? else {
        return Ok(());
    };
    match parent.remove_file(&leaf) {
        Ok(()) => sync_capability_directory(&parent).map_err(Into::into),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Write the appended bytes at their offset.
///
/// The log is opened for positioned writing rather than in append mode, which
/// ignores the offset and would append the bytes a second time, and it is
/// truncated to the offset plus their length, so a log a previous application
/// already extended converges.
fn append_events(roots: &CapabilityRoots, append: &EventAppend) -> Result<(), RedoApplyError> {
    let log = VirtualPath::EVENTS;
    let (root, relative) = resolve(roots, &log)?;
    let (parent, leaf) =
        open_capability_parent(root, relative)?.ok_or_else(|| unaddressable(&log))?;
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let mut file = parent.open_with(&leaf, &options)?;
    file.seek(SeekFrom::Start(append.offset))?;
    file.write_all(&append.bytes)?;
    file.set_len(append.offset + append.bytes.len() as u64)?;
    file.sync_all().map_err(Into::into)
}

/// Remove the applied record, tolerating one a previous application already
/// removed.
fn remove_record(roots: &CapabilityRoots) -> Result<(), RedoApplyError> {
    let data =
        roots
            .root(RepositoryRootClass::Data)
            .ok_or_else(|| RedoApplyError::Unaddressable {
                path: format!("{RECORD_SCRATCH_DIR}/{RECORD_FILE_NAME}"),
            })?;
    match data.remove_file(Path::new(RECORD_SCRATCH_DIR).join(RECORD_FILE_NAME)) {
        Ok(()) => sync_capability_directory(&open_child_dir_nofollow(data, RECORD_SCRATCH_DIR)?)
            .map_err(Into::into),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn unaddressable(path: &VirtualPath) -> RedoApplyError {
    RedoApplyError::Unaddressable {
        path: format!("{path:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::{FileMode, RepositoryRootClass};
    use crate::storage::discover_repository_layout;
    use crate::storage::temp_cleanup::cleanup_orphaned_temp_files;
    use proptest::prelude::*;
    use std::collections::{BTreeMap, HashSet};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    /// The event-log bytes a mutation captured before appending to them.
    const CAPTURED_LOG: &[u8] = b"{\"tag\":\"issue_created\"}\n";

    /// A repository whose worktree holds its data root, with capability handles
    /// for both.
    fn repository() -> (TempDir, CapabilityRoots) {
        let worktree = TempDir::new().unwrap();
        std::fs::create_dir(worktree.path().join(".jit")).unwrap();
        let layout =
            discover_repository_layout(worktree.path(), worktree.path().join(".jit")).unwrap();
        let roots = CapabilityRoots::open(&layout).unwrap().0;
        (worktree, roots)
    }

    fn data_root(worktree: &TempDir) -> PathBuf {
        worktree.path().join(".jit")
    }

    fn put(path: VirtualPath, bytes: &[u8]) -> PutFile {
        PutFile {
            path,
            bytes: bytes.to_vec(),
            mode: FileMode::Regular,
        }
    }

    /// An applier whose held injector lets every point proceed.
    fn uninterrupted() -> RedoApplier {
        RedoApplier::new(Arc::new(NoRedoInterruptions))
    }

    /// An applier over a recorder the caller keeps, so the points its
    /// applications checked stay readable.
    fn recording_applier() -> (Arc<RecordedPoints>, RedoApplier) {
        let recorded = Arc::new(RecordedPoints::default());
        let injector: Arc<dyn RedoInterruptionInjector> = recorded.clone();
        (recorded, RedoApplier::new(injector))
    }

    /// Records every point the pass checks, in order.
    #[derive(Default)]
    struct RecordedPoints(Mutex<Vec<RedoPoint>>);

    impl RecordedPoints {
        fn points(&self) -> Vec<RedoPoint> {
            self.0.lock().unwrap().clone()
        }
    }

    impl RedoInterruptionInjector for RecordedPoints {
        fn check(&self, point: &RedoPoint) -> std::io::Result<()> {
            self.0.lock().unwrap().push(point.clone());
            Ok(())
        }
    }

    /// Interrupts the pass at its `at`-th check, so exactly the steps before it
    /// ran.
    struct InterruptAt {
        at: usize,
        seen: AtomicUsize,
    }

    impl InterruptAt {
        fn check_number(at: usize) -> Self {
            Self {
                at,
                seen: AtomicUsize::new(0),
            }
        }
    }

    impl RedoInterruptionInjector for InterruptAt {
        fn check(&self, point: &RedoPoint) -> std::io::Result<()> {
            match self.seen.fetch_add(1, Ordering::Relaxed) == self.at {
                true => Err(std::io::Error::other(format!("interrupted at {point:?}"))),
                false => Ok(()),
            }
        }
    }

    /// Every file under `root`, keyed by its root-relative path, with the bytes
    /// and the executable bit that decide whether two repositories converged.
    fn snapshot(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, bool)> {
        std::fs::read_dir(root)
            .unwrap()
            .flat_map(|entry| {
                let entry = entry.unwrap();
                let path = entry.path();
                match entry.file_type().unwrap().is_dir() {
                    true => snapshot(&path)
                        .into_iter()
                        .map(|(nested, content)| {
                            (PathBuf::from(entry.file_name()).join(nested), content)
                        })
                        .collect::<BTreeMap<_, _>>(),
                    false => BTreeMap::from([(
                        PathBuf::from(entry.file_name()),
                        (std::fs::read(&path).unwrap(), is_executable(&path)),
                    )]),
                }
            })
            .collect()
    }

    #[cfg(unix)]
    fn is_executable(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;

        std::fs::metadata(path).unwrap().permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    fn is_executable(_path: &Path) -> bool {
        false
    }

    #[test]
    fn test_redo_applier_apply_publishes_every_operation_the_record_names() {
        let (worktree, roots) = repository();
        std::fs::write(data_root(&worktree).join("index.json"), b"captured").unwrap();
        std::fs::write(worktree.path().join("stale.md"), b"stale").unwrap();
        std::fs::write(data_root(&worktree).join("events.jsonl"), CAPTURED_LOG).unwrap();
        let record = RedoRecord {
            puts: vec![put(VirtualPath::data("index.json").unwrap(), b"published")],
            deletes: vec![VirtualPath::worktree("stale.md").unwrap()],
            event_append: Some(EventAppend {
                offset: CAPTURED_LOG.len() as u64,
                bytes: b"{\"tag\":\"issue_claimed\"}\n".to_vec(),
            }),
        };

        uninterrupted().apply(&roots, &record).unwrap();

        assert_eq!(
            std::fs::read(data_root(&worktree).join("index.json")).unwrap(),
            b"published"
        );
        assert!(
            !worktree.path().join("stale.md").exists(),
            "the deleted path survived the application"
        );
        assert_eq!(
            std::fs::read(data_root(&worktree).join("events.jsonl")).unwrap(),
            [CAPTURED_LOG, b"{\"tag\":\"issue_claimed\"}\n"].concat()
        );
    }

    #[test]
    fn test_redo_applier_apply_removes_the_record_it_applied() {
        let (worktree, roots) = repository();
        let record = RedoRecord {
            puts: vec![put(VirtualPath::data("index.json").unwrap(), b"published")],
            ..RedoRecord::default()
        };
        let committed = record.commit(&data_root(&worktree)).unwrap();

        uninterrupted().apply(&roots, &record).unwrap();

        assert!(
            !committed.exists(),
            "the applied record is still there for a later session to apply again"
        );
    }

    #[test]
    fn test_redo_applier_apply_creates_the_directories_a_put_needs() {
        let (worktree, roots) = repository();
        let record = RedoRecord {
            puts: vec![put(
                VirtualPath::data("issues/2026/a.json").unwrap(),
                b"published",
            )],
            ..RedoRecord::default()
        };

        uninterrupted().apply(&roots, &record).unwrap();

        assert_eq!(
            std::fs::read(data_root(&worktree).join("issues/2026/a.json")).unwrap(),
            b"published"
        );
    }

    /// The record is the only replay source for a put that created its own
    /// ancestors, so it outlives their publication: an interruption between the
    /// two leaves the record for the next application to converge on.
    #[test]
    fn test_redo_applier_apply_keeps_the_record_until_a_fresh_nested_put_is_published() {
        let (worktree, roots) = repository();
        let nested = VirtualPath::data("issues/2026/09/a.json").unwrap();
        let record = RedoRecord {
            puts: vec![put(nested, b"published")],
            ..RedoRecord::default()
        };
        let committed = record.commit(&data_root(&worktree)).unwrap();
        let published = data_root(&worktree).join("issues/2026/09/a.json");
        // BeforeApply, BeforePut { index: 0 }, then the removal this stops at.
        let before_removal = 2;

        let outcome = RedoApplier::new(Arc::new(InterruptAt::check_number(before_removal)))
            .apply(&roots, &record);

        assert!(
            matches!(outcome, Err(RedoApplyError::Interrupted { .. })),
            "the pass ran past the interruption before the record's removal"
        );
        assert!(
            published.exists(),
            "the put under fresh ancestors never reached its path"
        );
        assert!(
            committed.exists(),
            "the record was gone while the put it replays was still unfinished"
        );

        uninterrupted().apply(&roots, &record).unwrap();

        assert_eq!(std::fs::read(&published).unwrap(), b"published");
        assert!(
            !committed.exists(),
            "the replayed record survived its own application"
        );
    }

    #[test]
    fn test_redo_applier_apply_gives_each_put_the_mode_it_carries() {
        let (worktree, roots) = repository();
        let record = RedoRecord {
            puts: vec![PutFile {
                path: VirtualPath::worktree("scripts/publish.sh").unwrap(),
                bytes: b"#!/bin/sh\n".to_vec(),
                mode: FileMode::Executable,
            }],
            ..RedoRecord::default()
        };

        uninterrupted().apply(&roots, &record).unwrap();

        assert!(
            is_executable(&worktree.path().join("scripts/publish.sh")),
            "the published file does not carry the mode the record named"
        );
    }

    /// A stage name a test can occupy, standing in for the name a put draws at
    /// random.
    const OCCUPIED_STAGE: &str = ".redo.occupied.tmp";

    fn ambient_directory(path: &Path) -> Dir {
        Dir::open_ambient_dir(path, cap_std::ambient_authority()).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn test_write_staged_refuses_a_symlink_occupying_the_staged_name() {
        let directory = TempDir::new().unwrap();
        let unrelated = directory.path().join("unrelated.json");
        std::fs::write(&unrelated, b"a file the record never named").unwrap();
        // Relative and in-tree: a confined handle resolves it, where it refuses
        // an absolute target outright.
        std::os::unix::fs::symlink("unrelated.json", directory.path().join(OCCUPIED_STAGE))
            .unwrap();
        let parent = ambient_directory(directory.path());

        let outcome = write_staged(
            &parent,
            OCCUPIED_STAGE,
            &put(VirtualPath::data("index.json").unwrap(), b"published"),
        );

        assert!(
            outcome.is_err(),
            "the staging opened through a symlink waiting at its name"
        );
        assert_eq!(
            std::fs::read(&unrelated).unwrap(),
            b"a file the record never named",
            "the staged bytes landed on a path the record never named"
        );
    }

    #[test]
    fn test_write_staged_refuses_a_file_occupying_the_staged_name() {
        let directory = TempDir::new().unwrap();
        let occupant = directory.path().join(OCCUPIED_STAGE);
        std::fs::write(&occupant, b"a staging another application holds").unwrap();
        let parent = ambient_directory(directory.path());

        let outcome = write_staged(
            &parent,
            OCCUPIED_STAGE,
            &put(VirtualPath::data("index.json").unwrap(), b"published"),
        );

        assert!(outcome.is_err(), "the staging truncated an occupied name");
        assert_eq!(
            std::fs::read(&occupant).unwrap(),
            b"a staging another application holds"
        );
    }

    #[test]
    fn test_staged_name_is_unique_across_calls() {
        let names = (0..64).map(|_| staged_name()).collect::<HashSet<_>>();

        assert_eq!(
            names.len(),
            64,
            "two stagings drew one name, which a create-new open turns into a refused put"
        );
    }

    /// The counterpart of the record's survival: a staging is reconstructible
    /// from the record that named it, so the sweep is its reclaim path.
    #[test]
    fn test_staged_name_is_reclaimed_by_the_orphan_temp_sweep() {
        let directory = TempDir::new().unwrap();
        let orphan = directory.path().join(staged_name());
        std::fs::write(&orphan, b"a staging its application never renamed").unwrap();

        assert_eq!(cleanup_orphaned_temp_files(directory.path(), 0).unwrap(), 1);
        assert!(!orphan.exists(), "the sweep left the orphaned staging");
    }

    #[test]
    fn test_redo_applier_apply_writes_the_append_at_its_offset_over_a_longer_log() {
        let (worktree, roots) = repository();
        let appended = b"{\"tag\":\"issue_claimed\"}\n";
        // A log an interrupted application already extended, by more than the
        // appended bytes cover: append mode would land them past the residue,
        // and a pass that never truncates would leave its tail behind.
        std::fs::write(
            data_root(&worktree).join("events.jsonl"),
            [CAPTURED_LOG, &b"{\"tag\":\"residue\"}\n".repeat(3)].concat(),
        )
        .unwrap();
        let record = RedoRecord {
            event_append: Some(EventAppend {
                offset: CAPTURED_LOG.len() as u64,
                bytes: appended.to_vec(),
            }),
            ..RedoRecord::default()
        };

        uninterrupted().apply(&roots, &record).unwrap();

        assert_eq!(
            std::fs::read(data_root(&worktree).join("events.jsonl")).unwrap(),
            [CAPTURED_LOG, appended].concat()
        );
    }

    /// The injector belongs to the applier rather than to a call, so a second
    /// application reaches the same one without being handed it.
    #[test]
    fn test_redo_applier_apply_checks_every_application_through_the_held_injector() {
        let (_worktree, roots) = repository();
        let (recorded, applier) = recording_applier();
        let published = RedoRecord {
            puts: vec![put(VirtualPath::data("index.json").unwrap(), b"published")],
            ..RedoRecord::default()
        };
        let removed = RedoRecord {
            deletes: vec![VirtualPath::data("index.json").unwrap()],
            ..RedoRecord::default()
        };

        applier.apply(&roots, &published).unwrap();
        let after_first = recorded.points();
        applier.apply(&roots, &removed).unwrap();

        assert!(
            !after_first.is_empty(),
            "the first application never reached the held injector"
        );
        assert_eq!(
            recorded.points()[..after_first.len()],
            after_first,
            "the second application did not extend the first one's checks"
        );
        assert!(
            recorded.points().len() > after_first.len(),
            "the second application ran without reaching the applier's held injector"
        );
    }

    #[test]
    fn test_redo_applier_apply_checks_every_interruption_point_in_order() {
        let (worktree, roots) = repository();
        std::fs::write(data_root(&worktree).join("events.jsonl"), CAPTURED_LOG).unwrap();
        let record = RedoRecord {
            puts: vec![
                put(VirtualPath::data("index.json").unwrap(), b"published"),
                put(VirtualPath::worktree("notes.md").unwrap(), b"published"),
            ],
            deletes: vec![VirtualPath::worktree("stale.md").unwrap()],
            event_append: Some(EventAppend {
                offset: CAPTURED_LOG.len() as u64,
                bytes: b"{\"tag\":\"issue_claimed\"}\n".to_vec(),
            }),
        };
        let (recorded, applier) = recording_applier();

        applier.apply(&roots, &record).unwrap();

        assert_eq!(
            recorded.points(),
            vec![
                RedoPoint::BeforeApply,
                RedoPoint::BeforePut { index: 0 },
                RedoPoint::BeforePut { index: 1 },
                RedoPoint::BeforeDelete { index: 0 },
                RedoPoint::BeforeEventAppend,
                RedoPoint::BeforeRecordRemoval,
                RedoPoint::AfterApply,
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_redo_applier_apply_refuses_a_path_outside_the_roots_before_any_live_write() {
        let (worktree, roots) = repository();
        let outside = TempDir::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), data_root(&worktree).join("escape")).unwrap();
        let record = RedoRecord {
            puts: vec![
                put(VirtualPath::data("inside.json").unwrap(), b"published"),
                put(
                    VirtualPath::data("escape/captured.json").unwrap(),
                    b"published",
                ),
            ],
            ..RedoRecord::default()
        };

        assert!(uninterrupted().apply(&roots, &record).is_err());

        assert!(
            !data_root(&worktree).join("inside.json").exists(),
            "a live write ran before the record was refused"
        );
        assert!(
            !outside.path().join("captured.json").exists(),
            "the application wrote outside the selected roots"
        );
    }

    /// Paths a generated record and its pre-existing state draw from: two roots,
    /// nested and unnested, so puts and deletes collide on the same targets.
    fn path_vocabulary() -> impl Strategy<Value = VirtualPath> {
        prop_oneof![
            Just(VirtualPath::data("index.json").unwrap()),
            Just(VirtualPath::data("issues/2026/a.json").unwrap()),
            Just(VirtualPath::worktree("notes.md").unwrap()),
            Just(VirtualPath::worktree("dev/plan.md").unwrap()),
        ]
    }

    fn arbitrary_record() -> impl Strategy<Value = RedoRecord> {
        (
            prop::collection::vec(
                (
                    path_vocabulary(),
                    prop::collection::vec(any::<u8>(), 0..8),
                    prop_oneof![Just(FileMode::Regular), Just(FileMode::Executable)],
                ),
                0..4,
            ),
            prop::collection::vec(path_vocabulary(), 0..3),
            prop::option::of(prop::collection::vec(any::<u8>(), 0..8)),
        )
            .prop_map(|(puts, deletes, appended)| RedoRecord {
                puts: puts
                    .into_iter()
                    .map(|(path, bytes, mode)| PutFile { path, bytes, mode })
                    .collect(),
                deletes,
                event_append: appended.map(|bytes| EventAppend {
                    offset: CAPTURED_LOG.len() as u64,
                    bytes,
                }),
            })
    }

    /// Pre-existing targets, and whether the log carries residue beyond the
    /// offset the record captured.
    fn arbitrary_initial_state() -> impl Strategy<Value = (Vec<VirtualPath>, bool)> {
        (
            prop::collection::vec(path_vocabulary(), 0..3),
            any::<bool>(),
        )
    }

    fn seed(worktree: &TempDir, (occupied, residue): &(Vec<VirtualPath>, bool)) {
        let log = match residue {
            true => [CAPTURED_LOG, b"{\"tag\":\"residue\"}\n"].concat(),
            false => CAPTURED_LOG.to_vec(),
        };
        std::fs::write(data_root(worktree).join("events.jsonl"), log).unwrap();
        occupied.iter().for_each(|path| {
            let target = match path.root_class() {
                RepositoryRootClass::Data => data_root(worktree),
                RepositoryRootClass::Worktree => worktree.path().to_path_buf(),
            }
            .join(path.relative().as_path());
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, b"captured").unwrap();
        });
    }

    /// A repository seeded with `initial`, holding `record` as its committed
    /// mutation.
    fn committed(
        record: &RedoRecord,
        initial: &(Vec<VirtualPath>, bool),
    ) -> (TempDir, CapabilityRoots) {
        let (worktree, roots) = repository();
        seed(&worktree, initial);
        record.commit(&data_root(&worktree)).unwrap();
        (worktree, roots)
    }

    proptest! {
        // Each case applies the record once per prefix of its operation order,
        // each application over its own repository, so the case count stays low.
        #![proptest_config(ProptestConfig::with_cases(24))]

        /// The record's own point sequence enumerates every prefix, so an
        /// interruption at each one, resumed, is compared against a single clean
        /// application of the same record over the same initial state.
        #[test]
        fn prop_re_application_after_any_interrupted_prefix_converges(
            record in arbitrary_record(),
            initial in arbitrary_initial_state(),
        ) {
            let (clean, clean_roots) = committed(&record, &initial);
            let (recorded, applier) = recording_applier();
            applier.apply(&clean_roots, &record).unwrap();
            let converged = snapshot(clean.path());

            for prefix in 0..recorded.points().len() {
                let (interrupted, interrupted_roots) = committed(&record, &initial);
                let outcome = RedoApplier::new(Arc::new(InterruptAt::check_number(prefix)))
                    .apply(&interrupted_roots, &record);
                prop_assert!(
                    matches!(outcome, Err(RedoApplyError::Interrupted { .. })),
                    "the pass ran past the interruption at check {prefix}"
                );

                uninterrupted().apply(&interrupted_roots, &record).unwrap();

                prop_assert_eq!(
                    snapshot(interrupted.path()),
                    converged.clone(),
                    "re-application after check {} left a different repository",
                    prefix
                );
            }
        }
    }
}
