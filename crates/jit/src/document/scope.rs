//! Scope selector for `jit doc check-links`.
//!
//! Mirrors the [`SnapshotScope`](crate::snapshot::SnapshotScope) pattern: a small
//! typed enum with a [`FromStr`] parser and a [`Display`] that round-trips, so the
//! CLI parses the `--scope` argument once into a value instead of re-checking
//! `== "all"` / `strip_prefix("issue:")` inline.

use std::fmt;
use std::str::FromStr;

/// Error returned when a string cannot be parsed as a [`DocumentScope`].
///
/// # Examples
///
/// ```
/// use jit::document::{DocumentScope, DocumentScopeParseError};
/// use std::str::FromStr;
///
/// assert_eq!(
///     DocumentScope::from_str("label:epic"),
///     Err(DocumentScopeParseError::UnknownScope("label:epic".to_string()))
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DocumentScopeParseError {
    /// The input did not match `all` or `issue:<id>`.
    #[error("Invalid scope '{0}'. Use 'all' or 'issue:ID'")]
    UnknownScope(String),
}

/// Scope of a document link check.
///
/// `all` checks every issue's documents; `issue:<id>` checks just one issue's.
///
/// # Examples
///
/// ```
/// use jit::document::DocumentScope;
/// use std::str::FromStr;
///
/// assert_eq!(DocumentScope::from_str("all").unwrap(), DocumentScope::All);
/// assert_eq!(
///     DocumentScope::from_str("issue:abc123").unwrap(),
///     DocumentScope::Issue("abc123".to_string())
/// );
/// // Round-trips through Display.
/// assert_eq!(DocumentScope::All.to_string(), "all");
/// assert_eq!(
///     DocumentScope::Issue("abc123".to_string()).to_string(),
///     "issue:abc123"
/// );
/// // Unknown forms are rejected with a typed error.
/// assert!(DocumentScope::from_str("everything").is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentScope {
    /// Check the documents of every issue.
    All,
    /// Check the documents of a single issue, identified by id (possibly partial).
    Issue(String),
}

impl FromStr for DocumentScope {
    type Err = DocumentScopeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "all" {
            Ok(DocumentScope::All)
        } else if let Some(id) = s.strip_prefix("issue:") {
            Ok(DocumentScope::Issue(id.to_string()))
        } else {
            Err(DocumentScopeParseError::UnknownScope(s.to_string()))
        }
    }
}

impl fmt::Display for DocumentScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocumentScope::All => write!(f, "all"),
            DocumentScope::Issue(id) => write!(f, "issue:{}", id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_scope_parse_all() {
        assert_eq!(DocumentScope::from_str("all").unwrap(), DocumentScope::All);
        assert_eq!(DocumentScope::All.to_string(), "all");
    }

    #[test]
    fn test_document_scope_parse_issue() {
        let scope = DocumentScope::from_str("issue:abc123").unwrap();
        assert_eq!(scope, DocumentScope::Issue("abc123".to_string()));
        assert_eq!(scope.to_string(), "issue:abc123");
    }

    #[test]
    fn test_document_scope_parse_invalid() {
        let err = DocumentScope::from_str("label:epic").unwrap_err();
        assert_eq!(
            err,
            DocumentScopeParseError::UnknownScope("label:epic".to_string())
        );
        // The message preserves the original wording the CLI surfaced before.
        assert!(err.to_string().contains("Use 'all' or 'issue:ID'"));
    }

    #[test]
    fn test_document_scope_issue_roundtrips_empty_id() {
        // `issue:` with an empty id parses to an empty-id Issue and round-trips.
        let scope = DocumentScope::from_str("issue:").unwrap();
        assert_eq!(scope, DocumentScope::Issue(String::new()));
        assert_eq!(scope.to_string(), "issue:");
    }
}
