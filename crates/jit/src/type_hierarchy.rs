//! Type hierarchy validation for issue labels.
//!
//! This module provides validation for issue type hierarchies, ensuring that
//! dependencies flow from lower-level types (e.g., "task") to higher-level types
//! (e.g., "epic" or "milestone"). This is orthogonal to DAG validation:
//! - DAG validation prevents logical dependency cycles
//! - Hierarchy validation enforces organizational structure
//!
//! # Examples
//!
//! ```
//! use jit::type_hierarchy::{HierarchyConfig, validate_hierarchy};
//!
//! let config = HierarchyConfig::default();
//!
//! // Valid: task depends on task (same level)
//! assert!(validate_hierarchy(&config, "type:task", "type:task").is_ok());
//!
//! // Valid: task depends on story (higher level)
//! assert!(validate_hierarchy(&config, "type:task", "type:story").is_ok());
//!
//! // Invalid: epic depends on task (lower level)
//! assert!(validate_hierarchy(&config, "type:epic", "type:task").is_err());
//! ```

use std::collections::HashMap;
use thiserror::Error;

/// Errors that can occur during hierarchy validation.
#[derive(Debug, Error, PartialEq)]
pub enum HierarchyError {
    #[error("Type '{0}' depends on lower-level type '{1}' (level {2} -> {3})")]
    InvalidHierarchy(String, String, u8, u8),

    #[error("Unknown type: '{0}'")]
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
    /// A dependency violates the type hierarchy
    InvalidHierarchyDep {
        from_issue_id: String,
        from_type: String,
        to_issue_id: String,
        to_type: String,
        from_level: u8,
        to_level: u8,
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
    /// Reverse a dependency that violates the hierarchy
    ReverseDependency {
        from_issue_id: String,
        to_issue_id: String,
    },
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
/// Types at lower levels can depend on types at the same or higher levels.
/// For example, with default config:
/// - Level 1 (milestone) can depend on: milestone
/// - Level 2 (epic) can depend on: epic, milestone
/// - Level 3 (story) can depend on: story, epic, milestone
/// - Level 4 (task) can depend on: task, story, epic, milestone
#[derive(Debug, Clone, PartialEq)]
pub struct HierarchyConfig {
    /// Map of type name to its level (1 = highest, higher numbers = lower)
    types: HashMap<String, u8>,
}

impl Default for HierarchyConfig {
    /// Creates the default 4-level hierarchy:
    /// 1. milestone (strategic, highest)
    /// 2. epic (strategic, feature-level)
    /// 3. story (tactical, user story)
    /// 4. task (tactical, implementation detail)
    fn default() -> Self {
        let mut types = HashMap::new();
        types.insert("milestone".to_string(), 1);
        types.insert("epic".to_string(), 2);
        types.insert("story".to_string(), 3);
        types.insert("task".to_string(), 4);

        Self { types }
    }
}

impl HierarchyConfig {
    /// Creates a new hierarchy configuration from a map of type names to levels.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if:
    /// - Any type name is empty
    /// - Any level is 0
    /// - Multiple types have the same level (duplicate entries are allowed)
    pub fn new(types: HashMap<String, u8>) -> Result<Self, ConfigError> {
        for (name, level) in &types {
            if name.trim().is_empty() {
                return Err(ConfigError::EmptyTypeName(*level));
            }
            if *level == 0 {
                return Err(ConfigError::InvalidLevel(*level));
            }
        }

        Ok(Self { types })
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

/// Validates that a dependency respects the type hierarchy.
///
/// # Arguments
///
/// * `config` - The hierarchy configuration
/// * `from_label` - The label of the depending issue (format: "type:value")
/// * `to_label` - The label of the dependency (format: "type:value")
///
/// # Returns
///
/// - `Ok(())` if the dependency is valid (same level or higher)
/// - `Err(HierarchyError)` if the dependency violates the hierarchy
///
/// # Examples
///
/// ```
/// use jit::type_hierarchy::{HierarchyConfig, validate_hierarchy};
///
/// let config = HierarchyConfig::default();
///
/// // Valid: same level
/// assert!(validate_hierarchy(&config, "type:task", "type:task").is_ok());
///
/// // Valid: lower depends on higher
/// assert!(validate_hierarchy(&config, "type:task", "type:epic").is_ok());
///
/// // Invalid: higher depends on lower
/// assert!(validate_hierarchy(&config, "type:epic", "type:task").is_err());
/// ```
pub fn validate_hierarchy(
    config: &HierarchyConfig,
    from_label: &str,
    to_label: &str,
) -> Result<(), HierarchyError> {
    let from_type = extract_type(from_label)?;
    let to_type = extract_type(to_label)?;

    let from_level = config
        .get_level(&from_type)
        .ok_or_else(|| HierarchyError::UnknownType(from_type.clone()))?;

    let to_level = config
        .get_level(&to_type)
        .ok_or_else(|| HierarchyError::UnknownType(to_type.clone()))?;

    // Lower levels (higher numbers) can depend on same or higher levels (lower numbers)
    // e.g., task (4) can depend on story (3), epic (2), milestone (1)
    if from_level > to_level {
        // This is valid: lower level depending on higher level
        Ok(())
    } else if from_level == to_level {
        // Same level is also valid
        Ok(())
    } else {
        // from_level < to_level: higher level depending on lower level - invalid!
        Err(HierarchyError::InvalidHierarchy(
            from_type, to_type, from_level, to_level,
        ))
    }
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
/// use jit::type_hierarchy::{HierarchyConfig, suggest_type_fix};
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

/// Detects all hierarchy validation issues for a given issue and its dependencies.
///
/// This function checks:
/// 1. Whether the issue has a valid type label
/// 2. Whether any dependencies violate the type hierarchy
///
/// # Arguments
///
/// * `config` - The hierarchy configuration
/// * `issue_id` - The ID of the issue to validate
/// * `labels` - The labels of the issue
/// * `dependencies` - Map of dependency ID to dependency labels
///
/// # Returns
///
/// A vector of validation issues found (empty if no issues)
pub fn detect_validation_issues(
    config: &HierarchyConfig,
    issue_id: &str,
    labels: &[String],
    dependencies: &[(String, Vec<String>)],
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
                } else {
                    // Check hierarchy for each dependency
                    let from_level = config.get_level(&type_name).unwrap();

                    for (dep_id, dep_labels) in dependencies {
                        if let Some(dep_type_label) =
                            dep_labels.iter().find(|l| l.starts_with("type:"))
                        {
                            if let Ok(dep_type) = extract_type(dep_type_label) {
                                if let Some(to_level) = config.get_level(&dep_type) {
                                    // Check if this violates hierarchy (from_level < to_level)
                                    if from_level < to_level {
                                        issues.push(ValidationIssue::InvalidHierarchyDep {
                                            from_issue_id: issue_id.to_string(),
                                            from_type: type_name.clone(),
                                            to_issue_id: dep_id.clone(),
                                            to_type: dep_type,
                                            from_level,
                                            to_level,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(_) => {
                // Invalid type label format - skip for now
                // This should be caught by label validation
            }
        }
    }

    issues
}

/// Generates fixes for validation issues.
///
/// # Arguments
///
/// * `issues` - The validation issues to fix
///
/// # Returns
///
/// A vector of fixes to apply
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
            ValidationIssue::InvalidHierarchyDep {
                from_issue_id,
                to_issue_id,
                ..
            } => Some(ValidationFix::ReverseDependency {
                from_issue_id: from_issue_id.clone(),
                to_issue_id: to_issue_id.clone(),
            }),
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

        let result = HierarchyConfig::new(types);
        assert_eq!(result, Err(ConfigError::EmptyTypeName(1)));
    }

    #[test]
    fn test_config_validation_invalid_level() {
        let mut types = HashMap::new();
        types.insert("task".to_string(), 0);

        let result = HierarchyConfig::new(types);
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
    fn test_validate_same_level() {
        let config = HierarchyConfig::default();

        assert!(validate_hierarchy(&config, "type:task", "type:task").is_ok());
        assert!(validate_hierarchy(&config, "type:story", "type:story").is_ok());
        assert!(validate_hierarchy(&config, "type:epic", "type:epic").is_ok());
        assert!(validate_hierarchy(&config, "type:milestone", "type:milestone").is_ok());
    }

    #[test]
    fn test_validate_lower_depends_on_higher() {
        let config = HierarchyConfig::default();

        // Task (4) depends on story (3)
        assert!(validate_hierarchy(&config, "type:task", "type:story").is_ok());

        // Task (4) depends on epic (2)
        assert!(validate_hierarchy(&config, "type:task", "type:epic").is_ok());

        // Task (4) depends on milestone (1)
        assert!(validate_hierarchy(&config, "type:task", "type:milestone").is_ok());

        // Story (3) depends on epic (2)
        assert!(validate_hierarchy(&config, "type:story", "type:epic").is_ok());

        // Story (3) depends on milestone (1)
        assert!(validate_hierarchy(&config, "type:story", "type:milestone").is_ok());

        // Epic (2) depends on milestone (1)
        assert!(validate_hierarchy(&config, "type:epic", "type:milestone").is_ok());
    }

    #[test]
    fn test_validate_higher_depends_on_lower_fails() {
        let config = HierarchyConfig::default();

        // Epic (2) depends on task (4) - INVALID
        let result = validate_hierarchy(&config, "type:epic", "type:task");
        assert!(matches!(
            result,
            Err(HierarchyError::InvalidHierarchy(_, _, 2, 4))
        ));

        // Milestone (1) depends on task (4) - INVALID
        let result = validate_hierarchy(&config, "type:milestone", "type:task");
        assert!(matches!(
            result,
            Err(HierarchyError::InvalidHierarchy(_, _, 1, 4))
        ));

        // Epic (2) depends on story (3) - INVALID
        let result = validate_hierarchy(&config, "type:epic", "type:story");
        assert!(matches!(
            result,
            Err(HierarchyError::InvalidHierarchy(_, _, 2, 3))
        ));
    }

    #[test]
    fn test_validate_unknown_type() {
        let config = HierarchyConfig::default();

        let result = validate_hierarchy(&config, "type:unknown", "type:task");
        assert!(matches!(result, Err(HierarchyError::UnknownType(_))));

        let result = validate_hierarchy(&config, "type:task", "type:unknown");
        assert!(matches!(result, Err(HierarchyError::UnknownType(_))));
    }

    #[test]
    fn test_custom_config() {
        let mut types = HashMap::new();
        types.insert("level1".to_string(), 1);
        types.insert("level2".to_string(), 2);
        types.insert("level3".to_string(), 3);

        let config = HierarchyConfig::new(types).unwrap();

        // Valid: level3 depends on level1
        assert!(validate_hierarchy(&config, "type:level3", "type:level1").is_ok());

        // Invalid: level1 depends on level3
        assert!(validate_hierarchy(&config, "type:level1", "type:level3").is_err());
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
        let deps = vec![];

        let issues = detect_validation_issues(&config, "01ABC", &labels, &deps);

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
    fn test_detect_invalid_hierarchy_dep() {
        use super::{detect_validation_issues, ValidationIssue};

        let config = HierarchyConfig::default();
        let labels = vec!["type:epic".to_string()];
        let deps = vec![("02DEF".to_string(), vec!["type:task".to_string()])];

        let issues = detect_validation_issues(&config, "01ABC", &labels, &deps);

        assert_eq!(issues.len(), 1);
        match &issues[0] {
            ValidationIssue::InvalidHierarchyDep {
                from_issue_id,
                from_type,
                to_issue_id,
                to_type,
                from_level,
                to_level,
            } => {
                assert_eq!(from_issue_id, "01ABC");
                assert_eq!(from_type, "epic");
                assert_eq!(to_issue_id, "02DEF");
                assert_eq!(to_type, "task");
                assert_eq!(*from_level, 2);
                assert_eq!(*to_level, 4);
            }
            _ => panic!("Expected InvalidHierarchyDep issue"),
        }
    }

    #[test]
    fn test_detect_no_issues_for_valid_hierarchy() {
        use super::detect_validation_issues;

        let config = HierarchyConfig::default();
        let labels = vec!["type:task".to_string()];
        let deps = vec![("02DEF".to_string(), vec!["type:epic".to_string()])];

        let issues = detect_validation_issues(&config, "01ABC", &labels, &deps);
        assert!(issues.is_empty());
    }

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

    #[test]
    fn test_generate_fixes_for_invalid_hierarchy() {
        use super::{generate_fixes, ValidationFix, ValidationIssue};

        let issues = vec![ValidationIssue::InvalidHierarchyDep {
            from_issue_id: "01ABC".to_string(),
            from_type: "epic".to_string(),
            to_issue_id: "02DEF".to_string(),
            to_type: "task".to_string(),
            from_level: 2,
            to_level: 4,
        }];

        let fixes = generate_fixes(&issues);

        assert_eq!(fixes.len(), 1);
        assert_eq!(
            fixes[0],
            ValidationFix::ReverseDependency {
                from_issue_id: "01ABC".to_string(),
                to_issue_id: "02DEF".to_string(),
            }
        );
    }

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
        /// Property: Validation is transitive
        /// If A can depend on B and B can depend on C, then A can depend on C
        #[test]
        fn prop_transitivity(level_a in 1u8..10, level_b in 1u8..10, level_c in 1u8..10) {
            let mut types = HashMap::new();
            types.insert("a".to_string(), level_a);
            types.insert("b".to_string(), level_b);
            types.insert("c".to_string(), level_c);

            let config = HierarchyConfig::new(types).unwrap();

            let a_to_b = validate_hierarchy(&config, "type:a", "type:b");
            let b_to_c = validate_hierarchy(&config, "type:b", "type:c");

            // If both A->B and B->C are valid, then A->C must be valid
            if a_to_b.is_ok() && b_to_c.is_ok() {
                let a_to_c = validate_hierarchy(&config, "type:a", "type:c");
                prop_assert!(a_to_c.is_ok());
            }
        }

        /// Property: No cycles possible with strict hierarchy
        /// If A depends on B, then B cannot depend on A (unless same level)
        #[test]
        fn prop_no_cycles_different_levels(level_a in 1u8..10, level_b in 1u8..10) {
            prop_assume!(level_a != level_b); // Only test different levels

            let mut types = HashMap::new();
            types.insert("a".to_string(), level_a);
            types.insert("b".to_string(), level_b);

            let config = HierarchyConfig::new(types).unwrap();

            let a_to_b = validate_hierarchy(&config, "type:a", "type:b");
            let b_to_a = validate_hierarchy(&config, "type:b", "type:a");

            // Exactly one direction should be valid (or neither if same level)
            prop_assert!(a_to_b.is_ok() != b_to_a.is_ok());
        }

        /// Property: Same level always valid
        #[test]
        fn prop_same_level_always_valid(level in 1u8..10) {
            let mut types = HashMap::new();
            types.insert("a".to_string(), level);
            types.insert("b".to_string(), level);

            let config = HierarchyConfig::new(types).unwrap();

            prop_assert!(validate_hierarchy(&config, "type:a", "type:b").is_ok());
            prop_assert!(validate_hierarchy(&config, "type:b", "type:a").is_ok());
        }

        /// Property: Level ordering is monotonic
        /// If A can depend on B, and B has higher level than C, then A can depend on C
        #[test]
        fn prop_monotonic_ordering(level_a in 1u8..10, level_b in 1u8..10, level_c in 1u8..10) {
            prop_assume!(level_a >= level_b && level_b >= level_c); // A lower or same, B in middle, C highest

            let mut types = HashMap::new();
            types.insert("a".to_string(), level_a);
            types.insert("b".to_string(), level_b);
            types.insert("c".to_string(), level_c);

            let config = HierarchyConfig::new(types).unwrap();

            // If A can depend on B, and B >= C in level, then A can depend on C
            if validate_hierarchy(&config, "type:a", "type:b").is_ok() {
                prop_assert!(validate_hierarchy(&config, "type:a", "type:c").is_ok());
            }
        }

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
