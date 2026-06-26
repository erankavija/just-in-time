//! Typed lookup errors for storage-backed resources.
//!
//! These errors let the CLI classify "resource does not exist" failures by
//! downcasting an [`anyhow::Error`] to a concrete type instead of scanning the
//! human-readable message. Both storage backends ([`InMemoryStorage`] and
//! [`JsonFileStorage`]) return the SAME typed error at their not-found origins,
//! so the CLI reaches a uniform, downcastable error regardless of backend.
//!
//! Each `Display` impl reproduces the originating phrasing verbatim, so wrapping
//! a previously-stringly error in a typed variant does not change user-facing
//! output.
//!
//! [`InMemoryStorage`]: crate::storage::InMemoryStorage
//! [`JsonFileStorage`]: crate::storage::JsonFileStorage

use thiserror::Error;

/// Error raised when an issue cannot be found by id or id prefix.
///
/// Returned by the storage layer's lookup paths (`load_issue`, `resolve_issue_id`,
/// `delete_issue`). The CLI downcasts to this type to classify the failure as a
/// not-found condition (exit code `3`) and to render a structured JSON error,
/// rather than scanning the message text.
///
/// # Examples
///
/// ```
/// use jit::storage::IssueNotFoundError;
///
/// // Looked up by id or prefix.
/// let by_id = IssueNotFoundError::new("a1b2c3d4");
/// assert_eq!(by_id.to_string(), "Issue not found: a1b2c3d4");
/// assert_eq!(by_id.id(), "a1b2c3d4");
///
/// // Absent from every read source (local `.jit`, git HEAD, main worktree).
/// let across = IssueNotFoundError::across_sources("a1b2c3d4");
/// assert_eq!(
///     across.to_string(),
///     "Issue a1b2c3d4 not found in local storage, git, or main worktree"
/// );
/// ```
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum IssueNotFoundError {
    /// No issue matched the given id or id prefix.
    #[error("Issue not found: {id}")]
    ById {
        /// The id or id-prefix that did not resolve.
        id: String,
    },
    /// The issue was absent from every read source: local `.jit/`, git `HEAD`,
    /// and the main worktree (the [`JsonFileStorage`](crate::storage::JsonFileStorage)
    /// fallback chain).
    #[error("Issue {id} not found in local storage, git, or main worktree")]
    AcrossSources {
        /// The fully-qualified id that was searched for.
        id: String,
    },
}

impl IssueNotFoundError {
    /// Build an [`IssueNotFoundError::ById`] for the given id or id prefix.
    pub fn new(id: impl Into<String>) -> Self {
        Self::ById { id: id.into() }
    }

    /// Build an [`IssueNotFoundError::AcrossSources`] for an id missing from
    /// every read source.
    pub fn across_sources(id: impl Into<String>) -> Self {
        Self::AcrossSources { id: id.into() }
    }

    /// The id (or id prefix) that did not resolve.
    pub fn id(&self) -> &str {
        match self {
            Self::ById { id } | Self::AcrossSources { id } => id,
        }
    }
}

/// Error raised when one or more gate keys are absent from the gate registry.
///
/// Returned by the gate-registry membership checks (`jit gate add`, gate-check,
/// template expansion, `gate show`/`remove`, `gate preset create`). The CLI
/// downcasts to this type to classify the failure as a not-found condition (exit
/// code `3`) and to render a `GATE_NOT_FOUND` JSON error, rather than scanning the
/// message text. Each variant preserves one of the distinct phrasings the callers
/// already produced, so a gate-not-found is always this dedicated type (never a
/// generic `NotFoundError`) while keeping the user-facing text byte-for-byte.
///
/// # Examples
///
/// ```
/// use jit::storage::GateNotFoundError;
///
/// let batch = GateNotFoundError::new(["lint", "tests"]);
/// assert_eq!(batch.to_string(), "Gates not found in registry: lint, tests");
///
/// let single = GateNotFoundError::single("lint");
/// assert_eq!(single.to_string(), "Gate 'lint' not found in registry");
///
/// let by_key = GateNotFoundError::by_key("lint");
/// assert_eq!(by_key.to_string(), "Gate 'lint' not found");
///
/// let in_registry = GateNotFoundError::in_registry("lint");
/// assert_eq!(in_registry.to_string(), "Gate not found in registry: lint");
/// ```
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum GateNotFoundError {
    /// One or more gate keys were missing (the batch `gate add` check).
    #[error("Gates not found in registry: {}", .0.join(", "))]
    Batch(Vec<String>),
    /// A single gate-key lookup missed (gate-check / template expansion).
    #[error("Gate '{0}' not found in registry")]
    Single(String),
    /// A single gate-key lookup missed, phrased without the "in registry" suffix
    /// (`gate show` / `gate remove`).
    #[error("Gate '{0}' not found")]
    ByKey(String),
    /// A single gate-key lookup missed during preset creation, phrased
    /// "Gate not found in registry: <key>" (`gate preset create`).
    #[error("Gate not found in registry: {0}")]
    InRegistry(String),
}

impl GateNotFoundError {
    /// Build a [`GateNotFoundError::Batch`] from the missing gate keys.
    pub fn new(keys: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Batch(keys.into_iter().map(Into::into).collect())
    }

    /// Build a [`GateNotFoundError::Single`] for one missing gate key
    /// ("Gate '<key>' not found in registry").
    pub fn single(key: impl Into<String>) -> Self {
        Self::Single(key.into())
    }

    /// Build a [`GateNotFoundError::ByKey`] for one missing gate key, using the
    /// `gate show`/`gate remove` phrasing ("Gate '<key>' not found").
    pub fn by_key(key: impl Into<String>) -> Self {
        Self::ByKey(key.into())
    }

    /// Build a [`GateNotFoundError::InRegistry`] for one missing gate key, using
    /// the `gate preset create` phrasing ("Gate not found in registry: <key>").
    pub fn in_registry(key: impl Into<String>) -> Self {
        Self::InRegistry(key.into())
    }
}

/// Error raised when a gate-run record cannot be found by its run id.
///
/// The CLI downcasts to this type to classify the failure as a not-found
/// condition (exit code `3`). `Display` reproduces the previous `anyhow!`
/// phrasing verbatim.
///
/// # Examples
///
/// ```
/// use jit::storage::GateRunNotFoundError;
///
/// let err = GateRunNotFoundError::new("run-123");
/// assert_eq!(err.to_string(), "Gate run 'run-123' not found");
/// ```
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error("Gate run '{run_id}' not found")]
pub struct GateRunNotFoundError {
    run_id: String,
}

impl GateRunNotFoundError {
    /// Build a [`GateRunNotFoundError`] for the given run id.
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
        }
    }
}

/// Error raised when a gate preset cannot be found by name.
///
/// The CLI downcasts to this type to classify the failure as a not-found
/// condition (exit code `3`). `Display` reproduces the previous `anyhow!`
/// phrasing verbatim.
///
/// # Examples
///
/// ```
/// use jit::storage::PresetNotFoundError;
///
/// let err = PresetNotFoundError::new("rust-ci");
/// assert_eq!(err.to_string(), "Preset not found: rust-ci");
/// ```
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error("Preset not found: {name}")]
pub struct PresetNotFoundError {
    name: String,
}

impl PresetNotFoundError {
    /// Build a [`PresetNotFoundError`] for the given preset name.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// Error raised when no `.jit` repository exists at the resolved data directory.
///
/// The CLI downcasts to this type to classify the failure as a not-found
/// condition (exit code `3`). `Display` reproduces the previous `anyhow!`
/// phrasing verbatim, including the multi-line initialization guidance.
///
/// # Examples
///
/// ```
/// use jit::storage::RepositoryNotFoundError;
///
/// let err = RepositoryNotFoundError::new("/tmp/x/.jit");
/// assert!(err.to_string().starts_with("JIT repository not found at '/tmp/x/.jit'"));
/// assert!(err.to_string().contains("jit init"));
/// ```
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error(
    "JIT repository not found at '{path}'\n\n\
     Initialize a repository with: jit init\n\
     Or set JIT_DATA_DIR environment variable to point to an existing repository."
)]
pub struct RepositoryNotFoundError {
    path: String,
}

impl RepositoryNotFoundError {
    /// Build a [`RepositoryNotFoundError`] for the given (missing) data-dir path.
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

/// Error raised when defining a gate whose key already exists in the registry.
///
/// Returned by the gate-define path when the requested key is already present.
/// The CLI downcasts to this type to classify the failure as an already-exists
/// condition (exit code `6`) instead of scanning the message text. `Display`
/// reproduces the previous `anyhow!` phrasing verbatim.
///
/// # Examples
///
/// ```
/// use jit::storage::GateAlreadyExistsError;
///
/// let err = GateAlreadyExistsError::new("lint");
/// assert_eq!(err.to_string(), "Gate 'lint' already exists");
/// assert_eq!(err.key(), "lint");
/// ```
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error("Gate '{key}' already exists")]
pub struct GateAlreadyExistsError {
    key: String,
}

impl GateAlreadyExistsError {
    /// Build a [`GateAlreadyExistsError`] for the given gate key.
    pub fn new(key: impl Into<String>) -> Self {
        Self { key: key.into() }
    }

    /// The gate key that already exists in the registry.
    pub fn key(&self) -> &str {
        &self.key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_issue_not_found_by_id_message_and_accessor() {
        let err = IssueNotFoundError::new("abcd1234");
        assert_eq!(err.to_string(), "Issue not found: abcd1234");
        assert_eq!(err.id(), "abcd1234");
    }

    #[test]
    fn test_issue_not_found_across_sources_message_and_accessor() {
        let err = IssueNotFoundError::across_sources("abcd1234");
        assert_eq!(
            err.to_string(),
            "Issue abcd1234 not found in local storage, git, or main worktree"
        );
        assert_eq!(err.id(), "abcd1234");
    }

    #[test]
    fn test_issue_not_found_downcasts_from_anyhow() {
        let err: anyhow::Error = IssueNotFoundError::new("abcd1234").into();
        let typed = err.downcast_ref::<IssueNotFoundError>();
        assert!(typed.is_some());
        assert_eq!(typed.unwrap().id(), "abcd1234");
    }

    #[test]
    fn test_gate_not_found_batch_and_single_messages() {
        let batch = GateNotFoundError::new(["lint", "tests"]);
        assert_eq!(
            batch.to_string(),
            "Gates not found in registry: lint, tests"
        );
        let single = GateNotFoundError::single("lint");
        assert_eq!(single.to_string(), "Gate 'lint' not found in registry");
    }

    #[test]
    fn test_gate_not_found_by_key_and_in_registry_messages() {
        // Per-origin Display lock: each variant reproduces the exact original text
        // of the call site it replaced (gate show/remove vs gate preset create),
        // so a gate-not-found is the dedicated type without changing user output.
        let by_key = GateNotFoundError::by_key("lint");
        assert_eq!(by_key.to_string(), "Gate 'lint' not found");

        let in_registry = GateNotFoundError::in_registry("lint");
        assert_eq!(in_registry.to_string(), "Gate not found in registry: lint");

        // Both still downcast to the dedicated type for exit-code classification.
        let any: anyhow::Error = by_key.into();
        assert!(any.downcast_ref::<GateNotFoundError>().is_some());
        let any: anyhow::Error = in_registry.into();
        assert!(any.downcast_ref::<GateNotFoundError>().is_some());
    }

    #[test]
    fn test_gate_not_found_downcasts_from_anyhow() {
        let err: anyhow::Error = GateNotFoundError::single("lint").into();
        assert!(err.downcast_ref::<GateNotFoundError>().is_some());
    }

    #[test]
    fn test_gate_run_preset_repository_not_found_messages() {
        assert_eq!(
            GateRunNotFoundError::new("run-123").to_string(),
            "Gate run 'run-123' not found"
        );
        assert_eq!(
            PresetNotFoundError::new("rust-ci").to_string(),
            "Preset not found: rust-ci"
        );
        let repo = RepositoryNotFoundError::new("/tmp/x/.jit");
        assert!(repo.to_string().starts_with("JIT repository not found at"));
        assert!(repo.to_string().contains("jit init"));
    }

    #[test]
    fn test_new_not_found_errors_downcast_from_anyhow() {
        let run: anyhow::Error = GateRunNotFoundError::new("r").into();
        assert!(run.downcast_ref::<GateRunNotFoundError>().is_some());
        let preset: anyhow::Error = PresetNotFoundError::new("p").into();
        assert!(preset.downcast_ref::<PresetNotFoundError>().is_some());
        let repo: anyhow::Error = RepositoryNotFoundError::new("d").into();
        assert!(repo.downcast_ref::<RepositoryNotFoundError>().is_some());
    }

    #[test]
    fn test_gate_already_exists_message_and_accessor() {
        let err = GateAlreadyExistsError::new("lint");
        assert_eq!(err.to_string(), "Gate 'lint' already exists");
        assert_eq!(err.key(), "lint");
    }

    #[test]
    fn test_gate_already_exists_downcasts_from_anyhow() {
        let err: anyhow::Error = GateAlreadyExistsError::new("lint").into();
        assert!(err.downcast_ref::<GateAlreadyExistsError>().is_some());
    }
}
