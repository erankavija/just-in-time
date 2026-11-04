//! Manifest-key references in coverage-credit labels.
//!
//! A batch manifest may credit a container it creates itself by naming the
//! container's manifest key where its short id belongs:
//! `<namespace>:{<key>.short_id}/<self-id>`. The `{<anchor>.short_id}` token is
//! the one graph templates already use (`{container.short_id}`); batch creation
//! resolves the key through the same key → created-id map that wires
//! `depends_on`.

use crate::domain::identity::IssueId;
use std::collections::HashSet;

/// The manifest key referenced by a label value of the form
/// `{<key>.short_id}/<self-id>`, or `None` for any other value.
pub fn referenced_key(value: &str) -> Option<&str> {
    let (token, self_id) = value.split_once('/')?;
    let key = token.strip_prefix('{')?.strip_suffix(".short_id}")?;
    (!key.is_empty() && !self_id.is_empty()).then_some(key)
}

/// A manifest key that names no created issue.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown manifest key '{0}'")]
pub struct UnknownManifestKey(pub String);

/// Resolve a manifest-key reference in `label` when its namespace is one of
/// `credit_namespaces`; any other label is returned unchanged.
///
/// # Errors
///
/// Returns the referenced key when `issue_id_of` does not know it.
pub fn resolve_key_reference(
    label: &str,
    credit_namespaces: &HashSet<String>,
    issue_id_of: impl Fn(&str) -> Option<IssueId>,
) -> Result<String, UnknownManifestKey> {
    let Some((namespace, value)) = label.split_once(':') else {
        return Ok(label.to_string());
    };
    let Some(key) = referenced_key(value).filter(|_| credit_namespaces.contains(namespace)) else {
        return Ok(label.to_string());
    };
    let id = issue_id_of(key).ok_or_else(|| UnknownManifestKey(key.to_string()))?;
    let short = id.short_id();
    let self_id = &value[value.find('/').map_or(value.len(), |i| i + 1)..];
    Ok(format!("{namespace}:{short}/{self_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credit() -> HashSet<String> {
        HashSet::from(["satisfies".to_string()])
    }

    #[test]
    fn test_referenced_key_accepts_only_the_short_id_token() {
        assert_eq!(
            referenced_key("{story-auth.short_id}/REQ-01"),
            Some("story-auth")
        );
        for value in [
            "REQ-01",
            "abc12345/REQ-01",
            "{story-auth.id}/REQ-01",
            "{.short_id}/REQ-01",
            "{story-auth.short_id}/",
            "@/issue/abc12345/requirement/REQ-01",
        ] {
            assert_eq!(referenced_key(value), None, "{value}");
        }
    }

    #[test]
    fn test_resolve_key_reference_substitutes_known_key_in_credit_namespace() {
        let resolved =
            resolve_key_reference("satisfies:{story.short_id}/REQ-01", &credit(), |key| {
                (key == "story").then(|| crate::test_utils::issue_id(1))
            });
        assert_eq!(
            resolved,
            Ok(format!(
                "satisfies:{}/REQ-01",
                crate::test_utils::issue_id(1).short_id()
            ))
        );
    }

    #[test]
    fn test_resolve_key_reference_reports_unknown_key() {
        let resolved =
            resolve_key_reference("satisfies:{ghost.short_id}/REQ-01", &credit(), |_| None);
        assert_eq!(resolved, Err(UnknownManifestKey("ghost".to_string())));
    }

    #[test]
    fn test_resolve_key_reference_leaves_other_namespaces_and_values() {
        for label in [
            "component:{story.short_id}/x",
            "satisfies:abc12345/REQ-01",
            "plain",
        ] {
            assert_eq!(
                resolve_key_reference(label, &credit(), |_| Some(crate::test_utils::issue_id(1))),
                Ok(label.to_string())
            );
        }
    }
}
