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

use crate::domain::identity::GateKey;
use crate::domain::identity::{GateRunId, IssueId, IssueIdPrefix};
use thiserror::Error;

/// Error raised when an issue cannot be found by id or id prefix.
///
/// Returned by the storage layer's lookup paths (`load_issue`, `resolve_issue_id`,
/// `delete_issue`). The CLI downcasts to this type to classify the failure as a
/// not-found condition (exit code `3`) and to render a structured JSON error,
/// rather than scanning the message text.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum IssueNotFoundError {
    /// No issue matched the given id.
    #[error("Issue not found: {id}")]
    ById {
        /// The id that did not resolve.
        id: IssueId,
    },
    /// No issue matched the requested prefix.
    #[error("Issue not found: {prefix}")]
    ByPrefix {
        /// The unresolved input.
        prefix: IssueIdPrefix,
    },
    /// The issue was absent from this checkout's store and the primary checkout's
    /// (the [`JsonFileStorage`](crate::storage::JsonFileStorage) fallback chain).
    #[error("Issue {id} not found in this checkout's store or the primary checkout")]
    AcrossSources {
        /// The fully-qualified id that was searched for.
        id: IssueId,
    },
}

impl IssueNotFoundError {
    /// Build an [`IssueNotFoundError::ById`] for a stored id.
    pub fn new(id: IssueId) -> Self {
        Self::ById { id }
    }

    /// Build an error for unresolved input.
    pub fn by_prefix(prefix: IssueIdPrefix) -> Self {
        Self::ByPrefix { prefix }
    }

    /// Build an [`IssueNotFoundError::AcrossSources`] for an id missing from
    /// every read source.
    pub fn across_sources(id: &IssueId) -> Self {
        Self::AcrossSources { id: id.clone() }
    }

    /// The id (or id prefix) that did not resolve.
    pub fn id(&self) -> &str {
        match self {
            Self::ById { id } | Self::AcrossSources { id } => id.as_str(),
            Self::ByPrefix { prefix } => prefix.as_str(),
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
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum GateNotFoundError {
    /// One or more gate keys were missing (the batch `gate add` check).
    #[error("Gates not found in registry: {}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "))]
    Batch(Vec<GateKey>),
    /// A single gate-key lookup missed (gate-check / template expansion).
    #[error("Gate '{0}' not found in registry")]
    Single(GateKey),
    /// A single gate-key lookup missed, phrased without the "in registry" suffix
    /// (`gate show` / `gate remove`).
    #[error("Gate '{0}' not found")]
    ByKey(GateKey),
    /// A single gate-key lookup missed during preset creation, phrased
    /// "Gate not found in registry: <key>" (`gate preset create`).
    #[error("Gate not found in registry: {0}")]
    InRegistry(GateKey),
}

impl GateNotFoundError {
    /// Build a [`GateNotFoundError::Batch`] from the missing gate keys.
    pub fn new(keys: impl IntoIterator<Item = GateKey>) -> Self {
        Self::Batch(keys.into_iter().collect())
    }

    /// Build a [`GateNotFoundError::Single`] for one missing gate key
    /// ("Gate '<key>' not found in registry").
    pub fn single(key: &GateKey) -> Self {
        Self::Single(key.clone())
    }

    /// Build a [`GateNotFoundError::ByKey`] for one missing gate key, using the
    /// `gate show`/`gate remove` phrasing ("Gate '<key>' not found").
    pub fn by_key(key: &GateKey) -> Self {
        Self::ByKey(key.clone())
    }

    /// Build a [`GateNotFoundError::InRegistry`] for one missing gate key, using
    /// the `gate preset create` phrasing ("Gate not found in registry: <key>").
    pub fn in_registry(key: &GateKey) -> Self {
        Self::InRegistry(key.clone())
    }
}

/// Error raised when a gate-run record cannot be found by its run id.
///
/// The CLI downcasts to this type to classify the failure as a not-found
/// condition (exit code `3`). `Display` reproduces the previous `anyhow!`
/// phrasing verbatim.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error("Gate run '{run_id}' not found")]
pub struct GateRunNotFoundError {
    run_id: GateRunId,
}

impl GateRunNotFoundError {
    /// Build a [`GateRunNotFoundError`] for the given run id.
    pub fn new(run_id: GateRunId) -> Self {
        Self { run_id }
    }
}

/// Error raised when a gate preset cannot be found by name.
///
/// The CLI downcasts to this type to classify the failure as a not-found
/// condition (exit code `3`). `Display` reproduces the previous `anyhow!`
/// phrasing verbatim.
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
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error("Gate '{key}' already exists")]
pub struct GateAlreadyExistsError {
    key: GateKey,
}

impl GateAlreadyExistsError {
    /// Build a [`GateAlreadyExistsError`] for the given gate key.
    pub fn new(key: &GateKey) -> Self {
        Self { key: key.clone() }
    }

    /// The gate key that already exists in the registry.
    pub fn key(&self) -> &GateKey {
        &self.key
    }
}

/// Error raised when a repository's on-disk format version is not the one this
/// binary accepts.
///
/// Reported as an external-dependency condition (exit code `10`): the binary and
/// the repository disagree, and neither is corrupt. `Display` is a single line
/// naming both versions.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error(
    "Repository format version {repository_version} is not the version this binary \
     supports ({supported_version}); install the jit matching this repository's format."
)]
pub struct RepositoryFormatUnsupportedError {
    repository_version: u32,
    supported_version: u32,
}

impl RepositoryFormatUnsupportedError {
    /// Build a [`RepositoryFormatUnsupportedError`] from the repository's on-disk
    /// format version and the version this binary supports.
    pub fn new(repository_version: u32, supported_version: u32) -> Self {
        Self {
            repository_version,
            supported_version,
        }
    }

    /// The repository's on-disk format version .
    pub fn repository_version(&self) -> u32 {
        self.repository_version
    }

    /// The one format version this binary accepts.
    pub fn supported_version(&self) -> u32 {
        self.supported_version
    }
}

/// Error raised when an id prefix matches more than one candidate.
///
/// Two origins share this type: resolving an issue-id prefix against the repo's
/// issues ([`Issue`](Self::Issue)) and removing a dependency edge by prefix
/// against an issue's own stored dependencies ([`Dependency`](Self::Dependency)).
/// The CLI downcasts to this type to classify the failure as an argument error
/// (exit code `2`) and to render an `AMBIGUOUS_ID` JSON error. Each variant's
/// `Display` reproduces the exact phrasing of the call site it replaced.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum AmbiguousIdError {
    /// An issue-id prefix matched multiple issues in the repository.
    #[error("Ambiguous ID '{prefix}' matches multiple issues:\n  {}", .matches.join("\n  "))]
    Issue {
        /// The prefix that resolved ambiguously.
        prefix: String,
        /// Human-readable descriptions of the matched issues ("short_id | title").
        matches: Vec<String>,
    },
    /// A dependency-edge prefix matched multiple of an issue's stored dependencies.
    #[error(
        "Ambiguous dependency id '{prefix}': matches {} stored dependencies ({}). \
         Use a longer id.",
        .matches.len(),
        .matches.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Dependency {
        /// The prefix that resolved ambiguously.
        prefix: String,
        /// The matched stored dependency ids.
        matches: Vec<IssueId>,
    },
}

impl AmbiguousIdError {
    /// Build an [`AmbiguousIdError::Issue`] for an ambiguous issue-id prefix.
    pub fn issue(prefix: impl Into<String>, matches: impl IntoIterator<Item = String>) -> Self {
        Self::Issue {
            prefix: prefix.into(),
            matches: matches.into_iter().collect(),
        }
    }

    /// Build an [`AmbiguousIdError::Dependency`] for an ambiguous dependency prefix.
    pub fn dependency(
        prefix: impl Into<String>,
        matches: impl IntoIterator<Item = IssueId>,
    ) -> Self {
        Self::Dependency {
            prefix: prefix.into(),
            matches: matches.into_iter().collect(),
        }
    }

    /// The prefix that resolved ambiguously.
    pub fn prefix(&self) -> &str {
        match self {
            Self::Issue { prefix, .. } | Self::Dependency { prefix, .. } => prefix,
        }
    }

    /// The matched candidate descriptions.
    pub fn matches(&self) -> Vec<&str> {
        match self {
            Self::Issue { matches, .. } => matches.iter().map(String::as_str).collect(),
            Self::Dependency { matches, .. } => matches.iter().map(|id| id.as_str()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_issue_not_found_by_id_message_and_accessor() {
        let id = crate::test_utils::issue_id(1);
        let err = IssueNotFoundError::new(id.clone());
        assert_eq!(err.to_string(), format!("Issue not found: {id}"));
        assert_eq!(err.id(), id.as_str());
    }

    #[test]
    fn test_issue_not_found_by_prefix_preserves_unresolved_input() {
        let prefix = "ABCD1234".parse::<IssueIdPrefix>().unwrap();
        let err = IssueNotFoundError::by_prefix(prefix.clone());
        assert_eq!(err.to_string(), format!("Issue not found: {prefix}"));
        assert_eq!(err.id(), prefix.as_str());
    }

    #[test]
    fn test_issue_not_found_across_sources_message_and_accessor() {
        let id = crate::test_utils::issue_id(1);
        let err = IssueNotFoundError::across_sources(&id);
        assert_eq!(
            err.to_string(),
            format!("Issue {id} not found in this checkout's store or the primary checkout")
        );
        assert_eq!(err.id(), id.as_str());
    }

    #[test]
    fn test_issue_not_found_downcasts_from_anyhow() {
        let id = crate::test_utils::issue_id(1);
        let err: anyhow::Error = IssueNotFoundError::new(id.clone()).into();
        let typed = err.downcast_ref::<IssueNotFoundError>();
        assert!(typed.is_some());
        assert_eq!(typed.unwrap().id(), id.as_str());
    }

    #[test]
    fn test_gate_not_found_batch_and_single_messages() {
        let batch = GateNotFoundError::new(["lint", "tests"].map(|key| key.parse().unwrap()));
        assert_eq!(
            batch.to_string(),
            "Gates not found in registry: lint, tests"
        );
        let single = GateNotFoundError::single(&"lint".parse().unwrap());
        assert_eq!(single.to_string(), "Gate 'lint' not found in registry");
    }

    #[test]
    fn test_gate_not_found_by_key_and_in_registry_messages() {
        // Per-origin Display lock: each variant reproduces the exact original text
        // of the call site it replaced (gate show/remove vs gate preset create),
        // so a gate-not-found is the dedicated type without changing user output.
        let by_key = GateNotFoundError::by_key(&"lint".parse().unwrap());
        assert_eq!(by_key.to_string(), "Gate 'lint' not found");

        let in_registry = GateNotFoundError::in_registry(&"lint".parse().unwrap());
        assert_eq!(in_registry.to_string(), "Gate not found in registry: lint");

        // Both still downcast to the dedicated type for exit-code classification.
        let any: anyhow::Error = by_key.into();
        assert!(any.downcast_ref::<GateNotFoundError>().is_some());
        let any: anyhow::Error = in_registry.into();
        assert!(any.downcast_ref::<GateNotFoundError>().is_some());
    }

    #[test]
    fn test_gate_not_found_downcasts_from_anyhow() {
        let err: anyhow::Error = GateNotFoundError::single(&"lint".parse().unwrap()).into();
        assert!(err.downcast_ref::<GateNotFoundError>().is_some());
    }

    #[test]
    fn test_gate_run_preset_repository_not_found_messages() {
        assert_eq!(
            GateRunNotFoundError::new(crate::test_utils::gate_run_id(1)).to_string(),
            format!("Gate run '{}' not found", crate::test_utils::gate_run_id(1))
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
        let run: anyhow::Error =
            GateRunNotFoundError::new(crate::test_utils::gate_run_id(1)).into();
        assert!(run.downcast_ref::<GateRunNotFoundError>().is_some());
        let preset: anyhow::Error = PresetNotFoundError::new("p").into();
        assert!(preset.downcast_ref::<PresetNotFoundError>().is_some());
        let repo: anyhow::Error = RepositoryNotFoundError::new("d").into();
        assert!(repo.downcast_ref::<RepositoryNotFoundError>().is_some());
    }

    #[test]
    fn test_gate_already_exists_message_and_accessor() {
        let err = GateAlreadyExistsError::new(&"lint".parse().unwrap());
        assert_eq!(err.to_string(), "Gate 'lint' already exists");
        assert_eq!(err.key(), &"lint".parse().unwrap());
    }

    #[test]
    fn test_gate_already_exists_downcasts_from_anyhow() {
        let err: anyhow::Error = GateAlreadyExistsError::new(&"lint".parse().unwrap()).into();
        assert!(err.downcast_ref::<GateAlreadyExistsError>().is_some());
    }

    #[test]
    fn test_repository_format_unsupported_message_and_accessors() {
        let err = RepositoryFormatUnsupportedError::new(3, 2);
        assert_eq!(err.repository_version(), 3);
        assert_eq!(err.supported_version(), 2);
        let msg = err.to_string();
        assert!(!msg.contains('\n'), "message must be single-line: {msg}");
        assert!(
            msg.contains('3') && msg.contains('2'),
            "must name both versions: {msg}"
        );
    }

    #[test]
    fn test_repository_format_unsupported_downcasts_from_anyhow() {
        let err: anyhow::Error = RepositoryFormatUnsupportedError::new(3, 2).into();
        let typed = err.downcast_ref::<RepositoryFormatUnsupportedError>();
        assert!(typed.is_some());
        assert_eq!(typed.unwrap().repository_version(), 3);
    }

    #[test]
    fn test_ambiguous_id_issue_variant_message_and_accessors() {
        let err = AmbiguousIdError::issue(
            "aaaa",
            ["aaaa1111 | One".to_string(), "aaaa2222 | Two".to_string()],
        );
        assert_eq!(
            err.to_string(),
            "Ambiguous ID 'aaaa' matches multiple issues:\n  aaaa1111 | One\n  aaaa2222 | Two"
        );
        assert_eq!(err.prefix(), "aaaa");
        assert_eq!(err.matches().len(), 2);
        let any: anyhow::Error = err.into();
        assert!(any.downcast_ref::<AmbiguousIdError>().is_some());
    }

    #[test]
    fn test_ambiguous_id_dependency_variant_message() {
        let [first, second] = [1, 2].map(crate::test_utils::issue_id);
        let err = AmbiguousIdError::dependency("bbbb", [first.clone(), second.clone()]);
        assert_eq!(
            err.to_string(),
            format!("Ambiguous dependency id 'bbbb': matches 2 stored dependencies ({first}, {second}). \
             Use a longer id.")
        );
    }
}
