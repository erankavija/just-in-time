//! Applying a redo record to the repository's filesystem roots.
//!
//! One forward pass over the record's operations through one directory-handle
//! map per root, opened before the first live write, then one fsync per changed
//! directory, then the record is removed. Re-application is the same pass: a
//! made directory tolerates a directory already there, a put overwrites, a
//! delete tolerates absence, the append rewrites its bytes at its offset, so a
//! record applied twice converges, and it converges from whatever subset of
//! unsynced entries a crash keeps, which is why directory syncs wait for the
//! pass's end. Staged-file and event-log data syncs stay per operation, so a
//! rename that persists names durable bytes. A session's apply writes one record
//! and applies it; a session open applies the one a crash left.

use crate::repository_state::{
    CaptureError, FileMode, RepositoryLayoutError, RepositoryRootClass, VirtualPath,
};
use crate::storage::atomic_write::{
    publish_file, sync_capability_directory, write_staged_file, Durability,
};
use crate::storage::capability_fs::{
    open_child_dir_nofollow, CapabilityFsError, CapabilityRoots, DataRoot, DirectoryHandles,
};
use crate::storage::redo_record::{
    EventAppend, PutFile, RedoRecord, RECORD_FILE_NAME, RECORD_SCRATCH_DIR,
};
use cap_primitives::fs::FollowSymlinks;
use cap_std::fs::{Dir, OpenOptions};
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

/// Write `record` under the scratch directory of `root`, returning once it is
/// durable: the bytes are synced and every directory entry naming them is
/// fsynced. This is the mutation's commit point — the first live write follows
/// it, because a redo-only protocol has no way back once a live file is
/// replaced.
///
/// The record is written in place rather than staged and renamed: an
/// interrupted write leaves a truncated record, which [`RedoRecord::parse`]
/// rejects; session open then refuses to proceed with `UnreadableRedoRecord`
/// rather than skipping it. An occupied record path is refused instead of
/// replaced: what occupies it is a mutation no session applied, and session
/// open decides its fate. A scratch directory this call creates carries the
/// root's own mode, since `mkdir`'s mode is masked by the umask.
///
/// # Errors
///
/// Returns an error when the record cannot be serialized, the record path is
/// occupied, or a write or sync fails.
pub fn commit_redo_record(root: &DataRoot, record: &RedoRecord) -> anyhow::Result<()> {
    let bytes = record.serialize()?;
    let dir = root.dir();
    let scratch = match dir.create_dir(RECORD_SCRATCH_DIR) {
        Ok(()) => {
            adopt_mode(dir, RECORD_SCRATCH_DIR)?;
            let scratch = open_child_dir_nofollow(dir, RECORD_SCRATCH_DIR)?;
            root.sync()?;
            scratch
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            open_child_dir_nofollow(dir, RECORD_SCRATCH_DIR)?
        }
        Err(error) => return Err(error.into()),
    };
    write_staged_file(
        &scratch,
        RECORD_FILE_NAME,
        bytes.as_slice(),
        FileMode::Regular,
    )?;
    sync_capability_directory(&scratch).map_err(Into::into)
}

#[cfg(unix)]
fn adopt_mode(root: &Dir, directory: &str) -> anyhow::Result<()> {
    use cap_std::fs::{Permissions, PermissionsExt};

    let mode = root.dir_metadata()?.permissions().mode() & 0o7777;
    root.set_permissions(directory, Permissions::from_mode(mode))
        .map_err(Into::into)
}

#[cfg(not(unix))]
fn adopt_mode(_root: &Dir, _directory: &str) -> anyhow::Result<()> {
    Ok(())
}

/// One step of a record's application, named so an interruption matrix can stop
/// the pass at it.
///
/// A point names the state before its step runs, and both backends step these
/// in this order, so an injected interruption means the same thing on either.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RedoPoint {
    /// Every path resolved, before the first live write.
    BeforeApply,
    /// Before make-directory `index` creates its path.
    BeforeMakeDirectory { index: usize },
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
    #[error("original redo worktree {path} is unavailable: {source}")]
    OriginalWorktree {
        path: std::path::PathBuf,
        #[source]
        source: anyhow::Error,
    },
    #[error("redo record addresses {path}, which the selected roots do not hold")]
    Unaddressable { path: String },
    #[error("redo application interrupted at {point:?}: {source}")]
    Interrupted {
        point: RedoPoint,
        #[source]
        source: std::io::Error,
    },
    #[error("a redo record no application consumed is already committed")]
    OccupiedRecord,
    #[error("redo record makes directory {path}, which something other than a directory occupies")]
    OccupiedDirectory { path: String },
    #[error(transparent)]
    Roots(#[from] CapabilityFsError),
    #[error(transparent)]
    Path(#[from] RepositoryLayoutError),
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// Named rather than transparent, so the failure stays in the error chain a
    /// caller classifies an exit code from.
    #[error("redo application failed: {0}")]
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
        roots.validate_record_root(record)?;
        self.apply_operations(roots, record)?;
        let injector = &*self.injector;
        check_interruption(injector, RedoPoint::BeforeRecordRemoval)?;
        roots.validate_record_root(record)?;
        remove_record(roots)?;
        check_interruption(injector, RedoPoint::AfterApply)
    }

    /// The forward pass over every operation, before the record is consumed,
    /// returning the handles it ran through once every directory it changed is
    /// synced.
    fn apply_operations<'r>(
        &self,
        roots: &'r CapabilityRoots,
        record: &RedoRecord,
    ) -> Result<PassHandles<'r>, RedoApplyError> {
        let injector = &*self.injector;
        let event_log = VirtualPath::EVENTS;
        let mut handles = PassHandles::new(roots);
        record
            .make_directories()
            .iter()
            .chain(record.puts().iter().map(|put| &put.path))
            .chain(record.deletes().iter())
            .chain(record.event_append().iter().map(|_| &event_log))
            .try_for_each(|path| handles.resolve(path))?;
        check_interruption(injector, RedoPoint::BeforeApply)?;
        record
            .make_directories()
            .iter()
            .enumerate()
            .try_for_each(|(index, path)| {
                check_interruption(injector, RedoPoint::BeforeMakeDirectory { index })?;
                roots.validate_record_root(record)?;
                make_directory(handles.of(path)?, path)
            })?;
        record
            .puts()
            .iter()
            .enumerate()
            .try_for_each(|(index, put)| {
                check_interruption(injector, RedoPoint::BeforePut { index })?;
                roots.validate_record_root(record)?;
                put_file(handles.of(&put.path)?, put)
            })?;
        record
            .deletes()
            .iter()
            .enumerate()
            .try_for_each(|(index, path)| {
                check_interruption(injector, RedoPoint::BeforeDelete { index })?;
                roots.validate_record_root(record)?;
                delete_file(handles.of(path)?, path)
            })?;
        if let Some(append) = record.event_append() {
            check_interruption(injector, RedoPoint::BeforeEventAppend)?;
            roots.validate_record_root(record)?;
            append_events(handles.of(&VirtualPath::EVENTS)?, append)?;
        }
        handles.sync_changed()?;
        Ok(handles)
    }
}

/// One pass's directory handles over each root it addresses.
struct PassHandles<'r> {
    worktree: Option<DirectoryHandles<'r>>,
    data: Option<DirectoryHandles<'r>>,
}

impl<'r> PassHandles<'r> {
    fn new(roots: &'r CapabilityRoots) -> Self {
        Self {
            worktree: roots
                .root(RepositoryRootClass::Worktree)
                .map(DirectoryHandles::new),
            data: roots
                .root(RepositoryRootClass::Data)
                .map(DirectoryHandles::new),
        }
    }

    /// The handles of the root `path` is addressed within.
    fn of(&mut self, path: &VirtualPath) -> Result<&mut DirectoryHandles<'r>, RedoApplyError> {
        match path.root_class() {
            RepositoryRootClass::Worktree => self.worktree.as_mut(),
            RepositoryRootClass::Data => self.data.as_mut(),
        }
        .ok_or_else(|| unaddressable(path))
    }

    /// Open the existing ancestors of `path`.
    ///
    /// The parent chain is walked no-follow here rather than at the write, so a
    /// symlinked ancestor — the one escape a canonical identity cannot carry — is
    /// refused while the repository is still untouched.
    fn resolve(&mut self, path: &VirtualPath) -> Result<(), RedoApplyError> {
        self.of(path)?.open_parent(path.relative().as_path())?;
        Ok(())
    }

    /// Each root's handles, mutably.
    fn each(&mut self) -> impl Iterator<Item = &mut DirectoryHandles<'r>> {
        self.worktree.iter_mut().chain(self.data.iter_mut())
    }

    fn sync_changed(&mut self) -> std::io::Result<()> {
        self.each().try_for_each(DirectoryHandles::sync_changed)
    }

    #[cfg(test)]
    fn opens(&mut self) -> usize {
        self.each().map(|handles| handles.opens()).sum()
    }

    #[cfg(test)]
    fn syncs(&mut self) -> usize {
        self.each().map(|handles| handles.syncs()).sum()
    }
}

/// Put one point to `injector`, naming it in the interruption it reports. Every
/// backend's pass reaches its points through this, so an interruption is
/// reported the same way whichever backend raised it.
pub(in crate::storage) fn check_interruption(
    injector: &dyn RedoInterruptionInjector,
    point: RedoPoint,
) -> Result<(), RedoApplyError> {
    injector
        .check(&point)
        .map_err(|source| RedoApplyError::Interrupted { point, source })
}

/// Make one directory, creating the directories it needs.
///
/// A directory already there is the re-application case and succeeds; any other
/// occupant is refused, because a record whose directory a regular file or a
/// symlink occupies can never apply (OD-7). The occupant is read no-follow, so a
/// link to a directory is that other occupant rather than the directory it names.
fn make_directory(
    handles: &mut DirectoryHandles,
    path: &VirtualPath,
) -> Result<(), RedoApplyError> {
    let (parent, leaf) = handles.open_or_create_parent(path.relative().as_path())?;
    match parent.create_dir(&leaf) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            match parent.symlink_metadata(&leaf)?.is_dir() {
                true => Ok(()),
                false => Err(occupied_directory(path)),
            }
        }
        Err(error) => Err(error.into()),
    }
}

/// Replace a put's path with its bytes, creating the directories it needs.
///
/// The publication defers its directory sync: the entries the put writes are
/// synced with the pass's other changed directories.
fn put_file(handles: &mut DirectoryHandles, put: &PutFile) -> Result<(), RedoApplyError> {
    let (parent, leaf) = handles.open_or_create_parent(put.path.relative().as_path())?;
    publish_file(
        parent,
        &leaf,
        &put.bytes,
        put.mode,
        Durability::Deferred,
        || Ok(()),
    )
}

/// Remove a deleted path, tolerating a target a previous application already
/// removed.
///
/// The parent is marked changed even then: the application that removed it
/// may never have synced it.
fn delete_file(handles: &mut DirectoryHandles, path: &VirtualPath) -> Result<(), RedoApplyError> {
    let relative = path.relative().as_path();
    let Some((parent, leaf)) = handles.open_parent(relative)? else {
        return Ok(());
    };
    match parent.remove_file(&leaf) {
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    handles.mark_parent_changed(relative);
    Ok(())
}

/// Write the appended bytes at their offset.
///
/// The log is opened for positioned writing rather than in append mode, which
/// ignores the offset and would append the bytes a second time, and it is
/// truncated to the offset plus their length, so a log a previous application
/// already extended converges. Its parent is marked changed, since an
/// application may have created the log.
fn append_events(
    handles: &mut DirectoryHandles,
    append: &EventAppend,
) -> Result<(), RedoApplyError> {
    let log = VirtualPath::EVENTS;
    let relative = log.relative().as_path();
    let (parent, leaf) = handles
        .open_parent(relative)?
        .ok_or_else(|| unaddressable(&log))?;
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let mut file = parent.open_with(&leaf, &options)?;
    file.seek(SeekFrom::Start(append.offset))?;
    file.write_all(&append.bytes)?;
    file.set_len(append.offset + append.bytes.len() as u64)?;
    file.sync_all()?;
    handles.mark_parent_changed(relative);
    Ok(())
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

/// The refusal a non-directory occupant of a made directory raises, shared by
/// both backends so a caller driving either reads one outcome.
pub(in crate::storage) fn occupied_directory(path: &VirtualPath) -> RedoApplyError {
    RedoApplyError::OccupiedDirectory {
        path: format!("{path:?}"),
    }
}

/// The record vocabulary every backend's application is tested against, and the
/// filesystem backend's own repository fixtures.
///
/// One home for them, so a backend's suite states what that backend adds rather
/// than restating the record shape and the interruption seam
/// (`@/inv/shared-test-contracts`).
#[cfg(test)]
pub(crate) mod test_fixtures {
    use super::*;
    use crate::storage::discover_repository_layout;
    use crate::storage::redo_record::PutFile;
    use proptest::prelude::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// The event-log bytes a mutation captured before appending to them.
    pub(crate) const CAPTURED_LOG: &[u8] = b"{\"tag\":\"issue_created\"}\n";

    /// The data root's name below the worktree [`repository`] builds.
    const DATA_ROOT_NAME: &str = ".jit";

    /// A repository whose worktree holds its data root, with capability handles
    /// for both.
    pub(crate) fn repository() -> (TempDir, CapabilityRoots) {
        let worktree = TempDir::new().unwrap();
        std::fs::create_dir(worktree.path().join(DATA_ROOT_NAME)).unwrap();
        let layout =
            discover_repository_layout(worktree.path(), worktree.path().join(DATA_ROOT_NAME))
                .unwrap();
        let roots = CapabilityRoots::open(&layout).unwrap().0;
        (worktree, roots)
    }

    pub(crate) fn data_root(worktree: &TempDir) -> PathBuf {
        worktree.path().join(DATA_ROOT_NAME)
    }

    /// The committed record's path below the worktree, so a reading of what a
    /// record published can leave the record itself out.
    pub(crate) fn record_relative_path() -> PathBuf {
        record_scratch_relative_path().join(RECORD_FILE_NAME)
    }

    /// The scratch directory the record lives in. It outlives the record it
    /// held, so a reading of what a record published leaves it out alongside the
    /// record itself.
    pub(crate) fn record_scratch_relative_path() -> PathBuf {
        Path::new(DATA_ROOT_NAME).join(RECORD_SCRATCH_DIR)
    }

    /// Hold every path in `occupied` as a captured file and an event log the
    /// record's append offset refers to. `residue` extends that log past the
    /// offset, standing for a log an interrupted application already wrote into.
    pub(crate) fn seed_repository(worktree: &TempDir, occupied: &[VirtualPath], residue: bool) {
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

    /// What one path holds: a file's bytes with its executable bit, or a
    /// directory. A directory is an entry of its own, so an empty one a record
    /// made is observable in a convergence comparison.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum SnapshotEntry {
        File { bytes: Vec<u8>, executable: bool },
        Directory,
    }

    /// Every entry under `root`, keyed by its root-relative path, in the detail
    /// that decides whether two repositories converged.
    pub(crate) fn snapshot(root: &Path) -> BTreeMap<PathBuf, SnapshotEntry> {
        std::fs::read_dir(root)
            .unwrap()
            .flat_map(|entry| {
                let entry = entry.unwrap();
                let path = entry.path();
                let name = PathBuf::from(entry.file_name());
                match entry.file_type().unwrap().is_dir() {
                    true => std::iter::once((name.clone(), SnapshotEntry::Directory))
                        .chain(
                            snapshot(&path)
                                .into_iter()
                                .map(move |(nested, held)| (name.join(nested), held)),
                        )
                        .collect::<BTreeMap<_, _>>(),
                    false => BTreeMap::from([(
                        name,
                        SnapshotEntry::File {
                            bytes: std::fs::read(&path).unwrap(),
                            executable: is_executable(&path),
                        },
                    )]),
                }
            })
            .collect()
    }

    #[cfg(unix)]
    pub(crate) fn is_executable(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;

        std::fs::metadata(path).unwrap().permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    pub(crate) fn is_executable(_path: &Path) -> bool {
        false
    }

    /// Records every point the pass checks, in order.
    #[derive(Default)]
    pub(crate) struct RecordedPoints(Mutex<Vec<RedoPoint>>);

    impl RecordedPoints {
        pub(crate) fn points(&self) -> Vec<RedoPoint> {
            self.0.lock().unwrap().clone()
        }
    }

    impl RedoInterruptionInjector for RecordedPoints {
        fn check(&self, point: &RedoPoint) -> std::io::Result<()> {
            self.0.lock().unwrap().push(point.clone());
            Ok(())
        }
    }

    /// Interrupts the pass at one named point, so exactly the steps before it
    /// ran.
    ///
    /// Keyed on the point rather than on a check count, and therefore stateless:
    /// one injector interrupts every backend it is handed to at the same step of
    /// the record's application.
    pub(crate) struct InterruptAt(RedoPoint);

    impl InterruptAt {
        pub(crate) fn point(point: RedoPoint) -> Self {
            Self(point)
        }
    }

    impl RedoInterruptionInjector for InterruptAt {
        fn check(&self, point: &RedoPoint) -> std::io::Result<()> {
            match *point == self.0 {
                true => Err(std::io::Error::other(format!("interrupted at {point:?}"))),
                false => Ok(()),
            }
        }
    }

    /// Paths a generated record's operations and its pre-existing state draw
    /// from: two roots, nested and unnested, so a put or delete can collide
    /// with a pre-existing occupant at the same target. No entry is an
    /// ancestor of another, so [`arbitrary_record`] can assign each one a
    /// role independently and stay disjoint by construction.
    fn path_vocabulary_paths() -> [VirtualPath; 4] {
        [
            VirtualPath::data("index.json").unwrap(),
            VirtualPath::data("issues/2026/a.json").unwrap(),
            VirtualPath::worktree("notes.md").unwrap(),
            VirtualPath::worktree("dev/plan.md").unwrap(),
        ]
    }

    /// A single path drawn from [`path_vocabulary_paths`], for pre-existing
    /// state that may legitimately collide with a generated record's targets.
    pub(crate) fn path_vocabulary() -> impl Strategy<Value = VirtualPath> {
        let [a, b, c, d] = path_vocabulary_paths();
        prop_oneof![Just(a), Just(b), Just(c), Just(d)]
    }

    /// Paths a generated record's make-directory operations draw from, disjoint
    /// from each other and from [`path_vocabulary_paths`]. They sit outside that
    /// vocabulary because it seeds regular-file occupants, and a non-directory
    /// occupant of a made directory is a refusal rather than a convergence case.
    pub(crate) fn directory_vocabulary_paths() -> [VirtualPath; 2] {
        [
            VirtualPath::data("templates").unwrap(),
            VirtualPath::worktree("dev/archive").unwrap(),
        ]
    }

    /// What a generated record does with one vocabulary path.
    ///
    /// Every path draws its role independently of every other path's, so a
    /// path is never assigned to both a put and a delete: the record
    /// `arbitrary_record` builds is disjoint by construction, with nothing to
    /// filter or reject.
    #[derive(Debug, Clone, Copy)]
    enum GeneratedRole {
        Unused,
        Put,
        Delete,
    }

    fn generated_role() -> impl Strategy<Value = GeneratedRole> {
        prop_oneof![
            Just(GeneratedRole::Unused),
            Just(GeneratedRole::Put),
            Just(GeneratedRole::Delete),
        ]
    }

    pub(crate) fn arbitrary_record() -> impl Strategy<Value = RedoRecord> {
        (
            prop::collection::vec(
                (
                    generated_role(),
                    prop::collection::vec(any::<u8>(), 0..8),
                    prop_oneof![Just(FileMode::Regular), Just(FileMode::Executable)],
                ),
                path_vocabulary_paths().len(),
            ),
            prop::collection::vec(any::<bool>(), directory_vocabulary_paths().len()),
            prop::option::of(prop::collection::vec(any::<u8>(), 0..8)),
        )
            .prop_map(|(roles, made, appended)| {
                let make_directories = directory_vocabulary_paths()
                    .into_iter()
                    .zip(made)
                    .filter_map(|(path, made)| made.then_some(path))
                    .collect();
                let (puts, deletes) = path_vocabulary_paths().into_iter().zip(roles).fold(
                    (Vec::new(), Vec::new()),
                    |(mut puts, mut deletes), (path, (role, bytes, mode))| {
                        match role {
                            GeneratedRole::Unused => {}
                            GeneratedRole::Put => puts.push(PutFile { path, bytes, mode }),
                            GeneratedRole::Delete => deletes.push(path),
                        }
                        (puts, deletes)
                    },
                );
                RedoRecord::for_layout(
                    &crate::storage::InMemoryStorage::new().repository_layout(),
                    make_directories,
                    puts,
                    deletes,
                    appended.map(|bytes| EventAppend {
                        offset: CAPTURED_LOG.len() as u64,
                        bytes,
                    }),
                )
                .expect("one role per vocabulary path cannot generate an overlap")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::*;
    use super::*;
    use crate::repository_state::FileMode;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn put(path: VirtualPath, bytes: &[u8]) -> PutFile {
        PutFile {
            path,
            bytes: bytes.to_vec(),
            mode: FileMode::Regular,
        }
    }

    fn sample_record() -> RedoRecord {
        RedoRecord::for_layout(
            &crate::storage::InMemoryStorage::new().repository_layout(),
            vec![VirtualPath::data("templates").unwrap()],
            vec![put(VirtualPath::data("records/a.json").unwrap(), b"a")],
            vec![VirtualPath::worktree("dev/note.md").unwrap()],
            None,
        )
        .unwrap()
    }

    fn held(path: &Path) -> DataRoot {
        DataRoot::open(path).unwrap()
    }

    fn committed_path(data_root: &Path) -> PathBuf {
        data_root.join(RECORD_SCRATCH_DIR).join(RECORD_FILE_NAME)
    }

    #[test]
    fn test_commit_redo_record_writes_a_record_a_later_session_parses() {
        let data_root = tempfile::tempdir().unwrap();
        let record = sample_record();

        commit_redo_record(&held(data_root.path()), &record).unwrap();

        assert_eq!(
            RedoRecord::parse(&std::fs::read(committed_path(data_root.path())).unwrap()).unwrap(),
            record
        );
    }

    #[test]
    fn test_commit_redo_record_lands_in_the_held_root_when_its_path_is_rebound() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("data");
        std::fs::create_dir(&path).unwrap();
        let root = held(&path);
        let moved = parent.path().join("moved");
        std::fs::rename(&path, &moved).unwrap();
        std::fs::create_dir(&path).unwrap();

        commit_redo_record(&root, &sample_record()).unwrap();

        assert!(
            committed_path(&moved).exists(),
            "the record missed the root the handle holds"
        );
        assert!(
            !committed_path(&path).exists(),
            "the record landed in the directory that replaced the held root"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_commit_redo_record_creates_an_absent_scratch_directory_with_the_data_root_mode() {
        use std::os::unix::fs::PermissionsExt;

        let mode =
            |directory: &Path| std::fs::metadata(directory).unwrap().permissions().mode() & 0o7777;
        let parent = tempfile::tempdir().unwrap();
        // A directory the ambient umask alone shaped, so the mode the data root
        // is given differs from it whatever that umask is: a scratch directory
        // left to the umask carries this control's mode, not the data root's.
        let control = parent.path().join("control");
        std::fs::create_dir(&control).unwrap();
        let data_mode = if mode(&control) == 0o700 {
            0o750
        } else {
            0o700
        };
        let data_root = parent.path().join("data");
        std::fs::create_dir(&data_root).unwrap();
        std::fs::set_permissions(&data_root, std::fs::Permissions::from_mode(data_mode)).unwrap();

        commit_redo_record(&held(&data_root), &sample_record()).unwrap();

        assert_eq!(
            mode(&data_root.join(RECORD_SCRATCH_DIR)),
            mode(&data_root),
            "the scratch directory does not carry the data root's mode"
        );
    }

    /// The record write follows the root's fsync: a root whose fsync fails
    /// receives no record.
    #[test]
    fn test_commit_redo_record_writes_nothing_when_the_root_fsync_fails() {
        let data_root = tempfile::tempdir().unwrap();
        let root = held(data_root.path()).refusing_sync();

        let outcome = commit_redo_record(&root, &sample_record());

        assert!(outcome.is_err(), "the unsyncable data root went unreported");
        assert!(
            !committed_path(data_root.path()).exists(),
            "the record was written under a scratch directory the data root never made durable"
        );
    }

    #[test]
    fn test_commit_redo_record_writes_into_a_scratch_directory_that_already_exists() {
        let data_root = tempfile::tempdir().unwrap();
        std::fs::create_dir(data_root.path().join(RECORD_SCRATCH_DIR)).unwrap();

        commit_redo_record(&held(data_root.path()), &sample_record()).unwrap();

        assert!(committed_path(data_root.path()).exists());
    }

    #[test]
    fn test_commit_redo_record_leaves_a_record_a_previous_mutation_left_behind() {
        let data_root = tempfile::tempdir().unwrap();
        let root = held(data_root.path());
        commit_redo_record(&root, &sample_record()).unwrap();
        let leftover = std::fs::read(committed_path(data_root.path())).unwrap();

        assert!(commit_redo_record(&root, &RedoRecord::default()).is_err());

        assert_eq!(
            std::fs::read(committed_path(data_root.path())).unwrap(),
            leftover,
            "the commit overwrote a record no session had applied"
        );
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

    #[test]
    fn test_redo_applier_apply_publishes_every_operation_the_record_names() {
        let (worktree, roots) = repository();
        std::fs::write(data_root(&worktree).join("index.json"), b"captured").unwrap();
        std::fs::write(worktree.path().join("stale.md"), b"stale").unwrap();
        std::fs::write(data_root(&worktree).join("events.jsonl"), CAPTURED_LOG).unwrap();
        let record = RedoRecord::for_layout(
            &roots.record_layout(),
            vec![VirtualPath::data("templates").unwrap()],
            vec![put(VirtualPath::data("index.json").unwrap(), b"published")],
            vec![VirtualPath::worktree("stale.md").unwrap()],
            Some(EventAppend {
                offset: CAPTURED_LOG.len() as u64,
                bytes: b"{\"tag\":\"issue_claimed\"}\n".to_vec(),
            }),
        )
        .unwrap();

        uninterrupted().apply(&roots, &record).unwrap();

        assert!(
            data_root(&worktree).join("templates").is_dir(),
            "the directory the record makes is not there"
        );
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
        let record = RedoRecord::for_layout(
            &roots.record_layout(),
            Vec::new(),
            vec![put(VirtualPath::data("index.json").unwrap(), b"published")],
            Vec::new(),
            None,
        )
        .unwrap();
        commit_redo_record(&held(&data_root(&worktree)), &record).unwrap();
        let committed = committed_path(&data_root(&worktree));

        uninterrupted().apply(&roots, &record).unwrap();

        assert!(
            !committed.exists(),
            "the applied record is still there for a later session to apply again"
        );
    }

    #[test]
    fn test_redo_applier_apply_creates_the_directories_a_put_needs() {
        let (worktree, roots) = repository();
        let record = RedoRecord::for_layout(
            &roots.record_layout(),
            Vec::new(),
            vec![put(
                VirtualPath::data("issues/2026/a.json").unwrap(),
                b"published",
            )],
            Vec::new(),
            None,
        )
        .unwrap();

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
        let record = RedoRecord::for_layout(
            &roots.record_layout(),
            Vec::new(),
            vec![put(nested, b"published")],
            Vec::new(),
            None,
        )
        .unwrap();
        commit_redo_record(&held(&data_root(&worktree)), &record).unwrap();
        let committed = committed_path(&data_root(&worktree));
        let published = data_root(&worktree).join("issues/2026/09/a.json");

        let outcome =
            RedoApplier::new(Arc::new(InterruptAt::point(RedoPoint::BeforeRecordRemoval)))
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
        let record = RedoRecord::for_layout(
            &roots.record_layout(),
            Vec::new(),
            vec![PutFile {
                path: VirtualPath::worktree("scripts/publish.sh").unwrap(),
                bytes: b"#!/bin/sh\n".to_vec(),
                mode: FileMode::Executable,
            }],
            Vec::new(),
            None,
        )
        .unwrap();

        uninterrupted().apply(&roots, &record).unwrap();

        assert!(
            is_executable(&worktree.path().join("scripts/publish.sh")),
            "the published file does not carry the mode the record named"
        );
    }

    /// A link to a directory is the one occupant whose target lies outside the
    /// roots the record resolved. It is not the directory the record makes, so
    /// it is refused rather than adopted.
    #[cfg(unix)]
    #[test]
    fn test_make_directory_refuses_a_symlink_to_a_directory_at_its_target() {
        let (worktree, roots) = repository();
        let outside = TempDir::new().unwrap();
        let target = VirtualPath::data("templates").unwrap();
        std::os::unix::fs::symlink(outside.path(), data_root(&worktree).join("templates")).unwrap();

        let mut handles = PassHandles::new(&roots);
        let outcome = make_directory(handles.of(&target).unwrap(), &target);

        assert!(
            matches!(outcome, Err(RedoApplyError::OccupiedDirectory { .. })),
            "the link was taken for the directory the record makes: {outcome:?}"
        );
    }

    /// Targets sharing ancestors are reached through one handle per ancestor,
    /// whichever operations and the pre-pass walk through it.
    #[test]
    fn test_redo_applier_apply_opens_each_shared_ancestor_directory_once() {
        let (worktree, roots) = repository();
        std::fs::create_dir_all(data_root(&worktree).join("issues/2026")).unwrap();
        std::fs::write(data_root(&worktree).join("issues/stale.json"), b"stale").unwrap();
        std::fs::write(data_root(&worktree).join("events.jsonl"), CAPTURED_LOG).unwrap();
        let record = RedoRecord::new(
            vec![VirtualPath::data("issues/2026/09").unwrap()],
            vec![
                put(VirtualPath::data("issues/2026/a.json").unwrap(), b"a"),
                put(VirtualPath::data("issues/2026/b.json").unwrap(), b"b"),
                put(VirtualPath::data("issues/c.json").unwrap(), b"c"),
            ],
            vec![VirtualPath::data("issues/stale.json").unwrap()],
            Some(EventAppend {
                offset: CAPTURED_LOG.len() as u64,
                bytes: b"{\"tag\":\"issue_claimed\"}\n".to_vec(),
            }),
        )
        .unwrap();
        let ancestors = ["issues", "issues/2026"].len();

        let mut handles = uninterrupted().apply_operations(&roots, &record).unwrap();

        assert_eq!(
            handles.opens(),
            ancestors,
            "an ancestor several targets share was opened more than once"
        );
    }

    /// Puts sharing a parent cost that parent one fsync, after the last of them.
    #[test]
    fn test_redo_applier_apply_syncs_a_parent_its_puts_share_once() {
        let (_worktree, roots) = repository();
        let record = RedoRecord::new(
            Vec::new(),
            vec![
                put(VirtualPath::data("index.json").unwrap(), b"index"),
                put(VirtualPath::data("config.toml").unwrap(), b"config"),
                put(VirtualPath::data("gates.toml").unwrap(), b"gates"),
            ],
            Vec::new(),
            None,
        )
        .unwrap();

        let mut handles = uninterrupted().apply_operations(&roots, &record).unwrap();

        assert_eq!(
            handles.syncs(),
            1,
            "the data root was synced once per put rather than once per application"
        );
    }

    /// A put replaces whatever occupies its path. A symlink there is the one
    /// occupant that could carry the bytes, or a mode, somewhere the roots do
    /// not reach: the staged write plus rename replaces the link itself.
    #[cfg(unix)]
    #[test]
    fn test_redo_applier_apply_replaces_a_symlink_at_its_target_without_following_it() {
        use crate::storage::redo_apply::test_fixtures::is_executable;

        let (worktree, roots) = repository();
        let outside = TempDir::new().unwrap();
        let outside_target = outside.path().join("outside.txt");
        std::fs::write(&outside_target, b"outside").unwrap();
        let target = VirtualPath::data("index.json").unwrap();
        std::os::unix::fs::symlink(&outside_target, data_root(&worktree).join("index.json"))
            .unwrap();
        let record = RedoRecord::for_layout(
            &roots.record_layout(),
            Vec::new(),
            vec![PutFile {
                path: target.clone(),
                bytes: b"published".to_vec(),
                mode: FileMode::Executable,
            }],
            Vec::new(),
            None,
        )
        .unwrap();

        uninterrupted().apply(&roots, &record).unwrap();

        let published = data_root(&worktree).join("index.json");
        assert!(
            !published.symlink_metadata().unwrap().is_symlink(),
            "the link is still at the target the put replaced"
        );
        assert_eq!(std::fs::read(&published).unwrap(), b"published");
        assert!(is_executable(&published));
        assert_eq!(
            std::fs::read(&outside_target).unwrap(),
            b"outside",
            "the put reached a file outside the repository roots"
        );
        assert!(
            !is_executable(&outside_target),
            "the put's mode reached a file outside the repository roots"
        );
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
        let record = RedoRecord::for_layout(
            &roots.record_layout(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(EventAppend {
                offset: CAPTURED_LOG.len() as u64,
                bytes: appended.to_vec(),
            }),
        )
        .unwrap();

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
        let published = RedoRecord::for_layout(
            &roots.record_layout(),
            Vec::new(),
            vec![put(VirtualPath::data("index.json").unwrap(), b"published")],
            Vec::new(),
            None,
        )
        .unwrap();
        let removed = RedoRecord::for_layout(
            &roots.record_layout(),
            Vec::new(),
            Vec::new(),
            vec![VirtualPath::data("index.json").unwrap()],
            None,
        )
        .unwrap();

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
        let record = RedoRecord::for_layout(
            &roots.record_layout(),
            vec![VirtualPath::data("templates").unwrap()],
            vec![
                put(VirtualPath::data("index.json").unwrap(), b"published"),
                put(VirtualPath::worktree("notes.md").unwrap(), b"published"),
            ],
            vec![VirtualPath::worktree("stale.md").unwrap()],
            Some(EventAppend {
                offset: CAPTURED_LOG.len() as u64,
                bytes: b"{\"tag\":\"issue_claimed\"}\n".to_vec(),
            }),
        )
        .unwrap();
        let (recorded, applier) = recording_applier();

        applier.apply(&roots, &record).unwrap();

        assert_eq!(
            recorded.points(),
            vec![
                RedoPoint::BeforeApply,
                RedoPoint::BeforeMakeDirectory { index: 0 },
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
        let record = RedoRecord::for_layout(
            &roots.record_layout(),
            Vec::new(),
            vec![
                put(VirtualPath::data("inside.json").unwrap(), b"published"),
                put(
                    VirtualPath::data("escape/captured.json").unwrap(),
                    b"published",
                ),
            ],
            Vec::new(),
            None,
        )
        .unwrap();

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
}
