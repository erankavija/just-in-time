//! In-memory storage implementation for testing.
//!
//! This backend stores all data in RAM using HashMaps, providing 10-100x faster
//! test execution compared to JSON file I/O. Thread-safe for concurrent access.

use crate::declarations::{parse_gate_registry, GateRegistry};
use crate::domain::event_log::parse_known_events;
use crate::domain::{Event, GateRunResult, Issue};
use crate::repository_state::{
    gate_run_result_relative_path, CapturedBytes, EntryIdentity, FileMode, RepositoryEntry,
    RepositoryRootClass, RootRelativePath, VirtualPath,
};
use crate::storage::redo_apply::{check_interruption, occupied_directory};
use crate::storage::{
    EventAppend, GateRunNotFoundError, IssueNotFoundError, IssueStore, PresetNotFoundError,
    PutFile, RedoApplyError, RedoInterruptionInjector, RedoPoint, RedoRecord, RepoWriteGuard,
    RepoWriteLock, RepositoryStateStoreError,
};
use anyhow::{anyhow, Context, Result};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// In-memory storage backend using HashMaps.
///
/// All data is stored in memory and lost when the instance is dropped.
/// Uses `Arc<Mutex<>>` for thread-safe shared interior mutability.
///
/// Every repository-owned typed record — issues, the gate registry, the audit
/// log, and gate-run results — is held solely as its canonical bytes in the
/// aggregate [`MemoryRepositoryState`] image, the single store a
/// mutation session captures and applies. There is no parallel typed cache: a
/// record read through [`IssueStore`] and one published by a session share one
/// source of truth and round-trip through the identical canonical serializers.
#[derive(Clone)]
pub struct InMemoryStorage {
    /// Unique root path for parallel test isolation.
    root_path: std::path::PathBuf,
    /// Removes a root [`new`](Self::new) minted once the last clone drops, so
    /// host files a test mirrors under it do not outlive the store.
    _minted_root: Option<Arc<MintedRoot>>,
    /// Outermost lock of every mutating path, shared by every clone. Process-local:
    /// this backend has no files, so there is no other process to exclude.
    repo_lock: Arc<RepoWriteLock>,
    /// Canonical aggregate repository image used by mutation sessions.
    pub(crate) repository_state: Arc<Mutex<MemoryRepositoryState>>,
    /// Interruption seam for the session's own moments: its open, its
    /// pre-commit check, and the steps of its redo-record application.
    pub(crate) mutation_interruptions: Arc<dyn crate::storage::MutationInterruptionInjector>,
    /// Canonical layout of the live mutation session, shared by every clone so
    /// reentry is admitted only for the same selected roots.
    pub(crate) active_mutation_layout:
        Arc<crate::storage::repository_state_store::ActiveLayoutTracker>,
    #[cfg(feature = "test-support")]
    repository_state_apply_conflicts: Arc<std::sync::atomic::AtomicUsize>,
}

struct MintedRoot(std::path::PathBuf);

impl Drop for MintedRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MemoryRepositoryState {
    pub(crate) entries:
        BTreeMap<crate::repository_state::VirtualPath, crate::repository_state::RepositoryEntry>,
    pub(crate) data_root_exists: bool,
    /// The redo record a committed mutation left beside the entries, and the
    /// whole of what its application replays: the backend holds no snapshot of
    /// the state the record moves away from, because the record is applied
    /// forward.
    pub(crate) record: Option<RedoRecord>,
}

impl InMemoryStorage {
    /// Create a new in-memory storage instance.
    pub fn new() -> Self {
        // Generate unique root path for parallel test isolation
        let unique_id = uuid::Uuid::new_v4();
        let root = std::path::PathBuf::from(format!("/tmp/jit-test-{unique_id}"));
        Self {
            _minted_root: Some(Arc::new(MintedRoot(root.clone()))),
            ..Self::rooted_at(root)
        }
    }

    /// An in-memory store whose synthetic layout is rooted at `root_path`.
    ///
    /// The aggregate stays in memory; the root only names the layout. A
    /// worktree-relative package location needs that name to reach a real
    /// directory, because a package is read from the location it names.
    /// [`new`](Self::new) mints a root that does not exist, which is what every
    /// other case wants.
    pub fn rooted_at(root_path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            root_path: root_path.into(),
            _minted_root: None,
            repo_lock: RepoWriteLock::in_process(),
            repository_state: Arc::new(Mutex::new(MemoryRepositoryState::default())),
            mutation_interruptions: Arc::new(crate::storage::NoMutationInterruptions),
            active_mutation_layout: Arc::new(Default::default()),
            #[cfg(feature = "test-support")]
            repository_state_apply_conflicts: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Canonical synthetic layout for command executors backed by this in-memory
    /// store. The paths are identities only—the backend performs no filesystem
    /// I/O through them—but each instance receives a distinct nested layout so it
    /// exercises the same typed session boundary as file-backed storage.
    pub fn repository_layout(&self) -> crate::repository_state::RepositoryLayout {
        let data = self.root_path.join(".jit");
        crate::repository_state::RepositoryLayout::new(
            crate::repository_state::RepositoryRootEvidence::new(
                &self.root_path,
                format!("memory-worktree:{}", self.root_path.display()),
                true,
            ),
            crate::repository_state::RepositoryRootEvidence::new(
                &data,
                format!("memory-data:{}", data.display()),
                true,
            ),
        )
        .expect("an in-memory store always has a valid nested synthetic layout")
    }

    /// An in-memory store whose mutation sessions are interrupted at the
    /// moments `interruptions` selects.
    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
    pub(crate) fn with_mutation_interruptions(
        interruptions: Arc<dyn crate::storage::MutationInterruptionInjector>,
    ) -> Self {
        let mut storage = Self::new();
        storage.mutation_interruptions = interruptions;
        storage
    }

    /// A view sharing this backend's aggregate state, lock, and layout tracker but
    /// with a clean injector. Mirrors opening a fresh `JsonFileStorage`
    /// over the same on-disk root for recovery: same state, no injected faults.
    #[cfg(test)]
    pub(crate) fn without_mutation_interruptions(&self) -> Self {
        Self {
            mutation_interruptions: Arc::new(crate::storage::NoMutationInterruptions),
            ..self.clone()
        }
    }

    /// A view sharing this backend's state whose sessions are interrupted at
    /// the moments `interruptions` selects.
    ///
    /// Reachable under `feature = "test-support"` in addition to `cfg(test)`:
    /// `commands::test_helpers::with_open_race` calls it, and `test_helpers`
    /// itself is reachable independent of `cfg(test)` when the feature is
    /// enabled without a test build (e.g. `cargo clippy --features
    /// test-support`). `with_mutation_interruptions` and
    /// `without_mutation_interruptions` stay `cfg(test)`-only: every other
    /// caller lives inside `#[cfg(test)]` test modules, which always compile
    /// with `test-support` active via the crate's own dev-dependency. Unused
    /// in the `feature`-on/`cfg(test)`-off configuration itself, since its
    /// only non-`cfg(test)` caller (`with_open_race`) is unreached there too.
    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
    pub(crate) fn with_mutation_interruption_view(
        &self,
        interruptions: Arc<dyn crate::storage::MutationInterruptionInjector>,
    ) -> Self {
        Self {
            mutation_interruptions: interruptions,
            ..self.clone()
        }
    }

    /// Setter half of the conflict-injection pair below `consume_...`, called
    /// only from `#[cfg(test)]` test modules (e.g. `bulk_update`'s), so it is
    /// unused in a `feature`-on/`cfg(test)`-off build such as `jit` linked
    /// into `crates/server`'s test binary.
    #[cfg(feature = "test-support")]
    #[allow(dead_code)]
    pub(crate) fn inject_repository_state_apply_conflicts(&self, count: usize) {
        self.repository_state_apply_conflicts
            .store(count, std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn consume_repository_state_apply_conflict(&self) -> bool {
        self.repository_state_apply_conflicts
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |remaining| remaining.checked_sub(1),
            )
            .is_ok()
    }

    pub(crate) fn repository_state(&self) -> std::sync::MutexGuard<'_, MemoryRepositoryState> {
        self.repository_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn mutation_interruptions(
        &self,
    ) -> Arc<dyn crate::storage::MutationInterruptionInjector> {
        Arc::clone(&self.mutation_interruptions)
    }

    pub(crate) fn active_mutation_layout(
        &self,
    ) -> Arc<crate::storage::repository_state_store::ActiveLayoutTracker> {
        Arc::clone(&self.active_mutation_layout)
    }

    fn gate_presets_from_repository_state(
        &self,
    ) -> Result<std::collections::HashMap<String, crate::gate_presets::GatePresetDefinition>> {
        let presets_prefix = format!(
            "{}/",
            crate::repository_state::VirtualPath::GATE_PRESETS
                .relative()
                .as_str()
        );
        let files = self
            .repository_state()
            .entries
            .iter()
            .filter_map(|(path, entry)| {
                if path.root_class() != RepositoryRootClass::Data {
                    return None;
                }
                let filename = path
                    .relative()
                    .as_str()
                    .strip_prefix(presets_prefix.as_str())?;
                if filename.contains('/') || !filename.ends_with(".json") {
                    return None;
                }
                Some(match entry {
                    RepositoryEntry::File { content, .. } => {
                        Ok((filename.to_string(), content.bytes().to_vec()))
                    }
                    _ => Err(crate::errors::InvalidArgumentError::new(format!(
                        "Custom preset file '{filename}' must be an ordinary file"
                    ))
                    .into()),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        crate::gate_presets::load_presets_from_custom_files(files)
    }

    /// Classify an `IssueStore` repository-relative input through the canonical
    /// layout authority.
    fn repo_file_vpath(
        &self,
        rel_path: &str,
    ) -> Result<crate::repository_state::VirtualPath, crate::storage::PathReadError> {
        crate::storage::validate_repo_relative_path(rel_path)?;
        self.repository_layout()
            .classify_repository_relative(rel_path)
            .map_err(|error| crate::storage::PathReadError::InvalidPath(error.to_string()))
    }

    /// Mark the aggregate repository image as an EXISTING data root.
    ///
    /// Backend API semantics, NOT a test convenience: seeding any repository-owned
    /// record or file establishes that the data root exists, so a mutation session
    /// opened on a seeded memory store models EXISTING-root publication — matching a
    /// [`JsonFileStorage`](crate::storage::JsonFileStorage) over an initialized
    /// `.jit/`, which gets the same semantics from the filesystem. An UNSEEDED store
    /// leaves `data_root_exists` false and models absent-root / fresh-init
    /// publication, so a session on it exercises the fresh-init path.
    fn mark_data_root_existing(state: &mut MemoryRepositoryState) {
        use crate::repository_state::{EntryIdentity, FileMode, RepositoryEntry, VirtualPath};
        state.data_root_exists = true;
        let root = VirtualPath::data("").expect("data root path is canonical");
        state
            .entries
            .entry(root)
            .or_insert_with(|| RepositoryEntry::Directory {
                identity: EntryIdentity::for_bytes("memory-directory:data-root", b"directory")
                    .expect("directory identity is hashable"),
                mode: FileMode::Executable,
            });
    }

    /// Store `content` as the captured `File` entry for `rel_path` in the aggregate
    /// repository image (the single store a mutation session captures and applies).
    fn insert_repo_file(
        &self,
        vpath: VirtualPath,
        content: &str,
    ) -> Result<(), crate::storage::PathReadError> {
        use crate::repository_state::{FileMode, RepositoryEntry};
        self.repository_layout()
            .ensure_canonical(&vpath)
            .map_err(|error| crate::storage::PathReadError::InvalidPath(error.to_string()))?;
        let content = CapturedBytes::new(
            format!(
                "memory:{:?}:{}",
                vpath.root_class(),
                vpath.relative().as_str()
            ),
            content.as_bytes().to_vec(),
        )
        .map_err(|e| crate::storage::PathReadError::Other(anyhow!("{e}")))?;
        let mut state = self.repository_state();
        if vpath.root_class() == RepositoryRootClass::Data {
            Self::mark_data_root_existing(&mut state);
        }
        Self::ensure_ancestor_dirs(&mut state, &vpath);
        state.entries.insert(
            vpath,
            RepositoryEntry::File {
                content,
                mode: FileMode::Regular,
            },
        );
        Ok(())
    }

    /// Seed an in-memory data-root file at `relative` with `content`.
    ///
    /// Mirrors the issue/gate seeding pattern: a test setter so command and domain
    /// tests can stage a config-declared file (e.g. a project-scope item source)
    /// and have [`IssueStore::read_repo_file`](crate::storage::IssueStore::read_repo_file)
    /// return it, without touching disk. The file lands in the same aggregate image
    /// a mutation session captures, so seeded state and session-published state
    /// share one store.
    pub fn add_data_file(&self, relative: impl AsRef<std::path::Path>, content: &str) {
        let path = VirtualPath::data(relative)
            .expect("add_data_file requires a canonical data-root-relative path");
        self.insert_repo_file(path, content)
            .expect("add_data_file requires a canonical repository path");
    }

    /// Seed an in-memory worktree file at `relative` with `content`.
    pub fn add_worktree_file(&self, relative: impl AsRef<std::path::Path>, content: &str) {
        let path = VirtualPath::worktree(relative)
            .expect("add_worktree_file requires a canonical worktree-relative path");
        self.insert_repo_file(path, content)
            .expect("add_worktree_file requires a canonical repository path");
    }

    /// Canonical `Data(...)` identity for one issue record.
    fn issue_vpath(id: &str) -> Result<VirtualPath> {
        VirtualPath::data(
            VirtualPath::ISSUES
                .relative()
                .as_path()
                .join(format!("{id}.json")),
        )
        .context("invalid issue record path")
    }

    /// Canonical `Data(...)` identity for one gate-run result.
    fn gate_run_vpath(run_id: &str) -> Result<VirtualPath> {
        Ok(VirtualPath::data(
            gate_run_result_relative_path(run_id)?.as_path(),
        )?)
    }

    fn gate_run_directory_vpath(run_id: &str) -> Result<VirtualPath> {
        let result = gate_run_result_relative_path(run_id)?;
        Ok(VirtualPath::data(result.as_path().parent().ok_or_else(
            || anyhow!("canonical gate-run result path has no parent"),
        )?)?)
    }

    fn gate_run_directory_id(vpath: &VirtualPath) -> Option<&str> {
        if vpath.root_class() != RepositoryRootClass::Data {
            return None;
        }
        let mut components = vpath.relative().as_str().split('/');
        let (Some(directory), Some(run_id), None) =
            (components.next(), components.next(), components.next())
        else {
            return None;
        };
        if directory != VirtualPath::GATE_RUNS.relative().as_str() {
            return None;
        }
        gate_run_result_relative_path(run_id)
            .is_ok_and(|_| Self::gate_run_directory_vpath(run_id).is_ok_and(|path| &path == vpath))
            .then_some(run_id)
    }

    fn gate_run_parent_is_directory(
        state: &MemoryRepositoryState,
        path: &VirtualPath,
    ) -> Result<bool> {
        match state.entries.get(path) {
            None | Some(RepositoryEntry::Absent) => Ok(false),
            Some(RepositoryEntry::Directory { .. }) => Ok(true),
            Some(_) => anyhow::bail!(
                "Gate run directory at {} must be an ordinary directory",
                path.relative().as_path().display()
            ),
        }
    }

    /// Publish a malformed fixture as a captured `File` entry.
    #[cfg(test)]
    fn put_data_entry(state: &mut MemoryRepositoryState, vpath: VirtualPath, bytes: Vec<u8>) {
        let object = vpath.relative().as_str().to_owned();
        let content = CapturedBytes::new(object, bytes).expect("entry identity is hashable");
        Self::mark_data_root_existing(state);
        Self::ensure_ancestor_dirs(state, &vpath);
        state.entries.insert(
            vpath,
            RepositoryEntry::File {
                content,
                mode: FileMode::Regular,
            },
        );
    }

    /// Model a `Directory` entry for every ancestor of `vpath` up to (but not
    /// including) the data root, so the aggregate image reflects the directory
    /// tree a real repository carries. A mutation session verifies that
    /// each written file's parent directory exists — exactly as the file backend
    /// does against the on-disk tree — so an issue/gate-run write under a nested
    /// `Data(...)` path must find its parent modeled here.
    fn ensure_ancestor_dirs(state: &mut MemoryRepositoryState, vpath: &VirtualPath) {
        let mut ancestor = vpath.relative().as_path().parent();
        while let Some(dir) = ancestor {
            if dir.as_os_str().is_empty() {
                break;
            }
            if let Ok(dir_path) = RootRelativePath::parse(dir)
                .and_then(|rel| VirtualPath::from_root(vpath.root_class(), rel))
            {
                state
                    .entries
                    .entry(dir_path)
                    .or_insert_with(|| memory_directory_entry(dir));
            }
            ancestor = dir.parent();
        }
    }

    /// Read the exact captured bytes for `vpath` from the aggregate image.
    fn data_entry_bytes(state: &MemoryRepositoryState, vpath: &VirtualPath) -> Option<Vec<u8>> {
        match state.entries.get(vpath) {
            Some(RepositoryEntry::File { content, .. }) => Some(content.bytes().to_vec()),
            _ => None,
        }
    }

    /// Enumerate the captured `Data(...)` file entries whose relative path lies
    /// directly under `dir` (one segment deep) and ends with `suffix`, returning
    /// `(leaf, bytes)` for each. Backs the issue and gate-run scans without a
    /// parallel typed index.
    fn data_files_in<'a>(
        state: &'a MemoryRepositoryState,
        dir: &'a str,
        suffix: &'a str,
    ) -> impl Iterator<Item = (String, Vec<u8>)> + 'a {
        let prefix = format!("{dir}/");
        state.entries.iter().filter_map(move |(vpath, entry)| {
            if vpath.root_class() != RepositoryRootClass::Data {
                return None;
            }
            let RepositoryEntry::File { content, .. } = entry else {
                return None;
            };
            let rel = vpath.relative().as_str();
            let leaf = rel.strip_prefix(&prefix)?;
            if leaf.contains('/') || !leaf.ends_with(suffix) {
                return None;
            }
            Some((leaf.to_owned(), content.bytes().to_vec()))
        })
    }

    /// Deserialize every captured issue record in the aggregate image.
    fn load_issues(state: &MemoryRepositoryState) -> Result<Vec<Issue>> {
        Self::data_files_in(state, VirtualPath::ISSUES.relative().as_str(), ".json")
            .map(|(_, bytes)| {
                serde_json::from_slice::<Issue>(&bytes)
                    .context("Failed to deserialize issue from repository image")
            })
            .collect()
    }

    /// Deserialize one captured issue record, if present.
    fn load_issue_entry(state: &MemoryRepositoryState, id: &str) -> Result<Option<Issue>> {
        Self::data_entry_bytes(state, &Self::issue_vpath(id)?)
            .map(|bytes| {
                serde_json::from_slice::<Issue>(&bytes)
                    .context("Failed to deserialize issue from repository image")
            })
            .transpose()
    }

    /// Deserialize the captured audit log, oldest event first.
    fn load_events(state: &MemoryRepositoryState) -> Result<Vec<Event>> {
        let Some(bytes) = Self::data_entry_bytes(state, &VirtualPath::EVENTS) else {
            return Ok(Vec::new());
        };
        parse_known_events(&String::from_utf8_lossy(&bytes))
            .context("Failed to deserialize event log from repository image")
    }
}

impl InMemoryStorage {
    /// Install `record` as the residue this backend applies forward, changing no
    /// state entry.
    ///
    /// The in-memory commit point, mirroring [`RedoRecord::commit`]: an
    /// installed record is refused rather than replaced, because what occupies
    /// the slot is a mutation no application consumed.
    ///
    /// # Errors
    ///
    /// Returns [`RedoApplyError::OccupiedRecord`] when a record is already
    /// installed.
    pub fn commit_record(&self, record: &RedoRecord) -> Result<(), RedoApplyError> {
        let mut state = self.repository_state();
        match state.record {
            Some(_) => Err(RedoApplyError::OccupiedRecord),
            None => {
                state.record = Some(record.clone());
                Ok(())
            }
        }
    }
}

/// The in-memory backend's record application, holding the injector every
/// application it runs checks its points through.
///
/// The counterpart of [`RedoApplier`](crate::storage::RedoApplier) over the
/// entry map: the same [`RedoPoint`] values in the same order, so an injected
/// interruption means the same thing on either backend.
pub struct MemoryRedoApplier {
    injector: Arc<dyn RedoInterruptionInjector>,
}

impl MemoryRedoApplier {
    pub fn new(injector: Arc<dyn RedoInterruptionInjector>) -> Self {
        Self { injector }
    }

    /// Apply `record` to `storage`'s entries and remove it, checking every
    /// [`RedoPoint`].
    ///
    /// # Errors
    ///
    /// Returns [`RedoApplyError`] when a path is unaddressable or an injected
    /// point interrupts the pass.
    pub fn apply(
        &self,
        storage: &InMemoryStorage,
        record: &RedoRecord,
    ) -> Result<(), RedoApplyError> {
        let injector = &*self.injector;
        let mut state = storage.repository_state();
        let event_log = VirtualPath::EVENTS;
        record
            .make_directories()
            .iter()
            .chain(record.puts().iter().map(|put| &put.path))
            .chain(record.deletes().iter())
            .chain(record.event_append().iter().map(|_| &event_log))
            .try_for_each(|path| resolve(&state, path))?;
        check_interruption(injector, RedoPoint::BeforeApply)?;
        record
            .make_directories()
            .iter()
            .enumerate()
            .try_for_each(|(index, path)| {
                check_interruption(injector, RedoPoint::BeforeMakeDirectory { index })?;
                make_directory(&mut state, path)
            })?;
        record
            .puts()
            .iter()
            .enumerate()
            .try_for_each(|(index, put)| {
                check_interruption(injector, RedoPoint::BeforePut { index })?;
                put_entry(&mut state, put)
            })?;
        record
            .deletes()
            .iter()
            .enumerate()
            .try_for_each(|(index, path)| {
                check_interruption(injector, RedoPoint::BeforeDelete { index })?;
                ensure_operable(&state, path)?;
                state.entries.remove(path);
                Ok::<(), RedoApplyError>(())
            })?;
        if let Some(append) = record.event_append() {
            check_interruption(injector, RedoPoint::BeforeEventAppend)?;
            append_events(&mut state, append)?;
        }
        check_interruption(injector, RedoPoint::BeforeRecordRemoval)?;
        state.record = None;
        check_interruption(injector, RedoPoint::AfterApply)
    }
}

/// Make one directory entry, with every ancestor it needs.
///
/// The counterpart of the file backend's [`make_directory`](crate::storage::redo_apply),
/// refusing the same occupants: a directory already there is the re-application
/// case, and anything else is the refusal a record that can never apply raises.
fn make_directory(
    state: &mut MemoryRepositoryState,
    path: &VirtualPath,
) -> Result<(), RedoApplyError> {
    resolve(state, path)?;
    match state.entries.get(path) {
        None | Some(RepositoryEntry::Absent) | Some(RepositoryEntry::Directory { .. }) => {
            InMemoryStorage::ensure_ancestor_dirs(state, path);
            state
                .entries
                .entry(path.clone())
                .or_insert_with(|| memory_directory_entry(path.relative().as_path()));
            Ok(())
        }
        Some(_) => Err(occupied_directory(path)),
    }
}

fn memory_directory_entry(relative: &std::path::Path) -> RepositoryEntry {
    RepositoryEntry::Directory {
        identity: EntryIdentity::for_bytes(
            format!("memory-directory:{}", relative.display()),
            b"directory",
        )
        .expect("directory identity is hashable"),
        mode: FileMode::Executable,
    }
}

/// Refuse a path this backend's tree cannot address.
///
/// The counterpart of the file backend's path resolution, which runs once over
/// the whole record before the first live write and again inside each
/// operation: a data path while the data root is absent has no root to land in,
/// and an ancestor that is not a directory is the no-follow parent walk's
/// refusal, reported by the same component the walk stopped at. An absent
/// ancestor is admitted, because an operation creates the directories its path
/// needs.
fn resolve(state: &MemoryRepositoryState, path: &VirtualPath) -> Result<(), RedoApplyError> {
    if path.root_class() == RepositoryRootClass::Data && !state.data_root_exists {
        return Err(unaddressable(path));
    }
    let relative = path.relative().as_path();
    let ancestors = relative
        .ancestors()
        .skip(1)
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .collect::<Vec<_>>();
    // Outermost first, so the component this refuses is the one the file
    // backend's walk reaches first.
    ancestors.into_iter().rev().try_for_each(|ancestor| {
        let component = ancestor
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let ancestor =
            VirtualPath::from_root(path.root_class(), RootRelativePath::parse(ancestor)?)?;
        match state.entries.get(&ancestor) {
            None | Some(RepositoryEntry::Absent) | Some(RepositoryEntry::Directory { .. }) => {
                Ok(())
            }
            Some(_) => Err(RepositoryStateStoreError::UnsafeTarget(component).into()),
        }
    })
}

/// Refuse what the filesystem refuses at the moment one operation runs, rather
/// than only over the state the record's application began in: an earlier put
/// can occupy an ancestor, and the target can be a directory a put would have
/// to rename over and a delete to unlink.
///
/// Every refusal leaves the operations before it applied and the record
/// installed, as the file backend does, so the next application meets the same
/// refusal rather than a repository the record no longer describes.
fn ensure_operable(
    state: &MemoryRepositoryState,
    path: &VirtualPath,
) -> Result<(), RedoApplyError> {
    resolve(state, path)?;
    match state.entries.get(path) {
        Some(RepositoryEntry::Directory { .. }) => Err(is_a_directory(path)),
        _ => Ok(()),
    }
}

/// Replace a put's path with its bytes, modeling the directories it creates.
fn put_entry(state: &mut MemoryRepositoryState, put: &PutFile) -> Result<(), RedoApplyError> {
    ensure_operable(state, &put.path)?;
    InMemoryStorage::ensure_ancestor_dirs(state, &put.path);
    state.entries.insert(
        put.path.clone(),
        RepositoryEntry::File {
            content: CapturedBytes::new(memory_file_object(&put.path), put.bytes.clone())?,
            mode: put.mode,
        },
    );
    Ok(())
}

/// Write the appended bytes at their offset.
///
/// The log is truncated to the offset plus their length, so a log a previous
/// application already extended converges rather than carrying its residue
/// behind the appended bytes.
pub(in crate::storage) fn append_events(
    state: &mut MemoryRepositoryState,
    append: &EventAppend,
) -> Result<(), RedoApplyError> {
    let path = VirtualPath::EVENTS;
    let offset = usize::try_from(append.offset)
        .map_err(|_| std::io::Error::other("event-log offset exceeds addressable memory"))?;
    let (mut bytes, mode) = match state.entries.get(&path) {
        Some(RepositoryEntry::File { content, mode }) => (content.bytes().to_vec(), *mode),
        None | Some(RepositoryEntry::Absent) => (Vec::new(), FileMode::Regular),
        Some(RepositoryEntry::Directory { .. }) => return Err(is_a_directory(&path)),
        Some(_) => return Err(unaddressable(&path)),
    };
    // One resize covers both directions: it drops a longer log's tail, and it
    // fills the hole a positioned write past the end of a shorter one leaves.
    bytes.resize(offset, 0);
    bytes.extend_from_slice(&append.bytes);
    let object = memory_file_object(&path);
    state.entries.insert(
        path,
        RepositoryEntry::File {
            content: CapturedBytes::new(object, bytes)?,
            mode,
        },
    );
    Ok(())
}

/// The capture identity of an in-memory file entry: a function of its path and
/// its bytes alone, so two applications of one record leave the same entry.
pub(crate) fn memory_file_object(path: &VirtualPath) -> String {
    format!("memory-file:{path:?}")
}

fn unaddressable(path: &VirtualPath) -> RedoApplyError {
    RedoApplyError::Unaddressable {
        path: format!("{path:?}"),
    }
}

/// The refusal a directory occupying an operation's target raises, carrying the
/// kind the filesystem raises for the same occupant, so a caller driving both
/// backends reads one outcome.
fn is_a_directory(path: &VirtualPath) -> RedoApplyError {
    std::io::Error::new(
        std::io::ErrorKind::IsADirectory,
        format!("{path:?} is a directory"),
    )
    .into()
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryStorage {
    /// Read every recorded gate run the predicate keeps.
    ///
    /// One enumeration of the gate-run entries behind every whole-history
    /// question, matching the file backend's own
    /// (`@/invariant/convention-convergence`).
    fn read_gate_runs(
        &self,
        mut keep: impl FnMut(&GateRunResult) -> bool,
    ) -> Result<Vec<GateRunResult>> {
        let state = self.repository_state();
        if !Self::gate_run_parent_is_directory(&state, &VirtualPath::GATE_RUNS)? {
            return Ok(Vec::new());
        }
        let run_ids = state
            .entries
            .iter()
            .filter_map(|(vpath, entry)| match entry {
                RepositoryEntry::Directory { .. } => Self::gate_run_directory_id(vpath),
                RepositoryEntry::Absent
                | RepositoryEntry::File { .. }
                | RepositoryEntry::FileTail { .. }
                | RepositoryEntry::FileKind { .. }
                | RepositoryEntry::Symlink { .. }
                | RepositoryEntry::Unsupported { .. } => None,
            })
            .collect::<Vec<_>>();
        run_ids
            .into_iter()
            .filter_map(|run_id| {
                let vpath = match Self::gate_run_vpath(run_id) {
                    Ok(vpath) => vpath,
                    Err(error) => return Some(Err(error)),
                };
                match state.entries.get(&vpath) {
                    None | Some(RepositoryEntry::Absent) => None,
                    Some(RepositoryEntry::File { content, .. }) => Some(
                        serde_json::from_slice::<GateRunResult>(content.bytes()).with_context(
                            || {
                                format!(
                                    "Failed to deserialize gate-run result at {}",
                                    vpath.relative().as_path().display()
                                )
                            },
                        ),
                    ),
                    Some(_) => Some(Err(anyhow!(
                        "Gate run result at {} must be an ordinary file",
                        vpath.relative().as_path().display()
                    ))),
                }
            })
            .filter_map(|result| match result {
                Ok(run) if keep(&run) => Some(Ok(run)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }
}

impl IssueStore for InMemoryStorage {
    fn repository_layout(&self) -> Result<crate::repository_state::RepositoryLayout> {
        Ok(InMemoryStorage::repository_layout(self))
    }

    fn acquire_repo_write_lock(&self) -> Result<RepoWriteGuard> {
        self.repo_lock.acquire()
    }

    fn load_issue(&self, id: &str) -> Result<Issue> {
        Self::load_issue_entry(&self.repository_state(), id)?
            .ok_or_else(|| IssueNotFoundError::new(id).into())
    }

    fn load_issue_or_not_found(&self, id: &str) -> Result<Issue, crate::storage::PathReadError> {
        use crate::storage::PathReadError;
        // In-memory: a missing issue is always NotFound (no I/O involved); a
        // present-but-corrupt record surfaces as a genuine parse failure.
        Self::load_issue_entry(&self.repository_state(), id)
            .map_err(PathReadError::Other)?
            .ok_or_else(|| PathReadError::NotFound(format!("Issue not found: {}", id)))
    }

    fn resolve_issue_id(
        &self,
        prefix: &crate::domain::identity::IssueIdPrefix,
    ) -> Result<crate::domain::identity::IssueId> {
        crate::storage::resolve_issue_id_among(
            prefix,
            &Self::load_issues(&self.repository_state())?,
        )
    }

    fn list_issues(&self) -> Result<Vec<Issue>> {
        Self::load_issues(&self.repository_state())
    }

    fn load_gate_registry(&self) -> Result<GateRegistry> {
        match Self::data_entry_bytes(&self.repository_state(), &VirtualPath::GATES) {
            Some(bytes) => Ok(parse_gate_registry(&bytes)?),
            None => Ok(GateRegistry::default()),
        }
    }

    fn read_events(&self) -> Result<Vec<Event>> {
        Self::load_events(&self.repository_state())
    }

    fn root(&self) -> &std::path::Path {
        // Return unique root path for parallel test isolation.
        &self.root_path
    }

    fn load_gate_run_result(&self, run_id: &str) -> Result<GateRunResult> {
        let path = Self::gate_run_vpath(run_id)?;
        let run_dir = Self::gate_run_directory_vpath(run_id)?;
        let state = self.repository_state();
        if !Self::gate_run_parent_is_directory(&state, &VirtualPath::GATE_RUNS)?
            || !Self::gate_run_parent_is_directory(&state, &run_dir)?
        {
            return Err(GateRunNotFoundError::new(run_id).into());
        }
        match state.entries.get(&path) {
            None | Some(RepositoryEntry::Absent) => Err(GateRunNotFoundError::new(run_id).into()),
            Some(RepositoryEntry::File { content, .. }) => serde_json::from_slice(content.bytes())
                .with_context(|| {
                    format!(
                        "Failed to deserialize gate-run result at {}",
                        path.relative().as_path().display()
                    )
                }),
            Some(_) => anyhow::bail!(
                "Gate run result at {} must be an ordinary file",
                path.relative().as_path().display()
            ),
        }
    }

    fn list_gate_runs_for_issue(&self, issue_id: &str) -> Result<Vec<GateRunResult>> {
        self.read_gate_runs(|result| result.issue_id == issue_id)
    }

    fn find_reusable_gate_run(
        &self,
        gate_key: &str,
        digest: &crate::domain::repository_inputs::InputsDigest,
    ) -> Result<Option<GateRunResult>> {
        Ok(crate::domain::latest_gate_run(self.read_gate_runs(
            |result| result.is_reusable_for(gate_key, digest),
        )?))
    }

    fn list_gate_presets(&self) -> Result<Vec<crate::gate_presets::PresetInfo>> {
        let mut presets = self
            .gate_presets_from_repository_state()?
            .values()
            .map(|preset| crate::gate_presets::PresetInfo {
                name: preset.name.clone(),
                description: preset.description.clone(),
                gate_count: preset.gates.len(),
            })
            .collect::<Vec<_>>();
        presets.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(presets)
    }

    fn get_gate_preset(&self, name: &str) -> Result<crate::gate_presets::GatePresetDefinition> {
        self.gate_presets_from_repository_state()?
            .get(name)
            .cloned()
            .ok_or_else(|| PresetNotFoundError::new(name).into())
    }

    fn read_repo_file(
        &self,
        rel_path: &str,
    ) -> Result<Option<String>, crate::storage::PathReadError> {
        // Enforce the SAME repo-relative path contract as JsonFileStorage (reject
        // empty, absolute, or `..`-bearing paths), then serve from the aggregate
        // repository image (the one store a mutation session captures): a captured
        // File -> Some, absent or non-file -> None.
        use crate::repository_state::RepositoryEntry;
        let vpath = self.repo_file_vpath(rel_path)?;
        Ok(match self.repository_state().entries.get(&vpath) {
            Some(RepositoryEntry::File { content, .. }) => {
                Some(String::from_utf8_lossy(content.bytes()).into_owned())
            }
            _ => None,
        })
    }

    fn read_path_bytes(
        &self,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<(Vec<u8>, String), crate::storage::PathReadError> {
        use crate::storage::PathReadError;
        let vpath = self.repo_file_vpath(path)?;
        if at_commit.is_some() {
            // Commit-pinned reads have no image of their own yet, so they still
            // read the synthetic root on disk.
            return std::fs::read(self.root_path.join(path))
                .map(|bytes| (bytes, "working-tree".to_string()))
                .map_err(|e| match e.kind() {
                    std::io::ErrorKind::NotFound => PathReadError::NotFound(path.to_string()),
                    _ => PathReadError::Other(anyhow!("Failed to read file {}: {}", path, e)),
                });
        }
        // A non-file entry is refused as the file backend's confined read
        // refuses one.
        match self.repository_state().entries.get(&vpath) {
            Some(RepositoryEntry::File { content, .. }) => {
                Ok((content.bytes().to_vec(), "working-tree".to_string()))
            }
            None | Some(RepositoryEntry::Absent) => Err(PathReadError::NotFound(path.to_string())),
            Some(_) => Err(PathReadError::OutsideRepoRoot(path.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate_run_parent_entry(kind: &str) -> RepositoryEntry {
        let identity =
            || EntryIdentity::for_bytes(format!("parent-{kind}"), kind.as_bytes()).unwrap();
        let captured =
            |bytes: &[u8]| CapturedBytes::new(format!("parent-{kind}"), bytes.to_vec()).unwrap();
        match kind {
            "absent" => RepositoryEntry::Absent,
            "directory" => RepositoryEntry::Directory {
                identity: identity(),
                mode: FileMode::Executable,
            },
            "file" => RepositoryEntry::File {
                content: captured(b"not a directory"),
                mode: FileMode::Regular,
            },
            "symlink" => RepositoryEntry::Symlink {
                target: captured(b"elsewhere"),
                mode: FileMode::Regular,
            },
            "unsupported" => RepositoryEntry::Unsupported {
                identity: identity(),
                reason: "special object".to_string(),
                mode: FileMode::Regular,
            },
            _ => unreachable!("test parent kind is closed"),
        }
    }

    #[test]
    fn test_gate_run_readers_match_only_exact_result_shape_and_name_malformed_path() {
        let storage = InMemoryStorage::new();
        let nested = VirtualPath::data(format!(
            "{}/outer/nested/result.json",
            VirtualPath::GATE_RUNS.relative().as_str()
        ))
        .unwrap();
        let canonical = InMemoryStorage::gate_run_vpath("corrupt-run").unwrap();
        let mut state = storage.repository_state();
        InMemoryStorage::put_data_entry(&mut state, nested, b"{ nested".to_vec());
        drop(state);
        assert!(storage
            .list_gate_runs_for_issue("any-issue")
            .unwrap()
            .is_empty());

        let mut state = storage.repository_state();
        InMemoryStorage::put_data_entry(&mut state, canonical, b"{ corrupt".to_vec());
        drop(state);

        let load_error = storage.load_gate_run_result("corrupt-run").unwrap_err();
        let record_path = format!(
            "{}/corrupt-run/result.json",
            VirtualPath::GATE_RUNS.relative().as_str()
        );
        assert!(format!("{load_error:#}").contains(&record_path));
        let list_error = storage.list_gate_runs_for_issue("any-issue").unwrap_err();
        assert!(format!("{list_error:#}").contains(&record_path));
    }

    #[test]
    fn test_gate_run_readers_reject_non_file_result_object() {
        let storage = InMemoryStorage::new();
        let path = InMemoryStorage::gate_run_vpath("directory-run").unwrap();
        let mut state = storage.repository_state();
        InMemoryStorage::ensure_ancestor_dirs(&mut state, &path);
        state.entries.insert(
            path,
            RepositoryEntry::Directory {
                identity: EntryIdentity::for_bytes("directory-run", b"directory").unwrap(),
                mode: FileMode::Executable,
            },
        );
        drop(state);

        for error in [
            storage.load_gate_run_result("directory-run").unwrap_err(),
            storage.list_gate_runs_for_issue("any-issue").unwrap_err(),
        ] {
            let message = format!("{error:#}");
            assert!(message.contains("directory-run/result.json"));
            assert!(message.contains("ordinary file"));
        }
    }

    #[test]
    fn test_gate_run_readers_validate_gate_runs_root_parent_kinds() {
        for kind in ["absent", "directory", "file", "symlink", "unsupported"] {
            let storage = InMemoryStorage::new();
            storage
                .repository_state()
                .entries
                .insert(VirtualPath::GATE_RUNS, gate_run_parent_entry(kind));

            let loaded = storage.load_gate_run_result("run-one");
            let listed = storage.list_gate_runs_for_issue("issue-one");
            if matches!(kind, "absent" | "directory") {
                assert!(loaded
                    .unwrap_err()
                    .downcast_ref::<GateRunNotFoundError>()
                    .is_some());
                assert!(listed.unwrap().is_empty());
            } else {
                assert!(format!("{:#}", loaded.unwrap_err()).contains("ordinary directory"));
                assert!(format!("{:#}", listed.unwrap_err()).contains("ordinary directory"));
            }
        }
    }

    #[test]
    fn test_gate_run_readers_validate_canonical_run_directory_parent_kinds() {
        for kind in ["absent", "directory", "file", "symlink", "unsupported"] {
            let storage = InMemoryStorage::new();
            let root = VirtualPath::GATE_RUNS;
            let run_dir = InMemoryStorage::gate_run_directory_vpath("run-one").unwrap();
            let mut state = storage.repository_state();
            state
                .entries
                .insert(root, gate_run_parent_entry("directory"));
            state.entries.insert(run_dir, gate_run_parent_entry(kind));
            drop(state);

            let loaded = storage.load_gate_run_result("run-one");
            let listed = storage.list_gate_runs_for_issue("issue-one");
            if matches!(kind, "absent" | "directory") {
                assert!(loaded
                    .unwrap_err()
                    .downcast_ref::<GateRunNotFoundError>()
                    .is_some());
                assert!(listed.unwrap().is_empty());
            } else {
                assert!(format!("{:#}", loaded.unwrap_err()).contains("ordinary directory"));
                assert!(
                    listed.unwrap().is_empty(),
                    "non-directory direct gate-runs children are ignored as root clutter"
                );
            }
        }
    }

    #[test]
    fn test_unseeded_store_models_absent_root_seeding_marks_existing() {
        use crate::repository_state::{RepositoryEntry, VirtualPath};
        // Guardrail: a fresh, unseeded store models an ABSENT data root, so a
        // mutation session on it exercises the fresh-init path — the implicit
        // seeded-means-existing flag must never silently flip this.
        let storage = InMemoryStorage::new();
        assert!(
            !storage.repository_state().data_root_exists,
            "an unseeded store is absent-root"
        );
        assert!(!storage
            .repository_state()
            .entries
            .contains_key(&VirtualPath::data("").unwrap()));

        // Seeding any repository-owned file marks the data root as existing and
        // publishes the Data("") directory entry.
        storage.add_data_file(
            VirtualPath::CONFIG.relative().as_str(),
            "[project]\nname = \"x\"\n",
        );
        assert!(
            storage.repository_state().data_root_exists,
            "a seeded store is existing-root"
        );
        assert!(matches!(
            storage
                .repository_state()
                .entries
                .get(&VirtualPath::data("").unwrap()),
            Some(RepositoryEntry::Directory { .. })
        ));
    }

    #[test]
    fn test_acquire_repo_write_lock_over_absent_data_root_leaves_it_absent() {
        let storage = InMemoryStorage::new();

        let _guard = storage.acquire_repo_write_lock().unwrap();

        assert!(!storage.repository_state().data_root_exists);
    }

    #[test]
    fn test_worktree_fixture_does_not_materialize_data_root() {
        let storage = InMemoryStorage::new();
        storage.add_worktree_file("docs/guide.md", "guide");

        let state = storage.repository_state();
        assert!(!state.data_root_exists);
        assert!(!state.entries.contains_key(&VirtualPath::data("").unwrap()));
        assert!(matches!(
            state.entries.get(&VirtualPath::worktree("docs/guide.md").unwrap()),
            Some(RepositoryEntry::File { content, .. }) if content.bytes() == b"guide"
        ));
    }

    #[test]
    fn test_new_minted_root_is_removed_when_the_last_clone_drops() {
        let storage = InMemoryStorage::new();
        let clone = storage.clone();
        let root = storage.root().to_path_buf();
        std::fs::create_dir_all(root.join(".jit")).unwrap();
        std::fs::write(root.join(".jit/config.toml"), "").unwrap();

        drop(storage);
        assert!(root.exists(), "a live clone keeps the minted root");
        drop(clone);
        assert!(!root.exists(), "the minted root outlived every clone");
    }
}

/// This backend's own record fixtures, beside the file backend's in
/// [`test_fixtures`](crate::storage::redo_apply::test_fixtures), so the
/// `redo_conformance` matrix and this module's tests seed and read one backend
/// the same way (`@/inv/shared-test-contracts`).
#[cfg(test)]
pub(crate) mod test_fixtures {
    use super::*;
    use crate::storage::redo_apply::test_fixtures::CAPTURED_LOG;

    /// A backend whose data root exists, holding every path in `occupied` as a
    /// captured file and an event log the record's append offset refers to.
    /// `residue` extends that log past the offset, standing for a log an
    /// interrupted application already wrote into.
    pub(crate) fn seeded(occupied: &[VirtualPath], residue: bool) -> InMemoryStorage {
        let storage = InMemoryStorage::new();
        let log = match residue {
            true => [CAPTURED_LOG, b"{\"tag\":\"residue\"}\n"].concat(),
            false => CAPTURED_LOG.to_vec(),
        };
        storage.add_data_file(
            VirtualPath::EVENTS.relative().as_path(),
            &String::from_utf8(log).unwrap(),
        );
        occupied.iter().for_each(|path| {
            let relative = path.relative().as_path();
            match path.root_class() {
                RepositoryRootClass::Data => storage.add_data_file(relative, "captured"),
                RepositoryRootClass::Worktree => storage.add_worktree_file(relative, "captured"),
            }
        });
        storage
    }

    /// Everything this backend holds of a mutation: the entries, whether the
    /// data root exists, and the record still waiting to be applied.
    #[allow(clippy::type_complexity)]
    pub(crate) fn semantic_state(
        storage: &InMemoryStorage,
    ) -> (
        BTreeMap<VirtualPath, RepositoryEntry>,
        bool,
        Option<RedoRecord>,
    ) {
        let state = storage.repository_state();
        (
            state.entries.clone(),
            state.data_root_exists,
            state.record.clone(),
        )
    }
}

#[cfg(test)]
mod record_tests {
    use super::test_fixtures::{seeded, semantic_state};
    use super::*;
    use crate::storage::redo_apply::test_fixtures::{RecordedPoints, CAPTURED_LOG};
    use crate::storage::NoRedoInterruptions;

    fn put(path: VirtualPath, bytes: &[u8]) -> PutFile {
        PutFile {
            path,
            bytes: bytes.to_vec(),
            mode: FileMode::Regular,
        }
    }

    /// An applier whose held injector lets every point proceed.
    fn uninterrupted() -> MemoryRedoApplier {
        MemoryRedoApplier::new(Arc::new(NoRedoInterruptions))
    }

    /// An applier over a recorder the caller keeps, so the points its
    /// applications checked stay readable.
    fn recording_applier() -> (Arc<RecordedPoints>, MemoryRedoApplier) {
        let recorded = Arc::new(RecordedPoints::default());
        let injector: Arc<dyn RedoInterruptionInjector> = recorded.clone();
        (recorded, MemoryRedoApplier::new(injector))
    }

    fn file_bytes(storage: &InMemoryStorage, path: &VirtualPath) -> Option<Vec<u8>> {
        match storage.repository_state().entries.get(path) {
            Some(RepositoryEntry::File { content, .. }) => Some(content.bytes().to_vec()),
            _ => None,
        }
    }

    fn data(relative: &str) -> VirtualPath {
        VirtualPath::data(relative).unwrap()
    }

    fn issue_data(relative: &str) -> VirtualPath {
        VirtualPath::data(format!(
            "{}/{}",
            VirtualPath::ISSUES.relative().as_str(),
            relative
        ))
        .unwrap()
    }

    fn worktree(relative: &str) -> VirtualPath {
        VirtualPath::worktree(relative).unwrap()
    }

    #[test]
    fn test_commit_record_installs_the_record_before_any_entry_changes() {
        let storage = seeded(&[VirtualPath::INDEX, worktree("stale.md")], false);
        let before = semantic_state(&storage).0;
        let record = RedoRecord::new(
            Vec::new(),
            vec![put(VirtualPath::INDEX, b"published")],
            vec![worktree("stale.md")],
            Some(EventAppend {
                offset: CAPTURED_LOG.len() as u64,
                bytes: b"{\"tag\":\"issue_claimed\"}\n".to_vec(),
            }),
        )
        .unwrap();

        storage.commit_record(&record).unwrap();

        assert_eq!(
            semantic_state(&storage).0,
            before,
            "committing the record moved a state entry the record has not been applied to"
        );
        assert_eq!(
            semantic_state(&storage).2,
            Some(record),
            "the committed record is not the backend's residue"
        );
    }

    #[test]
    fn test_commit_record_refuses_a_record_a_previous_mutation_left_behind() {
        let storage = seeded(&[], false);
        let leftover = RedoRecord::new(
            Vec::new(),
            vec![put(VirtualPath::INDEX, b"leftover")],
            Vec::new(),
            None,
        )
        .unwrap();
        storage.commit_record(&leftover).unwrap();

        let outcome = storage.commit_record(
            &RedoRecord::new(Vec::new(), Vec::new(), vec![VirtualPath::INDEX], None).unwrap(),
        );

        assert!(
            matches!(outcome, Err(RedoApplyError::OccupiedRecord)),
            "a second commit replaced a record no application consumed"
        );
        assert_eq!(semantic_state(&storage).2, Some(leftover));
    }

    #[test]
    fn test_memory_redo_applier_apply_publishes_every_operation_the_record_names() {
        let storage = seeded(&[VirtualPath::INDEX, worktree("stale.md")], false);
        let appended = b"{\"tag\":\"issue_claimed\"}\n";
        let record = RedoRecord::new(
            Vec::new(),
            vec![put(VirtualPath::INDEX, b"published")],
            vec![worktree("stale.md")],
            Some(EventAppend {
                offset: CAPTURED_LOG.len() as u64,
                bytes: appended.to_vec(),
            }),
        )
        .unwrap();

        uninterrupted().apply(&storage, &record).unwrap();

        assert_eq!(
            file_bytes(&storage, &VirtualPath::INDEX).as_deref(),
            Some(b"published".as_slice())
        );
        assert!(
            !storage
                .repository_state()
                .entries
                .contains_key(&worktree("stale.md")),
            "the deleted path survived the application"
        );
        assert_eq!(
            file_bytes(&storage, &VirtualPath::EVENTS),
            Some([CAPTURED_LOG, appended].concat())
        );
    }

    #[test]
    fn test_memory_redo_applier_apply_removes_the_record_it_applied() {
        let storage = seeded(&[], false);
        let record = RedoRecord::new(
            Vec::new(),
            vec![put(VirtualPath::INDEX, b"published")],
            Vec::new(),
            None,
        )
        .unwrap();
        storage.commit_record(&record).unwrap();

        uninterrupted().apply(&storage, &record).unwrap();

        assert_eq!(
            semantic_state(&storage).2,
            None,
            "the applied record is still there for a later session to apply again"
        );
    }

    #[test]
    fn test_memory_redo_applier_apply_creates_the_directories_a_put_needs() {
        let storage = seeded(&[], false);
        let nested = issue_data("2026/a.json");
        let record = RedoRecord::new(
            Vec::new(),
            vec![put(nested.clone(), b"published")],
            Vec::new(),
            None,
        )
        .unwrap();

        uninterrupted().apply(&storage, &record).unwrap();

        assert_eq!(
            file_bytes(&storage, &nested).as_deref(),
            Some(b"published".as_slice())
        );
        assert!(
            matches!(
                storage.repository_state().entries.get(&issue_data("2026")),
                Some(RepositoryEntry::Directory { .. })
            ),
            "the put reached its path without the directories it needs"
        );
    }

    #[test]
    fn test_memory_redo_applier_apply_gives_each_put_the_mode_it_carries() {
        let storage = seeded(&[], false);
        let path = worktree("scripts/publish.sh");
        let record = RedoRecord::new(
            Vec::new(),
            vec![PutFile {
                path: path.clone(),
                bytes: b"#!/bin/sh\n".to_vec(),
                mode: FileMode::Executable,
            }],
            Vec::new(),
            None,
        )
        .unwrap();

        uninterrupted().apply(&storage, &record).unwrap();

        assert!(
            matches!(
                storage.repository_state().entries.get(&path),
                Some(RepositoryEntry::File {
                    mode: FileMode::Executable,
                    ..
                })
            ),
            "the published entry does not carry the mode the record named"
        );
    }

    #[test]
    fn test_memory_redo_applier_apply_writes_the_append_at_its_offset_over_a_longer_log() {
        // A log an interrupted application already extended: a pass that never
        // truncates would leave its residue behind the appended bytes.
        let storage = seeded(&[], true);
        let appended = b"{\"tag\":\"issue_claimed\"}\n";
        let record = RedoRecord::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(EventAppend {
                offset: CAPTURED_LOG.len() as u64,
                bytes: appended.to_vec(),
            }),
        )
        .unwrap();

        uninterrupted().apply(&storage, &record).unwrap();

        assert_eq!(
            file_bytes(&storage, &VirtualPath::EVENTS),
            Some([CAPTURED_LOG, appended].concat())
        );
    }

    #[test]
    fn test_memory_redo_applier_apply_checks_every_interruption_point_in_order() {
        let storage = seeded(&[], false);
        let record = RedoRecord::new(
            vec![data("templates")],
            vec![
                put(VirtualPath::INDEX, b"published"),
                put(worktree("notes.md"), b"published"),
            ],
            vec![worktree("stale.md")],
            Some(EventAppend {
                offset: CAPTURED_LOG.len() as u64,
                bytes: b"{\"tag\":\"issue_claimed\"}\n".to_vec(),
            }),
        )
        .unwrap();
        let (recorded, applier) = recording_applier();

        applier.apply(&storage, &record).unwrap();

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

    /// The injector belongs to the applier rather than to a call, so a second
    /// application reaches the same one without being handed it.
    #[test]
    fn test_memory_redo_applier_apply_checks_every_application_through_the_held_injector() {
        let storage = seeded(&[], false);
        let (recorded, applier) = recording_applier();
        let published = RedoRecord::new(
            Vec::new(),
            vec![put(VirtualPath::INDEX, b"published")],
            Vec::new(),
            None,
        )
        .unwrap();
        let removed =
            RedoRecord::new(Vec::new(), Vec::new(), vec![VirtualPath::INDEX], None).unwrap();

        applier.apply(&storage, &published).unwrap();
        let after_first = recorded.points();
        applier.apply(&storage, &removed).unwrap();

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

    /// The counterpart of the file backend's absent data capability: a record
    /// addressing the data root before it exists changes nothing at all.
    #[test]
    fn test_memory_redo_applier_apply_refuses_a_data_path_while_the_data_root_is_absent() {
        let storage = InMemoryStorage::new();
        let record = RedoRecord::new(
            Vec::new(),
            vec![
                put(worktree("notes.md"), b"published"),
                put(VirtualPath::INDEX, b"published"),
            ],
            Vec::new(),
            None,
        )
        .unwrap();

        let outcome = uninterrupted().apply(&storage, &record);

        assert!(
            matches!(outcome, Err(RedoApplyError::Unaddressable { .. })),
            "the record reached a data root the backend does not hold"
        );
        assert!(
            storage.repository_state().entries.is_empty(),
            "a live write ran before the record was refused"
        );
    }

    #[test]
    fn test_memory_redo_applier_apply_refuses_a_put_under_a_non_directory_ancestor() {
        let storage = seeded(&[], false);
        storage.add_data_file(
            VirtualPath::ISSUES.relative().as_str(),
            "an ordinary file where a directory is needed",
        );
        let record = RedoRecord::new(
            Vec::new(),
            vec![put(issue_data("a.json"), b"published")],
            Vec::new(),
            None,
        )
        .unwrap();
        storage.commit_record(&record).unwrap();

        let outcome = uninterrupted().apply(&storage, &record);

        assert!(
            matches!(
                outcome,
                Err(RedoApplyError::Roots(RepositoryStateStoreError::UnsafeTarget(
                    ref component
                ))) if component == VirtualPath::ISSUES.relative().as_str()
            ),
            "the put resolved through an ancestor that is not a directory"
        );
        assert!(
            !storage
                .repository_state()
                .entries
                .contains_key(&issue_data("a.json")),
            "the refused put reached its path anyway"
        );
        assert_eq!(
            semantic_state(&storage).2,
            Some(record),
            "the refused application cleared the record that still describes it"
        );
    }

    /// The file backend renames a put's bytes over its target, which fails on a
    /// directory: a directory the repository already held refuses the put and
    /// the record stays installed for the next application to meet the same
    /// refusal. Two of a record's own operations can no longer create this
    /// conflict between themselves — one naming the other's path is rejected
    /// at construction — so the occupying directory is seeded independently
    /// of the record under test.
    #[test]
    fn test_memory_redo_applier_apply_refuses_a_put_over_a_pre_existing_directory() {
        let storage = seeded(&[], false);
        storage.add_data_file(
            format!("{}/existing.json", VirtualPath::ISSUES.relative().as_str()),
            "residual",
        );
        let record = RedoRecord::new(
            Vec::new(),
            vec![put(VirtualPath::ISSUES, b"occupier")],
            Vec::new(),
            None,
        )
        .unwrap();
        storage.commit_record(&record).unwrap();

        let outcome = uninterrupted().apply(&storage, &record);

        assert!(
            matches!(&outcome, Err(RedoApplyError::Io(error))
                if error.kind() == std::io::ErrorKind::IsADirectory),
            "the put replaced a directory the file backend refuses to rename over: {outcome:?}"
        );
        assert!(
            matches!(
                storage.repository_state().entries.get(&VirtualPath::ISSUES),
                Some(RepositoryEntry::Directory { .. })
            ),
            "the refused put overwrote the directory at its target"
        );
        assert_eq!(
            file_bytes(&storage, &issue_data("existing.json")).as_deref(),
            Some(b"residual".as_slice()),
            "the refusal disturbed a file the record never named"
        );
        assert_eq!(
            semantic_state(&storage).2,
            Some(record),
            "the refused application cleared the record that still describes it"
        );
    }

    /// A refusal partway through a record's operations leaves the
    /// operations that already landed in place: redo-only convergence replays a record
    /// forward or not at all, never partially undoes it (`@/charter/D-17`).
    /// The earlier put and the refusing put must not overlap — one naming the
    /// other's path is rejected at construction — so the occupying directory
    /// is seeded independently of the record, as in the test above.
    #[test]
    fn test_memory_redo_applier_apply_leaves_an_earlier_put_applied_after_a_later_one_is_refused() {
        let storage = seeded(&[], false);
        storage.add_data_file(
            format!("{}/existing.json", VirtualPath::ISSUES.relative().as_str()),
            "residual",
        );
        let record = RedoRecord::new(
            Vec::new(),
            vec![
                put(VirtualPath::INDEX, b"published"),
                put(VirtualPath::ISSUES, b"occupier"),
            ],
            Vec::new(),
            None,
        )
        .unwrap();
        storage.commit_record(&record).unwrap();

        let outcome = uninterrupted().apply(&storage, &record);

        assert!(
            matches!(&outcome, Err(RedoApplyError::Io(error))
                if error.kind() == std::io::ErrorKind::IsADirectory),
            "the later put replaced a directory the file backend refuses to rename over: {outcome:?}"
        );
        assert_eq!(
            file_bytes(&storage, &VirtualPath::INDEX).as_deref(),
            Some(b"published".as_slice()),
            "the refusal undid a put that had already landed"
        );
        assert_eq!(
            semantic_state(&storage).2,
            Some(record),
            "the refused application cleared the record that still describes it"
        );
    }

    /// A delete names a file to unlink, which fails on a directory: the record
    /// survives rather than the backend silently orphaning the directory's
    /// children.
    #[test]
    fn test_memory_redo_applier_apply_refuses_a_delete_of_a_directory() {
        let storage = seeded(&[issue_data("2026/a.json")], false);
        let record =
            RedoRecord::new(Vec::new(), Vec::new(), vec![issue_data("2026")], None).unwrap();
        storage.commit_record(&record).unwrap();

        let outcome = uninterrupted().apply(&storage, &record);

        assert!(
            matches!(&outcome, Err(RedoApplyError::Io(error))
                if error.kind() == std::io::ErrorKind::IsADirectory),
            "the delete unlinked a directory the file backend refuses: {outcome:?}"
        );
        assert!(
            matches!(
                storage.repository_state().entries.get(&issue_data("2026")),
                Some(RepositoryEntry::Directory { .. })
            ),
            "the refused delete removed the directory anyway"
        );
        assert!(
            file_bytes(&storage, &issue_data("2026/a.json")).is_some(),
            "the refused delete orphaned the directory's child"
        );
        assert_eq!(semantic_state(&storage).2, Some(record));
    }
}
