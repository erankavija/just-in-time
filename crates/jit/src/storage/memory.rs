//! In-memory storage implementation for testing.
//!
//! This backend stores all data in RAM using HashMaps, providing 10-100x faster
//! test execution compared to JSON file I/O. Thread-safe for concurrent access.

use crate::declarations::{parse_gate_registry, GateRegistry};
use crate::domain::{Event, GateRunResult, Issue};
use crate::repository_state::{
    gate_run_result_relative_path, EntryIdentity, FileMode, RepositoryEntry, RepositoryRootClass,
    RootRelativePath, VirtualPath,
};
use crate::storage::{
    AmbiguousIdError, GateRunNotFoundError, InvalidIdPrefixError, IssueNotFoundError, IssueStore,
    PresetNotFoundError, RepoWriteGuard, RepoWriteLock, MIN_ID_PREFIX_LENGTH,
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
/// aggregate [`MemoryRepositoryState`] image, the single store a recovered
/// mutation session captures and applies. There is no parallel typed cache: a
/// record read through [`IssueStore`] and one published by a session share one
/// source of truth and round-trip through the identical canonical serializers.
#[derive(Clone)]
#[allow(dead_code)] // Public API used only in tests, not in binary
pub struct InMemoryStorage {
    /// Unique root path for parallel test isolation
    root_path: std::path::PathBuf,
    /// Outermost lock of every mutating path, shared by every clone. Process-local:
    /// this backend has no files, so there is no other process to exclude.
    repo_lock: Arc<RepoWriteLock>,
    /// Canonical aggregate repository image used by recovered mutation sessions.
    pub(crate) repository_state: Arc<Mutex<MemoryRepositoryState>>,
    pub(crate) repository_state_failures: Arc<dyn crate::storage::TransactionFailureInjector>,
    /// Canonical layout of the live mutation session, shared by every clone so
    /// reentry is admitted only for the same selected roots.
    pub(crate) active_mutation_layout:
        Arc<crate::storage::repository_state_store::ActiveLayoutTracker>,
    #[cfg(feature = "test-support")]
    repository_state_apply_conflicts: Arc<std::sync::atomic::AtomicUsize>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MemoryRepositoryState {
    pub(crate) entries:
        BTreeMap<crate::repository_state::VirtualPath, crate::repository_state::RepositoryEntry>,
    pub(crate) data_root_exists: bool,
    pub(crate) recovery: Option<MemoryRecoveryResidue>,
}

#[derive(Debug, Clone)]
pub(crate) enum MemoryRecoveryResidue {
    Prepared {
        original: Box<MemoryRepositoryState>,
        final_state: Box<MemoryRepositoryState>,
        _plan_hash: String,
    },
    Committed {
        final_state: Box<MemoryRepositoryState>,
        _plan_hash: String,
    },
}

impl MemoryRepositoryState {
    pub(crate) fn clone_without_recovery(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            data_root_exists: self.data_root_exists,
            recovery: None,
        }
    }
}

impl InMemoryStorage {
    /// Create a new in-memory storage instance.
    #[allow(dead_code)] // Public API used only in tests, not in binary
    pub fn new() -> Self {
        // Generate unique root path for parallel test isolation
        let unique_id = uuid::Uuid::new_v4();
        let root_path = std::path::PathBuf::from(format!("/tmp/jit-test-{}", unique_id));

        Self {
            root_path,
            repo_lock: RepoWriteLock::in_process(),
            repository_state: Arc::new(Mutex::new(MemoryRepositoryState::default())),
            repository_state_failures: Arc::new(crate::storage::NoTransactionFailures),
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

    #[cfg(test)]
    pub(crate) fn with_repository_state_failures(
        failures: Arc<dyn crate::storage::TransactionFailureInjector>,
    ) -> Self {
        let mut storage = Self::new();
        storage.repository_state_failures = failures;
        storage
    }

    /// A view sharing this backend's aggregate state, lock, and layout tracker but
    /// with a clean failure injector. Mirrors opening a fresh `JsonFileStorage`
    /// over the same on-disk root for recovery: same state, no injected faults.
    #[cfg(test)]
    pub(crate) fn without_repository_state_failures(&self) -> Self {
        Self {
            repository_state_failures: Arc::new(crate::storage::NoTransactionFailures),
            ..self.clone()
        }
    }

    /// Reachable under `feature = "test-support"` in addition to `cfg(test)`:
    /// `commands::test_helpers::with_open_race` calls it, and `test_helpers`
    /// itself is reachable independent of `cfg(test)` when the feature is
    /// enabled without a test build (e.g. `cargo clippy --features
    /// test-support`). `with_repository_state_failures` and
    /// `without_repository_state_failures` stay `cfg(test)`-only: every other
    /// caller lives inside `#[cfg(test)]` test modules, which always compile
    /// with `test-support` active via the crate's own dev-dependency. Unused
    /// in the `feature`-on/`cfg(test)`-off configuration itself, since its
    /// only non-`cfg(test)` caller (`with_open_race`) is unreached there too.
    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
    pub(crate) fn with_repository_state_failure_view(
        &self,
        failures: Arc<dyn crate::storage::TransactionFailureInjector>,
    ) -> Self {
        Self {
            repository_state_failures: failures,
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

    fn gate_presets_from_repository_state(
        &self,
    ) -> Result<(
        std::collections::HashMap<String, crate::gate_presets::GatePresetDefinition>,
        std::collections::HashSet<String>,
    )> {
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
                    .strip_prefix("config/gate-presets/")?;
                if filename.contains('/') || !filename.ends_with(".json") {
                    return None;
                }
                Some(match entry {
                    RepositoryEntry::File { bytes, .. } => {
                        Ok((filename.to_string(), bytes.clone()))
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
        use crate::repository_state::{EntryIdentity, FileMode, RepositoryEntry};
        self.repository_layout()
            .ensure_canonical(&vpath)
            .map_err(|error| crate::storage::PathReadError::InvalidPath(error.to_string()))?;
        let identity = EntryIdentity::for_bytes(
            format!(
                "memory:{:?}:{}",
                vpath.root_class(),
                vpath.relative().as_str()
            ),
            content.as_bytes(),
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
                identity,
                bytes: content.as_bytes().to_vec(),
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
    fn issue_vpath(id: &str) -> VirtualPath {
        VirtualPath::data(format!("issues/{id}.json")).expect("issue record path is canonical")
    }

    /// Canonical `Data(...)` identity for one gate-run result.
    fn gate_run_vpath(run_id: &str) -> Result<VirtualPath> {
        Ok(VirtualPath::data(
            gate_run_result_relative_path(run_id)?.as_path(),
        )?)
    }

    fn gate_runs_vpath() -> Result<VirtualPath> {
        Ok(VirtualPath::data("gate-runs")?)
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
        let (Some("gate-runs"), Some(run_id), None) =
            (components.next(), components.next(), components.next())
        else {
            return None;
        };
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

    /// Canonical `Data(...)` identity for the gate registry.
    fn gate_registry_vpath() -> VirtualPath {
        VirtualPath::data("gates.toml").expect("gate registry path is canonical")
    }

    /// Canonical `Data(...)` identity for the audit log.
    fn events_vpath() -> VirtualPath {
        VirtualPath::data("events.jsonl").expect("audit log path is canonical")
    }

    /// Publish a malformed fixture as a captured `File` entry.
    #[cfg(test)]
    fn put_data_entry(state: &mut MemoryRepositoryState, vpath: VirtualPath, bytes: Vec<u8>) {
        let object = vpath.relative().as_str().to_owned();
        let identity =
            EntryIdentity::for_bytes(object, &bytes).expect("entry identity is hashable");
        Self::mark_data_root_existing(state);
        Self::ensure_ancestor_dirs(state, &vpath);
        state.entries.insert(
            vpath,
            RepositoryEntry::File {
                identity,
                bytes,
                mode: FileMode::Regular,
            },
        );
    }

    /// Model a `Directory` entry for every ancestor of `vpath` up to (but not
    /// including) the data root, so the aggregate image reflects the directory
    /// tree a real repository carries. A recovered mutation session verifies that
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
                    .or_insert_with(|| RepositoryEntry::Directory {
                        identity: EntryIdentity::for_bytes(
                            format!("memory-directory:{}", dir.display()),
                            b"directory",
                        )
                        .expect("directory identity is hashable"),
                        mode: FileMode::Executable,
                    });
            }
            ancestor = dir.parent();
        }
    }

    /// Read the exact captured bytes for `vpath` from the aggregate image.
    fn data_entry_bytes(state: &MemoryRepositoryState, vpath: &VirtualPath) -> Option<Vec<u8>> {
        match state.entries.get(vpath) {
            Some(RepositoryEntry::File { bytes, .. }) => Some(bytes.clone()),
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
            let RepositoryEntry::File { bytes, .. } = entry else {
                return None;
            };
            let rel = vpath.relative().as_str();
            let leaf = rel.strip_prefix(&prefix)?;
            if leaf.contains('/') || !leaf.ends_with(suffix) {
                return None;
            }
            Some((leaf.to_owned(), bytes.clone()))
        })
    }

    /// Deserialize every captured issue record in the aggregate image.
    fn load_issues(state: &MemoryRepositoryState) -> Result<Vec<Issue>> {
        Self::data_files_in(state, "issues", ".json")
            .map(|(_, bytes)| {
                serde_json::from_slice::<Issue>(&bytes)
                    .context("Failed to deserialize issue from repository image")
            })
            .collect()
    }

    /// Deserialize one captured issue record, if present.
    fn load_issue_entry(state: &MemoryRepositoryState, id: &str) -> Result<Option<Issue>> {
        Self::data_entry_bytes(state, &Self::issue_vpath(id))
            .map(|bytes| {
                serde_json::from_slice::<Issue>(&bytes)
                    .context("Failed to deserialize issue from repository image")
            })
            .transpose()
    }

    /// Deserialize the captured audit log, oldest event first.
    fn load_events(state: &MemoryRepositoryState) -> Result<Vec<Event>> {
        let Some(bytes) = Self::data_entry_bytes(state, &Self::events_vpath()) else {
            return Ok(Vec::new());
        };
        String::from_utf8_lossy(&bytes)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str::<Event>(line)
                    .context("Failed to deserialize event from repository image")
            })
            .collect()
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
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

    fn resolve_issue_id(&self, partial_id: &str) -> Result<String> {
        // Normalize input: lowercase and remove hyphens
        let normalized = partial_id.to_lowercase().replace('-', "");

        // Full UUID check (fast path) - 32 hex chars without hyphens
        if normalized.len() == 32 {
            // Verify it exists
            return self
                .load_issue(partial_id)
                .map(|issue| issue.id)
                .map_err(|_| IssueNotFoundError::new(partial_id).into());
        }

        // Minimum length check
        if normalized.len() < MIN_ID_PREFIX_LENGTH {
            return Err(InvalidIdPrefixError::new(partial_id).into());
        }

        // Find matching issues from the captured records.
        let issues = Self::load_issues(&self.repository_state())?;
        let matches: Vec<&Issue> = issues
            .iter()
            .filter(|issue| {
                issue
                    .id
                    .replace('-', "")
                    .to_lowercase()
                    .starts_with(&normalized)
            })
            .collect();

        match matches.len() {
            0 => Err(IssueNotFoundError::new(partial_id).into()),
            1 => Ok(matches[0].id.clone()),
            _ => {
                // Get titles for better error message
                let issue_list: Vec<String> = matches
                    .iter()
                    .map(|issue| format!("{} | {}", issue.short_id(), issue.title))
                    .collect();
                Err(AmbiguousIdError::issue(partial_id, issue_list).into())
            }
        }
    }

    fn list_issues(&self) -> Result<Vec<Issue>> {
        Self::load_issues(&self.repository_state())
    }

    fn load_gate_registry(&self) -> Result<GateRegistry> {
        match Self::data_entry_bytes(&self.repository_state(), &Self::gate_registry_vpath()) {
            Some(bytes) => parse_gate_registry(&bytes)
                .context("Failed to deserialize gate registry from repository image"),
            None => Ok(GateRegistry::default()),
        }
    }

    fn read_events(&self) -> Result<Vec<Event>> {
        Self::load_events(&self.repository_state())
    }

    fn root(&self) -> &std::path::Path {
        // Return unique root path for parallel test isolation
        &self.root_path
    }

    fn load_gate_run_result(&self, run_id: &str) -> Result<GateRunResult> {
        let path = Self::gate_run_vpath(run_id)?;
        let run_dir = Self::gate_run_directory_vpath(run_id)?;
        let state = self.repository_state();
        if !Self::gate_run_parent_is_directory(&state, &Self::gate_runs_vpath()?)?
            || !Self::gate_run_parent_is_directory(&state, &run_dir)?
        {
            return Err(GateRunNotFoundError::new(run_id).into());
        }
        match state.entries.get(&path) {
            None | Some(RepositoryEntry::Absent) => Err(GateRunNotFoundError::new(run_id).into()),
            Some(RepositoryEntry::File { bytes, .. }) => serde_json::from_slice(bytes)
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
        let state = self.repository_state();
        if !Self::gate_run_parent_is_directory(&state, &Self::gate_runs_vpath()?)? {
            return Ok(Vec::new());
        }
        let run_ids = state
            .entries
            .iter()
            .filter_map(|(vpath, entry)| match entry {
                RepositoryEntry::Directory { .. } => Self::gate_run_directory_id(vpath),
                RepositoryEntry::Absent
                | RepositoryEntry::File { .. }
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
                    Some(RepositoryEntry::File { bytes, .. }) => Some(
                        serde_json::from_slice::<GateRunResult>(bytes).with_context(|| {
                            format!(
                                "Failed to deserialize gate-run result at {}",
                                vpath.relative().as_path().display()
                            )
                        }),
                    ),
                    Some(_) => Some(Err(anyhow!(
                        "Gate run result at {} must be an ordinary file",
                        vpath.relative().as_path().display()
                    ))),
                }
            })
            .filter_map(|result| match result {
                Ok(r) if r.issue_id == issue_id => Some(Ok(r)),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
            .collect()
    }

    fn list_gate_presets(&self) -> Result<Vec<crate::gate_presets::PresetInfo>> {
        let (presets, custom_names) = self.gate_presets_from_repository_state()?;
        let mut presets = presets
            .values()
            .map(|preset| crate::gate_presets::PresetInfo {
                name: preset.name.clone(),
                description: preset.description.clone(),
                gate_count: preset.gates.len(),
                builtin: !custom_names.contains(&preset.name),
            })
            .collect::<Vec<_>>();
        presets.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(presets)
    }

    fn get_gate_preset(&self, name: &str) -> Result<crate::gate_presets::GatePresetDefinition> {
        let (presets, _) = self.gate_presets_from_repository_state()?;
        presets
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
            Some(RepositoryEntry::File { bytes, .. }) => {
                Some(String::from_utf8_lossy(bytes).into_owned())
            }
            _ => None,
        })
    }

    fn read_path_bytes(
        &self,
        path: &str,
        _at_commit: Option<&str>,
    ) -> Result<(Vec<u8>, String), crate::storage::PathReadError> {
        use crate::storage::PathReadError;
        // InMemoryStorage has no filesystem or git backing of its own, but it
        // must enforce the same repo-relative path contract as JsonFileStorage
        // so that every IssueStore implementation uniformly rejects empty,
        // absolute, or `..`-bearing paths.  Containment against a real repo
        // root is not meaningful here (the synthetic root doesn't correspond
        // to an on-disk directory), but the shape-level checks still apply.
        if path.is_empty() {
            return Err(PathReadError::InvalidPath(
                "path must not be empty".to_string(),
            ));
        }
        if path.starts_with('/') {
            return Err(PathReadError::InvalidPath(format!(
                "absolute paths are not permitted: {}",
                path
            )));
        }
        for segment in path.split('/') {
            if segment == ".." {
                return Err(PathReadError::InvalidPath(format!(
                    "'..' segment not permitted: {}",
                    path
                )));
            }
        }
        // The InMemoryStorage root is a synthetic path and usually does not
        // exist on disk, so reads will normally fail with NotFound.  Tests
        // that need real file I/O use `JsonFileStorage` pointed at a tempdir.
        let joined = self.root_path.join(path);
        std::fs::read(&joined)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn gate_run_parent_entry(kind: &str) -> RepositoryEntry {
        let identity =
            || EntryIdentity::for_bytes(format!("parent-{kind}"), kind.as_bytes()).unwrap();
        match kind {
            "absent" => RepositoryEntry::Absent,
            "directory" => RepositoryEntry::Directory {
                identity: identity(),
                mode: FileMode::Executable,
            },
            "file" => RepositoryEntry::File {
                identity: identity(),
                bytes: b"not a directory".to_vec(),
                mode: FileMode::Regular,
            },
            "symlink" => RepositoryEntry::Symlink {
                identity: identity(),
                target: b"elsewhere".to_vec(),
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
        let nested = VirtualPath::data("gate-runs/outer/nested/result.json").unwrap();
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
        assert!(format!("{load_error:#}").contains("gate-runs/corrupt-run/result.json"));
        let list_error = storage.list_gate_runs_for_issue("any-issue").unwrap_err();
        assert!(format!("{list_error:#}").contains("gate-runs/corrupt-run/result.json"));
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
            storage.repository_state().entries.insert(
                InMemoryStorage::gate_runs_vpath().unwrap(),
                gate_run_parent_entry(kind),
            );

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
            let root = InMemoryStorage::gate_runs_vpath().unwrap();
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
        storage.add_data_file("config.toml", "[project]\nname = \"x\"\n");
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
    fn test_worktree_fixture_does_not_materialize_data_root() {
        let storage = InMemoryStorage::new();
        storage.add_worktree_file("docs/guide.md", "guide");

        let state = storage.repository_state();
        assert!(!state.data_root_exists);
        assert!(!state.entries.contains_key(&VirtualPath::data("").unwrap()));
        assert!(matches!(
            state.entries.get(&VirtualPath::worktree("docs/guide.md").unwrap()),
            Some(RepositoryEntry::File { bytes, .. }) if bytes == b"guide"
        ));
    }

    #[test]
    fn test_read_repo_file_present_absent_and_path_safety() {
        let storage = InMemoryStorage::new();
        // Present -> Some(content).
        storage.add_worktree_file("project-items.md", "hello");
        assert_eq!(
            storage
                .read_repo_file("project-items.md")
                .unwrap()
                .as_deref(),
            Some("hello")
        );
        // Absent -> None (graceful).
        assert!(storage.read_repo_file("missing.md").unwrap().is_none());
        // Path-safety: empty, absolute, and `..`-traversal are typed InvalidPath.
        assert!(matches!(
            storage.read_repo_file(""),
            Err(crate::storage::PathReadError::InvalidPath(_))
        ));
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
    fn test_load_nonexistent_issue_fails() {
        let storage = InMemoryStorage::new();

        let result = storage.load_issue("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_list_issues_empty() {
        let storage = InMemoryStorage::new();

        let issues = storage.list_issues().unwrap();
        assert_eq!(issues.len(), 0);
    }

    #[test]
    fn test_read_path_bytes_missing_file_returns_not_found() {
        use crate::storage::PathReadError;

        let storage = InMemoryStorage::new();

        // Pass a repo-relative path that does not exist on disk.  The synthetic
        // root points at `/tmp/jit-test-{uuid}` which normally doesn't exist,
        // so the read fails with NotFound.
        let result = storage.read_path_bytes("nonexistent/path/file.md", None);
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
    fn test_read_path_bytes_rejects_absolute_path() {
        use crate::storage::PathReadError;

        let storage = InMemoryStorage::new();

        let result = storage.read_path_bytes("/etc/passwd", None);
        assert!(
            matches!(result, Err(PathReadError::InvalidPath(_))),
            "absolute path must be rejected as InvalidPath"
        );
    }

    #[test]
    fn test_read_path_bytes_rejects_dotdot_segment() {
        use crate::storage::PathReadError;

        let storage = InMemoryStorage::new();

        let result = storage.read_path_bytes("../etc/passwd", None);
        assert!(
            matches!(result, Err(PathReadError::InvalidPath(_))),
            "`..` traversal must be rejected as InvalidPath"
        );
    }

    #[test]
    fn test_read_path_bytes_rejects_empty_path() {
        use crate::storage::PathReadError;

        let storage = InMemoryStorage::new();

        let result = storage.read_path_bytes("", None);
        assert!(
            matches!(result, Err(PathReadError::InvalidPath(_))),
            "empty path must be rejected as InvalidPath"
        );
    }
}
