//! JSON file-based storage implementation.
//!
//! Issues, the index, and events are read from a `.jit/` directory; the gate
//! registry is parsed from `.jit/gates.toml` through
//! [`crate::storage::gate_store`]. Repository-owned changes are published as
//! redo records through [`RepositoryStateStore`]. The directory
//! location can be overridden with the `JIT_DATA_DIR` environment variable.

use super::gate_run_reader::TreeNode;
use crate::declarations::GateRegistry;
use crate::domain::event_log::parse_known_events;
use crate::domain::identity::{GateRunId, IssueId, IssueIdPrefix};
use crate::domain::{Event, Issue};
use crate::repository_state::{
    RepositoryIndex, RepositoryIndexError, RepositoryLayout, RepositoryRootClass, VirtualPath,
    BOOTSTRAP_LOCK_FILE, EVENTS_LOCK_FILE, INDEX_LOCK_FILE,
};
use crate::storage::{
    FileLocker, IssueNotFoundError, IssueStore, RecoveryDispatchReport, RepoWriteGuard,
    RepoWriteLock, RepositoryFormatUnsupportedError, RepositoryMutationSession,
    RepositoryNotFoundError, RepositoryStateStore, WorktreePaths,
};
use anyhow::{anyhow, Context, Result};
use std::cell::RefCell;
use std::collections::{hash_map::Entry, BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

static NEXT_RETENTION_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// Mutation sessions contain thread-scoped lock-order guards and therefore
    /// remain on the thread that opened them. Shared storage clones carry only
    /// the owner thread id used to reject cross-thread release/reentry.
    static RETAINED_MUTATION_SESSIONS:
        RefCell<HashMap<u64, Box<dyn RepositoryMutationSession>>> = RefCell::new(HashMap::new());
}

/// The semantic records physically present in one selected JIT data root.
///
/// Unlike [`IssueStore`] readers, this snapshot never consults repository
/// history, tombstones, or another checkout's data root. Issue order is
/// stable by physical file name; event order remains append-log order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExactStoreSnapshot {
    /// Every ordinary `issues/*.json` record in the selected store.
    pub issues: Vec<Issue>,
    /// Every current-vocabulary record in the selected store's event log.
    pub events: Vec<Event>,
}

/// Read exactly one JIT data root without aggregation or mutation.
///
/// `layout` is the sole location authority, and well-known files are resolved
/// through it. The read does not construct a storage backend, acquire locks
/// that could create sidecar files, invoke Git, inspect the process working
/// directory, or fall back to a primary checkout. An absent data root, issue
/// directory, or event log contributes no records.
///
/// # Errors
///
/// Returns an error when the selected root contains an unsafe filesystem entry,
/// a record cannot be read, or a physical issue/current event cannot be parsed.
pub fn read_exact_store(layout: &RepositoryLayout) -> Result<ExactStoreSnapshot> {
    let data_root = layout.data_root();
    let issues_path = layout.resolve(&VirtualPath::ISSUES)?;
    let events_path = layout.resolve(&VirtualPath::EVENTS)?;
    let data = match super::capability_fs::open_absolute_dir_nofollow(data_root) {
        Ok(data) => data,
        Err(super::capability_fs::CapabilityFsError::Io(error))
            if error.kind() == std::io::ErrorKind::NotFound =>
        {
            return Ok(ExactStoreSnapshot::default())
        }
        Err(error) => return Err(error.into()),
    };

    let issues = match data.symlink_metadata(VirtualPath::ISSUES.relative().as_str()) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
        Ok(metadata) if metadata.is_symlink() || !metadata.is_dir() => {
            anyhow::bail!(
                "Issue root at {} must be an ordinary directory",
                issues_path.display()
            )
        }
        Ok(_) => {
            let issue_dir = super::capability_fs::open_child_dir_nofollow(
                &data,
                VirtualPath::ISSUES.relative().as_str(),
            )?;
            let mut names = issue_dir
                .entries()?
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<std::io::Result<Vec<_>>>()?;
            names.sort();

            names
                .into_iter()
                .filter(|name| Path::new(name).extension().is_some_and(|ext| ext == "json"))
                .map(|name| {
                    let leaf = name.into_string().map_err(|name| {
                        anyhow!(
                            "Issue record name at {} is not valid UTF-8",
                            issues_path.join(name).display()
                        )
                    })?;
                    let path = issues_path.join(&leaf);
                    let metadata = issue_dir.symlink_metadata(&leaf)?;
                    if metadata.is_symlink() || !metadata.is_file() {
                        anyhow::bail!(
                            "Issue record at {} must be an ordinary file",
                            path.display()
                        );
                    }
                    let mut file =
                        super::capability_fs::open_regular_file_nofollow(&issue_dir, &leaf)
                            .with_context(|| {
                                format!("Failed to open issue record at {}", path.display())
                            })?;
                    let mut bytes = Vec::new();
                    file.read_to_end(&mut bytes).with_context(|| {
                        format!("Failed to read issue record at {}", path.display())
                    })?;
                    serde_json::from_slice(&bytes).with_context(|| {
                        format!("Failed to deserialize issue record at {}", path.display())
                    })
                })
                .collect::<Result<Vec<Issue>>>()?
        }
    };

    let events = match data.symlink_metadata(VirtualPath::EVENTS.relative().as_str()) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
        Ok(metadata) if metadata.is_symlink() || !metadata.is_file() => {
            anyhow::bail!(
                "Event log at {} must be an ordinary file",
                events_path.display()
            )
        }
        Ok(_) => {
            let mut file = super::capability_fs::open_regular_file_nofollow(
                &data,
                VirtualPath::EVENTS.relative().as_str(),
            )
            .with_context(|| format!("Failed to open event log at {}", events_path.display()))?;
            let mut contents = Vec::new();
            file.read_to_end(&mut contents).with_context(|| {
                format!("Failed to read event log at {}", events_path.display())
            })?;
            parse_known_events(&contents).with_context(|| {
                format!(
                    "Failed to deserialize event log at {}",
                    events_path.display()
                )
            })?
        }
    };

    Ok(ExactStoreSnapshot { issues, events })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum RetainedSessionState {
    #[default]
    Idle,
    Active(ThreadId),
    Suspended(ThreadId),
}

/// A nested external-process request encountered a suspended startup session.
#[derive(Debug, thiserror::Error)]
#[error("an external process is already running outside the retained mutation session")]
pub struct RetainedSessionSuspendedError;

/// Thread-bound owner of one retained startup mutation session.
///
/// Dropping the guard releases the session and clears its ownership metadata,
/// including on ordinary error unwinds. It is deliberately not `Send` because
/// the session's lock-order guard is thread-scoped.
#[must_use = "dropping the guard releases startup mutation serialization"]
pub struct RetainedMutationSessionGuard {
    retention_id: u64,
    state: Arc<Mutex<RetainedSessionState>>,
    recovery_report: RecoveryDispatchReport,
    active: bool,
    _not_send: std::marker::PhantomData<Rc<()>>,
}

impl RetainedMutationSessionGuard {
    /// Recovery completed before this retained session became available.
    pub fn recovery_report(&self) -> &RecoveryDispatchReport {
        &self.recovery_report
    }

    /// Explicitly release this session before the end of its dispatch scope.
    pub fn release(mut self) -> Result<()> {
        self.release_inner()
    }

    fn release_inner(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        let current = std::thread::current().id();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match *state {
            RetainedSessionState::Idle => return Ok(()),
            RetainedSessionState::Active(owner) if owner == current => {}
            RetainedSessionState::Suspended(owner) if owner == current => {
                *state = RetainedSessionState::Idle;
                return Ok(());
            }
            RetainedSessionState::Active(_) | RetainedSessionState::Suspended(_) => {
                anyhow::bail!(
                    "A retained mutation session can only be released by its opening thread"
                )
            }
        }
        let session = RETAINED_MUTATION_SESSIONS
            .with(|sessions| sessions.borrow_mut().remove(&self.retention_id));
        let missing = session.is_none();
        drop(session);
        *state = RetainedSessionState::Idle;
        if missing {
            anyhow::bail!("The retained mutation session is missing from its opening thread");
        }
        Ok(())
    }
}

impl Drop for RetainedMutationSessionGuard {
    fn drop(&mut self) {
        let _ = self.release_inner();
    }
}

struct RetainedSessionSuspension {
    state: Arc<Mutex<RetainedSessionState>>,
    owner: ThreadId,
    active: bool,
}

impl RetainedSessionSuspension {
    fn restored(&mut self) {
        self.active = false;
    }
}

impl Drop for RetainedSessionSuspension {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *state == RetainedSessionState::Suspended(self.owner) {
            *state = RetainedSessionState::Idle;
        }
    }
}

type Index = RepositoryIndex;

/// Pinned-document evidence retained across captures, keyed by request.
pub(crate) type PinnedEvidenceCache =
    BTreeMap<(String, String), crate::repository_state::PinnedDocumentEvidence>;

#[cfg(all(test, unix))]
mod artifact_location_tests {
    use super::*;
    use crate::domain::artifact_classifier::ArtifactLocation;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    #[test]
    fn test_inspect_artifact_location_does_not_follow_leaf_or_traversal_symlinks() {
        let repo = TempDir::new().unwrap();
        fs::create_dir_all(repo.path().join(".jit")).unwrap();
        fs::create_dir_all(repo.path().join("real")).unwrap();
        fs::create_dir_all(repo.path().join("directory-artifact")).unwrap();
        fs::write(repo.path().join("real/file.md"), b"content").unwrap();
        symlink(
            repo.path().join("real/file.md"),
            repo.path().join("leaf.md"),
        )
        .unwrap();
        symlink(repo.path().join("real"), repo.path().join("linked-dir")).unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        storage.configure_repository_layout(&layout).unwrap();

        assert_eq!(
            storage.inspect_artifact_location("leaf.md").unwrap(),
            ArtifactLocation::Symlink
        );
        assert_eq!(
            storage
                .inspect_artifact_location("linked-dir/file.md")
                .unwrap(),
            ArtifactLocation::Symlink
        );
        assert_eq!(
            storage.inspect_artifact_location("missing.md").unwrap(),
            ArtifactLocation::Missing
        );
        assert!(matches!(
            storage.inspect_artifact_location("real/file.md").unwrap(),
            ArtifactLocation::Regular(_)
        ));
        assert_eq!(
            storage
                .inspect_artifact_location("directory-artifact")
                .unwrap(),
            ArtifactLocation::Unsupported
        );
        assert!(matches!(
            storage.inspect_artifact_location("real/file.md/child"),
            Err(crate::storage::PathReadError::Other(_))
        ));
    }
}

/// The tombstones of the index at `path`; an absent index holds none.
fn read_tombstones(path: &Path) -> Result<Vec<IssueId>> {
    match fs::read(path) {
        Ok(bytes) => Ok(parse_repository_index(&bytes)?.deleted_ids),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => {
            Err(error).with_context(|| format!("Failed to read file: {}", path.display()))
        }
    }
}

/// Merge one store's files and tombstones into `membership`, earlier sources winning.
fn merge_store_membership(
    index: &Path,
    issues: &Path,
    membership: &mut BTreeMap<IssueId, bool>,
) -> Result<()> {
    let names = match fs::read_dir(issues) {
        Ok(entries) => entries
            .map(|entry| {
                entry?
                    .file_name()
                    .into_string()
                    .map_err(|name| anyhow!("non-UTF-8 issue file name {name:?}"))
            })
            .collect::<Result<Vec<_>>>()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read file: {}", issues.display()))
        }
    };
    crate::repository_state::listed_issue_ids(&names)?
        .into_iter()
        .map(|id| (id, true))
        .chain(read_tombstones(index)?.into_iter().map(|id| (id, false)))
        .for_each(|(id, is_active)| {
            membership.entry(id).or_insert(is_active);
        });
    Ok(())
}

/// Parse and validate one captured `index.json` without consulting ambient state.
pub(crate) fn parse_repository_index(bytes: &[u8]) -> Result<Index> {
    match RepositoryIndex::parse(bytes) {
        Ok(index) => Ok(index),
        Err(RepositoryIndexError::UnsupportedVersion { found, supported }) => {
            Err(RepositoryFormatUnsupportedError::new(found, supported).into())
        }
        Err(error) => Err(anyhow!(error)),
    }
}

/// File-backed repository reader and mutation-session provider.
///
/// This implementation stores each issue as a separate JSON file in `.jit/issues/`,
/// events in `.jit/events.jsonl`, and gate definitions in `.jit/gates.toml` — a
/// `[[gates]]` array-of-tables parsed through [`crate::storage::gate_store`].
/// Publication is owned by its [`RepositoryStateStore`] implementation, which
/// applies complete repository deltas through one redo record.
///
/// A mutation session retains the repository-sibling bootstrap lock followed by
/// the repository write lock ([`RepoWriteLock`], `.repo-write.lock`) across
/// recovery, capture, and apply. Typed readers retain their narrower shared
/// locks; event-log publication takes the matching exclusive event lock only
/// around recovery or apply.
#[derive(Clone)]
pub struct JsonFileStorage {
    root: PathBuf,
    locker: FileLocker,
    /// Repository-sibling lock acquired before `.jit/.repo-write.lock`.
    ///
    /// It deliberately lives outside `.jit/`: a session over an absent data
    /// root retains this guard while that root does not exist yet, and a lock
    /// inode inside the root would be a different inode once it is published.
    bootstrap_lock: Arc<RepoWriteLock>,
    /// Shared by every clone of this instance, so a nested write inside a
    /// sequence that already holds the lock reenters it instead of deadlocking.
    repo_lock: Arc<RepoWriteLock>,
    /// Identity of this storage's thread-local retained-session slot.
    retention_id: u64,
    /// Lifecycle state of the canonical retained mutation session.
    ///
    /// The session itself is deliberately not shared or `Send`: its lock-order
    /// guard is thread-scoped. Clones use this metadata to fail cross-thread
    /// access instead of silently running outside the retained boundary.
    retained_state: Arc<Mutex<RetainedSessionState>>,
    /// Interruption seam for the session's own moments: its open, its
    /// pre-commit check, and the steps of its redo-record application.
    mutation_interruptions: Arc<dyn crate::storage::MutationInterruptionInjector>,
    /// Canonical layout of the live mutation session, shared by every clone so
    /// reentry is admitted only for the same selected roots.
    active_mutation_layout: Arc<crate::storage::repository_state_store::ActiveLayoutTracker>,
    /// Explicit worktree/data-root authority for repository-relative readers.
    repository_layout: Arc<Mutex<Option<RepositoryLayout>>>,
    /// Authoritative Git/non-Git classification for the selected data root.
    worktree_paths: Arc<Mutex<Option<WorktreePaths>>>,
    /// Shared commit-keyed pinned evidence retained for the most recent capture.
    pinned_target_cache: Arc<Mutex<BTreeMap<(String, String), crate::storage::PinnedTargetRead>>>,
    /// Pinned-document evidence built by the most recent capture that pinned
    /// any document, keyed by request. Evidence is a pure function of the
    /// request and the Git objects it resolved to, so a later capture reuses it
    /// while the request resolves to the same commit and object, skipping the
    /// content hashing that dominates large pinned documents.
    pinned_evidence_cache: Arc<Mutex<PinnedEvidenceCache>>,
    git_program: PathBuf,
}

impl JsonFileStorage {
    /// Create a new JSON file storage instance at the given root path.
    /// The root should be the `.jit` directory (or custom directory from JIT_DATA_DIR).
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        Self::with_lock_timeout(root, crate::storage::lock::configured_lock_timeout())
    }

    /// Create storage whose every lock waits `timeout` instead of the
    /// configured wait.
    fn with_lock_timeout<P: AsRef<Path>>(root: P, timeout: std::time::Duration) -> Self {
        let root = root.as_ref().to_path_buf();
        let bootstrap_lock_path = root
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(BOOTSTRAP_LOCK_FILE);
        // Shared by canonical path so a nested repo's data-root-parent bootstrap
        // lock is the very same reentrant instance as its worktree-root bootstrap
        // lock (they name one file), while a disjoint data root's parent lock is a
        // distinct instance held beneath the separate worktree bootstrap lock.
        let bootstrap_lock = RepoWriteLock::shared_for_lock_path(bootstrap_lock_path, timeout);
        Self {
            repo_lock: RepoWriteLock::for_storage_root_after(
                &root,
                timeout,
                Some(Arc::clone(&bootstrap_lock)),
            ),
            bootstrap_lock,
            retention_id: NEXT_RETENTION_ID.fetch_add(1, Ordering::Relaxed),
            retained_state: Arc::new(Mutex::new(RetainedSessionState::Idle)),
            mutation_interruptions: Arc::new(crate::storage::NoMutationInterruptions),
            active_mutation_layout: Arc::new(Default::default()),
            repository_layout: Arc::new(Mutex::new(None)),
            worktree_paths: Arc::new(Mutex::new(None)),
            pinned_target_cache: Arc::new(Mutex::new(BTreeMap::new())),
            pinned_evidence_cache: Arc::new(Mutex::new(BTreeMap::new())),
            git_program: PathBuf::from("git"),
            root,
            locker: FileLocker::new(timeout),
        }
    }

    /// Construct storage whose mutation sessions are interrupted at the moments
    /// `interruptions` selects.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_mutation_interruptions<P: AsRef<Path>>(
        root: P,
        interruptions: Arc<dyn crate::storage::MutationInterruptionInjector>,
    ) -> Self {
        let mut storage = Self::new(root);
        storage.mutation_interruptions = interruptions;
        storage
    }

    #[cfg(test)]
    pub(crate) fn with_git_program<P: AsRef<Path>, G: Into<PathBuf>>(
        root: P,
        git_program: G,
    ) -> Self {
        let mut storage = Self::new(root);
        storage.git_program = git_program.into();
        storage
    }

    pub(crate) fn inspect_pinned_targets(
        &self,
        repo_root: &Path,
        requests: &BTreeSet<(String, String)>,
        kind_only: &BTreeSet<(String, String)>,
    ) -> BTreeMap<
        (String, String),
        Result<crate::storage::PinnedTargetRead, crate::storage::GitRevisionError>,
    > {
        let mut cache = self
            .pinned_target_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        crate::storage::GitRevisionResolver::with_git_program(repo_root, self.git_program.clone())
            .inspect_pinned_targets(requests, kind_only, &mut cache)
    }

    /// Exclusive access to the pinned-evidence cache for one capture.
    pub(crate) fn pinned_evidence_cache(&self) -> std::sync::MutexGuard<'_, PinnedEvidenceCache> {
        self.pinned_evidence_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(test)]
    pub(crate) fn pinned_target_cache_keys(&self) -> Vec<(String, String)> {
        self.pinned_target_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .keys()
            .cloned()
            .collect()
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

    pub(crate) fn configured_layout(&self) -> Result<RepositoryLayout> {
        if let Some(layout) = self
            .repository_layout
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            return Ok(layout);
        }
        anyhow::bail!("no repository layout configured for storage reads")
    }

    fn path_layout(&self) -> Result<RepositoryLayout> {
        if let Ok(layout) = self.configured_layout() {
            return Ok(layout);
        }
        let worktree_root = self.root.parent().unwrap_or_else(|| Path::new("."));
        super::discover_repository_layout(worktree_root, &self.root).map_err(Into::into)
    }

    fn resolve_virtual_path(&self, path: &VirtualPath) -> Result<PathBuf> {
        Ok(self.path_layout()?.resolve(path)?)
    }

    /// The root `path` is relative to.
    fn class_root(&self, path: &VirtualPath) -> Result<PathBuf> {
        Ok(match path.root_class() {
            RepositoryRootClass::Data => self.root.clone(),
            RepositoryRootClass::Worktree => self.path_layout()?.worktree_root().to_path_buf(),
        })
    }

    fn primary_layout(&self) -> Result<Option<RepositoryLayout>> {
        let paths = self.configured_worktree_paths()?;
        let Some(worktree_root) = paths.primary_worktree_root()? else {
            return Ok(None);
        };
        let Some(data_root) = paths.primary_data_root()? else {
            return Ok(None);
        };
        super::discover_repository_layout(worktree_root, data_root)
            .map(Some)
            .map_err(Into::into)
    }

    /// Bind the invocation's authoritative classification of the selected store.
    pub fn configure_worktree_paths(&self, paths: &WorktreePaths) {
        let mut configured = self
            .worktree_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match configured.as_ref() {
            None => *configured = Some(paths.clone()),
            Some(current) if current == paths => *configured = Some(paths.clone()),
            Some(_) => {}
        }
    }

    pub(crate) fn configured_worktree_paths(&self) -> Result<WorktreePaths> {
        let configured = self
            .worktree_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(paths) = configured {
            return Ok(paths);
        }
        let layout = self.configured_layout()?;
        let detected =
            WorktreePaths::detect_for_data_root(layout.data_root(), layout.worktree_root())?;
        self.configure_worktree_paths(&detected);
        Ok(self
            .worktree_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .unwrap_or(detected))
    }

    /// Open and retain this storage's canonical startup mutation session.
    ///
    /// The session is opened from `self` for exactly `layout`; callers cannot
    /// inject a foreign session. It remains on the opening thread because its
    /// repository lock-order state is thread-scoped.
    pub fn open_and_retain_mutation_session(
        &self,
        layout: RepositoryLayout,
    ) -> Result<RetainedMutationSessionGuard> {
        let current = std::thread::current().id();
        let mut state = self
            .retained_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *state != RetainedSessionState::Idle {
            anyhow::bail!("A mutation session is already retained for this storage");
        }
        let session = self.open_mutation_session(layout)?;
        let report = session.recovery_report().clone();
        let inserted = RETAINED_MUTATION_SESSIONS.with(|sessions| {
            let mut sessions = sessions.borrow_mut();
            match sessions.entry(self.retention_id) {
                Entry::Vacant(entry) => {
                    entry.insert(session);
                    true
                }
                Entry::Occupied(_) => false,
            }
        });
        if !inserted {
            anyhow::bail!("A mutation session slot is already occupied for this storage");
        }
        *state = RetainedSessionState::Active(current);
        Ok(RetainedMutationSessionGuard {
            retention_id: self.retention_id,
            state: Arc::clone(&self.retained_state),
            recovery_report: report,
            active: true,
            _not_send: std::marker::PhantomData,
        })
    }

    /// Acquire only the repository-sibling bootstrap lock.
    ///
    /// A session uses this before touching `.jit/`, because the root itself may
    /// be what the session publishes.
    pub(crate) fn acquire_bootstrap_write_lock(&self) -> Result<RepoWriteGuard> {
        self.bootstrap_lock.acquire()
    }

    /// Acquire the bootstrap lock keyed at the WORKTREE root, serializing the
    /// worktree-side namespace across every session sharing the worktree
    /// regardless of its data root. For a nested repo this is the same reentrant
    /// instance as [`acquire_bootstrap_write_lock`]; for a disjoint data root it
    /// is a distinct, outer lock that prevents two such sessions from
    /// concurrently publishing into one worktree.
    pub(crate) fn acquire_worktree_bootstrap_lock(
        &self,
        worktree_root: &Path,
    ) -> Result<RepoWriteGuard> {
        RepoWriteLock::shared_for_lock_path(
            worktree_root.join(BOOTSTRAP_LOCK_FILE),
            crate::storage::lock::configured_lock_timeout(),
        )
        .acquire()
    }

    /// Acquire the repository lock after the bootstrap lock.
    ///
    /// The lock instance is shared by every storage clone, allowing startup
    /// recovery to retain it while command-layer writes re-enter it.
    pub(crate) fn acquire_repo_write_lock_raw(&self) -> Result<RepoWriteGuard> {
        self.repo_lock.acquire()
    }

    /// Acquire the event-log lock after the repository write lock.
    ///
    /// Repository-state publication appends to the event log through its redo
    /// record, so typed event readers are excluded while those bytes are
    /// applied.
    pub(crate) fn acquire_events_write_lock(&self) -> Result<crate::storage::lock::LockGuard> {
        self.locker
            .lock_exclusive(&self.root.join(EVENTS_LOCK_FILE))
    }

    pub(crate) fn events_lock_spec(&self) -> (FileLocker, PathBuf) {
        (self.locker.clone(), self.root.join(EVENTS_LOCK_FILE))
    }

    /// Check if the storage directory exists and is initialized.
    /// Returns an error with a helpful message if not.
    pub fn validate(&self) -> Result<()> {
        if !self.root.exists() {
            return Err(RepositoryNotFoundError::new(self.root.display().to_string()).into());
        }

        let index_path = self.resolve_virtual_path(&VirtualPath::INDEX)?;
        if !index_path.exists() {
            anyhow::bail!(
                "JIT repository at '{}' is not properly initialized (missing index.json)\n\n\
                 Initialize with: jit init",
                self.root.display()
            );
        }

        // `validate()` runs for every non-init command, so this version check guards
        // even commands that never load the index themselves (jit:def64ac4).
        self.load_index()?;

        Ok(())
    }

    /// Inspect one working-tree artifact without following symbolic links.
    ///
    /// Every existing path component is checked with `symlink_metadata` before
    /// bytes are read. This is stricter than general repository reads, which
    /// permit in-repository symlinks, because archival planning must classify
    /// roots, destinations, and traversals as `symlink-artifact` rather than
    /// silently operating on their referents. Existing non-regular leaves are
    /// returned as `Unsupported`; metadata and traversal failures remain typed
    /// storage errors.
    pub fn inspect_artifact_location(
        &self,
        path: &str,
    ) -> Result<crate::domain::artifact_classifier::ArtifactLocation, crate::storage::PathReadError>
    {
        use crate::domain::artifact_classifier::ArtifactLocation;
        use crate::domain::artifact_discovery::{ArtifactEvidence, ArtifactListingScope};
        use crate::domain::artifact_plan::ContentIdentity;
        crate::storage::validate_repo_relative_path(path)?;
        match crate::storage::artifact_planning::inspect_artifact_evidence(
            self,
            path,
            ArtifactListingScope::MetadataOnly,
        )? {
            ArtifactEvidence::Missing => Ok(ArtifactLocation::Missing),
            ArtifactEvidence::Symlink => Ok(ArtifactLocation::Symlink),
            ArtifactEvidence::File(bytes) => Ok(ArtifactLocation::Regular(
                ContentIdentity::from_bytes(&bytes),
            )),
            ArtifactEvidence::Directory { .. }
            | ArtifactEvidence::Unsupported
            | ArtifactEvidence::InvalidPath => Ok(ArtifactLocation::Unsupported),
        }
    }

    fn read_issue_record(&self, path: &Path) -> Result<Issue> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", path.display()))?;
        serde_json::from_str(&contents)
            .with_context(|| format!("Failed to deserialize issue record at {}", path.display()))
    }

    fn load_index(&self) -> Result<Index> {
        let index_path = self.resolve_virtual_path(&VirtualPath::INDEX)?;
        let bytes = fs::read(&index_path)
            .with_context(|| format!("Failed to read file: {}", index_path.display()))?;
        parse_repository_index(&bytes)
    }

    /// The active ids of this checkout's store layered over the primary checkout's.
    ///
    /// The first source that mentions an id wins, and within a source a file beats
    /// a tombstone, so a local delete hides the primary's file and a local file
    /// survives any tombstone. Only an absent file or directory is skipped; any
    /// other failure aborts rather than exposing a plausible but incomplete set.
    fn load_aggregated_issue_ids(&self) -> Result<Vec<IssueId>> {
        let mut membership = BTreeMap::new();
        merge_store_membership(
            &self.resolve_virtual_path(&VirtualPath::INDEX)?,
            &self.resolve_virtual_path(&VirtualPath::ISSUES)?,
            &mut membership,
        )?;
        if let Some(primary) = self.primary_layout()? {
            merge_store_membership(
                &primary.resolve(&VirtualPath::INDEX)?,
                &primary.resolve(&VirtualPath::ISSUES)?,
                &mut membership,
            )?;
        }
        Ok(membership
            .into_iter()
            .filter_map(|(id, is_active)| is_active.then_some(id))
            .collect())
    }

    /// Load every issue the aggregated membership names, under an
    /// already-held `.index.lock`.
    ///
    /// An id whose record vanished since membership was read is skipped; a
    /// record that exists and cannot be read or parsed fails the listing,
    /// naming it, so a listing never hides an issue.
    fn read_member_issues(&self) -> Result<Vec<Issue>> {
        self.load_aggregated_issue_ids()?
            .iter()
            .filter_map(|id| match self.load_issue(id) {
                Err(error) if error.downcast_ref::<IssueNotFoundError>().is_some() => None,
                loaded => Some(loaded),
            })
            .collect()
    }

    /// Load an issue from the main worktree's `.jit/` directory.
    ///
    /// Returns `Ok(Some(issue))` when found, `Ok(None)` when this store's tombstone
    /// hides it, when the issue is absent, or when this worktree setup does not have
    /// an accessible main worktree (not in git, already in main worktree,
    /// non-standard layout), and `Err` for genuine I/O or parse failures on a file
    /// that does exist.
    fn load_issue_from_main_worktree(&self, id: &IssueId) -> Result<Option<Issue>> {
        let Some(layout) = self.primary_layout()? else {
            return Ok(None);
        };
        if read_tombstones(&self.resolve_virtual_path(&VirtualPath::INDEX)?)?.contains(id) {
            return Ok(None);
        }
        let main_issue_path = layout.resolve(&VirtualPath::issue_record(id))?;

        if !main_issue_path.exists() {
            return Ok(None); // issue absent from main worktree
        }

        // File exists — read it; any I/O or parse error is a real failure.
        self.read_issue_record(&main_issue_path).map(Some)
    }
}

impl super::gate_run_reader::GateRunTree for JsonFileStorage {
    fn list_directory(&self, directory: &VirtualPath) -> Result<TreeNode<Vec<String>>> {
        let root = self.class_root(directory)?;
        let relative = directory.relative().as_path();
        let opened =
            super::capability_fs::open_absolute_dir_nofollow(&root).and_then(|root| match relative
                .as_os_str()
                .is_empty()
            {
                true => Ok(Some(root)),
                false => super::capability_fs::open_capability_parent(&root, relative)?
                    .map(|(parent, leaf)| {
                        super::capability_fs::open_child_dir_nofollow(&parent, leaf)
                    })
                    .transpose(),
            });
        let names = |directory: cap_std::fs::Dir| -> Result<Vec<String>> {
            directory
                .entries()?
                .filter_map(|entry| match entry {
                    Ok(entry) => entry.file_name().into_string().ok().map(Ok),
                    Err(error) => Some(Err(error.into())),
                })
                .collect()
        };
        match tree_node(opened)? {
            TreeNode::Present(directory) => names(directory).map(TreeNode::Present),
            TreeNode::Absent => Ok(TreeNode::Absent),
            TreeNode::OtherKind => Ok(TreeNode::OtherKind),
        }
    }

    fn read_regular_file(&self, file: &VirtualPath) -> Result<TreeNode<Vec<u8>>> {
        tree_node(super::capability_fs::read_repository_file_nofollow(
            &self.class_root(file)?,
            file.relative(),
        ))
    }
}

/// Classify a no-follow read: a missing path or ancestor is absent, and a
/// target refused as unsafe is an occupant of another kind.
fn tree_node<T>(
    read: Result<Option<T>, super::capability_fs::CapabilityFsError>,
) -> Result<TreeNode<T>> {
    match read {
        Ok(Some(found)) => Ok(TreeNode::Present(found)),
        Ok(None) => Ok(TreeNode::Absent),
        Err(super::capability_fs::CapabilityFsError::Io(error))
            if error.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(TreeNode::Absent)
        }
        Err(super::capability_fs::CapabilityFsError::UnsafeTarget(_)) => Ok(TreeNode::OtherKind),
        Err(error) => Err(error.into()),
    }
}

impl IssueStore for JsonFileStorage {
    fn configure_repository_layout(
        &self,
        layout: &RepositoryLayout,
    ) -> std::result::Result<(), crate::storage::RepositoryStateStoreError> {
        // Bind once. A refreshed layout for the same roots may replace stale
        // root-presence evidence after this process publishes an absent root;
        // a clone can never redirect readers to different roots.
        if self.root != layout.data_root() {
            return Err(
                crate::storage::RepositoryStateStoreError::RetryableConflict {
                    path: layout.data_root().display().to_string(),
                },
            );
        }
        let mut configured = self
            .repository_layout
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match configured.as_ref() {
            None => *configured = Some(layout.clone()),
            Some(current)
                if current.worktree_root() == layout.worktree_root()
                    && current.data_root() == layout.data_root() =>
            {
                *configured = Some(layout.clone());
            }
            Some(_) => {
                return Err(
                    crate::storage::RepositoryStateStoreError::RetryableConflict {
                        path: layout.worktree_root().display().to_string(),
                    },
                )
            }
        }
        Ok(())
    }

    fn repository_layout(&self) -> Result<RepositoryLayout> {
        self.configured_layout()
    }

    fn acquire_repo_write_lock(&self) -> Result<RepoWriteGuard> {
        self.repo_lock.acquire()
    }

    fn run_external_process<T>(&self, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        let current = std::thread::current().id();
        {
            let mut state = self
                .retained_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match *state {
                RetainedSessionState::Idle => {
                    drop(state);
                    return operation();
                }
                RetainedSessionState::Active(owner) if owner == current => {
                    *state = RetainedSessionState::Suspended(current);
                }
                RetainedSessionState::Suspended(_) => {
                    return Err(RetainedSessionSuspendedError.into());
                }
                RetainedSessionState::Active(_) => {
                    anyhow::bail!(
                        "An external process can only suspend a retained mutation session on its opening thread"
                    )
                }
            }
        }
        let session = RETAINED_MUTATION_SESSIONS
            .with(|sessions| sessions.borrow_mut().remove(&self.retention_id));
        let Some(session) = session else {
            let mut state = self
                .retained_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *state = RetainedSessionState::Idle;
            anyhow::bail!("The retained mutation session is missing from its opening thread");
        };
        let mut suspension = RetainedSessionSuspension {
            state: Arc::clone(&self.retained_state),
            owner: current,
            active: true,
        };

        // External checkers may invoke mutating jit subprocesses. Release the
        // process-local session guards before spawning them so those subprocesses
        // acquire the ordinary cross-process bootstrap → repository chain.
        let layout = session.layout().clone();
        drop(session);
        let operation_result = operation();

        // Re-establish the boundary before the caller can persist a verdict.
        // This also applies a record left by a checker-side mutation that
        // crashed after committing it. The external
        // process has already finished, so a lock wait that expires is retried
        // rather than discarding its outcome: another jit process holds the
        // chain, and that holder is itself a bounded operation.
        let reopened = loop {
            match self.open_mutation_session(layout.clone()) {
                Err(error) if crate::storage::repository_state_store::is_lock_timeout(&error) => {}
                attempt => break attempt,
            }
        };
        match reopened {
            Ok(session) => {
                let inserted = RETAINED_MUTATION_SESSIONS.with(|sessions| {
                    let mut sessions = sessions.borrow_mut();
                    match sessions.entry(self.retention_id) {
                        Entry::Vacant(entry) => {
                            entry.insert(session);
                            true
                        }
                        Entry::Occupied(_) => false,
                    }
                });
                if !inserted {
                    anyhow::bail!("The retained mutation session slot became occupied");
                }
                let mut state = self
                    .retained_state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if *state != RetainedSessionState::Suspended(current) {
                    RETAINED_MUTATION_SESSIONS
                        .with(|sessions| sessions.borrow_mut().remove(&self.retention_id));
                    anyhow::bail!("The retained mutation session suspension was lost");
                }
                *state = RetainedSessionState::Active(current);
                suspension.restored();
                operation_result
            }
            Err(recovery_error) => {
                let context = match operation_result {
                    Ok(_) => "Failed to restore recovery serialization after external process"
                        .to_string(),
                    Err(operation_error) => format!(
                        "Failed to restore recovery serialization after external process; \
                         the external process also failed: {operation_error:#}"
                    ),
                };
                Err(anyhow::Error::new(recovery_error).context(context))
            }
        }
    }

    fn load_issue(&self, id: &IssueId) -> Result<Issue> {
        // Try the selected data-root record first.
        let issue_path = self.resolve_virtual_path(&VirtualPath::issue_record(id))?;
        if issue_path.exists() {
            // Read directly, taking no per-issue lock. Issue JSON is published by
            // atomic temp-file-plus-rename (see atomic_write), so a reader can never
            // observe a torn file; writers serialize on `.repo-write.lock`, and
            // `list_issues` holds the repo-scoped `.index.lock` shared around its
            // load loop. A `<id>.lock` sidecar was therefore redundant and, created
            // once per issue ever read, grew the lock-file set without bound.
            //
            // Caller audit (fc744df6): production consumers use the returned owned
            // value for display, resolution, validation, or as a mutation snapshot.
            // Mutations publish through `RepositoryStateStore`, which compares the
            // files it writes under the repository write lock. The old `_lock` guard
            // was local and dropped before this method returned, so no caller ever
            // inherited serialization from it.
            return self.read_issue_record(&issue_path);
        }

        // Fallback: try reading from main worktree (if in secondary)
        match self.load_issue_from_main_worktree(id) {
            Ok(Some(issue)) => return Ok(issue),
            Ok(None) => {} // absent from main worktree or not applicable; continue
            Err(e) => return Err(e), // genuine I/O/parse failure
        }

        // Issue not found in any source
        Err(IssueNotFoundError::across_sources(id).into())
    }

    fn load_issue_or_not_found(
        &self,
        id: &IssueId,
    ) -> Result<Issue, crate::storage::PathReadError> {
        use crate::storage::PathReadError;

        let issue_path = self
            .resolve_virtual_path(&VirtualPath::issue_record(id))
            .map_err(PathReadError::Other)?;

        if issue_path.exists() {
            // File exists — delegate to load_issue for source selection and
            // deserialization; any remaining error is a genuine I/O/parse
            // failure → PathReadError::Other.
            return self.load_issue(id).map_err(PathReadError::Other);
        }

        // Main-worktree fallback (only meaningful in secondary-worktree setups).
        match self.load_issue_from_main_worktree(id) {
            Ok(Some(issue)) => return Ok(issue),
            Ok(None) => {} // absent or not applicable; fall through to NotFound
            Err(e) => return Err(PathReadError::Other(e)), // genuine I/O failure → 500
        }

        Err(PathReadError::NotFound(
            IssueNotFoundError::across_sources(id).to_string(),
        ))
    }

    fn resolve_issue_id(&self, prefix: &IssueIdPrefix) -> Result<crate::domain::identity::IssueId> {
        let ids = self.load_aggregated_issue_ids()?;
        crate::storage::resolve_unique_issue_id(prefix, ids.iter(), |id| {
            self.load_issue(id)
                .ok()
                .map(|issue| crate::storage::describe_issue(&issue))
        })
    }

    fn list_issues(&self) -> Result<Vec<Issue>> {
        let index_lock_path = self.root.join(INDEX_LOCK_FILE);
        let _lock = self.locker.lock_shared(&index_lock_path)?;

        self.read_member_issues()
    }

    fn load_gate_registry(&self) -> Result<GateRegistry> {
        // Lock-free (`@/charter/D-22`): the registry is replaced by atomic rename.
        crate::storage::gate_store::load_gate_registry(&self.path_layout()?)
    }

    fn read_events(&self) -> Result<Vec<Event>> {
        let events_path = self.resolve_virtual_path(&VirtualPath::EVENTS)?;
        if !events_path.exists() {
            return Ok(Vec::new());
        }

        // Lock-free (`@/charter/D-22`): the parser ignores a partly written final line.
        let contents = fs::read(&events_path).context("Failed to read events file")?;
        parse_known_events(&contents).context("Failed to deserialize event log")
    }

    fn read_archive_evidence_record(
        &self,
        target: &crate::domain::artifact_plan::PlanTarget,
    ) -> Result<Option<crate::domain::artifact_execution::ArchiveEvidenceRecord>> {
        let path = VirtualPath::archive_evidence_record(target);
        let contents = match crate::storage::capability_fs::read_repository_file_confined(
            &self.root,
            path.relative(),
        ) {
            Ok(contents) => contents,
            Err(super::capability_fs::CapabilityFsError::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                None
            }
            Err(error) => return Err(error.into()),
        };
        contents
            .map(|contents| {
                serde_json::from_slice(&contents).with_context(|| {
                    format!(
                        "Failed to deserialize archive evidence record at {}",
                        path.relative().as_path().display()
                    )
                })
            })
            .transpose()
    }

    fn load_gate_run_result(
        &self,
        issue_id: &IssueId,
        run_id: &GateRunId,
    ) -> Result<crate::domain::GateRunResult> {
        super::gate_run_reader::load_gate_run_result(self, issue_id, run_id)
    }

    fn list_gate_runs_for_issue(
        &self,
        issue_id: &IssueId,
    ) -> Result<Vec<crate::domain::GateRunResult>> {
        super::gate_run_reader::list_gate_runs_for_issue(self, issue_id)
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn is_file_backed(&self) -> bool {
        true
    }

    fn read_repo_file(
        &self,
        rel_path: &str,
    ) -> Result<Option<String>, crate::storage::PathReadError> {
        use crate::storage::PathReadError;
        crate::storage::validate_repo_relative_path(rel_path)?;
        let layout = self.configured_layout().map_err(PathReadError::Other)?;
        let path = layout
            .classify_repository_relative(rel_path)
            .map_err(|error| PathReadError::InvalidPath(error.to_string()))?;
        let root = match path.root_class() {
            crate::repository_state::RepositoryRootClass::Worktree => layout.worktree_root(),
            crate::repository_state::RepositoryRootClass::Data => layout.data_root(),
        };
        crate::storage::capability_fs::read_repository_file_confined(root, path.relative())
            .map_err(|error| map_confined_read_error(error, rel_path))?
            .map(|bytes| {
                String::from_utf8(bytes)
                    .map_err(anyhow::Error::from)
                    .map_err(PathReadError::Other)
            })
            .transpose()
    }

    fn list_gate_presets(&self) -> Result<Vec<crate::gate_presets::PresetInfo>> {
        let manager = crate::gate_presets::PresetManager::new(self.configured_layout()?)?;
        Ok(manager.list_presets())
    }

    fn get_gate_preset(&self, name: &str) -> Result<crate::gate_presets::GatePresetDefinition> {
        let manager = crate::gate_presets::PresetManager::new(self.configured_layout()?)?;
        let preset = manager.get_preset(name)?;
        Ok(preset.clone())
    }

    fn read_path_bytes(
        &self,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<(Vec<u8>, String), crate::storage::PathReadError> {
        use crate::storage::PathReadError;

        // Uniform repo-relative path validation (applied for both git-commit
        // reads and working-tree reads): reject empty paths, absolute paths,
        // and any `..` segment used for traversal.  This moves the invariant
        // out of individual route handlers and into the storage boundary so
        // every caller inherits it.
        crate::storage::validate_repo_relative_path(path)?;

        let layout = self.configured_layout().map_err(PathReadError::Other)?;
        let repo_root = layout.worktree_root();

        if let Some(commit_ref) = at_commit {
            use git2::Repository;

            let repo = Repository::open(repo_root).map_err(|e| {
                PathReadError::Other(anyhow!("Failed to open git repository: {}", e))
            })?;

            let commit_obj = repo
                .revparse_single(commit_ref)
                .map_err(|_| PathReadError::CommitNotFound(commit_ref.to_string()))?;

            let commit = commit_obj
                .peel_to_commit()
                .map_err(|e| PathReadError::Other(anyhow!("Failed to peel to commit: {}", e)))?;

            let tree = commit
                .tree()
                .map_err(|e| PathReadError::Other(anyhow!("Failed to get commit tree: {}", e)))?;

            let entry = tree
                .get_path(std::path::Path::new(path))
                .map_err(|_| PathReadError::NotFound(path.to_string()))?;

            let blob = repo
                .find_blob(entry.id())
                .map_err(|e| PathReadError::Other(anyhow!("Failed to read blob: {}", e)))?;

            let short_hash = format!("{:.7}", commit.id());
            Ok((blob.content().to_vec(), short_hash))
        } else {
            let relative = crate::domain::relative_path::RootRelativePath::parse(path)
                .map_err(|error| PathReadError::InvalidPath(error.to_string()))?;
            crate::storage::capability_fs::read_repository_file_confined(repo_root, &relative)
                .map_err(|error| map_confined_read_error(error, path))?
                .map(|bytes| (bytes, "working-tree".to_string()))
                .ok_or_else(|| PathReadError::NotFound(path.to_string()))
        }
    }
}

fn map_confined_read_error(
    error: super::capability_fs::CapabilityFsError,
    relative: &str,
) -> crate::storage::PathReadError {
    use crate::storage::PathReadError;
    match error {
        super::capability_fs::CapabilityFsError::UnsafeTarget(_) => {
            PathReadError::OutsideRepoRoot(relative.to_string())
        }
        super::capability_fs::CapabilityFsError::Io(error)
            if error.kind() == std::io::ErrorKind::NotFound =>
        {
            PathReadError::NotFound(relative.to_string())
        }
        error => PathReadError::Other(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::identity::IssueIdPrefix;
    use crate::repository_state::SUPPORTED_INDEX_SCHEMA_VERSION;
    use crate::storage::IssueStore;
    use tempfile::TempDir;

    fn configure_test_layout(storage: &JsonFileStorage, worktree: &Path, data: &Path) {
        let layout = crate::storage::discover_repository_layout(worktree, data).unwrap();
        storage.configure_repository_layout(&layout).unwrap();
    }

    #[test]
    fn test_acquire_bootstrap_locks_over_absent_data_root_create_no_data_root() {
        let temp = TempDir::new().unwrap();
        let data_root = temp.path().join(".jit");
        let storage = JsonFileStorage::new(&data_root);

        let _worktree = storage
            .acquire_worktree_bootstrap_lock(temp.path())
            .unwrap();
        let _bootstrap = storage.acquire_bootstrap_write_lock().unwrap();

        assert!(!data_root.exists());
    }

    #[test]
    fn test_layout_binding_rejects_different_roots_but_accepts_same_root_refresh() {
        let worktree = TempDir::new().unwrap();
        let other_worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        let storage = JsonFileStorage::new(&data);
        let absent = crate::storage::discover_repository_layout(worktree.path(), &data).unwrap();
        storage.configure_repository_layout(&absent).unwrap();

        let foreign =
            crate::storage::discover_repository_layout(other_worktree.path(), &data).unwrap();
        assert!(storage.configure_repository_layout(&foreign).is_err());
        assert!(!data.exists());
        assert!(matches!(
            storage.open_mutation_session(foreign),
            Err(crate::storage::RepositoryStateStoreError::RetryableConflict { .. })
        ));

        std::fs::create_dir(&data).unwrap();
        let present = crate::storage::discover_repository_layout(worktree.path(), &data).unwrap();
        storage.configure_repository_layout(&present).unwrap();
        assert_eq!(
            storage.configured_layout().unwrap().data_identity(),
            present.data_identity()
        );
    }

    #[test]
    fn test_external_process_reopens_retained_canonical_session() {
        let (temp, storage, _taxonomy) =
            crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
        let data = temp.path().join(".jit");
        let layout = crate::storage::discover_repository_layout(temp.path(), &data).unwrap();
        let retained = storage
            .open_and_retain_mutation_session(layout.clone())
            .unwrap();

        let competing = RepoWriteLock::for_lock_path(
            temp.path().join(".jit-bootstrap.lock"),
            std::time::Duration::from_millis(100),
        );
        storage
            .run_external_process(|| {
                let _guard = competing.acquire()?;
                Ok(())
            })
            .unwrap();
        assert!(storage
            .run_external_process::<()>(|| anyhow::bail!("checker failed"))
            .is_err());

        assert!(
            competing.acquire().is_err(),
            "the canonical session must be retained again before returning"
        );
        drop(retained);
    }

    #[test]
    fn test_run_external_process_keeps_finished_result_when_reopen_waits_past_timeout() {
        let (temp, storage, _taxonomy) =
            crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
        // The bootstrap lock is shared per path, so the fixture's instance must
        // be gone before the short-wait storage registers its own.
        let root = storage.root().to_path_buf();
        drop(storage);
        let storage =
            JsonFileStorage::with_lock_timeout(root, std::time::Duration::from_millis(50));
        let data = temp.path().join(".jit");
        let layout = crate::storage::discover_repository_layout(temp.path(), &data).unwrap();
        let retained = storage.open_and_retain_mutation_session(layout).unwrap();
        let bootstrap = temp.path().join(".jit-bootstrap.lock");
        let (acquired, holding) = std::sync::mpsc::channel();

        let result = storage.run_external_process(|| {
            // Another jit process takes the lock chain while the checker runs
            // and holds it for many lock waits after the checker finishes.
            let holder = std::thread::spawn(move || {
                let competing =
                    RepoWriteLock::for_lock_path(bootstrap, std::time::Duration::from_secs(5));
                let guard = competing.acquire().unwrap();
                acquired.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(400));
                drop(guard);
            });
            holding.recv().unwrap();
            drop(holder);
            Ok("verdict")
        });

        assert_eq!(result.unwrap(), "verdict");
        drop(retained);
    }

    #[test]
    fn test_read_events_does_not_wait_on_a_writer_holding_the_events_lock() {
        let (temp, storage, _taxonomy) =
            crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
        fs::write(temp.path().join(".jit/events.jsonl"), "").unwrap();
        let root = storage.root().to_path_buf();
        drop(storage);
        let storage =
            JsonFileStorage::with_lock_timeout(root, std::time::Duration::from_millis(50));
        let _writer = storage.acquire_events_write_lock().unwrap();

        assert_eq!(storage.read_events().unwrap(), Vec::new());
    }

    #[test]
    fn test_retained_session_rejects_other_layout_and_thread() {
        let other_worktree = TempDir::new().unwrap();
        let (temp, storage, _taxonomy) =
            crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
        let data = temp.path().join(".jit");
        let layout = crate::storage::discover_repository_layout(temp.path(), &data).unwrap();
        let retained = storage
            .open_and_retain_mutation_session(layout.clone())
            .unwrap();

        let other_layout =
            crate::storage::discover_repository_layout(other_worktree.path(), storage.root())
                .unwrap();
        assert!(matches!(
            storage.open_mutation_session(other_layout),
            Err(crate::storage::RepositoryStateStoreError::RetryableConflict { .. })
        ));

        let other_thread = storage.clone();
        assert!(
            std::thread::spawn(move || other_thread.run_external_process(|| Ok(())))
                .join()
                .unwrap()
                .is_err()
        );
        drop(retained);
    }

    #[test]
    fn test_retained_guard_error_unwind_clears_session_and_order_state() {
        fn fail_after_retaining(storage: JsonFileStorage, layout: RepositoryLayout) -> Result<()> {
            let _retained = storage.open_and_retain_mutation_session(layout)?;
            anyhow::bail!("dispatch failed")
        }

        let temp = TempDir::new().unwrap();
        let data = temp.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        let layout = crate::storage::discover_repository_layout(temp.path(), &data).unwrap();
        let storage = JsonFileStorage::new(&data);

        assert!(fail_after_retaining(storage, layout.clone()).is_err());
        let reopened = JsonFileStorage::new(&data);
        let session = reopened.open_mutation_session(layout).unwrap();
        drop(session);
        assert!(crate::storage::guard_order::CoordinationOrderGuard::enter().is_ok());
    }

    #[test]
    fn test_nested_external_process_fails_typed_without_deadlock() {
        let (temp, storage, _taxonomy) =
            crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
        let data = temp.path().join(".jit");
        let layout = crate::storage::discover_repository_layout(temp.path(), &data).unwrap();
        let retained = storage
            .open_and_retain_mutation_session(layout.clone())
            .unwrap();

        storage
            .run_external_process(|| {
                let error = storage.run_external_process(|| Ok(())).unwrap_err();
                assert!(error
                    .downcast_ref::<RetainedSessionSuspendedError>()
                    .is_some());
                Ok(())
            })
            .unwrap();

        drop(retained);
    }

    #[test]
    fn test_external_process_panic_does_not_leave_stale_owner() {
        let (temp, storage, _taxonomy) =
            crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
        let data = temp.path().join(".jit");
        let layout = crate::storage::discover_repository_layout(temp.path(), &data).unwrap();
        let retained = storage
            .open_and_retain_mutation_session(layout.clone())
            .unwrap();

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<()> = storage.run_external_process(|| panic!("checker panic"));
        }));
        assert!(panic.is_err());
        drop(retained);

        let session = storage.open_mutation_session(layout).unwrap();
        drop(session);
        assert!(crate::storage::guard_order::CoordinationOrderGuard::enter().is_ok());
    }

    #[test]
    fn test_disjoint_layout_reads_logical_data_and_worktree_roots() {
        let worktree = TempDir::new().unwrap();
        let external = TempDir::new().unwrap();
        let data = external.path().join("selected-data");
        let data_layout = crate::test_utils::repository_layout_for_data_root(&data);
        fs::create_dir(&data).unwrap();
        fs::create_dir(worktree.path().join("docs")).unwrap();
        fs::write(worktree.path().join("docs/guide.md"), "worktree").unwrap();
        fs::write(data_layout.resolve(&VirtualPath::CONFIG).unwrap(), "data").unwrap();

        let storage = JsonFileStorage::new(&data);
        configure_test_layout(&storage, worktree.path(), &data);

        assert_eq!(
            storage.read_repo_file("docs/guide.md").unwrap().as_deref(),
            Some("worktree")
        );
        assert_eq!(
            storage
                .read_repo_file(VirtualPath::CONFIG.repository_relative().as_str())
                .unwrap()
                .as_deref(),
            Some("data")
        );
        assert_eq!(
            storage.read_path_bytes("docs/guide.md", None).unwrap().0,
            b"worktree"
        );
        assert!(!external.path().join("docs/guide.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_held_gate_run_issue_directory_capability_cannot_escape_after_parent_symlink_swaps() {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join(".jit");
        let data_layout = crate::test_utils::repository_layout_for_data_root(&data);
        let issue = crate::test_utils::issue_id(1);
        let record_path = VirtualPath::gate_run_record(&issue, &crate::test_utils::gate_run_id(1));
        let record_name = record_path.file_name().unwrap();
        let original_run = data_layout
            .resolve(&VirtualPath::gate_run_issue_directory(&issue))
            .unwrap();
        let record = data_layout.resolve(&record_path).unwrap();
        fs::create_dir_all(&original_run).unwrap();
        fs::write(&record, b"inside").unwrap();

        let outside_run = TempDir::new().unwrap();
        fs::write(outside_run.path().join(record_name), b"outside-run").unwrap();
        let outside_root = TempDir::new().unwrap();
        fs::create_dir(outside_root.path().join(issue.as_str())).unwrap();
        fs::write(
            outside_root.path().join(issue.as_str()).join(record_name),
            b"outside-root",
        )
        .unwrap();

        let data_handle = super::super::capability_fs::open_absolute_dir_nofollow(&data).unwrap();
        let gate_runs_handle = super::super::capability_fs::open_child_dir_nofollow(
            &data_handle,
            VirtualPath::GATE_RUNS.relative().as_str(),
        )
        .unwrap();
        let run_handle =
            super::super::capability_fs::open_child_dir_nofollow(&gate_runs_handle, issue.as_str())
                .unwrap();

        fs::rename(
            &original_run,
            data_layout
                .resolve(&VirtualPath::GATE_RUNS)
                .unwrap()
                .join("run-held"),
        )
        .unwrap();
        std::os::unix::fs::symlink(outside_run.path(), &original_run).unwrap();
        fs::rename(
            data_layout.resolve(&VirtualPath::GATE_RUNS).unwrap(),
            data_layout
                .resolve(&VirtualPath::GATE_RUNS)
                .unwrap()
                .with_extension("held"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            outside_root.path(),
            data_layout.resolve(&VirtualPath::GATE_RUNS).unwrap(),
        )
        .unwrap();

        assert_eq!(
            fs::read(&record).unwrap(),
            b"outside-root",
            "the ambient path must demonstrate that the parent swap took effect"
        );
        let mut held_file =
            super::super::capability_fs::open_regular_file_nofollow(&run_handle, record_name)
                .unwrap();
        let mut held_bytes = Vec::new();
        held_file.read_to_end(&mut held_bytes).unwrap();
        assert_eq!(held_bytes, b"inside");
    }

    // Tests for cross-worktree issue visibility (TDD)
    mod cross_worktree_tests {
        use super::*;
        use std::fs;
        use std::process::Command;

        fn add_secondary_worktree(repo_path: &Path) -> (TempDir, PathBuf) {
            use std::time::{SystemTime, UNIX_EPOCH};

            let container = TempDir::new().unwrap();
            let worktree_path = container.path().join("worktree");
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let branch = format!("secondary-{suffix}");
            let output = Command::new("git")
                .args(["worktree", "add", "-b", &branch])
                .arg(&worktree_path)
                .current_dir(repo_path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            (container, worktree_path)
        }

        fn seed_index_preimage(storage: &JsonFileStorage, index: &Index) {
            fs::write(
                storage
                    .path_layout()
                    .unwrap()
                    .resolve(&VirtualPath::INDEX)
                    .unwrap(),
                index.to_pretty_bytes().unwrap(),
            )
            .unwrap();
        }

        fn seed_issue_preimage(storage: &JsonFileStorage, issue: &Issue) {
            let issue_path = storage
                .path_layout()
                .unwrap()
                .resolve(&VirtualPath::issue_record(&issue.id))
                .unwrap();
            fs::create_dir_all(issue_path.parent().unwrap()).unwrap();
            fs::write(
                issue_path,
                crate::repository_state::serialize_issue(issue).unwrap(),
            )
            .unwrap();
        }

        fn physical_files(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
            fn visit(
                root: &Path,
                current: &Path,
                files: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>,
            ) {
                let mut entries = fs::read_dir(current)
                    .unwrap()
                    .collect::<std::io::Result<Vec<_>>>()
                    .unwrap();
                entries.sort_by_key(std::fs::DirEntry::file_name);
                for entry in entries {
                    let path = entry.path();
                    if entry.file_type().unwrap().is_dir() {
                        visit(root, &path, files);
                    } else {
                        files.insert(
                            path.strip_prefix(root).unwrap().to_path_buf(),
                            fs::read(path).unwrap(),
                        );
                    }
                }
            }

            let mut files = std::collections::BTreeMap::new();
            visit(root, root, &mut files);
            files
        }

        #[test]
        fn test_read_exact_store_excludes_primary_only_records_without_writing_selected_store() {
            let (_temp_dir, main_storage, _taxonomy) =
                crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
            let repo_path = _temp_dir.path().to_path_buf();
            crate::test_cli::init_git_repository(&repo_path).unwrap();
            let add = Command::new("git")
                .args(["add", "."])
                .current_dir(&repo_path)
                .output()
                .unwrap();
            assert!(
                add.status.success(),
                "{}",
                String::from_utf8_lossy(&add.stderr)
            );
            let commit = Command::new("git")
                .args(["commit", "-m", "initialize repository"])
                .current_dir(&repo_path)
                .output()
                .unwrap();
            assert!(
                commit.status.success(),
                "{}",
                String::from_utf8_lossy(&commit.stderr)
            );

            let (_secondary_container, secondary_path) = add_secondary_worktree(&repo_path);
            let secondary_data = secondary_path.join(".jit");
            let secondary_data_layout =
                crate::test_utils::repository_layout_for_data_root(&secondary_data);
            let secondary_storage = JsonFileStorage::new(&secondary_data);
            configure_test_layout(&secondary_storage, &secondary_path, &secondary_data);

            let primary_only = crate::test_utils::fixture_issue(
                "Only in primary".to_string(),
                "The exact linked read must not aggregate this record".to_string(),
            );
            seed_issue_preimage(&main_storage, &primary_only);
            let linked_only = crate::test_utils::fixture_issue(
                "Only in linked".to_string(),
                "The exact linked read must return this physical record".to_string(),
            );
            let linked_path = secondary_data_layout
                .resolve(&VirtualPath::issue_record(&linked_only.id))
                .unwrap();
            fs::create_dir_all(linked_path.parent().unwrap()).unwrap();
            fs::write(
                linked_path,
                crate::repository_state::serialize_issue(&linked_only).unwrap(),
            )
            .unwrap();
            let primary_event = crate::test_utils::stamped_event(
                crate::domain::EventBody::draft_issue_created(&primary_only),
                "primary-event",
                chrono::Utc::now(),
            );
            fs::write(
                crate::test_utils::repository_layout_for_data_root(&repo_path.join(".jit"))
                    .resolve(&VirtualPath::EVENTS)
                    .unwrap(),
                format!("{}\n", serde_json::to_string(&primary_event).unwrap()),
            )
            .unwrap();
            let linked_issue = crate::domain::Issue::draft(
                crate::test_utils::issue_id(55),
                "Only in linked event log".to_string(),
                String::new(),
            );
            let linked_event = crate::test_utils::stamped_event(
                crate::domain::EventBody::draft_issue_created(&linked_issue),
                "linked-event",
                chrono::Utc::now(),
            );
            fs::write(
                secondary_data_layout.resolve(&VirtualPath::EVENTS).unwrap(),
                format!("{}\n", serde_json::to_string(&linked_event).unwrap()),
            )
            .unwrap();

            assert_ne!(std::env::current_dir().unwrap(), secondary_path);
            let before = physical_files(&secondary_data);
            let exact = read_exact_store(&secondary_storage.configured_layout().unwrap()).unwrap();
            assert_eq!(physical_files(&secondary_data), before);
            assert_eq!(exact.issues, vec![linked_only]);
            assert_eq!(exact.events, vec![linked_event]);

            let aggregated = secondary_storage.list_issues().unwrap();
            assert!(aggregated.iter().any(|issue| issue.id == primary_only.id));

            let absent_data = secondary_path.join("absent-store");
            assert!(!absent_data.exists());
            assert_eq!(
                read_exact_store(
                    &crate::storage::discover_repository_layout(&secondary_path, &absent_data)
                        .unwrap(),
                )
                .unwrap(),
                ExactStoreSnapshot::default()
            );
            assert!(!absent_data.exists());
        }

        #[test]
        fn test_load_issue_from_local_first() {
            // Setup: Create a local .jit directory
            let (_temp, storage) = crate::test_utils::setup_test_repo().unwrap();

            // Create and save an issue locally
            let issue = crate::test_utils::fixture_issue(
                "Local Issue".to_string(),
                "Description".to_string(),
            );
            let issue_id = issue.id.clone();
            seed_issue_preimage(&storage, &issue);

            // Should read from local storage
            let loaded = storage.load_issue(&issue_id).unwrap();
            assert_eq!(loaded.title, "Local Issue");
        }

        /// Count `*.lock` files anywhere under a directory tree.
        fn count_lock_files(root: &Path) -> usize {
            fn walk(dir: &Path, acc: &mut usize) {
                let Ok(entries) = fs::read_dir(dir) else {
                    return;
                };
                for entry in entries.filter_map(Result::ok) {
                    let path = entry.path();
                    if path.is_dir() {
                        walk(&path, acc);
                    } else if path.extension().is_some_and(|ext| ext == "lock") {
                        *acc += 1;
                    }
                }
            }
            let mut acc = 0;
            walk(root, &mut acc);
            acc
        }

        #[test]
        fn test_load_issue_creates_no_per_issue_sidecar_lock() {
            // REQ-01: reading an issue must not create a `<id>.lock` sidecar.
            let (_temp, storage) = crate::test_utils::setup_test_repo().unwrap();
            let issue = crate::test_utils::fixture_issue(
                "Sidecar-free".to_string(),
                "Description".to_string(),
            );
            let issue_id = issue.id.clone();
            seed_issue_preimage(&storage, &issue);

            let _ = storage.load_issue(&issue_id).unwrap();

            let sidecar = storage
                .path_layout()
                .unwrap()
                .resolve(&VirtualPath::ISSUES)
                .unwrap()
                .join(format!("{issue_id}.lock"));
            assert!(
                !sidecar.exists(),
                "load_issue must not create a per-issue sidecar lock at {}",
                sidecar.display()
            );
        }

        #[test]
        fn test_read_all_lock_file_count_is_independent_of_issue_count() {
            // REQ-02: after a read-all, the number of lock files is O(1) in the
            // issue count — the surviving fixed lock set does not grow with N.
            fn seed_issues(storage: &JsonFileStorage, issue_count: usize) {
                for i in 0..issue_count {
                    let issue = crate::test_utils::fixture_issue(
                        format!("Issue {i}"),
                        "Description".to_string(),
                    );
                    seed_issue_preimage(storage, &issue);
                }
            }

            let (few_temp, few_storage) = crate::test_utils::setup_test_repo().unwrap();
            seed_issues(&few_storage, 3);
            let few_loaded = few_storage.list_issues().unwrap();
            assert_eq!(few_loaded.len(), 3, "every seeded issue must load");
            // Count over the whole worktree so any per-issue sidecar would be
            // caught wherever it is written.
            let few = count_lock_files(few_temp.path());
            let (many_temp, many_storage) = crate::test_utils::setup_test_repo().unwrap();
            seed_issues(&many_storage, 12);
            let many_loaded = many_storage.list_issues().unwrap();
            assert_eq!(many_loaded.len(), 12, "every seeded issue must load");
            let many = count_lock_files(many_temp.path());
            assert_eq!(
                few, many,
                "lock-file count must be independent of issue count \
                 (got {few} for 3 issues, {many} for 12)"
            );
        }

        #[test]
        fn test_list_issues_changes_no_file_other_than_the_index_lock() {
            let (_temp, storage) = crate::test_utils::setup_test_repo().unwrap();
            let issue = crate::test_utils::fixture_issue(
                "Listed without writes".to_string(),
                "Description".to_string(),
            );
            seed_issue_preimage(&storage, &issue);
            // Warm the index lock so the comparison covers every other file.
            let _ = storage.list_issues().unwrap();

            let before = physical_files(&storage.root);
            let listed = storage.list_issues().unwrap();
            let after = physical_files(&storage.root);

            assert!(listed.iter().any(|listed| listed.id == issue.id));
            assert_eq!(before, after);
        }

        #[test]
        fn test_list_issues_fails_naming_a_record_it_cannot_parse() {
            let (_temp, storage) = crate::test_utils::setup_test_repo().unwrap();
            let sound = crate::test_utils::fixture_issue("Sound".to_string(), String::new());
            let broken = crate::test_utils::fixture_issue("Broken".to_string(), String::new());
            seed_issue_preimage(&storage, &sound);
            seed_issue_preimage(&storage, &broken);
            let broken_path = storage
                .path_layout()
                .unwrap()
                .resolve(&VirtualPath::issue_record(&broken.id))
                .unwrap();
            fs::write(&broken_path, "{ not an issue").unwrap();

            let error = storage
                .list_issues()
                .expect_err("a listing must not hide an unparseable record");

            assert!(
                format!("{error:#}").contains(broken.id.as_str()),
                "the failure names the record: {error:#}"
            );
        }

        #[test]
        fn test_load_issue_from_main_worktree_when_not_local() {
            // Setup: Create git repo with main worktree
            let (_temp_dir, main_storage, _taxonomy) =
                crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
            let repo_path = _temp_dir.path().to_path_buf();
            crate::test_cli::init_git_repository(&repo_path).unwrap();

            // Create an issue in main worktree (not committed)
            let issue = crate::test_utils::fixture_issue(
                "Main WT Issue".to_string(),
                "Uncommitted".to_string(),
            );
            let issue_id = issue.id.clone();
            seed_issue_preimage(&main_storage, &issue);

            // Create secondary worktree with unique name
            use std::time::SystemTime;
            let timestamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let worktree_name = format!("secondary-{}", timestamp);
            let branch_name = format!("feature-{}", timestamp);
            let secondary_rel_path = format!("../{}", worktree_name);

            Command::new("git")
                .args(["worktree", "add", "-b", &branch_name, &secondary_rel_path])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            let secondary_path = repo_path.parent().unwrap().join(&worktree_name);
            let secondary_jit = secondary_path.join(".jit");
            let secondary_jit_layout =
                crate::test_utils::repository_layout_for_data_root(&secondary_jit);
            fs::create_dir_all(secondary_jit_layout.resolve(&VirtualPath::ISSUES).unwrap())
                .unwrap();

            // Initialize secondary storage
            let secondary_storage = JsonFileStorage::new(&secondary_jit);
            configure_test_layout(&secondary_storage, &secondary_path, &secondary_jit);

            // Should fall back to reading from main worktree
            let loaded = secondary_storage.load_issue(&issue_id).unwrap();
            assert_eq!(loaded.title, "Main WT Issue");
            assert_eq!(loaded.description, "Uncommitted");
        }

        #[test]
        fn test_load_aggregated_issue_ids_includes_main_worktree_issues() {
            // Setup: Create git repo with main worktree
            let (_temp_dir, main_storage, _taxonomy) =
                crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
            let repo_path = _temp_dir.path().to_path_buf();
            crate::test_cli::init_git_repository(&repo_path).unwrap();

            // Create issue in main worktree (not committed)
            let issue = crate::test_utils::fixture_issue(
                "Main WT Issue".to_string(),
                "Uncommitted".to_string(),
            );
            let issue_id = issue.id.clone();
            seed_issue_preimage(&main_storage, &issue);

            // Create secondary worktree
            use std::time::SystemTime;
            let timestamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let worktree_name = format!("secondary-{}", timestamp);
            let branch_name = format!("feature-{}", timestamp);
            let secondary_rel_path = format!("../{}", worktree_name);

            Command::new("git")
                .args(["worktree", "add", "-b", &branch_name, &secondary_rel_path])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            let secondary_path = repo_path.parent().unwrap().join(&worktree_name);
            let secondary_jit = secondary_path.join(".jit");
            let secondary_jit_layout =
                crate::test_utils::repository_layout_for_data_root(&secondary_jit);
            fs::create_dir_all(secondary_jit_layout.resolve(&VirtualPath::ISSUES).unwrap())
                .unwrap();

            let secondary_storage = JsonFileStorage::new(&secondary_jit);
            configure_test_layout(&secondary_storage, &secondary_path, &secondary_jit);
            seed_index_preimage(&secondary_storage, &Index::default());

            let aggregated = secondary_storage.load_aggregated_issue_ids().unwrap();
            assert!(aggregated.contains(&issue_id));
        }

        #[test]
        fn test_aggregated_index_propagates_main_worktree_failures() {
            let (_temp_dir, _main_storage, _taxonomy) =
                crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
            let repo_path = _temp_dir.path().to_path_buf();
            crate::test_cli::init_git_repository(&repo_path).unwrap();
            let main_jit = repo_path.join(".jit");
            let main_jit_layout = crate::test_utils::repository_layout_for_data_root(&main_jit);
            let add = Command::new("git")
                .args(["add", VirtualPath::INDEX.repository_relative().as_str()])
                .current_dir(&repo_path)
                .output()
                .unwrap();
            assert!(add.status.success());
            let commit = Command::new("git")
                .args(["commit", "-m", "initialize index"])
                .current_dir(&repo_path)
                .output()
                .unwrap();
            assert!(commit.status.success());

            let (_secondary_container, secondary_path) = add_secondary_worktree(&repo_path);
            let secondary_storage = JsonFileStorage::new(secondary_path.join(".jit"));
            configure_test_layout(
                &secondary_storage,
                &secondary_path,
                &secondary_path.join(".jit"),
            );

            fs::write(
                main_jit_layout.resolve(&VirtualPath::INDEX).unwrap(),
                b"{ invalid json",
            )
            .unwrap();
            let malformed = secondary_storage
                .load_aggregated_issue_ids()
                .expect_err("a malformed main-worktree index must abort aggregation");
            assert!(malformed.to_string().contains("failed to parse index"));

            fs::remove_file(main_jit_layout.resolve(&VirtualPath::INDEX).unwrap()).unwrap();
            fs::create_dir(main_jit_layout.resolve(&VirtualPath::INDEX).unwrap()).unwrap();
            let unreadable = secondary_storage
                .load_aggregated_issue_ids()
                .expect_err("an unreadable main-worktree index must abort aggregation");
            assert!(unreadable.to_string().contains("Failed to read file"));
        }

        #[test]
        fn test_committed_issue_deleted_locally_is_absent_from_every_read() {
            use std::str::FromStr;

            let (temp_dir, storage, _taxonomy) =
                crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
            crate::test_cli::init_git_repository(temp_dir.path()).unwrap();
            let issue =
                crate::test_utils::fixture_issue("Committed".to_string(), "Test".to_string());
            seed_issue_preimage(&storage, &issue);
            commit_all(temp_dir.path());

            fs::remove_file(
                storage
                    .path_layout()
                    .unwrap()
                    .resolve(&VirtualPath::issue_record(&issue.id))
                    .unwrap(),
            )
            .unwrap();
            let mut index = storage.load_index().unwrap();
            index.mark_deleted(issue.id.clone());
            seed_index_preimage(&storage, &index);

            assert!(storage.load_issue(&issue.id).is_err());
            assert!(matches!(
                storage.load_issue_or_not_found(&issue.id),
                Err(crate::storage::PathReadError::NotFound(_))
            ));
            for prefix in [issue.id.as_str(), &issue.id.as_str()[..8]] {
                assert!(storage
                    .resolve_issue_id(&IssueIdPrefix::from_str(prefix).unwrap())
                    .is_err());
            }
            assert!(storage.list_issues().unwrap().is_empty());
        }

        fn linked_store(repo_path: &Path) -> (TempDir, JsonFileStorage) {
            let (container, secondary_path) = add_secondary_worktree(repo_path);
            let data = secondary_path.join(".jit");
            let storage = JsonFileStorage::new(&data);
            configure_test_layout(&storage, &secondary_path, &data);
            (container, storage)
        }

        fn commit_all(repo_path: &Path) {
            for args in [&["add", "."][..], &["commit", "-m", "seed"][..]] {
                let output = Command::new("git")
                    .args(args)
                    .current_dir(repo_path)
                    .output()
                    .unwrap();
                assert!(output.status.success(), "{output:?}");
            }
        }

        /// A linked store over a primary store; `linked` removes one store's file
        /// and records nothing else.
        fn stores_sharing_issue() -> (TempDir, JsonFileStorage, TempDir, JsonFileStorage, Issue) {
            let (temp_dir, main_storage, _taxonomy) =
                crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
            crate::test_cli::init_git_repository(temp_dir.path()).unwrap();
            let issue = crate::test_utils::fixture_issue("Shared".to_string(), String::new());
            seed_issue_preimage(&main_storage, &issue);
            commit_all(temp_dir.path());
            let (container, linked) = linked_store(temp_dir.path());
            (temp_dir, main_storage, container, linked, issue)
        }

        fn remove_issue_with_tombstone(storage: &JsonFileStorage, issue: &Issue) {
            fs::remove_file(
                storage
                    .path_layout()
                    .unwrap()
                    .resolve(&VirtualPath::issue_record(&issue.id))
                    .unwrap(),
            )
            .unwrap();
            add_tombstone(storage, issue);
        }

        fn add_tombstone(storage: &JsonFileStorage, issue: &Issue) {
            let mut index = storage.load_index().unwrap();
            index.mark_deleted(issue.id.clone());
            seed_index_preimage(storage, &index);
        }

        #[test]
        fn test_membership_local_delete_hides_the_primary_checkout_file() {
            let (_main_dir, _main, _container, linked, issue) = stores_sharing_issue();

            remove_issue_with_tombstone(&linked, &issue);

            assert!(linked.load_aggregated_issue_ids().unwrap().is_empty());
            assert!(linked.list_issues().unwrap().is_empty());
            assert!(linked
                .resolve_issue_id(&IssueIdPrefix::from(&issue.id))
                .is_err());
            assert!(linked.load_issue(&issue.id).is_err());
            assert!(matches!(
                linked.load_issue_or_not_found(&issue.id),
                Err(crate::storage::PathReadError::NotFound(_))
            ));
        }

        #[test]
        fn test_membership_local_file_wins_over_primary_checkout_tombstone() {
            let (_main_dir, main, _container, linked, issue) = stores_sharing_issue();

            remove_issue_with_tombstone(&main, &issue);

            assert_eq!(linked.load_aggregated_issue_ids().unwrap(), vec![issue.id]);
        }

        #[test]
        fn test_membership_file_wins_over_tombstone_within_one_source() {
            let (_main_dir, main, _container, _linked, issue) = stores_sharing_issue();

            add_tombstone(&main, &issue);

            assert_eq!(main.load_aggregated_issue_ids().unwrap(), vec![issue.id]);
        }

        #[test]
        fn test_membership_primary_checkout_file_shows_when_local_is_silent() {
            let (_main_dir, main, _container, linked, _issue) = stores_sharing_issue();
            let primary_only =
                crate::test_utils::fixture_issue("Primary".to_string(), String::new());

            seed_issue_preimage(&main, &primary_only);

            assert!(linked
                .load_aggregated_issue_ids()
                .unwrap()
                .contains(&primary_only.id));
        }

        #[test]
        fn test_membership_treats_a_missing_issues_directory_as_empty() {
            let (_temp, storage) = crate::test_utils::setup_test_repo().unwrap();
            fs::remove_dir_all(
                storage
                    .path_layout()
                    .unwrap()
                    .resolve(&VirtualPath::ISSUES)
                    .unwrap(),
            )
            .unwrap();

            assert!(storage.load_aggregated_issue_ids().unwrap().is_empty());
        }

        #[test]
        fn test_load_aggregated_issue_ids_deduplicates_an_issue_in_both_checkouts() {
            let (temp_dir, main_storage, _taxonomy) =
                crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
            crate::test_cli::init_git_repository(temp_dir.path()).unwrap();
            let issue =
                crate::test_utils::fixture_issue("Duplicate Issue".to_string(), "Test".to_string());
            seed_issue_preimage(&main_storage, &issue);
            commit_all(temp_dir.path());
            let (_container, linked) = linked_store(temp_dir.path());

            assert_eq!(linked.load_aggregated_issue_ids().unwrap(), vec![issue.id]);
        }

        #[test]
        fn test_load_issue_prefers_local_over_main_worktree() {
            let (temp_dir, main_storage, _taxonomy) =
                crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
            crate::test_cli::init_git_repository(temp_dir.path()).unwrap();
            let mut issue =
                crate::test_utils::fixture_issue("Original".to_string(), "Old version".to_string());
            seed_issue_preimage(&main_storage, &issue);
            commit_all(temp_dir.path());
            let (_container, linked) = linked_store(temp_dir.path());

            issue.title = "Updated Locally".to_string();
            seed_issue_preimage(&linked, &issue);

            assert_eq!(
                linked.load_issue(&issue.id).unwrap().title,
                "Updated Locally"
            );
        }
    }

    // Tests for schema v2 with deletion tracking
    mod schema_v2_tests {
        use super::*;

        #[test]
        fn test_index_default_holds_no_tombstones() {
            assert!(Index::default().deleted_ids.is_empty());
        }

        #[test]
        fn test_index_save_and_load_round_trips_tombstones() {
            let (_temp_dir, storage) = crate::test_utils::setup_test_repo().unwrap();
            let tombstone = crate::test_utils::issue_id(44);
            let index = Index {
                schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
                deleted_ids: vec![tombstone.clone()],
            };

            fs::write(
                storage
                    .path_layout()
                    .unwrap()
                    .resolve(&VirtualPath::INDEX)
                    .unwrap(),
                index.to_pretty_bytes().unwrap(),
            )
            .unwrap();

            assert_eq!(storage.load_index().unwrap().deleted_ids, vec![tombstone]);
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_read_path_bytes_accepts_symlink_inside_repo() {
        use std::os::unix::fs as unix_fs;

        let repo_root = TempDir::new().unwrap();
        let jit_dir = repo_root.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();

        // Target file inside the repo.
        let target = repo_root.path().join("target.md");
        let expected = b"hello from target";
        fs::write(&target, expected).unwrap();

        // Symlink inside the repo pointing to another file inside the repo.
        let link = repo_root.path().join("link.md");
        unix_fs::symlink(&target, &link).unwrap();

        let storage = JsonFileStorage::new(&jit_dir);
        configure_test_layout(&storage, repo_root.path(), &jit_dir);
        let (bytes, label) = storage
            .read_path_bytes("link.md", None)
            .expect("symlink inside repo must resolve and read successfully");
        assert_eq!(bytes, expected);
        assert_eq!(label, "working-tree");
    }

    #[cfg(unix)]
    #[test]
    fn test_read_path_bytes_rejects_symlink_escape() {
        use crate::storage::PathReadError;
        use std::os::unix::fs as unix_fs;

        let repo_root = TempDir::new().unwrap();
        let jit_dir = repo_root.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();

        // File outside the repo we try to reach via the symlink.
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"secret outside repo").unwrap();

        // Symlink inside the repo pointing to the outside file.
        let link = repo_root.path().join("docs").join("escape.txt");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        unix_fs::symlink(outside.path(), &link).unwrap();

        let storage = JsonFileStorage::new(&jit_dir);
        configure_test_layout(&storage, repo_root.path(), &jit_dir);
        let result = storage.read_path_bytes("docs/escape.txt", None);
        assert!(
            matches!(result, Err(PathReadError::OutsideRepoRoot(_))),
            "symlink pointing outside repo root must be rejected with OutsideRepoRoot, got: {result:?}"
        );
    }

    /// Repository-format compatibility guard (issue def64ac4).
    mod format_compat_tests {
        use super::*;
        use crate::storage::RepositoryFormatUnsupportedError;

        #[test]
        fn test_new_index_is_stamped_with_supported_version() {
            assert_eq!(
                Index::default().schema_version,
                SUPPORTED_INDEX_SCHEMA_VERSION
            );

            let (_temp_dir, storage) = crate::test_utils::setup_test_repo().unwrap();
            assert_eq!(
                storage.load_index().unwrap().schema_version,
                SUPPORTED_INDEX_SCHEMA_VERSION
            );
        }

        #[test]
        fn test_newer_repo_format_refused_on_direct_load() {
            let temp_dir = TempDir::new().unwrap();
            let newer = SUPPORTED_INDEX_SCHEMA_VERSION + 1;
            let data = temp_dir.path().join(".jit");
            let data_layout = crate::test_utils::repository_layout_for_data_root(&data);
            fs::create_dir(&data).unwrap();
            let storage = JsonFileStorage::new(&data);
            configure_test_layout(&storage, temp_dir.path(), &data);
            let index_path = data_layout.resolve(&VirtualPath::INDEX).unwrap();
            let raw = format!("{{\n  \"schema_version\": {newer},\n  \"deleted_ids\": []\n}}");
            fs::write(&index_path, raw).unwrap();

            let err = storage
                .load_index()
                .expect_err("newer format must be refused");
            let typed = err
                .downcast_ref::<RepositoryFormatUnsupportedError>()
                .expect("must be the typed format error, not a parse/read error");
            assert_eq!(typed.repository_version(), newer);
            assert_eq!(typed.supported_version(), SUPPORTED_INDEX_SCHEMA_VERSION);

            let line = typed.to_string();
            assert!(!line.contains('\n'), "message must be single-line: {line}");
            assert!(
                line.contains(&newer.to_string()),
                "must name repo version: {line}"
            );
            assert!(
                line.contains(&SUPPORTED_INDEX_SCHEMA_VERSION.to_string()),
                "must name supported version: {line}"
            );
        }

        // REQ-02 (consistency): the aggregated read path (used by list/status) also
        // surfaces the format error instead of swallowing it.
        #[test]
        fn test_newer_repo_format_refused_on_aggregated_read() {
            let temp_dir = TempDir::new().unwrap();
            let newer = SUPPORTED_INDEX_SCHEMA_VERSION + 1;
            let data = temp_dir.path().join(".jit");
            let data_layout = crate::test_utils::repository_layout_for_data_root(&data);
            fs::create_dir(&data).unwrap();
            let storage = JsonFileStorage::new(&data);
            configure_test_layout(&storage, temp_dir.path(), &data);
            let index_path = data_layout.resolve(&VirtualPath::INDEX).unwrap();
            let raw = format!("{{\n  \"schema_version\": {newer},\n  \"deleted_ids\": []\n}}");
            fs::write(&index_path, raw).unwrap();

            let err = storage
                .list_issues()
                .expect_err("aggregated read must be refused");
            assert!(err
                .downcast_ref::<RepositoryFormatUnsupportedError>()
                .is_some());
        }

        #[test]
        fn test_aggregated_read_rejects_every_invalid_local_index_shape() {
            let id = crate::test_utils::issue_id(1);
            let cases = [
                (
                    serde_json::json!({"schema_version": SUPPORTED_INDEX_SCHEMA_VERSION, "deleted_ids": [id, id]}).to_string(),
                    format!("duplicate deleted issue id '{id}'"),
                ),
                (
                    "{ invalid json".to_string(),
                    "failed to parse index".to_string(),
                ),
            ];

            for (raw, expected) in cases {
                let temp_dir = TempDir::new().unwrap();
                let data = temp_dir.path().join(".jit");
                let data_layout = crate::test_utils::repository_layout_for_data_root(&data);
                fs::create_dir(&data).unwrap();
                let storage = JsonFileStorage::new(&data);
                configure_test_layout(&storage, temp_dir.path(), &data);
                fs::write(data_layout.resolve(&VirtualPath::INDEX).unwrap(), raw).unwrap();
                let error = storage
                    .load_aggregated_issue_ids()
                    .expect_err("invalid local index must abort aggregation");
                assert!(
                    error.to_string().contains(&expected),
                    "expected {expected:?} in {error:#}"
                );
            }
        }

        // REQ-03: a repo whose version EQUALS the binary's support operates normally
        // (critical no-regression case: version == supported == today's repo).
        #[test]
        fn test_equal_version_operates_normally() {
            let temp_dir = TempDir::new().unwrap();
            let data = temp_dir.path().join(".jit");
            let data_layout = crate::test_utils::repository_layout_for_data_root(&data);
            fs::create_dir(&data).unwrap();
            let storage = JsonFileStorage::new(&data);
            configure_test_layout(&storage, temp_dir.path(), &data);
            let index_path = data_layout.resolve(&VirtualPath::INDEX).unwrap();
            let raw = format!(
                "{{\n  \"schema_version\": {SUPPORTED_INDEX_SCHEMA_VERSION},\n  \"deleted_ids\": []\n}}"
            );
            fs::write(&index_path, raw).unwrap();
            assert!(storage.load_index().is_ok());
            assert!(storage.list_issues().is_ok());
        }

        #[test]
        fn test_older_repo_format_refused_on_direct_load() {
            let temp_dir = TempDir::new().unwrap();
            let older = SUPPORTED_INDEX_SCHEMA_VERSION - 1;
            let data = temp_dir.path().join(".jit");
            let data_layout = crate::test_utils::repository_layout_for_data_root(&data);
            fs::create_dir(&data).unwrap();
            let storage = JsonFileStorage::new(&data);
            configure_test_layout(&storage, temp_dir.path(), &data);
            let index_path = data_layout.resolve(&VirtualPath::INDEX).unwrap();
            let raw = format!("{{\n  \"schema_version\": {older},\n  \"deleted_ids\": []\n}}");
            fs::write(&index_path, raw).unwrap();
            let err = storage
                .load_index()
                .expect_err("older format must be refused");
            let typed = err
                .downcast_ref::<RepositoryFormatUnsupportedError>()
                .expect("must be the typed format error");
            assert_eq!(typed.repository_version(), older);
        }
    }
}
