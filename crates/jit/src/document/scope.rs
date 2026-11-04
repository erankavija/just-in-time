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
    use crate::domain::identity::IssueId;

    #[test]
    fn test_document_scope_parse_all() {
        assert_eq!(
            DocumentScope::parse("all", |id| Ok(id.parse::<IssueId>()?)).unwrap(),
            DocumentScope::All
        );
        assert_eq!(DocumentScope::All.to_string(), "all");
    }

    #[test]
    fn test_document_scope_parse_issue() {
        let resolved = crate::test_utils::issue_id(0x4000_8000_0000_0000_0001_0f0e_0d0c);
        let scope = DocumentScope::parse("issue:abc123", |id| {
            assert_eq!(id, "abc123");
            Ok(resolved.clone())
        })
        .unwrap();
        assert_eq!(scope, DocumentScope::Issue(resolved.clone()));
        assert_eq!(scope.to_string(), format!("issue:{resolved}"));
    }

    #[test]
    fn test_document_scope_parse_invalid() {
        let err = DocumentScope::parse("label:epic", |id| Ok(id.parse::<IssueId>()?)).unwrap_err();
        assert_eq!(
            err.downcast_ref::<DocumentScopeParseError>().unwrap(),
            &DocumentScopeParseError::UnknownScope("label:epic".to_string())
        );
        // The message preserves the original wording the CLI surfaced before.
        assert!(err.to_string().contains("Use 'all' or 'issue:ID'"));
    }

    #[test]
    fn test_document_scope_parse_issue_delegates_empty_id_to_resolver() {
        let err = DocumentScope::parse("issue:", |id| {
            assert!(id.is_empty());
            anyhow::bail!("empty issue argument")
        })
        .unwrap_err();
        assert_eq!(err.to_string(), "empty issue argument");
    }
}
