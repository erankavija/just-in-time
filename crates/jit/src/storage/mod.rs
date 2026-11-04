//! Storage read interfaces and the repository-state publication boundary.
//!
//! [`IssueStore`] exposes typed reads plus lock/session lifecycle controls shared
//! by file-backed and in-memory backends. Repository-owned publication goes only
//! through [`RepositoryStateStore`] capture/apply sessions.

use crate::declarations::GateRegistry;
use crate::domain::artifact_plan::PlanTarget;
use crate::domain::identity::{IssueId, IssueIdPrefix};
use crate::domain::{Event, Issue};
use anyhow::Result;

pub mod artifact_discovery;
pub mod artifact_planning;
pub(crate) mod atomic_write;
pub(crate) mod capability_fs;
pub mod claim_coordinator;
pub mod clock;
#[cfg(test)]
pub(crate) mod contention_probe;
pub mod discovery;
pub mod errors;
pub(crate) mod external_publish;
pub(crate) mod gate_run_reader;
pub mod gate_runs;
pub mod gate_store;
pub mod git;
pub mod git_revision;
pub mod guard_order;
pub(crate) mod init_publish;
#[cfg(test)]
mod issue_store_conformance;
pub mod json;
pub mod lock;
pub mod lock_cleanup;
pub mod memory;
pub mod mutation_interruption;
pub mod path_errors;
pub mod redo_apply;
#[cfg(test)]
mod redo_conformance;
pub mod redo_record;
#[cfg(any(test, feature = "test-support"))]
pub mod reference;
pub mod repo_lock;
pub mod repository_state_store;
pub mod ruleset_store;
pub mod temp_cleanup;
#[cfg(feature = "test-support")]
mod test_support;
pub mod warnings;
pub mod worktree_identity;
pub mod worktree_paths;

// Re-export for convenience
pub use artifact_discovery::{discover_archive_artifacts, ArtifactDiscoveryError};
pub(crate) use artifact_planning::collect_citation_scan_evidence;
pub use artifact_planning::{collect_artifact_classification_facts, resolve_container_destination};
pub use claim_coordinator::{ClaimAcquireLimits, ClaimCoordinator, Lease};
pub use clock::{Clock, SystemClock};
pub use errors::{
    AmbiguousIdError, GateAlreadyExistsError, GateNotFoundError, GateRunNotFoundError,
    IssueNotFoundError, PresetNotFoundError, RepositoryFormatUnsupportedError,
    RepositoryNotFoundError,
};
pub use git_revision::{
    GitRevisionError, GitRevisionResolver, PinnedArtifactRead, PinnedTargetRead,
};
pub use json::{
    read_exact_store, ExactStoreSnapshot, JsonFileStorage, RetainedMutationSessionGuard,
    RetainedSessionSuspendedError,
};
pub use lock::{
    configured_lock_timeout, is_lock_timeout, FileLocker, LockMetadata, LockMode, LockTimeout,
};
pub(in crate::storage) use mutation_interruption::SessionRedoInterruptions;
pub use mutation_interruption::{
    MutationInterruptionInjector, MutationPoint, NoMutationInterruptions,
};
pub use path_errors::{validate_repo_relative_path, PathReadError};
pub use redo_apply::{
    NoRedoInterruptions, RedoApplier, RedoApplyError, RedoInterruptionInjector, RedoPoint,
};
pub use redo_record::{
    EventAppend, PutFile, RedoRecord, RedoRecordError, RECORD_FILE_NAME, RECORD_SCRATCH_DIR,
};
#[cfg(any(test, feature = "test-support"))]
pub use reference::{render_reference_markdown, GateRunField};
pub use repo_lock::{RepoWriteGuard, RepoWriteLock};
pub use repository_state_store::{
    discover_repository_layout, RecoveryDispatchReport, RepositoryApplyOutcome,
    RepositoryMutationSession, RepositoryStateStore, RepositoryStateStoreError,
};
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use test_support::publish_fixture_directory_noreplace;
pub use warnings::StorageWarning;
pub use worktree_paths::WorktreePaths;

pub use memory::MemoryRedoApplier;

pub use memory::InMemoryStorage;

/// The one id among `ids` that `prefix` names, else [`IssueNotFoundError`] or
/// the error `ambiguous` builds from the several matches.
fn resolve_unique_id<'a>(
    prefix: &IssueIdPrefix,
    ids: impl IntoIterator<Item = &'a IssueId>,
    ambiguous: impl FnOnce(&[&'a IssueId]) -> AmbiguousIdError,
) -> Result<IssueId> {
    let matches = ids
        .into_iter()
        .filter(|id| prefix.matches(id))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(IssueNotFoundError::by_prefix(prefix.clone()).into()),
        [id] => Ok((*id).clone()),
        _ => Err(ambiguous(&matches).into()),
    }
}

/// The one id among `ids` that `prefix` names, else the lookup error naming
/// it; `describe` renders each candidate of an ambiguous match.
pub(crate) fn resolve_unique_issue_id<'a>(
    prefix: &IssueIdPrefix,
    ids: impl IntoIterator<Item = &'a IssueId>,
    describe: impl Fn(&IssueId) -> Option<String>,
) -> Result<IssueId> {
    resolve_unique_id(prefix, ids, |matches| {
        AmbiguousIdError::issue(
            prefix.as_str(),
            matches.iter().filter_map(|id| describe(id)),
        )
    })
}

/// The one of an issue's stored `dependencies` that `prefix` names. Matching
/// the stored edges rather than the issue files keeps an edge whose target was
/// deleted resolvable (jit:f847df3f).
///
/// # Errors
///
/// [`IssueNotFoundError`] for no match, or [`AmbiguousIdError::Dependency`] for
/// several.
pub fn resolve_dependency_id(prefix: &IssueIdPrefix, dependencies: &[IssueId]) -> Result<IssueId> {
    resolve_unique_id(prefix, dependencies.iter(), |matches| {
        AmbiguousIdError::dependency(prefix.as_str(), matches.iter().map(|id| (*id).clone()))
    })
}

/// The id of the one of `issues` that `prefix` names.
///
/// # Errors
///
/// [`IssueNotFoundError`] for no match, or [`AmbiguousIdError`] for several.
pub(crate) fn resolve_issue_id_among(prefix: &IssueIdPrefix, issues: &[Issue]) -> Result<IssueId> {
    resolve_unique_issue_id(prefix, issues.iter().map(|issue| &issue.id), |id| {
        issues
            .iter()
            .find(|issue| &issue.id == id)
            .map(describe_issue)
    })
}

/// An ambiguous-match candidate as `short_id | title`.
pub(crate) fn describe_issue(issue: &Issue) -> String {
    format!("{} | {}", issue.short_id(), issue.title)
}

/// Typed repository readers and session lifecycle controls.
///
/// Publication is deliberately absent: commands publish complete semantic
/// changes through [`RepositoryStateStore`]. Implementations must be `Clone` to
/// support shared access patterns.
pub trait IssueStore: Clone {
    /// Bind the canonical repository layout used by repository-relative read
    /// helpers. File-backed storage retains this explicit authority instead of
    /// inferring a worktree from the selected data-root parent.
    #[doc(hidden)]
    fn configure_repository_layout(&self, _layout: &crate::repository_state::RepositoryLayout) {}

    /// Return the explicit repository layout bound to this backend.
    ///
    /// Repository-relative readers use this authority instead of deriving a
    /// worktree from the data-root spelling.
    #[doc(hidden)]
    fn repository_layout(&self) -> Result<crate::repository_state::RepositoryLayout> {
        anyhow::bail!("repository layout is not available for this storage backend")
    }
    /// Acquire this backend's repository-wide write lock, held until the returned
    /// guard drops.
    ///
    /// [`RepositoryStateStore`] sessions retain this as their outer
    /// serialization guard across recovery, capture, the pre-commit check, and
    /// the application of the redo record. The lock is
    /// [reentrant](repo_lock::RepoWriteLock#reentrancy) so a retained startup
    /// session can reenter the same backend boundary.
    ///
    /// File storage acquires a repository-sibling bootstrap lock followed by
    /// `.jit/.repo-write.lock`; neither lives in the git control plane, so the
    /// chain guards the store with or without git (`@/charter/D-4`).
    ///
    /// # Errors
    ///
    /// Returns an error when the lock cannot be acquired within the backend's
    /// timeout.
    fn acquire_repo_write_lock(&self) -> Result<RepoWriteGuard>;

    /// Run an external process outside any startup mutation session retained by
    /// this backend, then restore recovery serialization before returning.
    ///
    /// File-backed CLI storage overrides this to release the bootstrap and
    /// repository locks while a checker subprocess runs. The locks are
    /// reacquired and a record the subprocess left is applied before the caller
    /// can publish its result through its session. Backends without a retained
    /// startup session execute `operation` directly.
    ///
    /// # Errors
    ///
    /// Returns an error from `operation`, or from restoring the recovery
    /// boundary after the external process exits.
    fn run_external_process<T>(&self, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        operation()
    }

    /// Load an issue by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist or cannot be deserialized.
    fn load_issue(&self, id: &IssueId) -> Result<Issue>;

    /// Load an issue by ID, returning a typed [`PathReadError`].
    ///
    /// This is the typed-error variant of [`IssueStore::load_issue`] used by
    /// document command operations that surface HTTP status codes.  Backends
    /// **must** override this default to return [`PathReadError::NotFound`] for
    /// missing issues and [`PathReadError::Other`] for genuine I/O failures.
    ///
    /// The default implementation maps every error to [`PathReadError::Other`].
    /// This is deliberately conservative: an unknown backend that forgets to
    /// override will return HTTP 500 for all errors rather than risk silently
    /// hiding errors behind a wrong 404.  Any backend that can structurally
    /// distinguish "issue not found" from "I/O failure" **should** override.
    ///
    /// # Errors
    ///
    /// - [`PathReadError::NotFound`] when the issue does not exist (backend must
    ///   override to produce this).
    /// - [`PathReadError::Other`] for storage or deserialization failures (and
    ///   for any error from a backend that uses the default implementation).
    fn load_issue_or_not_found(&self, id: &IssueId) -> Result<Issue, PathReadError> {
        // Conservative fallback: map every anyhow error to Other.
        // Backends that can distinguish NotFound structurally must override.
        self.load_issue(id).map_err(PathReadError::Other)
    }

    /// Resolve an issue id or unique id prefix to the stored id.
    ///
    /// # Errors
    ///
    /// [`IssueNotFoundError`] for no match, or [`AmbiguousIdError`] for
    /// several.
    fn resolve_issue_id(&self, prefix: &IssueIdPrefix) -> Result<IssueId>;

    /// List all issues in the repository.
    ///
    /// # Errors
    ///
    /// Returns an error if issues cannot be loaded.
    fn list_issues(&self) -> Result<Vec<Issue>>;

    /// Load the gate registry.
    ///
    /// # Errors
    ///
    /// Returns an error if the registry cannot be loaded.
    fn load_gate_registry(&self) -> Result<GateRegistry>;

    /// Read all events. Malformed JSON, a malformed record, and a tag outside the
    /// current event vocabulary are errors.
    ///
    /// # Errors
    ///
    /// Returns an error if events cannot be read.
    fn read_events(&self) -> Result<Vec<Event>>;

    /// Read the evidence record addressed by one archive target.
    ///
    /// Returns `None` when the target has no prior executions.
    ///
    /// # Errors
    ///
    /// Returns an error if the record cannot be read or decoded.
    fn read_archive_evidence_record(
        &self,
        target: &PlanTarget,
    ) -> Result<Option<crate::domain::artifact_execution::ArchiveEvidenceRecord>>;

    /// Load a gate run result by issue and run ID.
    ///
    /// # Errors
    ///
    /// Returns not found if the result is absent or belongs to another issue,
    /// or an error if it cannot be read or deserialized.
    fn load_gate_run_result(
        &self,
        issue_id: &IssueId,
        run_id: &crate::domain::identity::GateRunId,
    ) -> Result<crate::domain::GateRunResult>;

    /// List all gate run results for a specific issue.
    ///
    /// # Errors
    ///
    /// Returns an error if results cannot be loaded.
    fn list_gate_runs_for_issue(
        &self,
        issue_id: &IssueId,
    ) -> Result<Vec<crate::domain::GateRunResult>>;

    /// Get the root directory path for this storage backend.
    ///
    /// Returns the path where configuration files are stored.
    /// For file-based storage, this is the .jit directory.
    /// For in-memory storage, this returns a temporary path.
    fn root(&self) -> &std::path::Path;

    /// Whether `root()` names a filesystem-backed JIT data directory.
    ///
    /// Consumers use this capability instead of inferring the backend from
    /// files that may legitimately be absent in a structurally invalid partial
    /// repository. Non-filesystem and test-double backends default to `false`.
    fn is_file_backed(&self) -> bool {
        false
    }

    /// Read a repository-local text file by its path relative to the repository
    /// root (the parent of the `.jit` directory).
    ///
    /// This is the storage-owned read entry point for command/domain code that needs to
    /// read a config-declared file (e.g. a project-scope item source) WITHOUT
    /// reaching into the filesystem directly. Writes use [`RepositoryStateStore`]
    /// instead. The path is enforced repository-local:
    /// an absolute path or any `..` traversal is rejected with the typed
    /// [`PathReadError::InvalidPath`] before any I/O.
    ///
    /// Returns:
    /// - `Ok(None)` when the file is **absent** (a graceful "no content" so callers
    ///   like the project-scope indexer contribute no items rather than erroring),
    /// - `Ok(Some(content))` when the file is present and readable,
    /// - `Err` when the path is invalid, escapes the repo root, or the file is
    ///   present but unreadable.
    ///
    /// # Errors
    ///
    /// - [`PathReadError::InvalidPath`] for an empty, absolute, or `..`-bearing
    ///   path.
    /// - [`PathReadError::OutsideRepoRoot`] when the resolved path escapes the repo
    ///   root (e.g. via a symlink).
    /// - [`PathReadError::Other`] for any other read failure.
    fn read_repo_file(&self, rel_path: &str) -> Result<Option<String>, PathReadError>;

    /// List the gate presets this repository declares.
    ///
    /// # Errors
    ///
    /// Returns an error if presets cannot be loaded.
    fn list_gate_presets(&self) -> Result<Vec<crate::gate_presets::PresetInfo>>;

    /// Get a specific gate preset by name.
    ///
    /// # Errors
    ///
    /// Returns an error if the preset is not found or cannot be loaded.
    fn get_gate_preset(&self, name: &str) -> Result<crate::gate_presets::GatePresetDefinition>;

    /// Read file bytes from the repository, optionally at a specific git commit.
    ///
    /// When `at_commit` is `None`, reads from the working tree.  Returns
    /// `(bytes, commit_label)` where `commit_label` is the short git hash when
    /// reading from git, or `"working-tree"` when reading from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the file does not exist or cannot be read.
    fn read_path_bytes(
        &self,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<(Vec<u8>, String), PathReadError>;

    /// Read file content as UTF-8 text, optionally at a specific git commit.
    ///
    /// Delegates to [`IssueStore::read_path_bytes`] and decodes the result as
    /// UTF-8.  Bytes that are not valid UTF-8 are replaced with the Unicode
    /// replacement character (U+FFFD) so that text-oriented callers receive a
    /// `String` without an extra error path for encoding failures.
    ///
    /// Returns `(text, commit_label)` with the same commit-label semantics as
    /// `read_path_bytes`.
    ///
    /// # Errors
    ///
    /// Propagates `PathReadError::NotFound`, `PathReadError::CommitNotFound`,
    /// and `PathReadError::Other` from the underlying `read_path_bytes` call.
    fn read_path_text(
        &self,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<(String, String), PathReadError> {
        let (bytes, label) = self.read_path_bytes(path, at_commit)?;
        Ok((String::from_utf8_lossy(&bytes).into_owned(), label))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A prefix resolves to the one stored dependency it names, whether or not
    /// that dependency's issue still exists.
    #[test]
    fn test_resolve_dependency_id_matches_the_one_stored_dependency_named() {
        let dependencies = vec![
            crate::test_utils::issue_id(0x1_abcd_1234),
            crate::test_utils::issue_id(0x2_ef01_2345),
        ];
        let prefix = "ABCD1234".parse::<IssueIdPrefix>().unwrap();

        assert_eq!(
            resolve_dependency_id(&prefix, &dependencies).unwrap(),
            dependencies[0]
        );
    }

    #[test]
    fn test_resolve_dependency_id_reports_not_found_and_ambiguous_dependency() {
        let dependencies = vec![
            crate::test_utils::issue_id(0x1_abcd_1234),
            crate::test_utils::issue_id(0x2_abcd_1234),
        ];
        let resolve = |prefix: &str| {
            resolve_dependency_id(&prefix.parse::<IssueIdPrefix>().unwrap(), &dependencies)
                .unwrap_err()
        };

        assert!(resolve("ffff").is::<IssueNotFoundError>());
        assert!(matches!(
            resolve("abcd1234").downcast_ref::<AmbiguousIdError>(),
            Some(AmbiguousIdError::Dependency { matches, .. }) if matches.len() == 2
        ));
    }
}
