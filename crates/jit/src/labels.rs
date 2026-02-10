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
/// - value: `[a-zA-Z0-9][a-zA-Z0-9._-]*` (alphanumeric, dots, hyphens, underscores)
/// - separator: exactly one colon `':'`
static LABEL_REGEX: OnceLock<Regex> = OnceLock::new();

fn label_regex() -> &'static Regex {
    LABEL_REGEX.get_or_init(|| {
        Regex::new(r"^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$")
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
#[allow(dead_code)] // Will be used in Phase 1.4 for query by label
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

/// Validate label operations (add/remove) against namespace constraints
///
/// Checks format of all labels being added and validates uniqueness constraints
/// for the final set of labels after applying operations.
///
/// # Arguments
///
/// * `existing_labels` - Current labels on the issue
/// * `add_labels` - Labels to be added
/// * `remove_labels` - Labels to be removed
/// * `namespaces` - Namespace configuration (HashMap from config)
///
/// # Examples
///
/// ```
/// use jit::labels::validate_label_operations;
/// use std::collections::HashMap;
/// use jit::domain::LabelNamespace;
///
/// let existing = vec!["type:task".to_string()];
/// let add = vec!["milestone:v1.0".to_string()];
/// let remove = vec![];
/// let mut namespaces = HashMap::new();
/// namespaces.insert("type".to_string(), LabelNamespace::new("Issue type", true));
///
/// // Should pass - no conflicts
/// assert!(validate_label_operations(&existing, &add, &remove, &namespaces).is_ok());
/// ```
pub fn validate_label_operations(
    existing_labels: &[String],
    add_labels: &[String],
    remove_labels: &[String],
    namespaces: &std::collections::HashMap<String, crate::domain::LabelNamespace>,
) -> Result<()> {
    // First, validate format of all labels being added
    for label in add_labels {
        validate_label(label)?;
    }

    // Compute final labels after operations
    let mut final_labels = existing_labels.to_vec();

    // Remove labels
    for label in remove_labels {
        if let Some(pos) = final_labels.iter().position(|l| l == label) {
            final_labels.remove(pos);
        }
    }

    // Add new labels (avoid duplicates)
    for label in add_labels {
        if !final_labels.contains(label) {
            final_labels.push(label.clone());
        }
    }

    // Check uniqueness constraints on final set
    let mut unique_namespaces_seen = std::collections::HashSet::new();

    for label in &final_labels {
        if let Ok((namespace, _)) = parse_label(label) {
            if let Some(ns_config) = namespaces.get(&namespace) {
                if ns_config.unique && !unique_namespaces_seen.insert(namespace.clone()) {
                    return Err(anyhow!(
                        "Cannot add multiple labels from unique namespace '{}' to the same issue",
                        namespace
                    ));
                }
            }
        }
    }

    Ok(())
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
    if assignee.is_empty() {
        return Err(anyhow!("Assignee cannot be empty"));
    }

    if !assignee.contains(':') {
        return Err(anyhow!(
            "Assignee must be in format 'type:identifier' (e.g., 'agent:copilot', 'user:alice'). Got: '{}'",
            assignee
        ));
    }

    let parts: Vec<&str> = assignee.splitn(2, ':').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(anyhow!(
            "Assignee must be in format 'type:identifier' with non-empty type and identifier. Got: '{}'",
            assignee
        ));
    }

    Ok(())
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

    #[test]
    fn test_validate_label_operations_valid() {
        use std::collections::HashMap;

        let existing = vec!["type:task".to_string()];
        let add = vec!["milestone:v1.0".to_string(), "epic:auth".to_string()];
        let remove = vec![];
        let namespaces = HashMap::new();

        assert!(validate_label_operations(&existing, &add, &remove, &namespaces).is_ok());
    }

    #[test]
    fn test_validate_label_operations_rejects_invalid_format() {
        use std::collections::HashMap;

        let existing = vec!["type:task".to_string()];
        let add = vec!["bad_label_no_colon".to_string()];
        let remove = vec![];
        let namespaces = HashMap::new();

        let result = validate_label_operations(&existing, &add, &remove, &namespaces);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid label format"));
    }

    #[test]
    fn test_validate_label_operations_rejects_duplicate_unique_namespace() {
        use crate::domain::LabelNamespace;
        use std::collections::HashMap;

        let existing = vec!["type:task".to_string()];
        let add = vec!["type:epic".to_string()];
        let remove = vec![];

        let mut namespaces = HashMap::new();
        namespaces.insert("type".to_string(), LabelNamespace::new("Issue type", true));

        let result = validate_label_operations(&existing, &add, &remove, &namespaces);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unique namespace"));
    }

    #[test]
    fn test_validate_label_operations_allows_replacing_unique_namespace() {
        use crate::domain::LabelNamespace;
        use std::collections::HashMap;

        let existing = vec!["type:task".to_string()];
        let add = vec!["type:epic".to_string()];
        let remove = vec!["type:task".to_string()];

        let mut namespaces = HashMap::new();
        namespaces.insert("type".to_string(), LabelNamespace::new("Issue type", true));

        // Should succeed - removing old type before adding new one
        assert!(validate_label_operations(&existing, &add, &remove, &namespaces).is_ok());
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
}
