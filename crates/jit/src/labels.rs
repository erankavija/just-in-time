//! Label validation and parsing for issue categorization and hierarchy.
//!
//! Labels follow a strict `namespace:value` format to enable deterministic
//! querying and AI-friendly semantics. This module provides validation,
//! parsing, and error handling with helpful suggestions.

use anyhow::{anyhow, Result};
use regex::Regex;
use std::sync::OnceLock;

/// Regex for valid label format: `namespace:value`
///
/// - namespace: `[a-z][a-z0-9-]*` (lowercase, alphanumeric, hyphens)
/// - value: `[a-zA-Z0-9][a-zA-Z0-9._/-]*` (alphanumeric, dots, hyphens,
///   underscores, and `/`). The `/` admits a QUALIFIED link reference value
///   `<issue>/<self-id>` (e.g. `satisfies:56ab0224/REQ-01`) so generic
///   node→item links can be authored as labels (REQ-05). The unqualified value
///   form is unchanged.
/// - separator: exactly one colon `':'`
static LABEL_REGEX: OnceLock<Regex> = OnceLock::new();

fn label_regex() -> &'static Regex {
    LABEL_REGEX.get_or_init(|| {
        Regex::new(r"^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._/-]*$")
            .expect("Label regex should compile")
    })
}

/// Validate a label format
///
/// Returns an error if the label doesn't match the expected format,
/// with suggestions for common mistakes.
///
/// # Examples
///
/// ```
/// use jit::labels::validate_label;
///
/// assert!(validate_label("milestone:v1.0").is_ok());
/// assert!(validate_label("epic:auth").is_ok());
/// assert!(validate_label("type:task").is_ok());
///
/// assert!(validate_label("auth").is_err()); // Missing namespace
/// assert!(validate_label("Milestone:v1.0").is_err()); // Uppercase namespace
/// assert!(validate_label("milestone-v1.0").is_err()); // Wrong separator
/// ```
pub fn validate_label(label: &str) -> Result<()> {
    if !label_regex().is_match(label) {
        // Provide helpful suggestions for common errors
        let suggestion = suggest_label_fix(label);
        let mut msg = format!(
            "Invalid label format: '{}'. Expected 'namespace:value'",
            label
        );
        if let Some(hint) = suggestion {
            msg.push_str(&format!(". {}", hint));
        }
        return Err(anyhow!(msg));
    }
    Ok(())
}

/// Parse a label into namespace and value components
///
/// # Examples
///
/// ```
/// use jit::labels::parse_label;
///
/// let (ns, val) = parse_label("milestone:v1.0").unwrap();
/// assert_eq!(ns, "milestone");
/// assert_eq!(val, "v1.0");
/// ```
pub fn parse_label(label: &str) -> Result<(String, String)> {
    validate_label(label)?;

    let parts: Vec<&str> = label.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(anyhow!(
            "Invalid label '{}': missing colon separator",
            label
        ));
    }

    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Suggest a corrected label format for common mistakes
fn suggest_label_fix(label: &str) -> Option<String> {
    // No colon: suggest adding namespace
    if !label.contains(':') {
        return Some(format!(
            "Did you mean 'type:{}' or 'component:{}'?",
            label.to_lowercase(),
            label.to_lowercase()
        ));
    }

    // Wrong separator (dash instead of colon)
    if label.contains('-') && !label.contains(':') {
        let fixed = label.replacen('-', ":", 1);
        return Some(format!("Did you mean '{}'?", fixed));
    }

    // Uppercase namespace
    if label.starts_with(|c: char| c.is_uppercase()) {
        let fixed = label.chars().next().unwrap().to_lowercase().to_string() + &label[1..];
        return Some(format!(
            "Namespace must be lowercase. Did you mean '{}'?",
            fixed
        ));
    }

    // Multiple colons
    if label.matches(':').count() > 1 {
        return Some("Label can only have one colon separator".to_string());
    }

    None
}

/// The namespace used for the issue type label.
///
/// Type labels have the form `type:<value>` (e.g. `type:task`, `type:epic`).
/// Use this const — not a raw `"type:"` string — anywhere the type namespace is
/// referenced so the encoding is centralized in a single place.
///
/// # Examples
///
/// ```
/// use jit::labels::TYPE_NAMESPACE;
///
/// let label = format!("{}:task", TYPE_NAMESPACE);
/// assert_eq!(label, "type:task");
/// ```
pub const TYPE_NAMESPACE: &str = "type";

/// Return `true` when `label` is a valid `type:*` label.
///
/// Delegates to [`parse_label`] so the `namespace:value` encoding is owned by
/// a single primitive. Returns `false` for invalid label formats.
///
/// # Examples
///
/// ```
/// use jit::labels::is_type_label;
///
/// assert!(is_type_label("type:task"));
/// assert!(is_type_label("type:epic"));
/// assert!(!is_type_label("priority:high")); // wrong namespace
/// assert!(!is_type_label("type:"));         // missing value — invalid format
/// assert!(!is_type_label("notacolon"));     // invalid label format
/// ```
pub fn is_type_label(label: &str) -> bool {
    parse_label(label)
        .map(|(ns, _)| ns == TYPE_NAMESPACE)
        .unwrap_or(false)
}

/// Extract the value of the `type:` label from a label list, if present.
///
/// Returns a reference to the value part (after the colon) of the first
/// `type:*` label found in `labels`, or `None` when no type label is present.
///
/// Internally delegates to [`parse_label`] (the canonical `namespace:value`
/// primitive) for namespace identification. The per-issue invariant is a
/// single `type:*` label; this function returns the first match. Presence
/// checks become `.is_some()`.
///
/// # Examples
///
/// ```
/// use jit::labels::type_label_value;
///
/// // Type present: returns the value
/// let labels = vec!["type:task".to_string(), "priority:high".to_string()];
/// assert_eq!(type_label_value(&labels), Some("task"));
///
/// // First type label wins when multiple are present
/// let multi = vec!["epic:auth".to_string(), "type:story".to_string()];
/// assert_eq!(type_label_value(&multi), Some("story"));
///
/// // No type label: returns None
/// let no_type = vec!["priority:high".to_string()];
/// assert_eq!(type_label_value(&no_type), None);
///
/// // Empty slice: returns None
/// assert_eq!(type_label_value(&[]), None);
/// ```
pub fn type_label_value(labels: &[String]) -> Option<&str> {
    labels.iter().find_map(|l| {
        // parse_label is the canonical encoding primitive; split_once is used
        // only to borrow the value from the original string without allocating.
        let (ns, _) = parse_label(l).ok()?;
        (ns == TYPE_NAMESPACE)
            .then(|| l.split_once(':').map(|(_, v)| v))
            .flatten()
    })
}

/// Check if a pattern matches any label in a collection
///
/// Supports:
/// - Exact match: `"epic:auth"` matches only `"epic:auth"`
/// - Wildcard: `"epic:*"` matches any label in epic namespace
///
/// Invalid labels in the collection are silently ignored during matching.
///
/// # Examples
///
/// ```
/// use jit::labels::matches_pattern;
///
/// let labels = vec!["epic:auth".to_string(), "type:task".to_string()];
/// assert!(matches_pattern(&labels, "epic:auth"));
/// assert!(matches_pattern(&labels, "epic:*"));
/// assert!(!matches_pattern(&labels, "milestone:*"));
/// ```
pub fn matches_pattern(issue_labels: &[String], pattern: &str) -> bool {
    if let Some(namespace) = pattern.strip_suffix(":*") {
        // Wildcard: match any label in namespace
        issue_labels.iter().any(|label| {
            parse_label(label)
                .map(|(ns, _)| ns == namespace)
                .unwrap_or(false)
        })
    } else {
        // Exact match
        issue_labels.contains(&pattern.to_string())
    }
}

/// Check if a single label matches a pattern
///
/// Convenience wrapper around `matches_pattern` for single label checks.
///
/// # Examples
///
/// ```
/// use jit::labels::label_matches;
///
/// assert!(label_matches("epic:auth", "epic:auth"));
/// assert!(label_matches("epic:auth", "epic:*"));
/// assert!(!label_matches("epic:auth", "milestone:*"));
/// ```
pub fn label_matches(label: &str, pattern: &str) -> bool {
    matches_pattern(&[label.to_string()], pattern)
}

/// Validate assignee format
///
/// Assignees must follow the format `type:identifier` (e.g., `agent:copilot`, `user:alice`)
///
/// # Examples
///
/// ```
/// use jit::labels::validate_assignee_format;
///
/// assert!(validate_assignee_format("agent:copilot").is_ok());
/// assert!(validate_assignee_format("user:alice").is_ok());
/// assert!(validate_assignee_format("invalid").is_err());
/// assert!(validate_assignee_format("").is_err());
/// ```
pub fn validate_assignee_format(assignee: &str) -> Result<()> {
    // Single source of truth for the `{type}:{identifier}` split lives in
    // `Assignee::from_str`; this is the thin `anyhow`-returning adapter for
    // callers that only need to validate (not retain) the parsed value.
    assignee
        .parse::<crate::domain::Assignee>()
        .map(|_| ())
        .map_err(|e| anyhow!(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_label_valid_formats() {
        assert!(validate_label("milestone:v1.0").is_ok());
        assert!(validate_label("epic:auth").is_ok());
        assert!(validate_label("component:backend").is_ok());
        assert!(validate_label("type:task").is_ok());
        assert!(validate_label("priority:p0").is_ok());
        assert!(validate_label("team:platform-eng").is_ok());
        assert!(validate_label("custom-namespace:value").is_ok());
        assert!(validate_label("ns:val-with-dash").is_ok());
        assert!(validate_label("ns:val_with_underscore").is_ok());
        assert!(validate_label("ns:val.with.dots").is_ok());
        assert!(validate_label("ns:MixedCase").is_ok());
        // Qualified link-reference value `<issue>/<self-id>` (REQ-05).
        assert!(validate_label("satisfies:56ab0224/REQ-01").is_ok());
    }

    #[test]
    fn test_validate_label_invalid_missing_namespace() {
        let result = validate_label("auth");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Invalid label format"));
    }

    #[test]
    fn test_validate_label_invalid_uppercase_namespace() {
        let result = validate_label("Milestone:v1.0");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_label_invalid_wrong_separator() {
        let result = validate_label("milestone-v1.0");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_label_invalid_empty_value() {
        let result = validate_label("milestone:");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_label_invalid_multiple_colons() {
        let result = validate_label("milestone:v1.0:extra");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_label_invalid_empty_namespace() {
        let result = validate_label(":value");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_label_valid() {
        let (ns, val) = parse_label("milestone:v1.0").unwrap();
        assert_eq!(ns, "milestone");
        assert_eq!(val, "v1.0");

        let (ns, val) = parse_label("epic:user-auth").unwrap();
        assert_eq!(ns, "epic");
        assert_eq!(val, "user-auth");
    }

    #[test]
    fn test_parse_label_preserves_case_in_value() {
        let (ns, val) = parse_label("component:MyComponent").unwrap();
        assert_eq!(ns, "component");
        assert_eq!(val, "MyComponent");
    }

    #[test]
    fn test_parse_label_invalid() {
        assert!(parse_label("invalid").is_err());
        assert!(parse_label("Invalid:value").is_err());
        assert!(parse_label("ns:val:extra").is_err());
    }

    #[test]
    fn test_suggest_label_fix_missing_namespace() {
        let suggestion = suggest_label_fix("auth");
        assert!(suggestion.is_some());
        assert!(suggestion.unwrap().contains("type:auth"));
    }

    #[test]
    fn test_suggest_label_fix_uppercase_namespace() {
        let suggestion = suggest_label_fix("Milestone:v1.0");
        assert!(suggestion.is_some());
        assert!(suggestion.unwrap().contains("milestone:v1.0"));
    }

    #[test]
    fn test_suggest_label_fix_multiple_colons() {
        let suggestion = suggest_label_fix("ns:val:extra");
        assert!(suggestion.is_some());
        assert!(suggestion.unwrap().contains("one colon"));
    }

    #[test]
    fn test_validate_assignee_format_valid() {
        assert!(validate_assignee_format("agent:copilot").is_ok());
        assert!(validate_assignee_format("user:alice").is_ok());
        assert!(validate_assignee_format("team:platform").is_ok());
    }

    #[test]
    fn test_validate_assignee_format_invalid_no_colon() {
        let result = validate_assignee_format("invalid");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("type:identifier"));
    }

    #[test]
    fn test_validate_assignee_format_invalid_empty() {
        let result = validate_assignee_format("");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_validate_assignee_format_invalid_empty_parts() {
        assert!(validate_assignee_format(":identifier").is_err());
        assert!(validate_assignee_format("type:").is_err());
    }

    // Tests for is_type_label
    #[test]
    fn test_is_type_label_true_for_type_labels() {
        assert!(is_type_label("type:task"));
        assert!(is_type_label("type:epic"));
        assert!(is_type_label("type:story"));
    }

    #[test]
    fn test_is_type_label_false_for_other_namespaces() {
        assert!(!is_type_label("priority:high"));
        assert!(!is_type_label("epic:auth"));
        assert!(!is_type_label("milestone:v1.0"));
    }

    #[test]
    fn test_is_type_label_false_for_invalid_labels() {
        assert!(!is_type_label("type:")); // missing value
        assert!(!is_type_label("notacolon"));
        assert!(!is_type_label(""));
    }

    // Tests for TYPE_NAMESPACE and type_label_value
    #[test]
    fn test_type_label_value_type_present() {
        let labels = vec!["type:task".to_string(), "priority:high".to_string()];
        assert_eq!(type_label_value(&labels), Some("task"));
    }

    #[test]
    fn test_type_label_value_type_absent() {
        let labels = vec!["priority:high".to_string(), "epic:auth".to_string()];
        assert_eq!(type_label_value(&labels), None);
    }

    #[test]
    fn test_type_label_value_empty() {
        assert_eq!(type_label_value(&[]), None);
    }

    #[test]
    fn test_type_label_value_multiple_labels_returns_first_type() {
        // First type: label in the list wins
        let labels = vec![
            "epic:auth".to_string(),
            "type:story".to_string(),
            "type:task".to_string(),
        ];
        assert_eq!(type_label_value(&labels), Some("story"));
    }

    #[test]
    fn test_type_label_value_value_extraction() {
        let labels = vec!["type:epic".to_string()];
        assert_eq!(type_label_value(&labels), Some("epic"));

        let labels = vec!["type:feature-request".to_string()];
        assert_eq!(type_label_value(&labels), Some("feature-request"));
    }

    // Tests for matches_pattern function
    #[test]
    fn test_matches_pattern_exact_match() {
        let labels = vec!["epic:auth".to_string(), "type:task".to_string()];

        assert!(matches_pattern(&labels, "epic:auth"));
        assert!(matches_pattern(&labels, "type:task"));
        assert!(!matches_pattern(&labels, "epic:other"));
        assert!(!matches_pattern(&labels, "milestone:v1"));
    }

    #[test]
    fn test_matches_pattern_wildcard() {
        let labels = vec![
            "epic:auth".to_string(),
            "type:task".to_string(),
            "component:backend".to_string(),
        ];

        // Should match any label in namespace
        assert!(matches_pattern(&labels, "epic:*"));
        assert!(matches_pattern(&labels, "type:*"));
        assert!(matches_pattern(&labels, "component:*"));

        // Should not match namespaces not present
        assert!(!matches_pattern(&labels, "milestone:*"));
        assert!(!matches_pattern(&labels, "priority:*"));
    }

    #[test]
    fn test_matches_pattern_empty_labels() {
        let labels: Vec<String> = vec![];

        assert!(!matches_pattern(&labels, "epic:auth"));
        assert!(!matches_pattern(&labels, "epic:*"));
    }

    #[test]
    fn test_matches_pattern_invalid_label_in_collection() {
        // Collection contains an invalid label
        let labels = vec![
            "epic:auth".to_string(),
            "invalid_no_colon".to_string(),
            "type:task".to_string(),
        ];

        // Should still match valid labels
        assert!(matches_pattern(&labels, "epic:auth"));
        assert!(matches_pattern(&labels, "type:task"));

        // Wildcard should ignore invalid labels
        assert!(matches_pattern(&labels, "epic:*"));
    }

    #[test]
    fn test_matches_pattern_case_sensitive() {
        let labels = vec!["epic:Auth".to_string()];

        // Exact match is case-sensitive
        assert!(matches_pattern(&labels, "epic:Auth"));
        assert!(!matches_pattern(&labels, "epic:auth"));

        // Wildcard matching namespace is case-sensitive
        assert!(matches_pattern(&labels, "epic:*"));
        assert!(!matches_pattern(&labels, "Epic:*"));
    }

    #[test]
    fn test_matches_pattern_multiple_labels_same_namespace() {
        let labels = vec![
            "epic:auth".to_string(),
            "epic:payments".to_string(),
            "type:task".to_string(),
        ];

        // Exact matches
        assert!(matches_pattern(&labels, "epic:auth"));
        assert!(matches_pattern(&labels, "epic:payments"));

        // Wildcard should match if any label in namespace
        assert!(matches_pattern(&labels, "epic:*"));
    }

    #[test]
    fn test_matches_pattern_edge_cases() {
        let labels = vec!["ns:value".to_string()];

        // Pattern without colon should not match
        assert!(!matches_pattern(&labels, "ns"));

        // Empty pattern should not match
        assert!(!matches_pattern(&labels, ""));

        // Just wildcard without namespace should not match
        assert!(!matches_pattern(&labels, ":*"));

        // Pattern with multiple colons
        assert!(!matches_pattern(&labels, "ns:val:extra"));
    }

    // Tests for label_matches function
    #[test]
    fn test_label_matches_exact() {
        assert!(label_matches("epic:auth", "epic:auth"));
        assert!(label_matches("type:task", "type:task"));
        assert!(!label_matches("epic:auth", "epic:other"));
    }

    #[test]
    fn test_label_matches_wildcard() {
        assert!(label_matches("epic:auth", "epic:*"));
        assert!(label_matches("type:task", "type:*"));
        assert!(!label_matches("epic:auth", "milestone:*"));
    }

    #[test]
    fn test_label_matches_invalid_label() {
        // Exact match works even for invalid labels (no validation in matching)
        assert!(label_matches("invalid_no_colon", "invalid_no_colon"));

        // Wildcard won't match invalid labels (parse_label fails)
        assert!(!label_matches("invalid", "invalid:*"));
        assert!(!label_matches("Invalid:value", "Invalid:*"));
    }
}
