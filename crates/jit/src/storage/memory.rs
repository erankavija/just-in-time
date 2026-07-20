//! In-memory storage implementation for testing.
//!
//! This backend stores all data in RAM using HashMaps, providing 10-100x faster
//! test execution compared to JSON file I/O. Thread-safe for concurrent access.

use crate::declarations::{parse_gate_registry, serialize_gate_registry, GateRegistry};
use crate::domain::{Event, GateRunResult, Issue};
use crate::repository_state::{
    serialize_event, serialize_gate_run, serialize_issue, EntryIdentity, FileMode, RepositoryEntry,
    RepositoryRootClass, RootRelativePath, VirtualPath,
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
/// record written through an [`IssueStore`] method and one published by a
/// session share one source of truth and round-trip through the identical
/// canonical serializers.
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
        }
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

    /// Map a repo-relative path to its canonical [`VirtualPath`] key in the
    /// aggregate repository image: a `.jit/`-prefixed path is a `Data(...)` entry,
    /// every other repo-relative path a `Worktree(...)` entry. This is the SAME
    /// mapping the materialization producers and command capture specs use, so a
    /// file written through [`IssueStore::write_repo_file`] is captured by a
    /// mutation session under the identical key.
    fn repo_file_vpath(
        rel_path: &str,
    ) -> Result<crate::repository_state::VirtualPath, crate::storage::PathReadError> {
        use crate::repository_state::VirtualPath;
        crate::storage::validate_repo_relative_path(rel_path)?;
        match rel_path.strip_prefix(".jit/") {
            Some(rest) => VirtualPath::data(rest),
            None => VirtualPath::worktree(rel_path),
        }
        .map_err(|e| crate::storage::PathReadError::InvalidPath(e.to_string()))
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
        rel_path: &str,
        content: &str,
    ) -> Result<(), crate::storage::PathReadError> {
        use crate::repository_state::{EntryIdentity, FileMode, RepositoryEntry};
        let vpath = Self::repo_file_vpath(rel_path)?;
        let identity = EntryIdentity::for_bytes(rel_path, content.as_bytes())
            .map_err(|e| crate::storage::PathReadError::Other(anyhow!("{e}")))?;
        let mut state = self.repository_state();
        Self::mark_data_root_existing(&mut state);
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

    /// Seed an in-memory repository file at `rel_path` with `content`.
    ///
    /// Mirrors the issue/gate seeding pattern: a test setter so command and domain
    /// tests can stage a config-declared file (e.g. a project-scope item source)
    /// and have [`IssueStore::read_repo_file`](crate::storage::IssueStore::read_repo_file)
    /// return it, without touching disk. The file lands in the same aggregate image
    /// a mutation session captures, so seeded state and session-published state
    /// share one store.
    pub fn add_repo_file(&self, rel_path: &str, content: &str) {
        self.insert_repo_file(rel_path, content)
            .expect("add_repo_file requires a valid repo-relative path");
    }

    /// Canonical `Data(...)` identity for one issue record.
    fn issue_vpath(id: &str) -> VirtualPath {
        VirtualPath::data(format!("issues/{id}.json")).expect("issue record path is canonical")
    }

    /// Canonical `Data(...)` identity for one gate-run result.
    fn gate_run_vpath(run_id: &str) -> VirtualPath {
        VirtualPath::data(format!("gate-runs/{run_id}/result.json"))
            .expect("gate-run result path is canonical")
    }

    /// Canonical `Data(...)` identity for the gate registry.
    fn gate_registry_vpath() -> VirtualPath {
        VirtualPath::data("gates.toml").expect("gate registry path is canonical")
    }

    /// Canonical `Data(...)` identity for the audit log.
    fn events_vpath() -> VirtualPath {
        VirtualPath::data("events.jsonl").expect("audit log path is canonical")
    }

    /// Publish `bytes` as the captured `File` entry for `vpath` in the aggregate
    /// image, marking the data root existing. This is the single byte-writing
    /// path behind every repository-owned typed record so that seeded state,
    /// per-method writes, and session-published deltas share one store.
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
            if let Ok(dir_path) =
                RootRelativePath::parse(dir).and_then(|rel| VirtualPath::from_root(vpath.root_class(), rel))
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

    /// Insert `issue` under the repository write lock.
    ///
    /// The single write path behind [`IssueStore::save_issue`] and
    /// [`IssueStore::restore_issue_verbatim`], which differ only in whether the
    /// caller's `updated_at` is stamped before the write reaches here. Both
    /// backends must agree on that, or the atomicity tests over this one prove
    /// nothing about [`JsonFileStorage`](crate::storage::JsonFileStorage). The
    /// issue is persisted as its canonical bytes in the aggregate image, exactly
    /// as a mutation session publishes it.
    fn persist_issue(&self, issue: Issue) -> Result<()> {
        let _repo_lock = self.repo_lock.acquire()?;
        let bytes = serialize_issue(&issue).map_err(|e| anyhow!("{e}"))?;
        Self::put_data_entry(
            &mut self.repository_state(),
            Self::issue_vpath(&issue.id),
            bytes,
        );
        Ok(())
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl IssueStore for InMemoryStorage {
    fn init(&self) -> Result<()> {
        // In-memory storage is already initialized
        // Configuration is managed by ConfigManager
        Ok(())
    }

    fn acquire_repo_write_lock(&self) -> Result<RepoWriteGuard> {
        self.repo_lock.acquire()
    }

    fn save_issue(&self, mut issue: Issue) -> Result<()> {
        // Update the updated_at timestamp (storage responsibility)
        issue.updated_at = chrono::Utc::now();
        self.persist_issue(issue)
    }

    fn restore_issue_verbatim(&self, issue: Issue) -> Result<()> {
        self.persist_issue(issue)
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

    fn delete_issue(&self, id: &str) -> Result<()> {
        let _repo_lock = self.repo_lock.acquire()?;
        let mut state = self.repository_state();
        let vpath = Self::issue_vpath(id);
        if state.entries.remove(&vpath).is_none() {
            return Err(IssueNotFoundError::new(id).into());
        }
        Ok(())
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

    fn save_gate_registry(&self, registry: &GateRegistry) -> Result<()> {
        let _repo_lock = self.repo_lock.acquire()?;
        let bytes = serialize_gate_registry(registry).map_err(|e| anyhow!("{e}"))?;
        Self::put_data_entry(
            &mut self.repository_state(),
            Self::gate_registry_vpath(),
            bytes,
        );
        Ok(())
    }

    fn append_event(&self, event: &Event) -> Result<()> {
        let _repo_lock = self.repo_lock.acquire()?;
        let mut line = serialize_event(event).map_err(|e| anyhow!("{e}"))?;
        line.push(b'\n');
        let mut state = self.repository_state();
        // Append to the exact captured audit-log bytes, preserving every prior
        // byte, exactly as the canonical finalizer appends one JSONL record.
        let mut bytes = Self::data_entry_bytes(&state, &Self::events_vpath()).unwrap_or_default();
        bytes.extend_from_slice(&line);
        Self::put_data_entry(&mut state, Self::events_vpath(), bytes);
        Ok(())
    }

    fn read_events(&self) -> Result<Vec<Event>> {
        Self::load_events(&self.repository_state())
    }

    fn root(&self) -> &std::path::Path {
        // Return unique root path for parallel test isolation
        &self.root_path
    }

    fn save_gate_run_result(&self, result: &GateRunResult) -> Result<()> {
        let bytes = serialize_gate_run(result).map_err(|e| anyhow!("{e}"))?;
        Self::put_data_entry(
            &mut self.repository_state(),
            Self::gate_run_vpath(&result.run_id),
            bytes,
        );
        Ok(())
    }

    fn load_gate_run_result(&self, run_id: &str) -> Result<GateRunResult> {
        Self::data_entry_bytes(&self.repository_state(), &Self::gate_run_vpath(run_id))
            .map(|bytes| {
                serde_json::from_slice::<GateRunResult>(&bytes)
                    .context("Failed to deserialize gate-run result from repository image")
            })
            .transpose()?
            .ok_or_else(|| GateRunNotFoundError::new(run_id).into())
    }

    fn list_gate_runs_for_issue(&self, issue_id: &str) -> Result<Vec<GateRunResult>> {
        let state = self.repository_state();
        state
            .entries
            .iter()
            .filter_map(|(vpath, entry)| {
                if vpath.root_class() != RepositoryRootClass::Data {
                    return None;
                }
                let RepositoryEntry::File { bytes, .. } = entry else {
                    return None;
                };
                let rel = vpath.relative().as_str();
                // Match exactly `gate-runs/<run_id>/result.json`.
                let inner = rel
                    .strip_prefix("gate-runs/")?
                    .strip_suffix("/result.json")?;
                if inner.is_empty() || inner.contains('/') {
                    return None;
                }
                Some(bytes.clone())
            })
            .map(|bytes| {
                serde_json::from_slice::<GateRunResult>(&bytes)
                    .context("Failed to deserialize gate-run result from repository image")
            })
            .filter_map(|result| match result {
                Ok(r) if r.issue_id == issue_id => Some(Ok(r)),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
            .collect()
    }

    fn list_gate_presets(&self) -> Result<Vec<crate::gate_presets::PresetInfo>> {
        // InMemoryStorage only supports builtin presets (no custom presets in tests)
        let presets = crate::gate_presets::BuiltinPresets::load()?;

        Ok(presets
            .values()
            .map(|preset| crate::gate_presets::PresetInfo {
                name: preset.name.clone(),
                description: preset.description.clone(),
                gate_count: preset.gates.len(),
                builtin: true,
            })
            .collect())
    }

    fn get_gate_preset(&self, name: &str) -> Result<crate::gate_presets::GatePresetDefinition> {
        let presets = crate::gate_presets::BuiltinPresets::load()?;
        presets
            .get(name)
            .cloned()
            .ok_or_else(|| PresetNotFoundError::new(name).into())
    }

    fn save_gate_preset(
        &self,
        _preset: &crate::gate_presets::GatePresetDefinition,
    ) -> Result<std::path::PathBuf> {
        // InMemoryStorage doesn't support saving custom presets
        Err(anyhow!(
            "InMemoryStorage does not support saving custom presets"
        ))
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
        let vpath = Self::repo_file_vpath(rel_path)?;
        Ok(match self.repository_state().entries.get(&vpath) {
            Some(RepositoryEntry::File { bytes, .. }) => {
                Some(String::from_utf8_lossy(bytes).into_owned())
            }
            _ => None,
        })
    }

    fn write_repo_file(
        &self,
        rel_path: &str,
        content: &str,
    ) -> Result<(), crate::storage::PathReadError> {
        // Enforce the SAME repo-relative path contract as JsonFileStorage (reject
        // empty, absolute, or `..`-bearing paths), then store into the aggregate
        // repository image. A subsequent `read_repo_file` for the same path returns
        // the written content, and a mutation session captures the same bytes.
        self.insert_repo_file(rel_path, content)
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
    use crate::declarations::GateDefinition;
    use crate::domain::{Priority, State};

    #[test]
    fn test_init_is_noop() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        storage.init().unwrap(); // Should be idempotent
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
        storage.add_repo_file(".jit/config.toml", "[project]\nname = \"x\"\n");
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
    fn test_read_repo_file_present_absent_and_path_safety() {
        let storage = InMemoryStorage::new();
        // Present -> Some(content).
        storage.add_repo_file("project-items.md", "hello");
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
    fn test_save_and_load_issue() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let issue = Issue::new("Test".to_string(), "Description".to_string());
        storage.save_issue(issue.clone()).unwrap();

        let loaded = storage.load_issue(&issue.id).unwrap();
        assert_eq!(loaded.id, issue.id);
        assert_eq!(loaded.title, "Test");
        assert_eq!(loaded.description, "Description");
    }

    #[test]
    fn test_save_updates_existing_issue() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let mut issue = Issue::new("Original".to_string(), "Desc".to_string());
        storage.save_issue(issue.clone()).unwrap();

        issue.title = "Updated".to_string();
        storage.save_issue(issue.clone()).unwrap();

        let loaded = storage.load_issue(&issue.id).unwrap();
        assert_eq!(loaded.title, "Updated");

        // Should only have one issue
        let issues = storage.list_issues().unwrap();
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn test_load_nonexistent_issue_fails() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let result = storage.load_issue("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_delete_issue() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let issue = Issue::new("Delete me".to_string(), "Test".to_string());
        storage.save_issue(issue.clone()).unwrap();

        storage.delete_issue(&issue.id).unwrap();

        let result = storage.load_issue(&issue.id);
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_nonexistent_issue_fails() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let result = storage.delete_issue("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_issues() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let issue1 = Issue::new("Issue 1".to_string(), "First".to_string());
        let issue2 = Issue::new("Issue 2".to_string(), "Second".to_string());

        storage.save_issue(issue1.clone()).unwrap();
        storage.save_issue(issue2.clone()).unwrap();

        let issues = storage.list_issues().unwrap();
        assert_eq!(issues.len(), 2);

        let titles: Vec<_> = issues.iter().map(|i| i.title.as_str()).collect();
        assert!(titles.contains(&"Issue 1"));
        assert!(titles.contains(&"Issue 2"));
    }

    #[test]
    fn test_list_issues_empty() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let issues = storage.list_issues().unwrap();
        assert_eq!(issues.len(), 0);
    }

    #[test]
    fn test_gate_registry_operations() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let registry = storage.load_gate_registry().unwrap();
        assert_eq!(registry.gates.len(), 0);

        let mut new_registry = GateRegistry::default();
        let gate = GateDefinition {
            version: 1,
            key: "test-gate".to_string(),
            title: "Test Gate".to_string(),
            description: "A test gate".to_string(),
            stage: crate::declarations::GateStage::Postcheck,
            mode: crate::declarations::GateMode::Manual,
            checker: None,
            priority: 100,
            reserved: std::collections::HashMap::new(),
            auto: false,
            example_integration: None,
        };
        new_registry.gates.insert("test-gate".to_string(), gate);

        storage.save_gate_registry(&new_registry).unwrap();

        let loaded = storage.load_gate_registry().unwrap();
        assert_eq!(loaded.gates.len(), 1);
        assert!(loaded.gates.contains_key("test-gate"));
    }

    #[test]
    fn test_event_log_operations() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let issue = Issue::new("Event test".to_string(), "Test".to_string());
        let event = Event::new_issue_created(&issue);

        storage.append_event(&event).unwrap();

        let events = storage.read_events().unwrap();
        assert_eq!(events.len(), 1);
        matches!(events[0], Event::IssueCreated { .. });
    }

    #[test]
    fn test_multiple_events() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let issue1 = Issue::new("Issue 1".to_string(), "Test".to_string());
        let issue2 = Issue::new("Issue 2".to_string(), "Test".to_string());

        storage
            .append_event(&Event::new_issue_created(&issue1))
            .unwrap();
        storage
            .append_event(&Event::new_issue_created(&issue2))
            .unwrap();

        let events = storage.read_events().unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_clone_shares_storage() {
        let storage1 = InMemoryStorage::new();
        storage1.init().unwrap();

        let issue1 = Issue::new("Issue 1".to_string(), "In storage 1".to_string());
        storage1.save_issue(issue1.clone()).unwrap();

        // Clone shares the same underlying storage (via RefCell)
        let storage2 = storage1.clone();
        let loaded = storage2.load_issue(&issue1.id).unwrap();
        assert_eq!(loaded.title, "Issue 1");

        // Verify they share the same underlying storage
        let issue2 = Issue::new("Issue 2".to_string(), "In storage 2".to_string());
        storage2.save_issue(issue2.clone()).unwrap();

        // Both see the same data because they share the RefCell
        let issues1 = storage1.list_issues().unwrap();
        let issues2 = storage2.list_issues().unwrap();
        assert_eq!(issues1.len(), 2);
        assert_eq!(issues2.len(), 2);
    }

    #[test]
    fn test_works_with_complex_issue_state() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        let mut issue = Issue::new("Complex".to_string(), "Test".to_string());
        issue.priority = Priority::Critical;
        issue.state = State::InProgress;
        issue.assignee = Some("agent:test".parse().unwrap());
        issue.dependencies = vec!["dep1".to_string(), "dep2".to_string()];
        issue.gates_required = vec!["gate1".to_string()];
        issue.context.insert("key".to_string(), "value".to_string());

        storage.save_issue(issue.clone()).unwrap();

        let loaded = storage.load_issue(&issue.id).unwrap();
        assert_eq!(loaded.priority, Priority::Critical);
        assert_eq!(loaded.state, State::InProgress);
        assert_eq!(loaded.assignee, Some("agent:test".parse().unwrap()));
        assert_eq!(loaded.dependencies.len(), 2);
        assert_eq!(loaded.gates_required.len(), 1);
        assert_eq!(loaded.context.get("key").unwrap(), "value");
    }

    #[test]
    fn test_read_path_bytes_missing_file_returns_not_found() {
        use crate::storage::PathReadError;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();

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
        storage.init().unwrap();

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
        storage.init().unwrap();

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
        storage.init().unwrap();

        let result = storage.read_path_bytes("", None);
        assert!(
            matches!(result, Err(PathReadError::InvalidPath(_))),
            "empty path must be rejected as InvalidPath"
        );
    }
}
