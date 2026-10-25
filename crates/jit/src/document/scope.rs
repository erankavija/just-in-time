//! Scope selector for `jit doc check-links`.
//!
use std::fmt;

/// Error returned when a string cannot be parsed as a [`DocumentScope`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DocumentScopeParseError {
    /// The input did not match `all` or `issue:<id>`.
    #[error("Invalid scope '{0}'. Use 'all' or 'issue:ID'")]
    UnknownScope(String),
}

/// Scope of a document link check.
///
/// `all` checks every issue's documents; `issue:<id>` checks just one issue's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentScope {
    /// Check the documents of every issue.
    All,
    /// Check the documents of one resolved issue.
    Issue(crate::domain::identity::IssueId),
}

impl DocumentScope {
    /// Parse the selector, delegating its issue argument to the input boundary.
    pub fn parse(
        s: &str,
        resolve: impl FnOnce(&str) -> anyhow::Result<crate::domain::identity::IssueId>,
    ) -> anyhow::Result<Self> {
        if s == "all" {
            Ok(DocumentScope::All)
        } else if let Some(id) = s.strip_prefix("issue:") {
            Ok(DocumentScope::Issue(resolve(id)?))
        } else {
            Err(DocumentScopeParseError::UnknownScope(s.to_string()).into())
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
        assert_eq!(
            DocumentScope::parse("all", |id| Ok(id.to_string())).unwrap(),
            DocumentScope::All
        );
        assert_eq!(DocumentScope::All.to_string(), "all");
    }

    #[test]
    fn test_document_scope_parse_issue() {
        let scope = DocumentScope::parse("issue:abc123", |id| Ok(id.to_string())).unwrap();
        assert_eq!(scope, DocumentScope::Issue("abc123".to_string()));
        assert_eq!(scope.to_string(), "issue:abc123");
    }

    #[test]
    fn test_document_scope_parse_invalid() {
        let err = DocumentScope::parse("label:epic", |id| Ok(id.to_string())).unwrap_err();
        assert_eq!(
            err.downcast_ref::<DocumentScopeParseError>().unwrap(),
            &DocumentScopeParseError::UnknownScope("label:epic".to_string())
        );
        // The message preserves the original wording the CLI surfaced before.
        assert!(err.to_string().contains("Use 'all' or 'issue:ID'"));
    }

    #[test]
    fn test_document_scope_issue_roundtrips_empty_id() {
        // `issue:` with an empty id parses to an empty-id Issue and round-trips.
        let scope = DocumentScope::parse("issue:", |id| Ok(id.to_string())).unwrap();
        assert_eq!(scope, DocumentScope::Issue(String::new()));
        assert_eq!(scope.to_string(), "issue:");
    }
}
