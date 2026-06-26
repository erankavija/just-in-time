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
/// Returned when `jit gate add` (and other registry-membership checks) targets a
/// gate key that has not been defined. The CLI downcasts to this type to classify
/// the failure as a not-found condition and to render a `GATE_NOT_FOUND` JSON
/// error, rather than scanning the message text.
///
/// # Examples
///
/// ```
/// use jit::storage::GateNotFoundError;
///
/// let err = GateNotFoundError::new(["lint", "tests"]);
/// assert_eq!(err.to_string(), "Gates not found in registry: lint, tests");
/// assert_eq!(err.keys().len(), 2);
/// ```
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[error("Gates not found in registry: {}", .keys.join(", "))]
pub struct GateNotFoundError {
    keys: Vec<String>,
}

impl GateNotFoundError {
    /// Build a [`GateNotFoundError`] from the missing gate keys.
    pub fn new(keys: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            keys: keys.into_iter().map(Into::into).collect(),
        }
    }

    /// The gate keys that were not found in the registry.
    pub fn keys(&self) -> &[String] {
        &self.keys
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
    fn test_gate_not_found_message_and_keys() {
        let err = GateNotFoundError::new(["lint", "tests"]);
        assert_eq!(err.to_string(), "Gates not found in registry: lint, tests");
        assert_eq!(err.keys(), &["lint".to_string(), "tests".to_string()]);
    }

    #[test]
    fn test_gate_not_found_downcasts_from_anyhow() {
        let err: anyhow::Error = GateNotFoundError::new(["lint"]).into();
        assert!(err.downcast_ref::<GateNotFoundError>().is_some());
    }
}
