//! Label validation and enforcement.
//!
//! This module provides validation rules for issue labels to ensure consistency
//! and prevent common mistakes by agents and users.

use anyhow::{anyhow, Result};

/// Validate that labels include exactly one type:* label
///
/// # Examples
///
/// ```
/// use jit::label_validation::validate_required_labels;
///
/// // Valid: exactly one type label
/// let labels = vec!["type:task".to_string(), "epic:auth".to_string()];
/// assert!(validate_required_labels(&labels).is_ok());
///
/// // Invalid: no type label
/// let labels = vec!["epic:auth".to_string()];
/// assert!(validate_required_labels(&labels).is_err());
///
/// // Invalid: multiple type labels
/// let labels = vec!["type:task".to_string(), "type:bug".to_string()];
/// assert!(validate_required_labels(&labels).is_err());
/// ```
pub fn validate_required_labels(labels: &[String]) -> Result<()> {
    let type_labels: Vec<_> = labels.iter().filter(|l| l.starts_with("type:")).collect();

    if type_labels.is_empty() {
        return Err(anyhow!(
            "Issue must have exactly one type label. \
             Valid types: type:task, type:epic, type:milestone, type:bug, type:feature, type:research"
        ));
    }

    if type_labels.len() > 1 {
        let type_list = type_labels
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(anyhow!(
            "Issue can only have ONE type label. Found: {}. \
             Use --remove-label to remove existing type before adding new one.",
            type_list
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_required_labels_valid_single_type() {
        let labels = vec!["type:task".to_string()];
        assert!(validate_required_labels(&labels).is_ok());
    }

    #[test]
    fn test_validate_required_labels_valid_with_other_labels() {
        let labels = vec![
            "type:task".to_string(),
            "epic:auth".to_string(),
            "milestone:v1.0".to_string(),
        ];
        assert!(validate_required_labels(&labels).is_ok());
    }

    #[test]
    fn test_validate_required_labels_missing_type() {
        let labels = vec!["epic:auth".to_string()];
        let result = validate_required_labels(&labels);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("must have exactly one type label"));
    }

    #[test]
    fn test_validate_required_labels_empty_labels() {
        let labels: Vec<String> = vec![];
        let result = validate_required_labels(&labels);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("must have exactly one type label"));
    }

    #[test]
    fn test_validate_required_labels_multiple_types() {
        let labels = vec!["type:task".to_string(), "type:bug".to_string()];
        let result = validate_required_labels(&labels);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("can only have ONE type label"));
        assert!(msg.contains("type:task"));
        assert!(msg.contains("type:bug"));
    }

    #[test]
    fn test_validate_required_labels_all_valid_types() {
        let valid_types = vec![
            "type:task",
            "type:epic",
            "type:milestone",
            "type:bug",
            "type:feature",
            "type:research",
        ];

        for type_label in valid_types {
            let labels = vec![type_label.to_string()];
            assert!(
                validate_required_labels(&labels).is_ok(),
                "Expected {} to be valid",
                type_label
            );
        }
    }

    #[test]
    fn test_validate_required_labels_case_sensitive() {
        // type:Task should work (value is case-insensitive)
        let labels = vec!["type:Task".to_string()];
        assert!(validate_required_labels(&labels).is_ok());

        // Type:task should NOT work (namespace is case-sensitive)
        let labels = vec!["Type:task".to_string()];
        let result = validate_required_labels(&labels);
        assert!(result.is_err());
    }
}
