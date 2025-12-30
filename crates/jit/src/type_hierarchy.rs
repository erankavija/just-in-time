//! Type hierarchy validation for issue labels.
//!
//! # CRITICAL: Type Hierarchy is Orthogonal to the Dependency DAG
//!
//! This module validates **type labels only**. It does NOT validate dependencies.
//!
//! ## Two Separate Concepts:
//!
//! 1. **Dependency DAG** (work sequencing - unrestricted):
//!    - Expresses "what must complete before what"
//!    - ✅ task → epic (task needs epic defined)
//!    - ✅ epic → task (epic needs specific task done)
//!    - ✅ milestone → task (milestone needs task completed)
//!    - **NO RESTRICTIONS** - any issue can depend on any other for work flow
//!
//! 2. **Type Hierarchy** (organizational labels - future validation):
//!    - Expresses "what belongs to what" via labels (`epic:auth`, `milestone:v1.0`)
//!    - ✅ task with `epic:auth` = task belongs to auth epic (future validation)
//!    - ❌ epic with `task:xyz` = nonsensical (future validation)
//!    - **NOT YET IMPLEMENTED** - currently only validates type labels are known
//!
//! ## Current Scope
//!
//! This module currently ONLY validates:
//! - Type labels exist and are known (`type:task` is valid, `type:taks` is typo)
//! - Suggests fixes for unknown/typo type labels
//!
//! It does NOT:
//! - Validate dependencies (dependencies are unrestricted by type)
//! - Check organizational membership (future: via `epic:*`, `milestone:*` labels)
//!
//! # Examples
//!
//! ```
//! use jit::type_hierarchy::{extract_type, HierarchyConfig};
//!
//! let config = HierarchyConfig::default();
//!
//! // Extract and validate type labels
//! assert_eq!(extract_type("type:task"), Ok("task".to_string()));
//! assert_eq!(extract_type("type:epic"), Ok("epic".to_string()));
//! assert!(config.contains_type("task"));
//! assert!(!config.contains_type("unknown"));
//! ```

use std::collections::HashMap;
use thiserror::Error;

/// Errors that can occur during hierarchy validation.
#[derive(Debug, Error, PartialEq)]
pub enum HierarchyError {
    #[error("Unknown type: '{0}'")]
    #[allow(dead_code)] // Used in validation logic, may be used by external consumers
    UnknownType(String),

    #[error("Invalid label format: '{0}' (expected 'type:value')")]
    InvalidLabel(String),
}

/// Represents a validation issue found in the repository.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationIssue {
    /// An issue has an unknown type label
    UnknownType {
        issue_id: String,
        unknown_type: String,
        suggested_fix: Option<String>,
    },
    /// An issue has a membership label referencing a non-existent issue
    #[allow(dead_code)] // Reserved for future membership validation feature
    InvalidMembershipReference {
        issue_id: String,
        label: String,
        namespace: String,
        value: String,
        reason: String,
    },
}

/// Represents a fix to apply to resolve a validation issue.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationFix {
    /// Replace an unknown type with a suggested type
    ReplaceType {
        issue_id: String,
        old_type: String,
        new_type: String,
    },
}

/// Validation warnings (soft constraints that don't block operations).
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationWarning {
    /// A strategic type issue (epic, milestone) is missing its identifying label
    MissingStrategicLabel {
        issue_id: String,
        type_name: String,
        expected_namespace: String,
    },
    /// A leaf-level issue (task) has no parent association labels
    OrphanedLeaf { issue_id: String, type_name: String },
}

/// Errors that can occur during configuration validation.
#[derive(Debug, Error, PartialEq)]
pub enum ConfigError {
    /// Duplicate type at a hierarchy level.
    ///
    /// Note: Currently unused but kept for future validation enhancements
    /// where we may want to detect duplicate types across levels.
    #[allow(dead_code)]
    #[error("Duplicate type '{0}' at level {1}")]
    DuplicateType(String, u8),

    #[error("Empty type name at level {0}")]
    EmptyTypeName(u8),

    #[error("Invalid level {0}: must be > 0")]
    InvalidLevel(u8),
}

/// Configuration for issue type hierarchy.
///
/// Defines the hierarchy levels and their associated membership label namespaces.
///
/// # Examples
///
/// ```
/// use jit::type_hierarchy::HierarchyConfig;
/// use std::collections::HashMap;
///
/// let config = HierarchyConfig::default();
/// assert!(config.contains_type("epic"));
/// assert_eq!(config.get_membership_namespace("epic"), Some("epic"));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct HierarchyConfig {
    /// Map of type name to its level (1 = highest, higher numbers = lower)
    types: HashMap<String, u8>,

    /// Map of type name to its membership label namespace
    /// e.g., "epic" -> "epic" means type:epic uses epic:* labels
    /// e.g., "release" -> "milestone" means type:release uses milestone:* labels
    label_associations: HashMap<String, String>,
}

impl Default for HierarchyConfig {
    /// Creates the default 4-level hierarchy:
    /// 1. milestone (strategic, highest) - uses milestone:* labels
    /// 2. epic (strategic, feature-level) - uses epic:* labels
    /// 3. story (tactical, user story) - uses story:* labels
    /// 4. task (tactical, implementation detail) - no membership labels
    fn default() -> Self {
        let mut types = HashMap::new();
        types.insert("milestone".to_string(), 1);
        types.insert("epic".to_string(), 2);
        types.insert("story".to_string(), 3);
        types.insert("task".to_string(), 4);

        let mut label_associations = HashMap::new();
        label_associations.insert("milestone".to_string(), "milestone".to_string());
        label_associations.insert("epic".to_string(), "epic".to_string());
        label_associations.insert("story".to_string(), "story".to_string());

        Self {
            types,
            label_associations,
        }
    }
}

impl HierarchyConfig {
    /// Creates a new hierarchy configuration from a map of type names to levels.
    ///
    /// # Arguments
    ///
    /// * `types` - Map of type names to hierarchy levels
    /// * `label_associations` - Map of type names to membership label namespaces
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if:
    /// - Any type name is empty
    /// - Any level is 0
    pub fn new(
        types: HashMap<String, u8>,
        label_associations: HashMap<String, String>,
    ) -> Result<Self, ConfigError> {
        for (name, level) in &types {
            if name.trim().is_empty() {
                return Err(ConfigError::EmptyTypeName(*level));
            }
            if *level == 0 {
                return Err(ConfigError::InvalidLevel(*level));
            }
        }

        Ok(Self {
            types,
            label_associations,
        })
    }

    /// Returns the level of a type, or None if the type is not in the hierarchy.
    pub fn get_level(&self, type_name: &str) -> Option<u8> {
        self.types.get(type_name).copied()
    }

    /// Returns true if the type is in the hierarchy.
    pub fn contains_type(&self, type_name: &str) -> bool {
        self.types.contains_key(type_name)
    }

    /// Returns an iterator over all types in the hierarchy.
    pub fn types(&self) -> impl Iterator<Item = (&String, &u8)> {
        self.types.iter()
    }

    /// Returns the membership label namespace for a type, if configured.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::type_hierarchy::HierarchyConfig;
    ///
    /// let config = HierarchyConfig::default();
    /// assert_eq!(config.get_membership_namespace("epic"), Some("epic"));
    /// assert_eq!(config.get_membership_namespace("task"), None);
    /// ```
    pub fn get_membership_namespace(&self, type_name: &str) -> Option<&str> {
        self.label_associations.get(type_name).map(String::as_str)
    }

    /// Returns an iterator over all (type_name, namespace) pairs for membership labels.
    ///
    /// This provides the reverse mapping: for each membership namespace, which types use it.
    pub fn membership_namespaces(&self) -> impl Iterator<Item = (&String, &String)> {
        self.label_associations.iter()
    }
}

/// Extracts the type value from a label with format "type:value".
///
/// Returns the normalized type name (lowercase, trimmed) or an error.
pub fn extract_type(label: &str) -> Result<String, HierarchyError> {
    let parts: Vec<&str> = label.split(':').collect();

    if parts.len() != 2 || parts[0] != "type" {
        return Err(HierarchyError::InvalidLabel(label.to_string()));
    }

    let type_name = parts[1].trim().to_lowercase();

    if type_name.is_empty() {
        return Err(HierarchyError::InvalidLabel(label.to_string()));
    }

    Ok(type_name)
}

/// Calculates the Levenshtein distance between two strings.
///
/// Used for finding the closest matching type when an unknown type is encountered.
#[allow(clippy::needless_range_loop)]
fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let chars1: Vec<char> = s1.chars().collect();
    let chars2: Vec<char> = s2.chars().collect();
    let len1 = chars1.len();
    let len2 = chars2.len();

    if len1 == 0 {
        return len2;
    }
    if len2 == 0 {
        return len1;
    }

    let mut matrix = vec![vec![0; len2 + 1]; len1 + 1];

    for i in 0..=len1 {
        matrix[i][0] = i;
    }
    for j in 0..=len2 {
        matrix[0][j] = j;
    }

    for i in 0..len1 {
        for j in 0..len2 {
            let cost = if chars1[i] == chars2[j] { 0 } else { 1 };
            matrix[i + 1][j + 1] = std::cmp::min(
                std::cmp::min(
                    matrix[i][j + 1] + 1, // deletion
                    matrix[i + 1][j] + 1, // insertion
                ),
                matrix[i][j] + cost, // substitution
            );
        }
    }

    matrix[len1][len2]
}

/// Finds the closest matching type for an unknown type name.
///
/// Returns the best match based on Levenshtein distance, or None if no reasonable match exists.
/// A match is considered reasonable if the distance is <= 3.
///
/// # Examples
///
/// ```
/// use jit::type_hierarchy::{suggest_type_fix, HierarchyConfig};
///
/// let config = HierarchyConfig::default();
///
/// assert_eq!(suggest_type_fix(&config, "taks"), Some("task".to_string()));
/// assert_eq!(suggest_type_fix(&config, "epik"), Some("epic".to_string()));
/// assert_eq!(suggest_type_fix(&config, "unknown_xyz_123"), None);
/// ```
pub fn suggest_type_fix(config: &HierarchyConfig, unknown_type: &str) -> Option<String> {
    let max_distance = 3;

    let mut best_match: Option<(String, usize)> = None;

    for (type_name, _) in config.types() {
        let distance = levenshtein_distance(unknown_type, type_name);

        if distance <= max_distance {
            match best_match {
                None => best_match = Some((type_name.clone(), distance)),
                Some((_, best_dist)) if distance < best_dist => {
                    best_match = Some((type_name.clone(), distance));
                }
                _ => {}
            }
        }
    }

    best_match.map(|(name, _)| name)
}

/// Detects all hierarchy validation issues for a given issue.
///
/// This function checks:
/// 1. Whether the issue has a valid type label
///
/// Note: Type hierarchy is orthogonal to the dependency DAG. Dependencies can flow in any
/// direction regardless of type. The hierarchy only validates type labels are known/valid.
///
/// # Arguments
///
/// * `config` - The hierarchy configuration
/// * `issue_id` - The ID of the issue to validate
/// * `labels` - The labels of the issue
///
/// # Returns
///
/// A vector of validation issues found (empty if no issues)
pub fn detect_validation_issues(
    config: &HierarchyConfig,
    issue_id: &str,
    labels: &[String],
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // Check if issue has a type label
    let type_label = labels.iter().find(|l| l.starts_with("type:"));

    if let Some(type_label) = type_label {
        match extract_type(type_label) {
            Ok(type_name) => {
                // Check if type is known
                if !config.contains_type(&type_name) {
                    let suggested_fix = suggest_type_fix(config, &type_name);
                    issues.push(ValidationIssue::UnknownType {
                        issue_id: issue_id.to_string(),
                        unknown_type: type_name.clone(),
                        suggested_fix,
                    });
                }
                // Note: We do NOT check dependencies here. Type hierarchy is orthogonal to DAG.
                // A task can depend on an epic, milestone can depend on task, etc.
                // The hierarchy only describes organizational grouping, not work flow.
            }
            Err(_) => {
                // Invalid type label format - skip for now
                // This should be caught by label validation
            }
        }
    }

    issues
}

/// Validates that strategic types have their identifying labels.
///
/// Strategic types (epic, milestone) should have matching labels for identification.
/// For example, an issue with `type:epic` should have an `epic:*` label.
///
/// # Arguments
///
/// * `config` - The hierarchy configuration
/// * `issue` - The issue to validate
///
/// # Returns
///
/// A vector of warnings if strategic labels are missing
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::type_hierarchy::{validate_strategic_labels, HierarchyConfig};
///
/// let config = HierarchyConfig::default();
/// let mut epic = Issue::new("Auth".to_string(), "Epic description".to_string());
/// epic.labels = vec!["type:epic".to_string()];
///
/// let warnings = validate_strategic_labels(&config, &epic);
/// assert_eq!(warnings.len(), 1); // Missing epic:* label
/// ```
pub fn validate_strategic_labels(
    config: &HierarchyConfig,
    issue: &crate::domain::Issue,
) -> Vec<ValidationWarning> {
    let mut warnings = Vec::new();

    // Find type label
    let type_label = issue.labels.iter().find(|l| l.starts_with("type:"));
    let type_name = match type_label {
        Some(label) => match extract_type(label) {
            Ok(t) => t,
            Err(_) => return warnings, // Invalid type label, handled by other validation
        },
        None => return warnings, // No type label, handled by other validation
    };

    // Check if this type has a membership namespace (strategic types do)
    if let Some(expected_namespace) = config.get_membership_namespace(&type_name) {
        // Check if issue has a label in that namespace
        let has_label = issue
            .labels
            .iter()
            .any(|l| l.starts_with(&format!("{}:", expected_namespace)));

        if !has_label {
            warnings.push(ValidationWarning::MissingStrategicLabel {
                issue_id: issue.id.clone(),
                type_name: type_name.clone(),
                expected_namespace: expected_namespace.to_string(),
            });
        }
    }

    warnings
}

/// Validates that leaf-level issues have parent association labels.
///
/// Leaf-level issues (like tasks) should belong to higher-level organizational units
/// (epics, milestones) for better tracking and strategic visibility.
///
/// # Arguments
///
/// * `config` - The hierarchy configuration
/// * `issue` - The issue to validate
///
/// # Returns
///
/// A vector of warnings if the issue is an orphaned leaf
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::type_hierarchy::{validate_orphans, HierarchyConfig};
///
/// let config = HierarchyConfig::default();
/// let mut task = Issue::new("Login".to_string(), "Task description".to_string());
/// task.labels = vec!["type:task".to_string()];
///
/// let warnings = validate_orphans(&config, &task);
/// assert_eq!(warnings.len(), 1); // Orphaned task
/// ```
pub fn validate_orphans(
    config: &HierarchyConfig,
    issue: &crate::domain::Issue,
) -> Vec<ValidationWarning> {
    let mut warnings = Vec::new();

    // Find type label
    let type_label = issue.labels.iter().find(|l| l.starts_with("type:"));
    let type_name = match type_label {
        Some(label) => match extract_type(label) {
            Ok(t) => t,
            Err(_) => return warnings, // Invalid type label, handled by other validation
        },
        None => return warnings, // No type label, handled by other validation
    };

    // Check if this is a leaf-level type
    let type_level = match config.get_level(&type_name) {
        Some(level) => level,
        None => return warnings, // Unknown type, handled by other validation
    };

    // Find the maximum level (lowest in hierarchy)
    let max_level = config.types.values().max().copied().unwrap_or(0);
    if type_level != max_level {
        return warnings; // Not a leaf type
    }

    // Check if it has any parent association labels
    let has_parent_label = config.membership_namespaces().any(|(namespace, _)| {
        issue
            .labels
            .iter()
            .any(|l| l.starts_with(&format!("{}:", namespace)))
    });

    if !has_parent_label {
        warnings.push(ValidationWarning::OrphanedLeaf {
            issue_id: issue.id.clone(),
            type_name: type_name.clone(),
        });
    }

    warnings
}

/// Detects membership validation issues for an issue.
///
/// Checks that membership labels (epic:*, milestone:*, etc.) reference actual issues
/// with matching types.
///
/// # Arguments
///
/// * `config` - The hierarchy configuration
/// * `issue` - The issue to validate
/// * `all_issues` - All issues in the repository (for reference lookup)
///
/// # Returns
///
/// A vector of validation issues found (empty if no issues)
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::type_hierarchy::{detect_membership_issues, HierarchyConfig};
///
/// let config = HierarchyConfig::default();
/// let mut task = Issue::new("Login".to_string(), "Task description".to_string());
/// task.labels = vec!["type:task".to_string(), "epic:auth".to_string()];
/// let mut epic = Issue::new("Auth".to_string(), "Epic description".to_string());
/// epic.labels = vec!["type:epic".to_string(), "epic:auth".to_string()];
/// let all_issues = vec![task.clone(), epic];
///
/// let issues = detect_membership_issues(&config, &task, &all_issues);
/// assert!(issues.is_empty()); // Valid reference
/// ```
#[allow(dead_code)] // Reserved for future membership validation feature
pub fn detect_membership_issues(
    config: &HierarchyConfig,
    issue: &crate::domain::Issue,
    all_issues: &[crate::domain::Issue],
) -> Vec<ValidationIssue> {
    use crate::labels::parse_label;

    let mut issues = Vec::new();

    // Build reverse map: namespace -> expected_type
    // e.g., "epic" -> "epic", "milestone" -> "milestone"
    let namespace_to_type: HashMap<&str, Vec<&str>> = {
        let mut map: HashMap<&str, Vec<&str>> = HashMap::new();
        for (type_name, namespace) in config.membership_namespaces() {
            map.entry(namespace.as_str())
                .or_default()
                .push(type_name.as_str());
        }
        map
    };

    for label in &issue.labels {
        // Parse the label
        let (namespace, value) = match parse_label(label) {
            Ok(pair) => pair,
            Err(_) => continue, // Invalid format, skip (handled by label validation)
        };

        // Check if this namespace is a configured membership namespace
        if let Some(expected_types) = namespace_to_type.get(namespace.as_str()) {
            // Find issues with matching label that have one of the expected types
            let matching_with_correct_type: Vec<_> = all_issues
                .iter()
                .filter(|i| {
                    i.labels.contains(&label.clone())
                        && i.labels
                            .iter()
                            .any(|l| expected_types.iter().any(|t| l == &format!("type:{}", t)))
                })
                .collect();

            if matching_with_correct_type.is_empty() {
                // Check if ANY issues have this label (for better error message)
                let any_matches = all_issues
                    .iter()
                    .any(|i| i.id != issue.id && i.labels.contains(&label.clone()));

                if !any_matches {
                    // No other issue found with this label at all
                    issues.push(ValidationIssue::InvalidMembershipReference {
                        issue_id: issue.id.clone(),
                        label: label.clone(),
                        namespace: namespace.clone(),
                        value: value.clone(),
                        reason: format!(
                            "No issue found with label '{}'. Expected an issue with type: {}",
                            label,
                            expected_types
                                .iter()
                                .map(|t| format!("type:{}", t))
                                .collect::<Vec<_>>()
                                .join(" or ")
                        ),
                    });
                } else {
                    // Found issues but none have the correct type
                    let found_types: Vec<String> = all_issues
                        .iter()
                        .filter(|i| i.id != issue.id && i.labels.contains(&label.clone()))
                        .filter_map(|i| i.labels.iter().find(|l| l.starts_with("type:")).cloned())
                        .collect();

                    issues.push(ValidationIssue::InvalidMembershipReference {
                        issue_id: issue.id.clone(),
                        label: label.clone(),
                        namespace: namespace.clone(),
                        value: value.clone(),
                        reason: format!(
                            "Issue(s) with label '{}' have type {:?}, expected type: {}",
                            label,
                            found_types,
                            expected_types
                                .iter()
                                .map(|t| format!("type:{}", t))
                                .collect::<Vec<_>>()
                                .join(" or ")
                        ),
                    });
                }
            }
        }
    }

    issues
}

/// Generates fixes for validation issues.
///
/// Currently only generates fixes for unknown type labels with suggested replacements.
///
/// # Arguments
///
/// * `issues` - The validation issues to fix
///
/// # Returns
///
/// A vector of fixes to apply (empty if no fixable issues)
pub fn generate_fixes(issues: &[ValidationIssue]) -> Vec<ValidationFix> {
    issues
        .iter()
        .filter_map(|issue| match issue {
            ValidationIssue::UnknownType {
                issue_id,
                unknown_type,
                suggested_fix: Some(suggested),
            } => Some(ValidationFix::ReplaceType {
                issue_id: issue_id.clone(),
                old_type: unknown_type.clone(),
                new_type: suggested.clone(),
            }),
            ValidationIssue::UnknownType { .. } => None, // No suggestion available
            ValidationIssue::InvalidMembershipReference { .. } => None, // No auto-fix for membership issues
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = HierarchyConfig::default();

        assert_eq!(config.get_level("milestone"), Some(1));
        assert_eq!(config.get_level("epic"), Some(2));
        assert_eq!(config.get_level("story"), Some(3));
        assert_eq!(config.get_level("task"), Some(4));
        assert_eq!(config.get_level("unknown"), None);
    }

    #[test]
    fn test_config_validation_empty_name() {
        let mut types = HashMap::new();
        types.insert("".to_string(), 1);
        let label_associations = HashMap::new();

        let result = HierarchyConfig::new(types, label_associations);
        assert_eq!(result, Err(ConfigError::EmptyTypeName(1)));
    }

    #[test]
    fn test_config_validation_invalid_level() {
        let mut types = HashMap::new();
        types.insert("task".to_string(), 0);
        let label_associations = HashMap::new();

        let result = HierarchyConfig::new(types, label_associations);
        assert_eq!(result, Err(ConfigError::InvalidLevel(0)));
    }

    #[test]
    fn test_extract_type_valid() {
        assert_eq!(extract_type("type:task"), Ok("task".to_string()));
        assert_eq!(extract_type("type:epic"), Ok("epic".to_string()));
        assert_eq!(extract_type("type:Task"), Ok("task".to_string())); // normalized
        assert_eq!(extract_type("type:  TASK  "), Ok("task".to_string())); // trimmed
    }

    #[test]
    fn test_extract_type_invalid_format() {
        assert!(matches!(
            extract_type("milestone:v1.0"),
            Err(HierarchyError::InvalidLabel(_))
        ));

        assert!(matches!(
            extract_type("type"),
            Err(HierarchyError::InvalidLabel(_))
        ));

        assert!(matches!(
            extract_type("type:"),
            Err(HierarchyError::InvalidLabel(_))
        ));
    }

    #[test]
    fn test_config_contains_type() {
        let config = HierarchyConfig::default();

        assert!(config.contains_type("task"));
        assert!(config.contains_type("epic"));
        assert!(!config.contains_type("unknown"));
    }

    #[test]
    fn test_config_types_iterator() {
        let config = HierarchyConfig::default();

        let types: HashMap<String, u8> = config.types().map(|(k, v)| (k.clone(), *v)).collect();

        assert_eq!(types.len(), 4);
        assert_eq!(types.get("milestone"), Some(&1));
        assert_eq!(types.get("epic"), Some(&2));
        assert_eq!(types.get("story"), Some(&3));
        assert_eq!(types.get("task"), Some(&4));
    }

    #[test]
    fn test_levenshtein_distance() {
        use super::levenshtein_distance;

        assert_eq!(levenshtein_distance("task", "task"), 0);
        assert_eq!(levenshtein_distance("task", "taks"), 2); // swap is 2 operations
        assert_eq!(levenshtein_distance("epic", "epik"), 1);
        assert_eq!(levenshtein_distance("story", "storry"), 1);
        assert_eq!(levenshtein_distance("milestone", "mileston"), 1);
        assert_eq!(levenshtein_distance("task", "epic"), 4);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
    }

    #[test]
    fn test_suggest_type_fix_close_match() {
        use super::suggest_type_fix;

        let config = HierarchyConfig::default();

        // Single character typos
        assert_eq!(suggest_type_fix(&config, "taks"), Some("task".to_string()));
        assert_eq!(suggest_type_fix(&config, "taak"), Some("task".to_string()));
        assert_eq!(suggest_type_fix(&config, "epik"), Some("epic".to_string()));
        assert_eq!(
            suggest_type_fix(&config, "storey"),
            Some("story".to_string())
        );
    }

    #[test]
    fn test_suggest_type_fix_no_match() {
        use super::suggest_type_fix;

        let config = HierarchyConfig::default();

        // Too different - no reasonable match
        assert_eq!(suggest_type_fix(&config, "unknown"), None);
        assert_eq!(suggest_type_fix(&config, "xyz"), None);
        assert_eq!(suggest_type_fix(&config, "completely_different"), None);
    }

    #[test]
    fn test_suggest_type_fix_exact_match() {
        use super::suggest_type_fix;

        let config = HierarchyConfig::default();

        // Even exact matches should be found
        assert_eq!(suggest_type_fix(&config, "task"), Some("task".to_string()));
        assert_eq!(suggest_type_fix(&config, "epic"), Some("epic".to_string()));
    }

    #[test]
    fn test_detect_unknown_type() {
        use super::{detect_validation_issues, ValidationIssue};

        let config = HierarchyConfig::default();
        let labels = vec!["type:taks".to_string()]; // typo

        let issues = detect_validation_issues(&config, "01ABC", &labels);

        assert_eq!(issues.len(), 1);
        match &issues[0] {
            ValidationIssue::UnknownType {
                issue_id,
                unknown_type,
                suggested_fix,
            } => {
                assert_eq!(issue_id, "01ABC");
                assert_eq!(unknown_type, "taks");
                assert_eq!(suggested_fix, &Some("task".to_string()));
            }
            _ => panic!("Expected UnknownType issue"),
        }
    }

    #[test]
    fn test_detect_no_issues_for_valid_type() {
        use super::detect_validation_issues;

        let config = HierarchyConfig::default();
        let labels = vec!["type:task".to_string()];

        let issues = detect_validation_issues(&config, "01ABC", &labels);
        assert!(issues.is_empty());
    }

    // Note: Removed test_detect_invalid_hierarchy_dep - dependencies are not restricted by type
    // Type hierarchy is orthogonal to DAG. Any issue can depend on any other issue.

    #[test]
    fn test_generate_fixes_for_unknown_type() {
        use super::{generate_fixes, ValidationFix, ValidationIssue};

        let issues = vec![ValidationIssue::UnknownType {
            issue_id: "01ABC".to_string(),
            unknown_type: "taks".to_string(),
            suggested_fix: Some("task".to_string()),
        }];

        let fixes = generate_fixes(&issues);

        assert_eq!(fixes.len(), 1);
        assert_eq!(
            fixes[0],
            ValidationFix::ReplaceType {
                issue_id: "01ABC".to_string(),
                old_type: "taks".to_string(),
                new_type: "task".to_string(),
            }
        );
    }

    // Note: Removed test_generate_fixes_for_invalid_hierarchy
    // We no longer generate fixes for dependencies - type hierarchy is orthogonal to DAG

    #[test]
    fn test_generate_fixes_no_suggestion() {
        use super::{generate_fixes, ValidationIssue};

        let issues = vec![ValidationIssue::UnknownType {
            issue_id: "01ABC".to_string(),
            unknown_type: "completely_unknown".to_string(),
            suggested_fix: None,
        }];

        let fixes = generate_fixes(&issues);
        assert!(fixes.is_empty()); // No fix generated without suggestion
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    // Strategy to generate valid type names
    fn type_name() -> impl Strategy<Value = String> {
        prop::string::string_regex("[a-z][a-z0-9]{0,15}").unwrap()
    }

    proptest! {

        /// Property: Extract type is stable and normalized
        #[test]
        fn prop_extract_type_normalization(name in type_name()) {
            let label = format!("type:{}", name);
            let extracted = extract_type(&label).unwrap();

            // Should be lowercase and trimmed
            let normalized = name.trim().to_lowercase();
            prop_assert_eq!(&extracted, &normalized);

            // Should be idempotent
            let label2 = format!("type:{}", extracted);
            let extracted2 = extract_type(&label2).unwrap();
            prop_assert_eq!(extracted2, normalized);
        }

        /// Property: Invalid labels always rejected
        #[test]
        fn prop_invalid_labels_rejected(
            namespace in prop::string::string_regex("[a-z]{1,10}").unwrap(),
            value in prop::string::string_regex("[a-z]{1,10}").unwrap()
        ) {
            prop_assume!(namespace != "type"); // Must not be "type"

            let label = format!("{}:{}", namespace, value);
            prop_assert!(extract_type(&label).is_err());
        }
    }
}
