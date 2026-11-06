//! Shared issue selection for commands over issue records.

use super::identity::{InvalidIdPrefixError, IssueId, IssueIdPrefix};
use super::Issue;
use std::{fmt, str::FromStr};

/// A selector before or after its issue prefix has been resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueScope<I> {
    All,
    Issue(I),
    Label { namespace: String, value: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IssueScopeParseError {
    #[error("Invalid scope '{0}'. Expected 'all', 'issue:ID', or 'label:namespace:value'")]
    UnknownScope(String),
    #[error("Invalid label scope '{0}'. Expected 'label:namespace:value' with non-empty namespace and value")]
    MalformedLabel(String),
    #[error("{0}")]
    InvalidIdPrefix(#[from] InvalidIdPrefixError),
}

impl FromStr for IssueScope<IssueIdPrefix> {
    type Err = IssueScopeParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input == "all" {
            Ok(Self::All)
        } else if let Some(prefix) = input.strip_prefix("issue:") {
            Ok(Self::Issue(prefix.parse()?))
        } else if let Some(label) = input.strip_prefix("label:") {
            let (namespace, value) = label
                .split_once(':')
                .filter(|(namespace, value)| !namespace.is_empty() && !value.is_empty())
                .ok_or_else(|| IssueScopeParseError::MalformedLabel(input.to_string()))?;
            Ok(Self::Label {
                namespace: namespace.to_string(),
                value: value.to_string(),
            })
        } else {
            Err(IssueScopeParseError::UnknownScope(input.to_string()))
        }
    }
}

impl IssueScope<IssueIdPrefix> {
    /// Resolve only the issue variant, preserving the resolver's error.
    pub fn resolve<E>(
        self,
        resolver: impl FnOnce(&IssueIdPrefix) -> Result<IssueId, E>,
    ) -> Result<IssueScope<IssueId>, E> {
        Ok(match self {
            Self::All => IssueScope::All,
            Self::Issue(prefix) => IssueScope::Issue(resolver(&prefix)?),
            Self::Label { namespace, value } => IssueScope::Label { namespace, value },
        })
    }
}

impl<I: fmt::Display> fmt::Display for IssueScope<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => write!(f, "all"),
            Self::Issue(id) => write!(f, "issue:{id}"),
            Self::Label { namespace, value } => write!(f, "label:{namespace}:{value}"),
        }
    }
}

/// Select exact ids or labels, preserving the input order.
pub fn select_issues(issues: &[Issue], scope: &IssueScope<IssueId>) -> Vec<Issue> {
    match scope {
        IssueScope::All => issues.to_vec(),
        IssueScope::Issue(id) => issues
            .iter()
            .filter(|issue| &issue.id == id)
            .cloned()
            .collect(),
        IssueScope::Label { namespace, value } => {
            let label = format!("{namespace}:{value}");
            issues
                .iter()
                .filter(|issue| issue.labels.contains(&label))
                .cloned()
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Result<IssueScope<IssueIdPrefix>, IssueScopeParseError> {
        input.parse()
    }

    #[test]
    fn test_issue_scope_parse_valid_forms() {
        assert_eq!(parse("all").unwrap(), IssueScope::All);
        assert_eq!(
            parse("issue:abc123").unwrap(),
            IssueScope::Issue("abc123".parse().unwrap())
        );
        for value in ["auth", "auth:login", "@/rule/label-format"] {
            assert_eq!(
                parse(&format!("label:epic:{value}")).unwrap(),
                IssueScope::Label {
                    namespace: "epic".to_string(),
                    value: value.to_string(),
                }
            );
        }
    }

    #[test]
    fn test_issue_scope_parse_rejects_invalid_forms_with_typed_errors() {
        for prefix in ["", "abc"] {
            assert_eq!(
                parse(&format!("issue:{prefix}")),
                Err(IssueScopeParseError::InvalidIdPrefix(
                    InvalidIdPrefixError::new(prefix)
                ))
            );
        }
        for input in ["label:epic", "label:epic:", "label::auth", "label:"] {
            assert_eq!(
                parse(input),
                Err(IssueScopeParseError::MalformedLabel(input.to_string()))
            );
        }
        assert_eq!(
            parse("unknown:123"),
            Err(IssueScopeParseError::UnknownScope(
                "unknown:123".to_string()
            ))
        );
    }

    #[test]
    fn test_issue_scope_display_round_trips() {
        for input in [
            "all",
            "issue:AbC123",
            "label:epic:auth",
            "label:epic:auth:login",
        ] {
            let scope = parse(input).unwrap();
            assert_eq!(scope.to_string().parse(), Ok(scope));
        }
    }

    #[test]
    fn test_issue_scope_resolve_maps_only_issue_prefix_and_propagates_errors() {
        let id = crate::test_utils::issue_id(1);
        let resolved = parse("issue:abc123").unwrap().resolve(|prefix| {
            assert_eq!(prefix.as_str(), "abc123");
            Ok::<_, ()>(id.clone())
        });
        assert_eq!(resolved, Ok(IssueScope::Issue(id)));
        assert_eq!(
            parse("issue:abc123")
                .unwrap()
                .resolve(|_| Err("unresolved")),
            Err("unresolved")
        );
        for input in ["all", "label:epic:auth"] {
            let resolved = parse(input)
                .unwrap()
                .resolve(|_| Err("unexpected resolution"));
            assert_eq!(resolved.unwrap().to_string(), input);
        }
    }

    #[test]
    fn test_select_issues_preserves_order_and_matches_exact_ids_and_labels() {
        let issues: Vec<_> = ["team:auth", "team:auth:login", "other:auth", "team:auth"]
            .iter()
            .map(|label| {
                let mut issue = crate::test_utils::fixture_issue(label.to_string(), String::new());
                issue.labels = vec![label.to_string()];
                issue
            })
            .collect();
        let cases = [
            (IssueScope::All, issues.clone()),
            (
                IssueScope::Issue(issues[1].id.clone()),
                vec![issues[1].clone()],
            ),
            (IssueScope::Issue(crate::test_utils::issue_id(1)), vec![]),
            (
                IssueScope::Label {
                    namespace: "team".into(),
                    value: "auth".into(),
                },
                vec![issues[0].clone(), issues[3].clone()],
            ),
            (
                IssueScope::Label {
                    namespace: "team".into(),
                    value: "auth:login".into(),
                },
                vec![issues[1].clone()],
            ),
            (
                IssueScope::Label {
                    namespace: "team".into(),
                    value: "missing".into(),
                },
                vec![],
            ),
        ];
        for (scope, expected) in cases {
            assert_eq!(select_issues(&issues, &scope), expected);
            assert!(select_issues(&[], &scope).is_empty());
        }
    }
}
