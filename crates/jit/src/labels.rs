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
}
