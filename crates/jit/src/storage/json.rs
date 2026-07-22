//! JSON file-based storage implementation.
//!
//! Issues, the index, and events are stored as JSON files in a `.jit/` directory
//! with atomic writes; the gate registry is `.jit/gates.toml`, persisted through
//! [`crate::storage::gate_store`] (see that module for the TOML-specific
//! layout). The directory location can be overridden with the `JIT_DATA_DIR`
//! environment variable.

use crate::declarations::GateRegistry;
use crate::domain::{parse_known_events, Event, Issue};
use crate::repository_state::{
    gate_run_result_relative_path, RepositoryIndex, RepositoryIndexError, RepositoryLayout,
    SUPPORTED_INDEX_SCHEMA_VERSION,
};
use crate::storage::{
    AmbiguousIdError, FileLocker, GateRunNotFoundError, InvalidIdPrefixError, IssueNotFoundError,
    IssueStore, RecoveryDispatchReport, RepoWriteGuard, RepoWriteLock, RepositoryFormatTooNewError,
    RepositoryMutationSession, RepositoryNotFoundError, RepositoryStateStore,
    RepositoryStateStoreError, MIN_ID_PREFIX_LENGTH,
};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{hash_map::Entry, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::Duration;

const ISSUES_DIR: &str = "issues";
const INDEX_FILE: &str = "index.json";
const EVENTS_FILE: &str = "events.jsonl";
const GATE_RUNS_DIR: &str = "gate-runs";

static NEXT_RETENTION_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// Mutation sessions contain thread-scoped lock-order guards and therefore
    /// remain on the thread that opened them. Shared storage clones carry only
    /// the owner thread id used to reject cross-thread release/reentry.
    static RETAINED_MUTATION_SESSIONS:
        RefCell<HashMap<u64, Box<dyn RepositoryMutationSession>>> = RefCell::new(HashMap::new());
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

/// Parse and validate one captured `index.json` without consulting ambient state.
pub(crate) fn parse_repository_index(bytes: &[u8]) -> Result<Index> {
    match RepositoryIndex::parse(bytes) {
        Ok(index) => Ok(index),
        Err(RepositoryIndexError::UnsupportedVersion { found, supported }) => {
            Err(RepositoryFormatTooNewError::new(found, supported).into())
        }
        Err(error) => Err(anyhow!(error)),
    }
}

/// JSON file-based storage for issues, gates, and events.
///
/// This implementation stores each issue as a separate JSON file in `.jit/issues/`,
/// events in `.jit/events.jsonl`, and gate definitions in `.jit/gates.toml` — a
/// `[[gates]]` array-of-tables persisted through [`crate::storage::gate_store`]
/// (the sole source of truth for the gate registry; TOML, not JSON, despite the
/// module name, which reflects the issue/event storage this type otherwise owns).
/// All file writes are atomic (write to temp file, then rename).
///
/// File locking is used to prevent race conditions in concurrent access:
/// - Every mutating path takes the repository-sibling bootstrap lock before the
///   repository write lock ([`RepoWriteLock`], `.repo-write.lock`), so a caller
///   holding the chain across a multi-write sequence excludes all other writers
///   for the whole sequence
/// - Index updates are protected with exclusive locks
/// - Individual issue updates use per-file locks
/// - Gate registry and event log use exclusive locks for writes
#[derive(Clone)]
pub struct JsonFileStorage {
    root: PathBuf,
    locker: FileLocker,
    /// Repository-sibling lock acquired before `.jit/.repo-write.lock`.
    ///
    /// It deliberately lives outside `.jit-bootstrap/`: fresh-root recovery
    /// removes that control directory while retaining this guard, and deleting
    /// the backing lock inode would let another process lock a replacement.
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
    repository_state_failures: Arc<dyn crate::storage::TransactionFailureInjector>,
    /// Canonical layout of the live mutation session, shared by every clone so
    /// reentry is admitted only for the same selected roots.
    active_mutation_layout: Arc<crate::storage::repository_state_store::ActiveLayoutTracker>,
}

/// The configured storage-lock acquisition timeout (`JIT_LOCK_TIMEOUT` seconds,
/// or the runtime default), shared by every file-backed lock this backend opens.
fn configured_lock_timeout() -> Duration {
    std::env::var("JIT_LOCK_TIMEOUT")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ))
}

impl JsonFileStorage {
    /// Create a new JSON file storage instance at the given root path.
    /// The root should be the `.jit` directory (or custom directory from JIT_DATA_DIR).
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        let timeout = configured_lock_timeout();

        let root = root.as_ref().to_path_buf();
        let bootstrap_lock_path = root
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".jit-bootstrap.lock");
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
            repository_state_failures: Arc::new(crate::storage::NoTransactionFailures),
            active_mutation_layout: Arc::new(Default::default()),
            root,
            locker: FileLocker::new(timeout),
        }
    }

    /// Construct storage with deterministic recovered-session failure injection.
    #[cfg(test)]
    pub(crate) fn with_repository_state_failures<P: AsRef<Path>>(
        root: P,
        failures: Arc<dyn crate::storage::TransactionFailureInjector>,
    ) -> Self {
        let mut storage = Self::new(root);
        storage.repository_state_failures = failures;
        storage
    }

    pub(crate) fn repository_state_failures(
        &self,
    ) -> Arc<dyn crate::storage::TransactionFailureInjector> {
        Arc::clone(&self.repository_state_failures)
    }

    pub(crate) fn active_mutation_layout(
        &self,
    ) -> Arc<crate::storage::repository_state_store::ActiveLayoutTracker> {
        Arc::clone(&self.active_mutation_layout)
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
    /// Startup recovery uses this before touching `.jit/`, because a prepared
    /// fresh-root transaction may need to restore the complete absence of that
    /// directory.
    pub(crate) fn acquire_bootstrap_write_lock(&self) -> Result<RepoWriteGuard> {
        self.bootstrap_lock.acquire()
    }

    /// Acquire the bootstrap lock keyed at the WORKTREE root, serializing the
    /// worktree-side `.jit-bootstrap` transactions namespace across every session
    /// sharing the worktree regardless of its data root. For a nested repo this is
    /// the same reentrant instance as [`acquire_bootstrap_write_lock`]; for a
    /// disjoint data root it is a distinct, outer lock that prevents two such
    /// sessions from concurrently mutating one worktree's companions and journals.
    pub(crate) fn acquire_worktree_bootstrap_lock(
        &self,
        worktree_root: &Path,
    ) -> Result<RepoWriteGuard> {
        RepoWriteLock::shared_for_lock_path(
            worktree_root.join(".jit-bootstrap.lock"),
            configured_lock_timeout(),
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
    /// Profile application replaces the complete next event-log image inside a
    /// file-set transaction, so readers must be excluded for that publication
    /// just as they are for the ordinary append path.
    pub(crate) fn acquire_events_write_lock(&self) -> Result<crate::storage::lock::LockGuard> {
        self.locker.lock_exclusive(&self.root.join(".events.lock"))
    }

    pub(crate) fn events_lock_spec(&self) -> (FileLocker, PathBuf) {
        (self.locker.clone(), self.root.join(".events.lock"))
    }

    /// Check if the storage directory exists and is initialized.
    /// Returns an error with a helpful message if not.
    pub fn validate(&self) -> Result<()> {
        if !self.root.exists() {
            return Err(RepositoryNotFoundError::new(self.root.display().to_string()).into());
        }

        let index_path = self.root.join(INDEX_FILE);
        if !index_path.exists() {
            anyhow::bail!(
                "JIT repository at '{}' is not properly initialized (missing index.json)\n\n\
                 Initialize with: jit init",
                self.root.display()
            );
        }

        // Fail fast at startup when the on-disk format is newer than this binary
        // supports. `validate()` runs for every non-init command, so routing the
        // version check through it (via the shared typed index codec) guards even
        // commands that never load the index themselves — e.g. `jit gate list`,
        // which reads `gates.toml` directly — rather than letting them misread
        // newer data (jit:def64ac4).
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
        use crate::domain::artifact_plan::ContentIdentity;
        use crate::storage::PathReadError;

        validate_repo_relative_input(path)?;
        let repo_root = self.root.parent().ok_or_else(|| {
            PathReadError::Other(
                crate::errors::InvalidArgumentError::new("Invalid storage path").into(),
            )
        })?;
        let mut candidate = repo_root.to_path_buf();
        for component in Path::new(path).components() {
            candidate.push(component.as_os_str());
            match fs::symlink_metadata(&candidate) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Ok(ArtifactLocation::Symlink);
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(ArtifactLocation::Missing);
                }
                Err(error) => return Err(PathReadError::Other(error.into())),
            }
        }

        let metadata = fs::symlink_metadata(&candidate)?;
        if !metadata.is_file() {
            return Ok(ArtifactLocation::Unsupported);
        }
        fs::read(candidate)
            .map(|bytes| ArtifactLocation::Regular(ContentIdentity::from_bytes(&bytes)))
            .map_err(PathReadError::from)
    }

    fn issue_path(&self, id: &str) -> PathBuf {
        self.root.join(ISSUES_DIR).join(format!("{}.json", id))
    }

    /// Write `issue` to its file and register it in the index, taking the
    /// repository, index and issue locks in that order.
    ///
    /// The single write path behind [`IssueStore::save_issue`] and
    /// [`IssueStore::restore_issue_verbatim`], which differ only in whether the
    /// caller's `updated_at` is stamped before the write reaches here.
    fn persist_issue(&self, issue: &Issue) -> Result<()> {
        let issue_path = self.issue_path(&issue.id);
        let index_lock_path = self.root.join(".index.lock");
        let issue_lock_path = issue_path.with_extension("lock");

        // Lock order: repository write lock first, then index, then issue.
        // Use separate .lock files to avoid conflicts with atomic writes
        let _repo_lock = self.repo_lock.acquire()?;
        let _index_lock = self.locker.lock_exclusive(&index_lock_path)?;
        let mut index = self.load_index()?;
        let needs_index_update = !index.all_ids.contains(&issue.id);

        // Lock the issue (exclusive) and write
        let _issue_lock = self.locker.lock_exclusive(&issue_lock_path)?;
        self.write_json(&issue_path, issue)?;

        // Update index if this is a new issue
        if needs_index_update {
            index.all_ids.push(issue.id.clone());
            self.save_index(&index)?;
        }

        Ok(())
    }

    fn write_json<T: Serialize>(&self, path: &Path, data: &T) -> Result<()> {
        let json = serde_json::to_string_pretty(data).context("Failed to serialize data")?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create parent directory")?;
        }

        // Route through the canonical atomic-write primitive (temp file + rename
        // with a UNIQUE per-process temp name, safe under concurrent writers).
        crate::storage::atomic_write::write_file_atomic(path, &json)
    }

    fn read_json<T: for<'de> Deserialize<'de>>(&self, path: &Path) -> Result<T> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", path.display()))?;
        serde_json::from_str(&contents).context("Failed to deserialize data")
    }

    fn load_index(&self) -> Result<Index> {
        let index_path = self.root.join(INDEX_FILE);
        let bytes = fs::read(&index_path)
            .with_context(|| format!("Failed to read file: {}", index_path.display()))?;
        parse_repository_index(&bytes)
    }

    fn save_index(&self, index: &Index) -> Result<()> {
        let index_path = self.root.join(INDEX_FILE);
        let bytes = index
            .to_pretty_bytes()
            .context("Failed to serialize index")?;
        crate::storage::atomic_write::write_file_atomic(&index_path, std::str::from_utf8(&bytes)?)
    }

    /// Load aggregated index from all sources (local + git + main worktree).
    ///
    /// This aggregates issue IDs from:
    /// 1. Local .jit/index.json
    /// 2. Git HEAD:.jit/index.json (if in git)
    /// 3. Main worktree .jit/index.json (if in secondary worktree)
    ///
    /// Deduplicates IDs across sources.
    fn load_aggregated_index(&self) -> Result<Index> {
        use std::collections::BTreeMap;

        // The first source that mentions an id wins: local, then HEAD, then the
        // main worktree. This mirrors issue loading and lets a local restore
        // override a stale lower-priority tombstone (and vice versa).
        let mut membership = BTreeMap::new();

        // Only a genuinely absent source may be skipped. Once index bytes exist,
        // every read, parse, version, and membership-structure failure aborts the
        // aggregation instead of exposing a plausible but incomplete issue set.
        let merge_source = |loaded: Result<Option<Index>>,
                            membership: &mut BTreeMap<String, bool>|
         -> Result<()> {
            match loaded {
                Ok(Some(index)) => {
                    index.all_ids.into_iter().for_each(|id| {
                        membership.entry(id).or_insert(true);
                    });
                    index.deleted_ids.into_iter().for_each(|id| {
                        membership.entry(id).or_insert(false);
                    });
                    Ok(())
                }
                Ok(None) => Ok(()),
                Err(error) => Err(error),
            }
        };

        // 1. Load local index
        let local = match fs::read(self.root.join(INDEX_FILE)) {
            Ok(bytes) => Some(parse_repository_index(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("Failed to read local index"),
        };
        merge_source(Ok(local), &mut membership)?;

        // 2. Try loading index from git
        merge_source(self.load_index_from_git(), &mut membership)?;

        // 3. Try loading index from main worktree
        merge_source(self.load_index_from_main_worktree(), &mut membership)?;

        Ok(Index {
            schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
            all_ids: membership
                .into_iter()
                .filter_map(|(id, is_active)| is_active.then_some(id))
                .collect(),
            deleted_ids: vec![], // Don't propagate deleted_ids in aggregated index
        })
    }

    /// Load index from git HEAD.
    fn load_index_from_git(&self) -> Result<Option<Index>> {
        let repo_root = self
            .root
            .parent()
            .ok_or_else(|| crate::errors::InvalidArgumentError::new("Invalid .jit path"))?;
        let repository = match git2::Repository::discover(repo_root) {
            Ok(repository) => repository,
            Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(None),
            Err(error) => return Err(error).context("Failed to open git repository"),
        };
        let head = match repository.head() {
            Ok(head) => head,
            Err(error)
                if matches!(
                    error.code(),
                    git2::ErrorCode::NotFound | git2::ErrorCode::UnbornBranch
                ) =>
            {
                return Ok(None)
            }
            Err(error) => return Err(error).context("Failed to resolve git HEAD"),
        };
        let tree = head
            .peel_to_tree()
            .context("Failed to resolve the git HEAD tree")?;
        let entry = match tree.get_path(Path::new(".jit/index.json")) {
            Ok(entry) => entry,
            Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(None),
            Err(error) => return Err(error).context("Failed to resolve index from git HEAD"),
        };
        let object = entry
            .to_object(&repository)
            .context("Failed to load index object from git HEAD")?;
        let blob = object
            .as_blob()
            .ok_or_else(|| anyhow!("git HEAD index path is not a file"))?;

        parse_repository_index(blob.content())
            .map(Some)
            .context("Failed to parse index from git")
    }

    /// Load index from main worktree.
    fn load_index_from_main_worktree(&self) -> Result<Option<Index>> {
        let repo_root = self
            .root
            .parent()
            .ok_or_else(|| crate::errors::InvalidArgumentError::new("Invalid .jit path"))?;
        let repository = match git2::Repository::discover(repo_root) {
            Ok(repository) => repository,
            Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(None),
            Err(error) => return Err(error).context("Failed to open git repository"),
        };
        let common_dir = repository.commondir();

        // A normal repository points directly at its common directory. Only a
        // linked worktree has a distinct per-worktree repository path.
        if repository.path() == common_dir {
            return Ok(None);
        }

        if common_dir.file_name().and_then(|name| name.to_str()) != Some(".git") {
            return Err(anyhow!(
                "git common directory is not a main-worktree .git directory: {}",
                common_dir.display()
            ));
        }
        let main_worktree_root = common_dir.parent().ok_or_else(|| {
            anyhow!(
                "git common directory has no main-worktree parent: {}",
                common_dir.display()
            )
        })?;

        let main_index_path = main_worktree_root.join(".jit/index.json");
        let bytes = match fs::read(&main_index_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Failed to read file: {}", main_index_path.display()))
            }
        };
        parse_repository_index(&bytes).map(Some)
    }

    /// Load an issue from git HEAD.
    ///
    /// This is used as a fallback when an issue doesn't exist in local storage
    /// but may be committed in git (e.g., reading from a secondary worktree).
    /// Load an issue from `HEAD:.jit/issues/<id>.json`.
    ///
    /// Returns `Ok(Some(issue))` when found, `Ok(None)` when absent from git
    /// (git ran successfully but reported the path is missing), and `Err` for
    /// genuine failures (git not available, repo corrupt, parse error, etc.).
    fn load_issue_from_git(&self, id: &str) -> Result<Option<Issue>> {
        // Run git from repository root, not from .jit directory
        let repo_root = self
            .root
            .parent()
            .ok_or_else(|| crate::errors::InvalidArgumentError::new("Invalid .jit path"))?;

        let git_path = format!("HEAD:.jit/issues/{}.json", id);
        let output = Command::new("git")
            .arg("show")
            .arg(&git_path)
            .current_dir(repo_root)
            .output()
            .context("Failed to execute git command")?;

        if !output.status.success() {
            // git ran but the path is absent — structurally "not found".
            return Ok(None);
        }

        let issue =
            serde_json::from_slice(&output.stdout).context("Failed to parse issue from git")?;
        Ok(Some(issue))
    }

    /// Load an issue from the main worktree's .jit/ directory.
    ///
    /// This is used when in a secondary worktree to read uncommitted issues
    /// from the main worktree.
    /// Load an issue from the main worktree's `.jit/` directory.
    ///
    /// Returns `Ok(Some(issue))` when found, `Ok(None)` when the issue is
    /// absent or this worktree setup does not have an accessible main worktree
    /// (not in git, already in main worktree, non-standard layout), and `Err`
    /// for genuine I/O or parse failures on a file that does exist.
    fn load_issue_from_main_worktree(&self, id: &str) -> Result<Option<Issue>> {
        // self.root is .jit/, we need to go up one level to the repo root
        let repo_root = self
            .root
            .parent()
            .ok_or_else(|| crate::errors::InvalidArgumentError::new("Invalid .jit path"))?;

        // We need to detect worktree context from git commands
        // First check if we're in a git repo at all
        let output = Command::new("git")
            .args(["rev-parse", "--git-common-dir"])
            .current_dir(repo_root)
            .output();

        let output = match output {
            Ok(o) if o.status.success() => o,
            _ => return Ok(None), // not in git or git unavailable
        };

        let common_dir = PathBuf::from(String::from_utf8(output.stdout)?.trim());

        // Get worktree root
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(repo_root)
            .output()?;

        if !output.status.success() {
            return Ok(None); // cannot determine worktree layout
        }

        let worktree_root = PathBuf::from(String::from_utf8(output.stdout)?.trim());

        // Check if we're in main worktree — if so, there's nothing more to try
        let is_main = common_dir == worktree_root.join(".git");
        if is_main {
            return Ok(None);
        }

        // Calculate main worktree path
        let main_worktree_root = if common_dir.file_name().unwrap() == ".git" {
            common_dir.parent().unwrap().to_path_buf()
        } else {
            // Bare repo or non-standard setup — cannot locate main worktree
            return Ok(None);
        };

        let main_issue_path = main_worktree_root
            .join(".jit/issues")
            .join(format!("{}.json", id));

        if !main_issue_path.exists() {
            return Ok(None); // issue absent from main worktree
        }

        // File exists — read it; any I/O or parse error is a real failure.
        self.read_json(&main_issue_path).map(Some)
    }

    /// Check if the current worktree is a secondary git worktree.
    ///
    /// Returns true if this is a secondary worktree, false if main worktree or not in git.
    pub fn is_secondary_worktree(&self) -> bool {
        let repo_root = match self.root.parent() {
            Some(root) => root,
            None => return false,
        };

        // Check if .git exists
        let git_path = repo_root.join(".git");
        if !git_path.exists() {
            return false;
        }

        // Secondary worktrees have .git as a file (pointing to worktree metadata)
        // Main worktree has .git as a directory
        git_path.is_file()
    }
}

impl IssueStore for JsonFileStorage {
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
        // This also repairs a journal left by a checker-side mutation that
        // crashed after preparing or committing its transaction.
        match self.open_mutation_session(layout) {
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

    fn save_issue(&self, mut issue: Issue) -> Result<()> {
        // Update the updated_at timestamp (storage responsibility)
        issue.updated_at = chrono::Utc::now();
        self.persist_issue(&issue)
    }

    fn restore_issue_verbatim(&self, issue: Issue) -> Result<()> {
        self.persist_issue(&issue)
    }

    fn load_issue(&self, id: &str) -> Result<Issue> {
        // Try local .jit/issues/ first (current behavior)
        let issue_path = self.issue_path(id);
        if issue_path.exists() {
            let issue_lock_path = issue_path.with_extension("lock");
            let _lock = self.locker.lock_shared(&issue_lock_path)?;
            return self.read_json(&issue_path);
        }

        // Fallback 1: Try reading from git HEAD.
        // load_issue_from_git returns Ok(Some(issue)), Ok(None) (absent),
        // or Err (git infrastructure failure).
        match self.load_issue_from_git(id) {
            Ok(Some(issue)) => return Ok(issue),
            Ok(None) => {} // file absent from git; continue
            Err(_) => {}   // git unavailable or broken; continue to next source
        }

        // Fallback 2: Try reading from main worktree (if in secondary)
        match self.load_issue_from_main_worktree(id) {
            Ok(Some(issue)) => return Ok(issue),
            Ok(None) => {} // absent from main worktree or not applicable; continue
            Err(e) => return Err(e), // genuine I/O/parse failure
        }

        // Issue not found in any source
        Err(IssueNotFoundError::across_sources(id).into())
    }

    fn load_issue_or_not_found(&self, id: &str) -> Result<Issue, crate::storage::PathReadError> {
        use crate::storage::PathReadError;

        let issue_path = self.issue_path(id);

        if issue_path.exists() {
            // File exists — delegate to load_issue which handles locking and
            // deserialization; any remaining error is a genuine I/O/parse
            // failure → PathReadError::Other.
            return self.load_issue(id).map_err(PathReadError::Other);
        }

        // Local file is absent.  Try git and main-worktree fallbacks.
        //
        // load_issue_from_git now returns Ok(Some(issue)), Ok(None) (absent),
        // or Err (infrastructure failure).  We can branch without string-matching:
        //   Ok(Some(…)) → found, return it.
        //   Ok(None)    → absent from git, continue to next source.
        //   Err(…)      → git is broken or the object is corrupt → Other.
        match self.load_issue_from_git(id) {
            Ok(Some(issue)) => return Ok(issue),
            Ok(None) => {} // absent from git; continue
            Err(e) => return Err(PathReadError::Other(e)),
        }

        // Main-worktree fallback (only meaningful in secondary-worktree setups).
        // Now returns Ok(Option) so we can distinguish real I/O failures from
        // "not applicable / not present".
        match self.load_issue_from_main_worktree(id) {
            Ok(Some(issue)) => return Ok(issue),
            Ok(None) => {} // absent or not applicable; fall through to NotFound
            Err(e) => return Err(PathReadError::Other(e)), // genuine I/O failure → 500
        }

        Err(PathReadError::NotFound(format!(
            "Issue {} not found in local storage, git, or main worktree",
            id
        )))
    }

    fn resolve_issue_id(&self, partial_id: &str) -> Result<String> {
        // Normalize input: lowercase and remove hyphens
        let normalized = partial_id.to_lowercase().replace('-', "");

        // Full UUID check (fast path) - 32 hex chars without hyphens
        if normalized.len() == 32 {
            // Try loading to verify it exists
            return self
                .load_issue(partial_id)
                .map(|issue| issue.id)
                .map_err(|_| IssueNotFoundError::new(partial_id).into());
        }

        // Minimum length check
        if normalized.len() < MIN_ID_PREFIX_LENGTH {
            return Err(InvalidIdPrefixError::new(partial_id).into());
        }

        // Load aggregated index to search across all sources
        let index = self.load_aggregated_index()?;
        let matches: Vec<String> = index
            .all_ids
            .iter()
            .filter(|id| id.replace('-', "").to_lowercase().starts_with(&normalized))
            .cloned()
            .collect();

        match matches.len() {
            0 => Err(IssueNotFoundError::new(partial_id).into()),
            1 => Ok(matches[0].clone()),
            _ => {
                // Load issue titles for better error message
                let issue_list: Vec<String> = matches
                    .iter()
                    .filter_map(|id| {
                        self.load_issue(id)
                            .ok()
                            .map(|issue| format!("{} | {}", issue.short_id(), issue.title))
                    })
                    .collect();
                Err(AmbiguousIdError::issue(partial_id, issue_list).into())
            }
        }
    }

    fn list_issues(&self) -> Result<Vec<Issue>> {
        let index_lock_path = self.root.join(".index.lock");
        let _lock = self.locker.lock_shared(&index_lock_path)?;

        // Use aggregated index to see all issues across sources
        let index = self.load_aggregated_index()?;

        // Load issues using fallback chain (load_issue handles local/git/main)
        let issues = index
            .all_ids
            .iter()
            .filter_map(|id| {
                // Use load_issue which has fallback logic
                self.load_issue(id).ok()
            })
            .collect();

        Ok(issues)
    }

    fn load_gate_registry(&self) -> Result<GateRegistry> {
        let gates_lock_path = self.root.join(".gates.lock");
        let _lock = self.locker.lock_shared(&gates_lock_path)?;
        crate::storage::gate_store::load_gate_registry(&self.root)
    }

    fn save_gate_registry(&self, registry: &GateRegistry) -> Result<()> {
        let gates_lock_path = self.root.join(".gates.lock");
        let _repo_lock = self.repo_lock.acquire()?;
        let _lock = self.locker.lock_exclusive(&gates_lock_path)?;
        crate::storage::gate_store::save_gate_registry(&self.root, registry)
    }

    fn append_event(&self, event: &Event) -> Result<()> {
        let events_lock_path = self.root.join(".events.lock");
        let _repo_lock = self.repo_lock.acquire()?;
        let _lock = self.locker.lock_exclusive(&events_lock_path)?;

        let events_path = self.root.join(EVENTS_FILE);
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&events_path)
            .context("Failed to open events file")?;

        let json = serde_json::to_string(event).context("Failed to serialize event")?;
        let length = file
            .metadata()
            .context("Failed to inspect events file")?
            .len();
        if length > 0 {
            file.seek(SeekFrom::End(-1))
                .context("Failed to inspect events tail")?;
            let mut tail = [0_u8; 1];
            file.read_exact(&mut tail)
                .context("Failed to read events tail")?;
            if tail[0] != b'\n' {
                file.write_all(b"\n")
                    .context("Failed to isolate torn event tail")?;
            }
        }
        writeln!(file, "{}", json).context("Failed to write event")?;
        Ok(())
    }

    fn read_events(&self) -> Result<Vec<Event>> {
        let events_path = self.root.join(EVENTS_FILE);
        if !events_path.exists() {
            return Ok(Vec::new());
        }

        let events_lock_path = self.root.join(".events.lock");
        let _lock = self.locker.lock_shared(&events_lock_path)?;

        let contents = fs::read_to_string(&events_path).context("Failed to read events file")?;
        parse_known_events(&contents).context("Failed to deserialize event log")
    }

    fn read_artifact_archive_events(&self) -> Result<Vec<Event>> {
        let events_path = self.root.join(EVENTS_FILE);
        if !events_path.exists() {
            return Ok(Vec::new());
        }
        let events_lock_path = self.root.join(".events.lock");
        let _lock = self.locker.lock_shared(&events_lock_path)?;
        let reader =
            BufReader::new(fs::File::open(&events_path).context("Failed to open events file")?);
        let lines = reader
            .lines()
            .collect::<std::io::Result<Vec<_>>>()
            .context("Failed to read line from events file")?;
        let mut events = Vec::new();
        for line in &lines {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<serde_json::Value>(line) {
                Ok(value) => {
                    let event_type = value
                        .as_object()
                        .and_then(|object| object.get("type"))
                        .and_then(serde_json::Value::as_str)
                        .context("Event record is missing a string type")?;
                    if event_type == "artifact_archive_executed" {
                        let event = serde_json::from_value::<Event>(value)
                            .context("Failed to deserialize artifact archive event")?;
                        events.push(event);
                    }
                }
                Err(error) if is_torn_event_prefix(line, &error) => {}
                Err(error) => return Err(error).context("Failed to deserialize event"),
            }
        }
        Ok(events)
    }

    fn load_gate_run_result(&self, run_id: &str) -> Result<crate::domain::GateRunResult> {
        let relative = gate_run_result_relative_path(run_id)?;
        let result_path = self.root.join(relative.as_path());
        let result_leaf = relative
            .as_path()
            .file_name()
            .and_then(|name| name.to_str())
            .context("canonical gate run result path has no file name")?;
        let data = match super::repository_state_store::open_absolute_dir_nofollow(&self.root) {
            Ok(data) => data,
            Err(RepositoryStateStoreError::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                return Err(GateRunNotFoundError::new(run_id).into())
            }
            Err(error) => return Err(error.into()),
        };
        let gate_runs = match data.symlink_metadata(GATE_RUNS_DIR) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(GateRunNotFoundError::new(run_id).into())
            }
            Err(error) => return Err(error.into()),
            Ok(metadata) if metadata.is_symlink() || !metadata.is_dir() => {
                anyhow::bail!(
                    "Gate run directory at {} must be an ordinary directory",
                    self.root.join(GATE_RUNS_DIR).display()
                )
            }
            Ok(_) => super::repository_state_store::open_child_dir_nofollow(&data, GATE_RUNS_DIR)?,
        };
        let run_dir = match gate_runs.symlink_metadata(run_id) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(GateRunNotFoundError::new(run_id).into())
            }
            Err(error) => return Err(error.into()),
            Ok(metadata) if metadata.is_symlink() || !metadata.is_dir() => {
                anyhow::bail!(
                    "Gate run directory at {} must be an ordinary directory",
                    self.root.join(GATE_RUNS_DIR).join(run_id).display()
                )
            }
            Ok(_) => super::repository_state_store::open_child_dir_nofollow(&gate_runs, run_id)?,
        };
        match run_dir.symlink_metadata(result_leaf) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(GateRunNotFoundError::new(run_id).into())
            }
            Err(error) => return Err(error.into()),
            Ok(metadata) if metadata.is_symlink() || !metadata.is_file() => {
                anyhow::bail!(
                    "Gate run result at {} must be an ordinary file",
                    result_path.display()
                )
            }
            Ok(_) => {}
        }
        let mut file = super::file_transaction::open_regular_file_nofollow(&run_dir, result_leaf)
            .with_context(|| {
            format!(
                "Failed to open gate run result at {}",
                result_path.display()
            )
        })?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).with_context(|| {
            format!(
                "Failed to read gate run result at {}",
                result_path.display()
            )
        })?;
        let result = serde_json::from_slice(&contents).with_context(|| {
            format!(
                "Failed to deserialize gate run result at {}",
                result_path.display()
            )
        })?;

        Ok(result)
    }

    fn list_gate_runs_for_issue(
        &self,
        issue_id: &str,
    ) -> Result<Vec<crate::domain::GateRunResult>> {
        let data = match super::repository_state_store::open_absolute_dir_nofollow(&self.root) {
            Ok(data) => data,
            Err(RepositoryStateStoreError::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(Vec::new())
            }
            Err(error) => return Err(error.into()),
        };
        let gate_runs = match data.symlink_metadata(GATE_RUNS_DIR) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
            Ok(metadata) if metadata.is_symlink() || !metadata.is_dir() => {
                anyhow::bail!(
                    "Gate run root at {} must be an ordinary directory",
                    self.root.join(GATE_RUNS_DIR).display()
                )
            }
            Ok(_) => super::repository_state_store::open_child_dir_nofollow(&data, GATE_RUNS_DIR)?,
        };

        let mut results = Vec::new();
        for entry in gate_runs.entries()? {
            let entry = entry?;
            let Ok(run_id) = entry.file_name().into_string() else {
                continue;
            };
            let Ok(relative) = gate_run_result_relative_path(&run_id) else {
                continue;
            };
            let metadata = gate_runs.symlink_metadata(&run_id)?;
            if metadata.is_symlink() || !metadata.is_dir() {
                continue;
            }
            let run_dir =
                super::repository_state_store::open_child_dir_nofollow(&gate_runs, &run_id)?;
            let result_path = self.root.join(relative.as_path());
            let result_leaf = relative
                .as_path()
                .file_name()
                .and_then(|name| name.to_str())
                .context("canonical gate run result path has no file name")?;
            match run_dir.symlink_metadata(result_leaf) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
                Ok(metadata) if metadata.is_symlink() || !metadata.is_file() => {
                    anyhow::bail!(
                        "Gate run result at {} must be an ordinary file",
                        result_path.display()
                    )
                }
                Ok(_) => {}
            }
            let mut file =
                super::file_transaction::open_regular_file_nofollow(&run_dir, result_leaf)
                    .with_context(|| {
                        format!(
                            "Failed to open gate run result at {}",
                            result_path.display()
                        )
                    })?;
            let mut contents = Vec::new();
            file.read_to_end(&mut contents).with_context(|| {
                format!(
                    "Failed to read gate run result at {}",
                    result_path.display()
                )
            })?;
            let result: crate::domain::GateRunResult = serde_json::from_slice(&contents)
                .with_context(|| {
                    format!(
                        "Failed to deserialize gate run result at {}",
                        result_path.display()
                    )
                })?;
            if result.issue_id == issue_id {
                results.push(result);
            }
        }

        Ok(results)
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
        // Reuse the validated, repo-root-relative working-tree read so the
        // path-safety and containment checks live in ONE place. An absent file is
        // mapped to `Ok(None)` (graceful); every other failure is propagated typed.
        match self.read_path_text(rel_path, None) {
            Ok((content, _label)) => Ok(Some(content)),
            Err(PathReadError::NotFound(_)) => Ok(None),
            Err(other) => Err(other),
        }
    }

    fn write_repo_file(
        &self,
        rel_path: &str,
        content: &str,
    ) -> Result<(), crate::storage::PathReadError> {
        use crate::storage::PathReadError;
        // Reuse the ONE shared path validator so the write path rejects exactly
        // the same shape-level inputs as the read path (empty, absolute,
        // `..`-escaping) before any I/O.
        validate_repo_relative_input(rel_path)?;
        let _repo_lock = self.repo_lock.acquire().map_err(PathReadError::Other)?;

        // Resolve repo root (parent of `.jit`) and join the validated relative
        // path. Shape validation alone is not enough: a symlinked directory
        // segment could still resolve OUTSIDE the repo, so mirror the read path's
        // canonicalize+containment check below before creating dirs or writing.
        let repo_root = self.root.parent().ok_or_else(|| {
            PathReadError::Other(
                crate::errors::InvalidArgumentError::new("Invalid storage path").into(),
            )
        })?;
        let target = repo_root.join(rel_path);
        let parent = target
            .parent()
            .ok_or_else(|| PathReadError::Other(anyhow!("target has no parent: {rel_path}")))?;

        // The target file (and possibly some parent dirs) may not exist yet, so
        // canonicalization must walk up to the DEEPEST EXISTING ancestor of the
        // parent and verify IT is contained in the repo. This rejects a symlink
        // anywhere along the existing prefix (e.g. `docs -> /external`) WITHOUT
        // first creating directories outside the repo.
        let mut existing_ancestor = parent;
        while !existing_ancestor.exists() {
            existing_ancestor = existing_ancestor.parent().ok_or_else(|| {
                PathReadError::Other(anyhow!("no existing ancestor for {rel_path}"))
            })?;
        }
        assert_canonical_contained(existing_ancestor, repo_root, rel_path)?;

        // Create intermediate directories — now known to be rooted within the
        // repo — so a config-declared nested target (e.g. `docs/reference/x.md`)
        // materializes.
        fs::create_dir_all(parent)
            .map_err(|e| PathReadError::Other(anyhow!("creating {}: {}", parent.display(), e)))?;

        // Re-verify containment of the now-existing parent: this catches the case
        // where the deepest missing segment was itself a symlink escaping the repo.
        assert_canonical_contained(parent, repo_root, rel_path)?;

        // Write through the SHARED atomic writer (temp file in the target's
        // directory + rename), preserving the atomic-write invariant (REQ-05).
        crate::storage::atomic_write::write_file_atomic(&target, content)
            .map_err(PathReadError::Other)
    }

    fn list_gate_presets(&self) -> Result<Vec<crate::gate_presets::PresetInfo>> {
        let manager = crate::gate_presets::PresetManager::new(self.root.clone())?;
        Ok(manager.list_presets())
    }

    fn get_gate_preset(&self, name: &str) -> Result<crate::gate_presets::GatePresetDefinition> {
        let manager = crate::gate_presets::PresetManager::new(self.root.clone())?;
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
        validate_repo_relative_input(path)?;

        // Resolve repo root: parent of the .jit directory.
        let repo_root = self.root.parent().ok_or_else(|| {
            PathReadError::Other(
                crate::errors::InvalidArgumentError::new("Invalid storage path").into(),
            )
        })?;

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
            // Working-tree read.  Canonicalize the joined path and the repo root,
            // then verify containment via the shared helper.  This catches
            // symlinks that point outside the repo root (e.g.
            // `docs/secret -> /etc/passwd`).
            let joined = repo_root.join(path);
            assert_canonical_contained(&joined, repo_root, path)?;
            let canonical_joined = std::fs::canonicalize(&joined).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    PathReadError::NotFound(path.to_string())
                } else {
                    PathReadError::Other(anyhow!("Failed to canonicalize {}: {}", path, e))
                }
            })?;

            fs::read(&canonical_joined)
                .map(|bytes| (bytes, "working-tree".to_string()))
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        PathReadError::NotFound(path.to_string())
                    } else {
                        PathReadError::Other(anyhow!("Failed to read file {}: {}", path, e))
                    }
                })
        }
    }
}

fn is_torn_event_prefix(line: &str, error: &serde_json::Error) -> bool {
    // `append_event` repairs a non-newline tail by terminating it before the
    // next append. Before that retry it is still the final line. Such a record
    // is therefore recognizable as an object-shaped JSON prefix that fails
    // only because input ended. This admits a final torn append and every
    // number of independently isolated predecessors (including prefixes ending
    // after a nested `}`), while syntax and shape/data corruption remain fatal.
    line.trim_start().starts_with('{') && error.is_eof()
}

/// Thin local alias for the shared
/// [`validate_repo_relative_path`](crate::storage::validate_repo_relative_path),
/// kept so the existing call sites in this module read unchanged.
fn validate_repo_relative_input(path: &str) -> Result<(), crate::storage::PathReadError> {
    crate::storage::validate_repo_relative_path(path)
}

/// Assert that `candidate` (an existing path) resolves WITHIN `repo_root`,
/// rejecting symlink escapes.
///
/// The single canonicalize-then-`starts_with` containment idiom shared by the
/// read and write paths: it canonicalizes both `candidate` and `repo_root`
/// (following symlinks) and verifies the resolved candidate is a descendant of
/// the resolved root. `candidate` must already exist (the caller resolves a
/// possibly-not-yet-created target to an existing ancestor first); `rel_for_error`
/// is the original repo-relative path used in the typed error messages.
fn assert_canonical_contained(
    candidate: &Path,
    repo_root: &Path,
    rel_for_error: &str,
) -> Result<(), crate::storage::PathReadError> {
    use crate::storage::PathReadError;
    let canonical_candidate = std::fs::canonicalize(candidate).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            PathReadError::NotFound(rel_for_error.to_string())
        } else {
            PathReadError::Other(anyhow!(
                "Failed to canonicalize {}: {}",
                candidate.display(),
                e
            ))
        }
    })?;
    let canonical_root = std::fs::canonicalize(repo_root).map_err(|e| {
        PathReadError::Other(anyhow!(
            "Failed to canonicalize repo root {}: {}",
            repo_root.display(),
            e
        ))
    })?;
    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(PathReadError::OutsideRepoRoot(rel_for_error.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::GateDefinition;
    use crate::storage::IssueStore;
    use tempfile::TempDir;

    fn retained_storage() -> (
        TempDir,
        RepositoryLayout,
        JsonFileStorage,
        RetainedMutationSessionGuard,
    ) {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join(".jit");
        let storage = JsonFileStorage::new(&data);
        let initial_layout =
            crate::storage::discover_repository_layout(temp.path(), &data).unwrap();
        crate::commands::CommandExecutor::new(storage.clone())
            .with_layout(initial_layout)
            .initialize_fresh_repository(
                temp.path(),
                &crate::hierarchy_templates::HierarchyTemplate::default(),
                None,
            )
            .unwrap();
        let layout = crate::storage::discover_repository_layout(temp.path(), &data).unwrap();
        let retained = storage
            .open_and_retain_mutation_session(layout.clone())
            .unwrap();
        (temp, layout, storage, retained)
    }

    #[test]
    fn test_external_process_reopens_retained_canonical_session() {
        let (temp, _, storage, retained) = retained_storage();

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
    fn test_retained_session_rejects_other_layout_and_thread() {
        let other_worktree = TempDir::new().unwrap();
        let (_temp, _, storage, retained) = retained_storage();

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
    fn test_retained_startup_session_allows_event_publication() {
        let (_temp, _, storage, retained) = retained_storage();

        let event = Event::IssueCreated {
            id: "startup-event".to_string(),
            issue_id: "issue-id".to_string(),
            timestamp: chrono::Utc::now(),
            title: "Created after startup recovery".to_string(),
            priority: crate::domain::Priority::Normal,
        };
        storage.append_event(&event).unwrap();

        assert_eq!(storage.read_events().unwrap(), vec![event]);
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
        let (_temp, _, storage, retained) = retained_storage();

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
        let (_temp, layout, storage, retained) = retained_storage();

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<()> = storage.run_external_process(|| panic!("checker panic"));
        }));
        assert!(panic.is_err());
        drop(retained);

        let session = storage.open_mutation_session(layout).unwrap();
        drop(session);
        assert!(crate::storage::guard_order::CoordinationOrderGuard::enter().is_ok());
    }

    fn assert_direct_writer_waits_for_repository_guard(
        storage: &JsonFileStorage,
        writer: impl FnOnce(JsonFileStorage) -> Result<()> + Send + 'static,
    ) {
        let guard = storage.repo_lock.acquire().unwrap();
        let writer_storage = storage.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            done_tx.send(writer(writer_storage)).unwrap();
        });

        started_rx.recv().unwrap();
        assert!(
            done_rx
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err(),
            "direct storage writer bypassed the held repository guard"
        );
        drop(guard);
        done_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("writer did not resume after repository guard release")
            .unwrap();
        handle.join().unwrap();
    }

    fn setup_storage() -> (TempDir, JsonFileStorage) {
        crate::test_utils::setup_test_repo().unwrap()
    }

    #[test]
    fn test_read_events_skips_structurally_valid_retired_tags_but_keeps_strict_records() {
        let (_temp, storage) = setup_storage();
        let known = Event::IssueCreated {
            id: "known-event".to_string(),
            issue_id: "issue-id".to_string(),
            timestamp: chrono::Utc::now(),
            title: "Known".to_string(),
            priority: crate::domain::Priority::Normal,
        };
        let unknown = concat!(
            "{\"type\":\"retired_",
            "event\",\"id\":\"historical\",",
            "\"timestamp\":\"2025-01-01T00:00:00Z\"}"
        );
        fs::write(
            storage.root.join(EVENTS_FILE),
            format!("{unknown}\n{}\n", serde_json::to_string(&known).unwrap()),
        )
        .unwrap();

        assert_eq!(storage.read_events().unwrap(), vec![known]);

        fs::write(
            storage.root.join(EVENTS_FILE),
            "{\"type\":\"issue_created\"}\n",
        )
        .unwrap();
        assert!(storage.read_events().is_err());

        fs::write(storage.root.join(EVENTS_FILE), "{not-json}\n").unwrap();
        assert!(storage.read_events().is_err());
    }

    #[test]
    fn test_read_repo_file_present_absent_and_path_safety() {
        // Repo root is the parent of the .jit dir, so create storage at temp/.jit.
        let temp = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        fs::create_dir(storage.root()).unwrap();

        // Absent -> None (graceful).
        assert!(storage
            .read_repo_file("project-items.md")
            .unwrap()
            .is_none());

        // Present -> Some(content).
        std::fs::write(temp.path().join("project-items.md"), "hello").unwrap();
        assert_eq!(
            storage
                .read_repo_file("project-items.md")
                .unwrap()
                .as_deref(),
            Some("hello")
        );

        // Path-safety: absolute and `..`-traversal are typed InvalidPath, never a
        // read outside the repository.
        assert!(matches!(
            storage.read_repo_file("/etc/passwd"),
            Err(crate::storage::PathReadError::InvalidPath(_))
        ));
        assert!(matches!(
            storage.read_repo_file("../escape.md"),
            Err(crate::storage::PathReadError::InvalidPath(_))
        ));
    }

    #[test]
    fn test_write_repo_file_atomic_creates_dirs_and_rejects_escaping() {
        use crate::storage::PathReadError;
        let temp = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        fs::create_dir(storage.root()).unwrap();

        // A nested target is written atomically, creating intermediate dirs; a
        // round-trip read returns the same content.
        storage
            .write_repo_file("docs/reference/inv.md", "## Invariants\n")
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(temp.path().join("docs/reference/inv.md")).unwrap(),
            "## Invariants\n"
        );
        assert_eq!(
            storage.read_repo_file("docs/reference/inv.md").unwrap(),
            Some("## Invariants\n".to_string())
        );
        // No leftover temp file in the target directory (atomic temp+rename).
        let leftovers: Vec<_> = std::fs::read_dir(temp.path().join("docs/reference"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no .tmp temp file should remain");

        // Path-safety: an absolute or `..`-escaping target is rejected with the
        // typed error BEFORE any write — nothing leaks outside the repo.
        assert!(matches!(
            storage.write_repo_file("/tmp/jit-escape.md", "x"),
            Err(PathReadError::InvalidPath(_))
        ));
        assert!(matches!(
            storage.write_repo_file("../escape.md", "x"),
            Err(PathReadError::InvalidPath(_))
        ));
        assert!(!temp.path().join("../escape.md").exists());
    }

    #[test]
    fn test_write_repo_file_waits_for_repository_guard() {
        let temp = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        fs::create_dir(storage.root()).unwrap();

        assert_direct_writer_waits_for_repository_guard(&storage, |storage| {
            storage
                .write_repo_file("docs/serialized.md", "serialized")
                .map_err(anyhow::Error::from)
        });

        assert_eq!(
            fs::read_to_string(temp.path().join("docs/serialized.md")).unwrap(),
            "serialized"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_write_repo_file_rejects_symlinked_parent_escape() {
        use crate::storage::PathReadError;
        // A shape-valid relative target whose PARENT directory is a symlink
        // pointing OUTSIDE the repo must be rejected (mirrors the read path's
        // containment check) — nothing is written into the external directory.
        let repo = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        fs::create_dir(storage.root()).unwrap();

        // `repo/docs -> outside/` (an existing symlinked directory segment).
        std::os::unix::fs::symlink(outside.path(), repo.path().join("docs")).unwrap();

        let err = storage
            .write_repo_file("docs/invariants.md", "leak")
            .unwrap_err();
        assert!(
            matches!(err, PathReadError::OutsideRepoRoot(_)),
            "symlink-escaping parent must be OutsideRepoRoot, got {err:?}"
        );
        // Nothing was written through the symlink into the external directory.
        assert!(!outside.path().join("invariants.md").exists());
    }

    #[test]
    fn test_list_gate_runs_errors_on_corrupt_result() {
        // A corrupt result.json must surface a contextual error (naming the
        // offending path), not be silently dropped from the listing.
        let (_temp, storage) = setup_storage();

        let run_dir = storage.root.join("gate-runs").join("corrupt-run");
        fs::create_dir_all(&run_dir).unwrap();
        let result_path = run_dir.join("result.json");
        fs::write(&result_path, "{ not valid json").unwrap();

        let load_err = storage
            .load_gate_run_result("corrupt-run")
            .expect_err("loading corrupt gate-run JSON must fail");
        assert!(
            format!("{load_err:#}").contains(&result_path.display().to_string()),
            "load errors must name the malformed record path"
        );

        let err = storage
            .list_gate_runs_for_issue("any-issue")
            .expect_err("corrupt gate-run record must produce an error, not a silent drop");

        // Top-level context plus the deserialize cause and offending path.
        let chain = format!("{err:#}");
        assert!(
            chain.contains("Failed to deserialize gate run result"),
            "expected a deserialize context in the chain, got: {chain}"
        );
        assert!(
            chain.contains("result.json"),
            "error chain must name the offending path, got: {chain}"
        );
    }

    #[test]
    fn test_list_gate_runs_ignores_noncanonical_nested_result_shape() {
        let (_temp, storage) = setup_storage();
        let nested = storage.root.join("gate-runs/outer/nested/result.json");
        fs::create_dir_all(nested.parent().unwrap()).unwrap();
        fs::write(nested, "{ not valid json").unwrap();
        fs::write(storage.root.join("gate-runs/direct-file"), "root clutter").unwrap();

        assert!(storage
            .list_gate_runs_for_issue("any-issue")
            .unwrap()
            .is_empty());
        assert!(format!(
            "{:#}",
            storage.load_gate_run_result("direct-file").unwrap_err()
        )
        .contains("ordinary directory"));
    }

    #[test]
    fn test_gate_run_readers_reject_non_directory_gate_runs_root() {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join(".jit");
        fs::create_dir(&data).unwrap();
        fs::write(data.join("gate-runs"), "not a directory").unwrap();
        let storage = JsonFileStorage::new(data);

        for error in [
            storage.load_gate_run_result("run-one").unwrap_err(),
            storage.list_gate_runs_for_issue("issue-one").unwrap_err(),
        ] {
            assert!(format!("{error:#}").contains("ordinary directory"));
        }
    }

    #[test]
    fn test_gate_run_readers_reject_non_file_result_object() {
        let (_temp, storage) = setup_storage();
        fs::create_dir_all(storage.root.join("gate-runs/directory-run/result.json")).unwrap();

        for error in [
            storage.load_gate_run_result("directory-run").unwrap_err(),
            storage.list_gate_runs_for_issue("any-issue").unwrap_err(),
        ] {
            let message = format!("{error:#}");
            assert!(message.contains("directory-run/result.json"));
            assert!(message.contains("ordinary file"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_gate_run_readers_reject_symlinked_result_object() {
        let (_temp, storage) = setup_storage();
        let run_dir = storage.root.join("gate-runs/symlink-run");
        fs::create_dir_all(&run_dir).unwrap();
        let target = storage.root.join("target.json");
        fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(target, run_dir.join("result.json")).unwrap();

        let error = storage.load_gate_run_result("symlink-run").unwrap_err();
        assert!(format!("{error:#}").contains("ordinary file"));
    }

    #[cfg(unix)]
    #[test]
    fn test_load_gate_run_rejects_symlinked_gate_runs_root() {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join(".jit");
        let outside = TempDir::new().unwrap();
        fs::create_dir(&data).unwrap();
        fs::create_dir_all(outside.path().join("escaped-run")).unwrap();
        fs::write(outside.path().join("escaped-run/result.json"), "{}").unwrap();
        std::os::unix::fs::symlink(outside.path(), data.join("gate-runs")).unwrap();
        let storage = JsonFileStorage::new(data);

        let error = storage.load_gate_run_result("escaped-run").unwrap_err();
        assert!(format!("{error:#}").contains("ordinary directory"));
    }

    #[cfg(unix)]
    #[test]
    fn test_gate_run_readers_reject_targeted_symlinked_run_directory_but_list_ignores_it() {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join(".jit");
        let outside = TempDir::new().unwrap();
        fs::create_dir_all(data.join("gate-runs")).unwrap();
        fs::write(outside.path().join("result.json"), "{}").unwrap();
        std::os::unix::fs::symlink(outside.path(), data.join("gate-runs/escaped-run")).unwrap();
        let storage = JsonFileStorage::new(data);

        let error = storage.load_gate_run_result("escaped-run").unwrap_err();
        assert!(format!("{error:#}").contains("ordinary directory"));
        assert!(storage
            .list_gate_runs_for_issue("any-issue")
            .unwrap()
            .is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_held_gate_run_directory_capability_cannot_escape_after_parent_symlink_swaps() {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join(".jit");
        let original_run = data.join("gate-runs/run-one");
        fs::create_dir_all(&original_run).unwrap();
        fs::write(original_run.join("result.json"), b"inside").unwrap();

        let outside_run = TempDir::new().unwrap();
        fs::write(outside_run.path().join("result.json"), b"outside-run").unwrap();
        let outside_root = TempDir::new().unwrap();
        fs::create_dir(outside_root.path().join("run-one")).unwrap();
        fs::write(
            outside_root.path().join("run-one/result.json"),
            b"outside-root",
        )
        .unwrap();

        let data_handle =
            super::super::repository_state_store::open_absolute_dir_nofollow(&data).unwrap();
        let gate_runs_handle = super::super::repository_state_store::open_child_dir_nofollow(
            &data_handle,
            "gate-runs",
        )
        .unwrap();
        let run_handle = super::super::repository_state_store::open_child_dir_nofollow(
            &gate_runs_handle,
            "run-one",
        )
        .unwrap();

        fs::rename(&original_run, data.join("gate-runs/run-held")).unwrap();
        std::os::unix::fs::symlink(outside_run.path(), &original_run).unwrap();
        fs::rename(data.join("gate-runs"), data.join("gate-runs-held")).unwrap();
        std::os::unix::fs::symlink(outside_root.path(), data.join("gate-runs")).unwrap();

        assert_eq!(
            fs::read(data.join("gate-runs/run-one/result.json")).unwrap(),
            b"outside-root",
            "the ambient path must demonstrate that the parent swap took effect"
        );
        let mut held_file =
            super::super::file_transaction::open_regular_file_nofollow(&run_handle, "result.json")
                .unwrap();
        let mut held_bytes = Vec::new();
        held_file.read_to_end(&mut held_bytes).unwrap();
        assert_eq!(held_bytes, b"inside");
    }

    #[test]
    fn test_save_and_load_issue() {
        let (_temp, storage) = setup_storage();

        let issue = crate::domain::types::fixture_issue(
            "Test Issue".to_string(),
            "Description".to_string(),
        );
        let issue_id = issue.id.clone();

        storage.save_issue(issue.clone()).unwrap();
        let loaded = storage.load_issue(&issue_id).unwrap();

        assert_eq!(loaded.id, issue.id);
        assert_eq!(loaded.title, issue.title);
        assert_eq!(loaded.description, issue.description);
    }

    #[test]
    fn test_save_issue_updates_index() {
        let (_temp, storage) = setup_storage();

        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Desc".to_string());
        storage.save_issue(issue.clone()).unwrap();

        let index = storage.load_index().unwrap();
        assert!(index.all_ids.contains(&issue.id));
    }

    #[test]
    fn test_save_issue_twice_doesnt_duplicate_in_index() {
        let (_temp, storage) = setup_storage();

        let mut issue = crate::domain::types::fixture_issue("Test".to_string(), "Desc".to_string());
        storage.save_issue(issue.clone()).unwrap();

        issue.title = "Updated".to_string();
        storage.save_issue(issue.clone()).unwrap();

        let index = storage.load_index().unwrap();
        assert_eq!(
            index.all_ids.iter().filter(|id| *id == &issue.id).count(),
            1
        );
    }

    #[test]
    fn test_list_issues_returns_all_issues() {
        let (_temp, storage) = setup_storage();

        let issue1 =
            crate::domain::types::fixture_issue("Issue 1".to_string(), "Desc 1".to_string());
        let issue2 =
            crate::domain::types::fixture_issue("Issue 2".to_string(), "Desc 2".to_string());

        storage.save_issue(issue1.clone()).unwrap();
        storage.save_issue(issue2.clone()).unwrap();

        let issues = storage.list_issues().unwrap();
        assert_eq!(issues.len(), 2);
        assert!(issues.iter().any(|i| i.id == issue1.id));
        assert!(issues.iter().any(|i| i.id == issue2.id));
    }

    #[test]
    fn test_load_nonexistent_issue_returns_error() {
        let (_temp, storage) = setup_storage();

        let result = storage.load_issue("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_gate_registry_operations() {
        let (_temp, storage) = setup_storage();

        let mut registry = storage.load_gate_registry().unwrap();
        assert!(registry.gates.is_empty());

        let gate = GateDefinition {
            version: 1,
            key: "review".to_string(),
            title: "Code Review".to_string(),
            description: "Manual code review".to_string(),
            stage: crate::declarations::GateStage::Postcheck,
            mode: crate::declarations::GateMode::Manual,
            checker: None,
            priority: 100,
            reserved: std::collections::HashMap::new(),
            auto: false,
            example_integration: None,
        };

        registry.gates.insert(gate.key.clone(), gate.clone());
        storage.save_gate_registry(&registry).unwrap();

        let loaded = storage.load_gate_registry().unwrap();
        assert_eq!(loaded.gates.len(), 1);
        assert_eq!(loaded.gates.get("review").unwrap().title, "Code Review");
    }

    // Concurrent access tests

    #[test]
    fn test_concurrent_issue_creates_no_corruption() {
        use std::sync::Arc;
        use std::thread;

        let (_temp_dir, storage) = setup_storage();
        let storage = Arc::new(storage);

        let num_threads = 10;
        let issues_per_thread = 5;

        let handles: Vec<_> = (0..num_threads)
            .map(|thread_id| {
                let storage = Arc::clone(&storage);
                thread::spawn(move || {
                    for i in 0..issues_per_thread {
                        let issue = crate::domain::types::fixture_issue(
                            format!("Thread {} Issue {}", thread_id, i),
                            format!("Description {}-{}", thread_id, i),
                        );
                        storage.save_issue(issue.clone()).unwrap();
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // Verify: exactly 50 issues, no duplicates in index
        let issues = storage.list_issues().unwrap();
        assert_eq!(issues.len(), num_threads * issues_per_thread);

        let index = storage.load_index().unwrap();
        assert_eq!(index.all_ids.len(), num_threads * issues_per_thread);

        // Check for duplicates
        let mut ids = index.all_ids.clone();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), num_threads * issues_per_thread);
    }

    #[test]
    fn test_concurrent_updates_to_different_issues() {
        use std::sync::Arc;
        use std::thread;

        let (_temp_dir, storage) = setup_storage();
        let storage = Arc::new(storage);

        // Create two issues
        let issue1 =
            crate::domain::types::fixture_issue("Issue 1".to_string(), "Desc 1".to_string());
        let issue2 =
            crate::domain::types::fixture_issue("Issue 2".to_string(), "Desc 2".to_string());
        let id1 = issue1.id.clone();
        let id2 = issue2.id.clone();

        storage.save_issue(issue1.clone()).unwrap();
        storage.save_issue(issue2.clone()).unwrap();

        // Update them concurrently
        let storage1 = Arc::clone(&storage);
        let storage2 = Arc::clone(&storage);
        let id1_clone = id1.clone();
        let id2_clone = id2.clone();

        let handle1 = thread::spawn(move || {
            for i in 0..10 {
                let mut issue = storage1.load_issue(&id1_clone).unwrap();
                issue.title = format!("Updated 1 - {}", i);
                storage1.save_issue(issue.clone()).unwrap();
            }
        });

        let handle2 = thread::spawn(move || {
            for i in 0..10 {
                let mut issue = storage2.load_issue(&id2_clone).unwrap();
                issue.title = format!("Updated 2 - {}", i);
                storage2.save_issue(issue.clone()).unwrap();
            }
        });

        handle1.join().unwrap();
        handle2.join().unwrap();

        // Both issues should exist and be loadable
        let loaded1 = storage.load_issue(&id1).unwrap();
        let loaded2 = storage.load_issue(&id2).unwrap();
        assert!(loaded1.title.starts_with("Updated 1"));
        assert!(loaded2.title.starts_with("Updated 2"));
    }

    #[test]
    fn test_concurrent_updates_to_same_issue() {
        use std::sync::Arc;
        use std::thread;

        let (_temp_dir, storage) = setup_storage();
        let storage = Arc::new(storage);

        let issue = crate::domain::types::fixture_issue(
            "Test Issue".to_string(),
            "Description".to_string(),
        );
        let issue_id = issue.id.clone();
        storage.save_issue(issue.clone()).unwrap();

        let num_threads = 5;
        let updates_per_thread = 3;

        let handles: Vec<_> = (0..num_threads)
            .map(|thread_id| {
                let storage = Arc::clone(&storage);
                let id = issue_id.clone();
                thread::spawn(move || {
                    for i in 0..updates_per_thread {
                        let mut issue = storage.load_issue(&id).unwrap();
                        issue.title = format!("Thread {} Update {}", thread_id, i);
                        storage.save_issue(issue.clone()).unwrap();
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // Issue should still be loadable and valid
        let final_issue = storage.load_issue(&issue_id).unwrap();
        assert!(final_issue.title.starts_with("Thread"));
        assert_eq!(final_issue.id, issue_id);
    }

    #[test]
    fn test_concurrent_read_write_issue() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let (_temp_dir, storage) = setup_storage();
        let storage = Arc::new(storage);

        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Desc".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(issue.clone()).unwrap();

        let barrier = Arc::new(Barrier::new(6)); // 1 writer + 5 readers

        // Writer thread
        let storage_writer = Arc::clone(&storage);
        let id_writer = issue_id.clone();
        let barrier_writer = Arc::clone(&barrier);
        let writer = thread::spawn(move || {
            barrier_writer.wait();
            for i in 0..5 {
                let mut issue = storage_writer.load_issue(&id_writer).unwrap();
                issue.title = format!("Updated {}", i);
                storage_writer.save_issue(issue.clone()).unwrap();
                thread::sleep(std::time::Duration::from_millis(10));
            }
        });

        // Reader threads
        let mut readers = vec![];
        for _ in 0..5 {
            let storage_reader = Arc::clone(&storage);
            let id_reader = issue_id.clone();
            let barrier_reader = Arc::clone(&barrier);
            readers.push(thread::spawn(move || {
                barrier_reader.wait();
                for _ in 0..10 {
                    let issue = storage_reader.load_issue(&id_reader).unwrap();
                    // Should always get valid data (not corrupted)
                    assert!(!issue.title.is_empty());
                    assert_eq!(issue.id, id_reader);
                }
            }));
        }

        writer.join().unwrap();
        for reader in readers {
            reader.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_dependency_operations() {
        use std::sync::Arc;
        use std::thread;

        let (_temp_dir, storage) = setup_storage();
        let storage = Arc::new(storage);

        // Create base issue
        let base = crate::domain::types::fixture_issue("Base".to_string(), "Desc".to_string());
        let base_id = base.id.clone();
        storage.save_issue(base.clone()).unwrap();

        // Add dependencies concurrently
        let handles: Vec<_> = (0..5)
            .map(|i| {
                let storage = Arc::clone(&storage);
                let base_id = base_id.clone();
                thread::spawn(move || {
                    let dep = crate::domain::types::fixture_issue(
                        format!("Dep {}", i),
                        "Desc".to_string(),
                    );
                    let dep_id = dep.id.clone();
                    storage.save_issue(dep.clone()).unwrap();

                    let mut base = storage.load_issue(&base_id).unwrap();
                    base.dependencies.push(dep_id);
                    storage.save_issue(base.clone()).unwrap();
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // Load and verify
        let final_base = storage.load_issue(&base_id).unwrap();
        // Note: Due to concurrent updates, we may lose some dependencies
        // (last write wins), but the data should not be corrupted
        assert!(!final_base.dependencies.is_empty());
        assert!(final_base.dependencies.len() <= 5);
    }

    #[test]
    fn test_concurrent_list_and_create() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let (_temp_dir, storage) = setup_storage();
        let storage = Arc::new(storage);

        let barrier = Arc::new(Barrier::new(4));

        // Create some initial issues
        for i in 0..3 {
            let issue =
                crate::domain::types::fixture_issue(format!("Initial {}", i), "Desc".to_string());
            storage.save_issue(issue.clone()).unwrap();
        }

        // Concurrent readers
        let mut handles = vec![];
        for _ in 0..3 {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                for _ in 0..5 {
                    let issues = storage.list_issues().unwrap();
                    assert!(issues.len() >= 3); // At least initial issues
                }
            }));
        }

        // Concurrent writer
        let storage_writer = Arc::clone(&storage);
        let barrier_writer = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier_writer.wait();
            for i in 0..5 {
                let issue =
                    crate::domain::types::fixture_issue(format!("New {}", i), "Desc".to_string());
                storage_writer.save_issue(issue.clone()).unwrap();
            }
        }));

        for handle in handles {
            handle.join().unwrap();
        }

        // Final check
        let final_issues = storage.list_issues().unwrap();
        assert_eq!(final_issues.len(), 8); // 3 initial + 5 new
    }

    // Tests for cross-worktree issue visibility (TDD)
    mod cross_worktree_tests {
        use super::*;
        use std::fs;
        use std::process::Command;

        fn setup_git_repo() -> (TempDir, PathBuf, JsonFileStorage) {
            let temp_dir = TempDir::new().unwrap();
            let repo_path = temp_dir.path().to_path_buf();

            // Initialize git repository
            Command::new("git")
                .args(["init"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            Command::new("git")
                .args(["config", "user.name", "Test"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            Command::new("git")
                .args(["config", "user.email", "test@example.com"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            let data = repo_path.join(".jit");
            let storage = JsonFileStorage::new(&data);
            let layout = crate::storage::discover_repository_layout(&repo_path, &data).unwrap();
            crate::commands::CommandExecutor::new(storage.clone())
                .with_layout(layout)
                .initialize_fresh_repository(
                    &repo_path,
                    &crate::hierarchy_templates::HierarchyTemplate::default(),
                    None,
                )
                .unwrap();

            (temp_dir, repo_path, storage)
        }

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

        #[test]
        fn test_load_issue_from_local_first() {
            // Setup: Create a local .jit directory
            let (_temp, storage) = setup_storage();

            // Create and save an issue locally
            let issue = crate::domain::types::fixture_issue(
                "Local Issue".to_string(),
                "Description".to_string(),
            );
            let issue_id = issue.id.clone();
            storage.save_issue(issue.clone()).unwrap();

            // Should read from local storage
            let loaded = storage.load_issue(&issue_id).unwrap();
            assert_eq!(loaded.title, "Local Issue");
        }

        #[test]
        fn test_load_issue_from_git_when_not_local() {
            // Setup: Create git repo with committed issue
            let (_temp_dir, repo_path, storage) = setup_git_repo();
            let jit_dir = repo_path.join(".jit");

            // Create an issue and commit it
            let issue = crate::domain::types::fixture_issue(
                "Committed Issue".to_string(),
                "From git".to_string(),
            );
            let issue_id = issue.id.clone();
            storage.save_issue(issue.clone()).unwrap();

            // Commit to git
            Command::new("git")
                .args(["add", ".jit"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            Command::new("git")
                .args(["commit", "-m", "Add issue"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            // Remove from local .jit (simulate reading from different worktree)
            let issue_path = jit_dir.join("issues").join(format!("{}.json", issue_id));
            fs::remove_file(&issue_path).unwrap();

            // Update index to not include the issue
            let index = Index {
                schema_version: 2,
                all_ids: vec![],
                deleted_ids: vec![],
            };
            storage
                .write_json(&jit_dir.join(INDEX_FILE), &index)
                .unwrap();

            // Should fall back to reading from git
            let loaded = storage.load_issue(&issue_id).unwrap();
            assert_eq!(loaded.title, "Committed Issue");
            assert_eq!(loaded.description, "From git");
        }

        #[test]
        fn test_load_issue_from_main_worktree_when_not_in_git() {
            // Setup: Create git repo with main worktree
            let (_temp_dir, repo_path, main_storage) = setup_git_repo();

            // Create an issue in main worktree (not committed)
            let issue = crate::domain::types::fixture_issue(
                "Main WT Issue".to_string(),
                "Uncommitted".to_string(),
            );
            let issue_id = issue.id.clone();
            main_storage.save_issue(issue.clone()).unwrap();

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
            fs::create_dir_all(secondary_jit.join("issues")).unwrap();

            // Initialize secondary storage
            let secondary_storage = JsonFileStorage::new(&secondary_jit);

            // Should fall back to reading from main worktree
            let loaded = secondary_storage.load_issue(&issue_id).unwrap();
            assert_eq!(loaded.title, "Main WT Issue");
            assert_eq!(loaded.description, "Uncommitted");
        }

        #[test]
        fn test_load_aggregated_index_includes_git_issues() {
            // Setup: Create git repo with committed issue
            let (_temp_dir, repo_path, storage) = setup_git_repo();
            let jit_dir = repo_path.join(".jit");

            // Create and commit issue
            let issue = crate::domain::types::fixture_issue(
                "Committed Issue".to_string(),
                "In git".to_string(),
            );
            let issue_id = issue.id.clone();
            storage.save_issue(issue.clone()).unwrap();

            Command::new("git")
                .args(["add", ".jit"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            Command::new("git")
                .args(["commit", "-m", "Add issue"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            // Remove from local index and storage (simulate fresh worktree)
            let issue_path = jit_dir.join("issues").join(format!("{}.json", issue_id));
            fs::remove_file(&issue_path).unwrap();

            let index = Index {
                schema_version: 2,
                all_ids: vec![],
                deleted_ids: vec![],
            };
            storage
                .write_json(&jit_dir.join(INDEX_FILE), &index)
                .unwrap();

            // load_aggregated_index should find it in git
            let aggregated = storage.load_aggregated_index().unwrap();
            assert!(aggregated.all_ids.contains(&issue_id));
        }

        #[test]
        fn test_load_aggregated_index_rejects_invalid_git_index() {
            let (_temp_dir, repo_path, storage) = setup_git_repo();
            let jit_dir = repo_path.join(".jit");

            fs::write(jit_dir.join(INDEX_FILE), b"{ invalid json").unwrap();
            Command::new("git")
                .args(["add", ".jit/index.json"])
                .current_dir(&repo_path)
                .output()
                .unwrap();
            Command::new("git")
                .args(["commit", "-m", "invalid index fixture"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            storage.save_index(&Index::default()).unwrap();
            let error = storage
                .load_aggregated_index()
                .expect_err("a malformed fallback index must not be skipped");
            assert!(error.to_string().contains("Failed to parse index from git"));
        }

        #[test]
        fn test_local_membership_overrides_git_membership_in_both_states() {
            let (_temp_dir, repo_path, storage) = setup_git_repo();

            storage
                .save_index(&Index {
                    schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
                    all_ids: vec!["deleted-locally".to_string()],
                    deleted_ids: vec!["restored-locally".to_string()],
                })
                .unwrap();
            let add = Command::new("git")
                .args(["add", ".jit/index.json"])
                .current_dir(&repo_path)
                .output()
                .unwrap();
            assert!(add.status.success());
            let commit = Command::new("git")
                .args(["commit", "-m", "record tombstone"])
                .current_dir(&repo_path)
                .output()
                .unwrap();
            assert!(commit.status.success());

            storage
                .save_index(&Index {
                    schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
                    all_ids: vec!["restored-locally".to_string()],
                    deleted_ids: vec!["deleted-locally".to_string()],
                })
                .unwrap();

            assert_eq!(
                storage.load_aggregated_index().unwrap().all_ids,
                vec!["restored-locally"]
            );
        }

        #[test]
        fn test_git_membership_overrides_main_worktree_in_both_states() {
            let (_temp_dir, repo_path, main_storage) = setup_git_repo();
            main_storage
                .save_index(&Index {
                    schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
                    all_ids: vec!["active-in-head".to_string()],
                    deleted_ids: vec!["deleted-in-head".to_string()],
                })
                .unwrap();
            let add = Command::new("git")
                .args(["add", ".jit/index.json"])
                .current_dir(&repo_path)
                .output()
                .unwrap();
            assert!(add.status.success());
            let commit = Command::new("git")
                .args(["commit", "-m", "record head membership"])
                .current_dir(&repo_path)
                .output()
                .unwrap();
            assert!(commit.status.success());

            let (_secondary_container, secondary_path) = add_secondary_worktree(&repo_path);
            fs::remove_file(secondary_path.join(".jit/index.json")).unwrap();
            main_storage
                .save_index(&Index {
                    schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
                    all_ids: vec!["deleted-in-head".to_string()],
                    deleted_ids: vec!["active-in-head".to_string()],
                })
                .unwrap();

            let secondary_storage = JsonFileStorage::new(secondary_path.join(".jit"));
            assert_eq!(
                secondary_storage.load_aggregated_index().unwrap().all_ids,
                vec!["active-in-head"]
            );
        }

        #[test]
        fn test_load_aggregated_index_includes_main_worktree_issues() {
            // Setup: Create git repo with main worktree
            let (_temp_dir, repo_path, main_storage) = setup_git_repo();

            // Create issue in main worktree (not committed)
            let issue = crate::domain::types::fixture_issue(
                "Main WT Issue".to_string(),
                "Uncommitted".to_string(),
            );
            let issue_id = issue.id.clone();
            main_storage.save_issue(issue.clone()).unwrap();

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
            fs::create_dir_all(secondary_jit.join("issues")).unwrap();

            let secondary_storage = JsonFileStorage::new(&secondary_jit);
            secondary_storage.save_index(&Index::default()).unwrap();

            // Aggregated index should include main worktree issue
            let aggregated = secondary_storage.load_aggregated_index().unwrap();
            assert!(aggregated.all_ids.contains(&issue_id));
        }

        #[test]
        fn test_aggregated_index_propagates_main_worktree_failures() {
            let (_temp_dir, repo_path, _main_storage) = setup_git_repo();
            let main_jit = repo_path.join(".jit");
            let add = Command::new("git")
                .args(["add", ".jit/index.json"])
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

            fs::write(main_jit.join(INDEX_FILE), b"{ invalid json").unwrap();
            let malformed = secondary_storage
                .load_aggregated_index()
                .expect_err("a malformed main-worktree index must abort aggregation");
            assert!(malformed.to_string().contains("failed to parse index"));

            fs::remove_file(main_jit.join(INDEX_FILE)).unwrap();
            fs::create_dir(main_jit.join(INDEX_FILE)).unwrap();
            let unreadable = secondary_storage
                .load_aggregated_index()
                .expect_err("an unreadable main-worktree index must abort aggregation");
            assert!(unreadable.to_string().contains("Failed to read file"));
        }

        #[test]
        fn test_load_aggregated_index_deduplicates() {
            // Setup: Create git repo
            let (_temp_dir, repo_path, storage) = setup_git_repo();

            // Create and commit issue
            let issue = crate::domain::types::fixture_issue(
                "Duplicate Issue".to_string(),
                "Test".to_string(),
            );
            let issue_id = issue.id.clone();
            storage.save_issue(issue.clone()).unwrap();

            Command::new("git")
                .args(["add", ".jit"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            Command::new("git")
                .args(["commit", "-m", "Add issue"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            // Issue is now in both local AND git
            // Aggregated index should deduplicate
            let aggregated = storage.load_aggregated_index().unwrap();
            let count = aggregated
                .all_ids
                .iter()
                .filter(|id| *id == &issue_id)
                .count();
            assert_eq!(count, 1, "Issue ID should appear exactly once");
        }

        #[test]
        fn test_load_issue_prefers_local_over_git() {
            // Setup: Create git repo with committed issue
            let (_temp_dir, repo_path, storage) = setup_git_repo();

            // Create and commit an issue
            let mut issue = crate::domain::types::fixture_issue(
                "Original".to_string(),
                "Old version".to_string(),
            );
            let issue_id = issue.id.clone();
            storage.save_issue(issue.clone()).unwrap();

            Command::new("git")
                .args(["add", ".jit"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            Command::new("git")
                .args(["commit", "-m", "Add issue"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            // Update issue locally (not committed)
            issue.title = "Updated Locally".to_string();
            issue.description = "New version".to_string();
            storage.save_issue(issue.clone()).unwrap();

            // Should prefer local version over git version
            let loaded = storage.load_issue(&issue_id).unwrap();
            assert_eq!(loaded.title, "Updated Locally");
            assert_eq!(loaded.description, "New version");
        }

        #[test]
        fn test_load_issue_fails_when_not_found_anywhere() {
            let (_temp, storage) = setup_storage();

            let fake_id = "00000000-0000-0000-0000-000000000000";
            let result = storage.load_issue(fake_id);

            assert!(result.is_err());
            let err_msg = result.unwrap_err().to_string();
            assert!(err_msg.contains("not found") || err_msg.contains("Issue"));
        }

        #[test]
        fn test_load_issue_from_git_handles_invalid_json() {
            // Setup: Create git repo with invalid JSON committed
            let (_temp_dir, repo_path, _storage) = setup_git_repo();
            let jit_dir = repo_path.join(".jit");
            let storage = JsonFileStorage::new(&jit_dir);

            // Create invalid JSON file
            let issue_id = "11111111-1111-1111-1111-111111111111";
            let issue_path = jit_dir.join("issues").join(format!("{}.json", issue_id));
            fs::write(&issue_path, "{ invalid json }").unwrap();

            // Commit it
            Command::new("git")
                .args(["add", ".jit"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            Command::new("git")
                .args(["commit", "-m", "Add invalid issue"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            // Remove from local
            fs::remove_file(&issue_path).unwrap();

            // Should fail gracefully with parse error (not panic)
            let result = storage.load_issue(issue_id);
            assert!(result.is_err());
        }
    }

    // Tests for schema v2 with deletion tracking
    mod schema_v2_tests {
        use super::*;

        #[test]
        fn test_index_v2_has_deleted_ids_field() {
            // Create a new index - should be v2
            let index = Index::default();

            assert_eq!(index.schema_version, 2);
            assert_eq!(index.all_ids, Vec::<String>::new());
            assert_eq!(index.deleted_ids, Vec::<String>::new());
        }

        #[test]
        fn test_index_v2_save_and_load() {
            let (_temp_dir, storage) = setup_storage();

            // Create index with deleted IDs
            let index = Index {
                schema_version: 2,
                all_ids: vec!["issue-1".to_string(), "issue-2".to_string()],
                deleted_ids: vec!["issue-3".to_string()],
            };

            // Save index
            storage.save_index(&index).unwrap();

            // Load it back
            let loaded = storage.load_index().unwrap();

            assert_eq!(loaded.schema_version, 2);
            assert_eq!(loaded.all_ids.len(), 2);
            assert!(loaded.all_ids.contains(&"issue-1".to_string()));
            assert!(loaded.all_ids.contains(&"issue-2".to_string()));
            assert_eq!(loaded.deleted_ids.len(), 1);
            assert!(loaded.deleted_ids.contains(&"issue-3".to_string()));
        }
    }

    // Tests for worktree deletion safety (Phase 3)
    mod deletion_safety_tests {
        use super::*;

        #[test]
        fn test_is_secondary_worktree_detection() {
            // Main worktree: .git is a directory, one level above the .jit
            // root. The git repository must live in a directory this test
            // owns, not in a bare `TempDir::new()`'s parent: that parent is
            // the shared, process-wide temp directory (every `TempDir` is
            // created directly under it), so `git init` there would leave a
            // stray `.git` at the root of the shared temp directory — silently
            // polluting it as an ancestor "repository" for every other test
            // that creates a `TempDir` for the rest of the process's lifetime.
            let outer = TempDir::new().unwrap();
            let main_dir = outer.path().join("jit-root");
            fs::create_dir_all(&main_dir).unwrap();
            let main_storage = JsonFileStorage::new(&main_dir);

            // Initialize git at the outer, privately-owned directory.
            Command::new("git")
                .arg("init")
                .current_dir(outer.path())
                .output()
                .unwrap();

            // Should detect as main worktree
            assert!(!main_storage.is_secondary_worktree());
        }
    }

    #[test]
    fn test_read_path_bytes_working_tree_resolves_against_repo_root() {
        use std::env;

        // Create a temp dir that acts as the repo root, with a .jit subdir.
        let repo_root = TempDir::new().unwrap();
        let jit_dir = repo_root.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();

        // Write a fixture file inside the repo root.
        let file_name = "test-doc.md";
        let expected = b"# Hello from repo root";
        fs::write(repo_root.path().join(file_name), expected).unwrap();

        // Build storage rooted at .jit (root = .jit dir, repo root = its parent).
        let storage = JsonFileStorage::new(&jit_dir);

        // Change CWD to a completely unrelated directory so that a naive
        // fs::read(path) using process CWD would fail to find the file.
        let original_cwd = env::current_dir().unwrap();
        env::set_current_dir(env::temp_dir()).unwrap();

        let result = storage.read_path_bytes(file_name, None);

        // Restore CWD regardless of outcome.
        let _ = env::set_current_dir(&original_cwd);

        let (bytes, label) =
            result.expect("read_path_bytes should succeed even when CWD != repo root");
        assert_eq!(bytes, expected, "file contents should match");
        assert_eq!(label, "working-tree");
    }

    #[test]
    fn test_read_path_bytes_missing_file_returns_not_found() {
        use crate::storage::PathReadError;

        let repo_root = TempDir::new().unwrap();
        let jit_dir = repo_root.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();

        let storage = JsonFileStorage::new(&jit_dir);

        let result = storage.read_path_bytes("does_not_exist.md", None);
        assert!(
            result.is_err(),
            "reading a missing file should return an error"
        );
        assert!(
            matches!(result.unwrap_err(), PathReadError::NotFound(_)),
            "missing file should yield PathReadError::NotFound, not Other"
        );
    }

    #[test]
    fn test_read_path_bytes_rejects_empty_path() {
        use crate::storage::PathReadError;

        let repo_root = TempDir::new().unwrap();
        let jit_dir = repo_root.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();
        let storage = JsonFileStorage::new(&jit_dir);

        let result = storage.read_path_bytes("", None);
        assert!(
            matches!(result, Err(PathReadError::InvalidPath(_))),
            "empty path must be rejected with InvalidPath"
        );
    }

    #[test]
    fn test_read_path_bytes_rejects_absolute_path() {
        use crate::storage::PathReadError;

        let repo_root = TempDir::new().unwrap();
        let jit_dir = repo_root.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();
        let storage = JsonFileStorage::new(&jit_dir);

        let result = storage.read_path_bytes("/etc/passwd", None);
        assert!(
            matches!(result, Err(PathReadError::InvalidPath(_))),
            "absolute path must be rejected with InvalidPath (both at working-tree and git-commit read paths)"
        );
    }

    #[test]
    fn test_read_path_bytes_rejects_dotdot_traversal() {
        use crate::storage::PathReadError;

        let repo_root = TempDir::new().unwrap();
        let jit_dir = repo_root.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();
        let storage = JsonFileStorage::new(&jit_dir);

        for bad in &["../etc/passwd", "a/../b", "foo/..", "a/../../b"] {
            let result = storage.read_path_bytes(bad, None);
            assert!(
                matches!(result, Err(PathReadError::InvalidPath(_))),
                "`..` traversal must be rejected with InvalidPath: {bad}"
            );
        }
    }

    /// Path validation must ALLOW legitimate filenames that contain `..` as
    /// part of a segment (e.g. `foo..bar.txt`).  Only `..` as a *whole segment*
    /// is forbidden — dots *inside* a segment are fine.  This is explicitly
    /// called out in the issue acceptance criteria and protects against over-
    /// zealous validators that reject filenames like `archive..2024.tar.gz`.
    #[test]
    fn test_read_path_bytes_allows_dots_inside_segment() {
        let repo_root = TempDir::new().unwrap();
        let jit_dir = repo_root.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();

        // Create the fixture file with `..` inside the segment name.
        let expected = b"legitimate file contents";
        fs::write(repo_root.path().join("foo..bar.txt"), expected).unwrap();
        // Also cover a nested variant to be sure the segment-walker doesn't
        // mis-classify a middle segment with embedded dots.
        let nested_dir = repo_root.path().join("docs");
        fs::create_dir_all(&nested_dir).unwrap();
        fs::write(nested_dir.join("archive..2024.md"), expected).unwrap();

        let storage = JsonFileStorage::new(&jit_dir);

        let (bytes, label) = storage
            .read_path_bytes("foo..bar.txt", None)
            .expect("filename with dots INSIDE a segment must be allowed");
        assert_eq!(bytes, expected);
        assert_eq!(label, "working-tree");

        let (bytes, label) = storage
            .read_path_bytes("docs/archive..2024.md", None)
            .expect("nested path with dots inside a segment must be allowed");
        assert_eq!(bytes, expected);
        assert_eq!(label, "working-tree");
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
        let result = storage.read_path_bytes("docs/escape.txt", None);
        assert!(
            matches!(result, Err(PathReadError::OutsideRepoRoot(_))),
            "symlink pointing outside repo root must be rejected with OutsideRepoRoot, got: {result:?}"
        );
    }

    /// Repository-format compatibility guard (issue def64ac4).
    mod format_compat_tests {
        use super::*;
        use crate::storage::RepositoryFormatTooNewError;

        /// Write a raw `index.json` carrying an explicit `schema_version` into a
        /// freshly-created `.jit`, returning the storage handle.
        fn storage_with_index_version(temp: &TempDir, version: u32) -> JsonFileStorage {
            let storage = JsonFileStorage::new(temp.path());
            let index_path = temp.path().join(INDEX_FILE);
            let raw = format!(
                "{{\n  \"schema_version\": {version},\n  \"all_ids\": [],\n  \"deleted_ids\": []\n}}"
            );
            fs::write(&index_path, raw).unwrap();
            storage
        }

        // REQ-01: the constant is the single authoritative marker — a new index is
        // stamped with SUPPORTED_INDEX_SCHEMA_VERSION.
        #[test]
        fn test_new_index_is_stamped_with_supported_version() {
            assert_eq!(
                Index::default().schema_version,
                SUPPORTED_INDEX_SCHEMA_VERSION
            );

            let (_temp_dir, storage) = setup_storage();
            assert_eq!(
                storage.load_index().unwrap().schema_version,
                SUPPORTED_INDEX_SCHEMA_VERSION
            );
        }

        // REQ-02: a repo whose format version exceeds the binary's support refuses
        // to load, via a typed error naming BOTH versions on a single line, rather
        // than a file-read/parse error.
        #[test]
        fn test_newer_repo_format_refused_on_direct_load() {
            let temp_dir = TempDir::new().unwrap();
            let newer = SUPPORTED_INDEX_SCHEMA_VERSION + 1;
            let storage = storage_with_index_version(&temp_dir, newer);

            let err = storage
                .load_index()
                .expect_err("newer format must be refused");
            let typed = err
                .downcast_ref::<RepositoryFormatTooNewError>()
                .expect("must be the typed format-too-new error, not a parse/read error");
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
        // surfaces the format-too-new error instead of swallowing it.
        #[test]
        fn test_newer_repo_format_refused_on_aggregated_read() {
            let temp_dir = TempDir::new().unwrap();
            let newer = SUPPORTED_INDEX_SCHEMA_VERSION + 1;
            let storage = storage_with_index_version(&temp_dir, newer);

            let err = storage
                .list_issues()
                .expect_err("aggregated read must be refused");
            assert!(err.downcast_ref::<RepositoryFormatTooNewError>().is_some());
        }

        #[test]
        fn test_aggregated_read_rejects_every_invalid_local_index_shape() {
            let cases = [
                (
                    r#"{"schema_version":2,"all_ids":["a","a"],"deleted_ids":[]}"#,
                    "duplicate active issue id 'a'",
                ),
                (
                    r#"{"schema_version":2,"all_ids":[],"deleted_ids":["a","a"]}"#,
                    "duplicate deleted issue id 'a'",
                ),
                (
                    r#"{"schema_version":2,"all_ids":["a"],"deleted_ids":["a"]}"#,
                    "both active and deleted",
                ),
                ("{ invalid json", "failed to parse index"),
            ];

            for (raw, expected) in cases {
                let temp_dir = TempDir::new().unwrap();
                let storage = JsonFileStorage::new(temp_dir.path());
                fs::write(temp_dir.path().join(INDEX_FILE), raw).unwrap();
                let error = storage
                    .load_aggregated_index()
                    .expect_err("invalid local index must abort aggregation");
                assert!(
                    error.to_string().contains(expected),
                    "expected {expected:?} in {error:#}"
                );
            }
        }

        // REQ-03: a repo whose version EQUALS the binary's support operates normally
        // (critical no-regression case: version == supported == today's repo).
        #[test]
        fn test_equal_version_operates_normally() {
            let temp_dir = TempDir::new().unwrap();
            let storage = storage_with_index_version(&temp_dir, SUPPORTED_INDEX_SCHEMA_VERSION);
            assert!(storage.load_index().is_ok());
            assert!(storage.list_issues().is_ok());
        }

        // REQ-03: a repo whose version is LESS than the binary's support also
        // operates normally.
        #[test]
        fn test_older_version_operates_normally() {
            let temp_dir = TempDir::new().unwrap();
            let older = SUPPORTED_INDEX_SCHEMA_VERSION - 1;
            let storage = storage_with_index_version(&temp_dir, older);
            let index = storage.load_index().expect("older format must still load");
            assert_eq!(index.schema_version, older);
            assert!(storage.list_issues().is_ok());
        }
    }
}
