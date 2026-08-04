//! Storage read interfaces and repository transaction boundary.
//!
//! [`IssueStore`] exposes typed reads plus lock/session lifecycle controls shared
//! by file-backed and in-memory backends. Repository-owned publication goes only
//! through [`RepositoryStateStore`] capture/apply sessions.

use crate::declarations::GateRegistry;
use crate::domain::{Event, Issue};
use anyhow::Result;

pub mod artifact_discovery;
pub mod artifact_planning;
pub(crate) mod atomic_write;
pub mod claim_coordinator;
pub mod clock;
#[cfg(test)]
pub(crate) mod contention_probe;
pub mod control_plane;
pub mod discovery;
pub mod errors;
pub(crate) mod external_publish;
pub(crate) mod file_transaction;
pub mod gate_runs;
pub mod gate_store;
pub mod git_revision;
pub mod guard_order;
pub mod json;
pub mod lock;
pub mod lock_cleanup;
pub mod memory;
pub mod path_errors;
pub mod reference;
pub mod repo_lock;
pub mod repository_state_store;
pub mod ruleset_store;
pub mod temp_cleanup;
#[cfg(feature = "test-support")]
mod test_support;
mod transaction_journal;
mod transaction_recovery;
mod transaction_staging;
pub mod user_config_store;
pub mod warnings;
pub mod worktree_identity;
pub mod worktree_paths;

// Re-export for convenience
pub use artifact_discovery::{discover_archive_artifacts, ArtifactDiscoveryError};
pub(crate) use artifact_planning::citation_scan_evidence_from_files;
pub use artifact_planning::{collect_artifact_classification_facts, resolve_container_destination};
pub use claim_coordinator::{ClaimAcquireLimits, ClaimCoordinator, Lease};
pub use clock::{Clock, SystemClock};
pub use errors::{
    AmbiguousIdError, GateAlreadyExistsError, GateNotFoundError, GateRunNotFoundError,
    InvalidIdPrefixError, IssueNotFoundError, PresetNotFoundError, RepositoryFormatTooNewError,
    RepositoryNotFoundError, MIN_ID_PREFIX_LENGTH,
};
// Storage-only: the transaction kernel publishes into repository roots, so no
// command may name it. Commands publish through `RepositoryStateStore` sessions.
pub(in crate::storage) use file_transaction::{FileTransactionKernel, TransactionControlLocation};
pub use git_revision::{
    GitRevisionError, GitRevisionResolver, PinnedArtifactRead, PinnedTargetRead,
};
pub use json::{JsonFileStorage, RetainedMutationSessionGuard, RetainedSessionSuspendedError};
pub use lock::{is_lock_timeout, FileLocker, LockMode, LockTimeout};
pub use path_errors::{validate_repo_relative_path, PathReadError};
pub use reference::{render_reference_markdown, GateRunField};
pub use repo_lock::{RepoWriteGuard, RepoWriteLock};
pub use repository_state_store::{
    discover_repository_layout, RecoveryDispatchReport, RepositoryApplyOutcome,
    RepositoryMutationSession, RepositoryStateStore, RepositoryStateStoreError,
};
pub(crate) use transaction_recovery::FileTransactionError;
// Failure-injection seam: the definitions in `transaction_recovery` compile
// unconditionally (production code threads them through every
// `repository_check` site), but their exposure as crate public API is gated
// behind `test-support`. The `pub(crate)` twin keeps every internal caller's
// resolution (`crate::storage::TransactionFailurePoint`, etc.) identical in
// both feature states.
#[cfg(feature = "test-support")]
pub use transaction_recovery::{
    FailurePoint as TransactionFailurePoint, NoTransactionFailures, TransactionFailureInjector,
};
#[cfg(not(feature = "test-support"))]
pub(crate) use transaction_recovery::{
    FailurePoint as TransactionFailurePoint, NoTransactionFailures, TransactionFailureInjector,
};
pub use warnings::StorageWarning;

#[allow(unused_imports)] // Public API used only in tests, not in binary
pub use memory::InMemoryStorage;

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
    /// serialization guard across recovery, capture, read-set revalidation, and
    /// transactional apply. The lock is
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
    /// reacquired and pending journals are recovered before the caller can
    /// publish the subprocess result through its session. Backends without a retained startup
    /// session execute `operation` directly.
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
    fn load_issue(&self, id: &str) -> Result<Issue>;

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
    fn load_issue_or_not_found(&self, id: &str) -> Result<Issue, PathReadError> {
        // Conservative fallback: map every anyhow error to Other.
        // Backends that can distinguish NotFound structurally must override.
        self.load_issue(id).map_err(PathReadError::Other)
    }

    /// Resolve a partial issue ID to its full UUID.
    ///
    /// Accepts either a full UUID or a unique prefix (minimum 4 characters).
    /// Returns the full UUID if a unique match is found.
    /// # Errors
    ///
    /// - Prefix too short (< 4 chars): "Issue ID prefix must be at least 4 characters"
    /// - No matching issue found: "Issue not found: {prefix}"
    /// - Multiple issues match (ambiguous): "Ambiguous ID '{prefix}' matches multiple issues: ..."
    fn resolve_issue_id(&self, partial_id: &str) -> Result<String>;

    /// List all issues in the repository.
    ///
    /// Backends may attach index maintenance to this path, so a caller that
    /// must leave the repository untouched reads through
    /// [`IssueStore::read_issues`] instead.
    ///
    /// # Errors
    ///
    /// Returns an error if issues cannot be loaded.
    fn list_issues(&self) -> Result<Vec<Issue>>;

    /// Return the same issue set as [`IssueStore::list_issues`], carrying no
    /// index maintenance.
    ///
    /// This is the enumeration a strictly read-only command reads through: it
    /// publishes, rewrites, and unlinks no repository content, so an advisory
    /// report can be stated over the whole issue set without becoming a
    /// mutation. A backend's retained advisory locks are the one thing it may
    /// still open create-if-absent, as every read path does; they hold no
    /// repository content and are never unlinked.
    ///
    /// The default implementation delegates to [`IssueStore::list_issues`],
    /// which is correct for a backend that attaches no maintenance to its
    /// read-all path; a backend that does **must** override this with the
    /// enumeration alone.
    ///
    /// # Errors
    ///
    /// Returns an error if issues cannot be loaded.
    fn read_issues(&self) -> Result<Vec<Issue>> {
        self.list_issues()
    }

    /// Load the gate registry.
    ///
    /// # Errors
    ///
    /// Returns an error if the registry cannot be loaded.
    fn load_gate_registry(&self) -> Result<GateRegistry>;

    /// Read all events whose tag is in the current event vocabulary.
    ///
    /// File storage skips structurally valid records with retired or otherwise
    /// unknown string tags so append-only history remains readable after a tag
    /// is removed. Malformed JSON and malformed records for known tags remain
    /// errors.
    ///
    /// # Errors
    ///
    /// Returns an error if events cannot be read.
    fn read_events(&self) -> Result<Vec<Event>>;

    /// Read dependency-aware archive events for reconciliation.
    ///
    /// File storage overrides this path so an isolated malformed torn-tail line
    /// cannot prevent a later archive run from repairing and reconciling state.
    fn read_artifact_archive_events(&self) -> Result<Vec<Event>> {
        Ok(self
            .read_events()?
            .into_iter()
            .filter(|event| matches!(event, Event::ArtifactArchiveExecuted { .. }))
            .collect())
    }

    /// Load a gate run result by run ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the result does not exist or cannot be deserialized.
    fn load_gate_run_result(&self, run_id: &str) -> Result<crate::domain::GateRunResult>;

    /// List all gate run results for a specific issue.
    ///
    /// # Errors
    ///
    /// Returns an error if results cannot be loaded.
    fn list_gate_runs_for_issue(&self, issue_id: &str)
        -> Result<Vec<crate::domain::GateRunResult>>;

    /// The most recent run of `gate_key` whose verdict an evaluation over
    /// inputs digesting to `digest` may carry.
    ///
    /// The search spans every recorded run, not one issue's: a whole-tree
    /// checker's verdict over an input set belongs to that input set, so the
    /// issue it was first derived for does not narrow who may cite it.
    ///
    /// A run qualifies only when it recorded the same digest, executed its own
    /// checker ([`GateVerdictOrigin::Executed`](crate::domain::GateVerdictOrigin)),
    /// and reached a verdict — `passed` or `failed`. A run that errored reached
    /// none, so it is never a source.
    ///
    /// # Errors
    ///
    /// Returns an error if recorded runs cannot be read.
    fn find_reusable_gate_run(
        &self,
        gate_key: &str,
        digest: &crate::domain::InputsDigest,
    ) -> Result<Option<crate::domain::GateRunResult>>;

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
