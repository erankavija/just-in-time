//! Issue validation logic based on configuration.
//!
//! This module provides validation for issues based on config.toml settings,
//! including type label requirements, label format validation, and namespace
//! registry enforcement.

use crate::config::ValidationConfig;
use crate::domain::{Issue, LabelNamespaces};
use crate::labels as label_utils;
use anyhow::{anyhow, Result};

/// Validates issues against configuration rules.
pub struct IssueValidator {
    validation_config: ValidationConfig,
    namespaces: LabelNamespaces,
}

impl IssueValidator {
    /// Create a new validator with the given configuration.
    pub fn new(validation_config: ValidationConfig, namespaces: LabelNamespaces) -> Self {
        Self {
            validation_config,
            namespaces,
        }
    }

    /// Validate an issue against all configured rules.
    ///
    /// Returns an error if validation fails and reject_malformed_labels is true,
    /// otherwise logs warnings.
    pub fn validate(&self, issue: &Issue) -> Result<Vec<String>> {
        let mut warnings = Vec::new();

        // Check type label requirement
        if let Some(warning) = self.validate_type_label(issue)? {
            warnings.push(warning);
        }

        // Validate label formats
        for warning in self.validate_label_formats(issue)? {
            warnings.push(warning);
        }

        // Validate namespace registry
        for warning in self.validate_namespace_registry(issue)? {
            warnings.push(warning);
        }

        Ok(warnings)
    }

    /// Apply default type label if missing and configured.
    ///
    /// Modifies the issue's labels in place if default_type is set and
    /// the issue has no type:* label.
    pub fn apply_default_type(&self, labels: &mut Vec<String>) {
        if let Some(ref default_type) = self.validation_config.default_type {
            // Check if issue already has a type label
            let has_type_label = labels.iter().any(|label| {
                if let Ok((namespace, _)) = label_utils::parse_label(label) {
                    namespace == "type"
                } else {
                    false
                }
            });

            if !has_type_label {
                labels.push(format!("type:{}", default_type));
            }
        }
    }

    /// Validate that issue has a type label if required.
    fn validate_type_label(&self, issue: &Issue) -> Result<Option<String>> {
        let require_type = self.validation_config.require_type_label.unwrap_or(false);

        if require_type {
            let has_type_label = issue.labels.iter().any(|label| {
                if let Ok((namespace, _)) = label_utils::parse_label(label) {
                    namespace == "type"
                } else {
                    false
                }
            });

            if !has_type_label {
                return Err(anyhow!(
                    "Issue must have a type:* label (require_type_label is enabled)"
                ));
            }
        }

        Ok(None)
    }

    /// Validate label formats against configured regex.
    fn validate_label_formats(&self, issue: &Issue) -> Result<Vec<String>> {
        let mut warnings = Vec::new();

        if let Some(ref regex_pattern) = self.validation_config.label_regex {
            let regex = regex::Regex::new(regex_pattern)
                .map_err(|e| anyhow!("Invalid label_regex in config: {}", e))?;

            for label in &issue.labels {
                if !regex.is_match(label) {
                    let message =
                        format!("Label '{}' does not match format: {}", label, regex_pattern);

                    if self
                        .validation_config
                        .reject_malformed_labels
                        .unwrap_or(false)
                    {
                        return Err(anyhow!(message));
                    } else {
                        warnings.push(message);
                    }
                }
            }
        }

        Ok(warnings)
    }

    /// Validate that all label namespaces are registered.
    fn validate_namespace_registry(&self, issue: &Issue) -> Result<Vec<String>> {
        let mut warnings = Vec::new();

        if !self
            .validation_config
            .enforce_namespace_registry
            .unwrap_or(false)
        {
            return Ok(warnings);
        }

        for label in &issue.labels {
            if let Ok((namespace, _)) = label_utils::parse_label(label) {
                if !self.namespaces.namespaces.contains_key(&namespace) {
                    let message = format!(
                        "Label namespace '{}' is not registered (from label '{}')",
                        namespace, label
                    );

                    if self
                        .validation_config
                        .reject_malformed_labels
                        .unwrap_or(false)
                    {
                        return Err(anyhow!(message));
                    } else {
                        warnings.push(message);
                    }
                }
            }
        }

        Ok(warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::LabelNamespace;
    use std::collections::HashMap;

    fn create_test_validator(
        validation_config: ValidationConfig,
        namespaces: LabelNamespaces,
    ) -> IssueValidator {
        IssueValidator::new(validation_config, namespaces)
    }

    fn create_test_issue(labels: Vec<String>) -> Issue {
        Issue::new_with_labels("Test issue".to_string(), "Description".to_string(), labels)
    }

    #[test]
    fn test_apply_default_type_when_missing() {
        let validation_config = ValidationConfig {
            strictness: None,
            default_type: Some("task".to_string()),
            require_type_label: None,
            label_regex: None,
            reject_malformed_labels: None,
            enforce_namespace_registry: None,
            warn_orphaned_leaves: None,
            warn_strategic_consistency: None,
        };

        let validator = create_test_validator(validation_config, LabelNamespaces::default());

        let mut labels = vec!["epic:auth".to_string()];
        validator.apply_default_type(&mut labels);

        assert_eq!(labels.len(), 2);
        assert!(labels.contains(&"type:task".to_string()));
    }

    #[test]
    fn test_apply_default_type_when_already_present() {
        let validation_config = ValidationConfig {
            strictness: None,
            default_type: Some("task".to_string()),
            require_type_label: None,
            label_regex: None,
            reject_malformed_labels: None,
            enforce_namespace_registry: None,
            warn_orphaned_leaves: None,
            warn_strategic_consistency: None,
        };

        let validator = create_test_validator(validation_config, LabelNamespaces::default());

        let mut labels = vec!["type:story".to_string(), "epic:auth".to_string()];
        validator.apply_default_type(&mut labels);

        // Should not add another type label
        assert_eq!(labels.len(), 2);
        assert!(labels.contains(&"type:story".to_string()));
        assert!(!labels.contains(&"type:task".to_string()));
    }

    #[test]
    fn test_require_type_label_enforced() {
        let validation_config = ValidationConfig {
            strictness: None,
            default_type: None,
            require_type_label: Some(true),
            label_regex: None,
            reject_malformed_labels: None,
            enforce_namespace_registry: None,
            warn_orphaned_leaves: None,
            warn_strategic_consistency: None,
        };

        let validator = create_test_validator(validation_config, LabelNamespaces::default());

        let issue = create_test_issue(vec!["epic:auth".to_string()]);
        let result = validator.validate(&issue);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("type:*"));
    }

    #[test]
    fn test_require_type_label_passes() {
        let validation_config = ValidationConfig {
            strictness: None,
            default_type: None,
            require_type_label: Some(true),
            label_regex: None,
            reject_malformed_labels: None,
            enforce_namespace_registry: None,
            warn_orphaned_leaves: None,
            warn_strategic_consistency: None,
        };

        let validator = create_test_validator(validation_config, LabelNamespaces::default());

        let issue = create_test_issue(vec!["type:story".to_string(), "epic:auth".to_string()]);
        let result = validator.validate(&issue);

        assert!(result.is_ok());
    }

    #[test]
    fn test_label_regex_validation_passes() {
        let validation_config = ValidationConfig {
            strictness: None,
            default_type: None,
            require_type_label: None,
            label_regex: Some(r"^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$".to_string()),
            reject_malformed_labels: Some(true),
            enforce_namespace_registry: None,
            warn_orphaned_leaves: None,
            warn_strategic_consistency: None,
        };

        let validator = create_test_validator(validation_config, LabelNamespaces::default());

        let issue = create_test_issue(vec![
            "type:story".to_string(),
            "epic:auth-service".to_string(),
        ]);
        let result = validator.validate(&issue);

        assert!(result.is_ok());
    }

    #[test]
    fn test_label_regex_validation_fails() {
        let validation_config = ValidationConfig {
            strictness: None,
            default_type: None,
            require_type_label: None,
            label_regex: Some(r"^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$".to_string()),
            reject_malformed_labels: Some(true),
            enforce_namespace_registry: None,
            warn_orphaned_leaves: None,
            warn_strategic_consistency: None,
        };

        let validator = create_test_validator(validation_config, LabelNamespaces::default());

        let issue = create_test_issue(vec!["INVALID:label".to_string()]);
        let result = validator.validate(&issue);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not match"));
    }

    #[test]
    fn test_label_regex_validation_warns_when_not_rejecting() {
        let validation_config = ValidationConfig {
            strictness: None,
            default_type: None,
            require_type_label: None,
            label_regex: Some(r"^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$".to_string()),
            reject_malformed_labels: Some(false),
            enforce_namespace_registry: None,
            warn_orphaned_leaves: None,
            warn_strategic_consistency: None,
        };

        let validator = create_test_validator(validation_config, LabelNamespaces::default());

        let issue = create_test_issue(vec!["INVALID:label".to_string()]);
        let result = validator.validate(&issue);

        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("does not match"));
    }

    #[test]
    fn test_enforce_namespace_registry() {
        let mut namespaces_map = HashMap::new();
        namespaces_map.insert(
            "type".to_string(),
            LabelNamespace::new("Issue type".to_string(), true),
        );
        namespaces_map.insert(
            "epic".to_string(),
            LabelNamespace::new("Epic".to_string(), false),
        );

        let namespaces = LabelNamespaces {
            schema_version: 2,
            namespaces: namespaces_map,
            type_hierarchy: None,
            label_associations: None,
            strategic_types: None,
        };

        let validation_config = ValidationConfig {
            strictness: None,
            default_type: None,
            require_type_label: None,
            label_regex: None,
            reject_malformed_labels: Some(true),
            enforce_namespace_registry: Some(true),
            warn_orphaned_leaves: None,
            warn_strategic_consistency: None,
        };

        let validator = create_test_validator(validation_config, namespaces);

        // Valid namespace
        let issue1 = create_test_issue(vec!["type:story".to_string()]);
        assert!(validator.validate(&issue1).is_ok());

        // Invalid namespace
        let issue2 = create_test_issue(vec!["unknown:value".to_string()]);
        let result = validator.validate(&issue2);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not registered"));
    }

    #[test]
    fn test_namespace_registry_warns_when_not_rejecting() {
        let mut namespaces_map = HashMap::new();
        namespaces_map.insert(
            "type".to_string(),
            LabelNamespace::new("Issue type".to_string(), true),
        );

        let namespaces = LabelNamespaces {
            schema_version: 2,
            namespaces: namespaces_map,
            type_hierarchy: None,
            label_associations: None,
            strategic_types: None,
        };

        let validation_config = ValidationConfig {
            strictness: None,
            default_type: None,
            require_type_label: None,
            label_regex: None,
            reject_malformed_labels: Some(false),
            enforce_namespace_registry: Some(true),
            warn_orphaned_leaves: None,
            warn_strategic_consistency: None,
        };

        let validator = create_test_validator(validation_config, namespaces);

        let issue = create_test_issue(vec!["unknown:value".to_string()]);
        let result = validator.validate(&issue);

        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("not registered"));
    }
}
