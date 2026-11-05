//! The taxonomy of type labels and their levels, and validation against it.
//!
//! Pure domain code: the taxonomy comes from the `[type_hierarchy]` section of
//! `.jit/config.toml`, carried here as a [`HierarchyConfig`]. Resolving the work
//! graph's containment structure is a separate concern living in
//! [`crate::graph::hierarchy`].
//!
//! # CRITICAL: The Type Taxonomy is Orthogonal to the Dependency DAG
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
//! 2. **Type Taxonomy** (organizational labels - future validation):
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

use crate::domain::identity::IssueId;
use crate::domain::LabelNamespaces;
use crate::labels::type_value_of;
use std::collections::HashMap;
use thiserror::Error;

/// Represents a validation issue found in the repository.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationIssue {
    /// An issue has an unknown type label
    UnknownType {
        issue_id: IssueId,
        unknown_type: String,
        suggested_fix: Option<String>,
    },
}

/// Represents a fix to apply to resolve a validation issue.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationFix {
    /// Replace an unknown type with a suggested type
    ReplaceType {
        issue_id: IssueId,
        old_type: String,
        new_type: String,
    },
}

/// Validation warnings (soft constraints that don't block operations).
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationWarning {
    /// A strategic type issue (epic, milestone) is missing its identifying label
    MissingStrategicLabel {
        issue_id: IssueId,
        type_name: String,
        expected_namespace: String,
    },
    /// A leaf-level issue (task) has no parent association labels
    OrphanedLeaf {
        issue_id: IssueId,
        type_name: String,
    },
}

/// Errors that can occur during configuration validation.
#[derive(Debug, Error, PartialEq)]
pub enum ConfigError {
    #[error("Empty type name at level {0}")]
    EmptyTypeName(u8),

    #[error("Invalid level {0}: must be > 0")]
    InvalidLevel(u8),
}

/// Configuration for issue type hierarchy.
///
/// Defines the hierarchy levels and their associated membership label namespaces.
#[derive(Debug, Clone, PartialEq)]
pub struct HierarchyConfig {
    /// Map of type name to its level (1 = highest, higher numbers = lower)
    types: HashMap<String, u8>,

    /// Map of type name to its membership label namespace
    /// e.g., "epic" -> "epic" means type:epic uses epic:* labels
    /// e.g., "release" -> "milestone" means type:release uses milestone:* labels
    label_associations: HashMap<String, String>,
}

impl HierarchyConfig {
    /// A hierarchy with no types and no membership associations, held by a
    /// repository whose `config.toml` declares no `[type_hierarchy]`.
    ///
    /// Every query over it resolves nothing: no type is known, no level is
    /// deepest, and no type is strategic, so the validations keyed on the
    /// hierarchy report nothing.
    pub fn empty() -> Self {
        Self {
            types: HashMap::new(),
            label_associations: HashMap::new(),
        }
    }

    /// Builds a hierarchy from the repository's type and membership declarations.
    pub fn from_declarations(declarations: &LabelNamespaces) -> Result<Self, ConfigError> {
        Self::new(
            declarations.declared_type_hierarchy(),
            declarations.label_associations.clone().unwrap_or_default(),
        )
    }

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
    issue_id: &IssueId,
    labels: &[String],
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // Check if issue has a type label
    // An invalid type label format is caught by label validation.
    // Type hierarchy is orthogonal to the dependency DAG, so dependencies are not checked here.
    if let Some(type_name) = labels
        .iter()
        .find_map(|label| type_value_of(label))
        .filter(|type_name| !config.contains_type(type_name))
    {
        issues.push(ValidationIssue::UnknownType {
            issue_id: issue_id.clone(),
            suggested_fix: suggest_type_fix(config, type_name),
            unknown_type: type_name.to_string(),
        });
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
pub fn validate_strategic_labels(
    config: &HierarchyConfig,
    issue: &crate::domain::Issue,
) -> Vec<ValidationWarning> {
    let mut warnings = Vec::new();

    // Find type label
    let type_name = match issue.labels.iter().find_map(|label| type_value_of(label)) {
        Some(type_name) => type_name,
        None => return warnings, // No type label, handled by other validation
    };

    // Check if this type has a membership namespace (strategic types do)
    if let Some(expected_namespace) = config.get_membership_namespace(type_name) {
        // Check if issue has a label in that namespace
        let has_label = issue
            .labels
            .iter()
            .any(|l| l.starts_with(&format!("{}:", expected_namespace)));

        if !has_label {
            warnings.push(ValidationWarning::MissingStrategicLabel {
                issue_id: issue.id.clone(),
                type_name: type_name.to_string(),
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
pub fn validate_orphans(
    config: &HierarchyConfig,
    issue: &crate::domain::Issue,
) -> Vec<ValidationWarning> {
    let mut warnings = Vec::new();

    // Find type label
    let type_name = match issue.labels.iter().find_map(|label| type_value_of(label)) {
        Some(type_name) => type_name,
        None => return warnings, // No type label, handled by other validation
    };

    // Check if this is a leaf-level type
    let type_level = match config.get_level(type_name) {
        Some(level) => level,
        None => return warnings, // Unknown type, handled by other validation
    };

    // Find the maximum level (lowest in hierarchy)
    let max_level = config.types.values().max().copied().unwrap_or(0);
    if type_level != max_level {
        return warnings; // Not a leaf type
    }

    // Check if it has any parent association labels
    let has_parent_label = config.membership_namespaces().any(|(_, namespace)| {
        issue
            .labels
            .iter()
            .any(|l| l.starts_with(&format!("{}:", namespace)))
    });

    if !has_parent_label {
        warnings.push(ValidationWarning::OrphanedLeaf {
            issue_id: issue.id.clone(),
            type_name: type_name.to_string(),
        });
    }

    warnings
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
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_type(level: u8) -> String {
        crate::test_taxonomy::test_taxonomy()
            .type_at_level(level)
            .to_string()
    }

    #[test]
    fn test_default_config() {
        let config = crate::test_taxonomy::test_taxonomy().hierarchy_config();

        assert_eq!(config.get_level(&test_type(1)), Some(1));
        assert_eq!(config.get_level(&test_type(2)), Some(2));
        assert_eq!(config.get_level(&test_type(3)), Some(3));
        assert_eq!(config.get_level(&test_type(4)), Some(4));
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
        types.insert(test_type(4), 0);
        let label_associations = HashMap::new();

        let result = HierarchyConfig::new(types, label_associations);
        assert_eq!(result, Err(ConfigError::InvalidLevel(0)));
    }

    #[test]
    fn test_from_declarations_invalid_hierarchy_returns_config_error() {
        let declarations = LabelNamespaces {
            type_hierarchy: Some(HashMap::from([(test_type(4), 0)])),
            ..LabelNamespaces::empty(1)
        };

        assert_eq!(
            HierarchyConfig::from_declarations(&declarations),
            Err(ConfigError::InvalidLevel(0))
        );
    }

    #[test]
    fn test_config_contains_type() {
        assert!(!HierarchyConfig::empty().contains_type(&test_type(4)));
        let config = crate::test_taxonomy::test_taxonomy().hierarchy_config();

        assert!(config.contains_type(&test_type(4)));
        assert!(config.contains_type(&test_type(2)));
        assert!(!config.contains_type("unknown"));
    }

    #[test]
    fn test_config_types_iterator() {
        let config = crate::test_taxonomy::test_taxonomy().hierarchy_config();

        let types: HashMap<String, u8> = config.types().map(|(k, v)| (k.clone(), *v)).collect();

        assert_eq!(types.len(), 4);
        assert_eq!(types.get(&test_type(1)), Some(&1));
        assert_eq!(types.get(&test_type(2)), Some(&2));
        assert_eq!(types.get(&test_type(3)), Some(&3));
        assert_eq!(types.get(&test_type(4)), Some(&4));
    }

    #[test]
    fn test_levenshtein_distance() {
        use super::levenshtein_distance;

        let action = test_type(4);
        let initiative = test_type(2);
        let deliverable = test_type(3);
        let objective = test_type(1);
        assert_eq!(levenshtein_distance(&action, &action), 0);
        assert_eq!(levenshtein_distance(&action, "actoin"), 2); // swap is 2 operations
        assert_eq!(levenshtein_distance(&initiative, "initative"), 1);
        assert_eq!(levenshtein_distance(&deliverable, "delivrable"), 1);
        assert_eq!(levenshtein_distance(&objective, "objectiv"), 1);
        assert_eq!(levenshtein_distance(&action, &objective), 6);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
    }

    #[test]
    fn test_suggest_type_fix_close_match() {
        use super::suggest_type_fix;

        let config = crate::test_taxonomy::test_taxonomy().hierarchy_config();

        // Single character typos
        assert_eq!(suggest_type_fix(&config, "acton"), Some(test_type(4)));
        assert_eq!(suggest_type_fix(&config, "actoin"), Some(test_type(4)));
        assert_eq!(suggest_type_fix(&config, "initative"), Some(test_type(2)));
        assert_eq!(suggest_type_fix(&config, "delivrable"), Some(test_type(3)));
    }

    #[test]
    fn test_suggest_type_fix_no_match() {
        use super::suggest_type_fix;

        let config = crate::test_taxonomy::test_taxonomy().hierarchy_config();

        // Too different - no reasonable match
        assert_eq!(suggest_type_fix(&config, "unknown"), None);
        assert_eq!(suggest_type_fix(&config, "xyz"), None);
        assert_eq!(suggest_type_fix(&config, "completely_different"), None);
    }

    #[test]
    fn test_suggest_type_fix_exact_match() {
        use super::suggest_type_fix;

        let config = crate::test_taxonomy::test_taxonomy().hierarchy_config();

        // Even exact matches should be found
        assert_eq!(suggest_type_fix(&config, &test_type(4)), Some(test_type(4)));
        assert_eq!(suggest_type_fix(&config, &test_type(2)), Some(test_type(2)));
    }

    #[test]
    fn test_detect_unknown_type() {
        use super::{detect_validation_issues, ValidationIssue};

        let config = crate::test_taxonomy::test_taxonomy().hierarchy_config();
        let labels = vec!["type:acton".to_string()]; // typo

        let issues = detect_validation_issues(&config, &crate::test_utils::issue_id(68), &labels);

        assert_eq!(issues.len(), 1);
        match &issues[0] {
            ValidationIssue::UnknownType {
                issue_id,
                unknown_type,
                suggested_fix,
            } => {
                assert_eq!(issue_id, &crate::test_utils::issue_id(68));
                assert_eq!(unknown_type, "acton");
                assert_eq!(suggested_fix, &Some(test_type(4)));
            }
        }
    }

    #[test]
    fn test_detect_validation_issues_preserves_unknown_type_value() {
        let config =
            HierarchyConfig::new(HashMap::from([("epic".to_string(), 1)]), HashMap::new()).unwrap();
        let issue_id = crate::test_utils::issue_id(68);

        assert_eq!(
            detect_validation_issues(&config, &issue_id, &["type:Epic".to_string()]),
            vec![ValidationIssue::UnknownType {
                issue_id,
                unknown_type: "Epic".to_string(),
                suggested_fix: Some("epic".to_string()),
            }]
        );
    }

    #[test]
    fn test_detect_no_issues_for_valid_type() {
        use super::detect_validation_issues;

        let config = crate::test_taxonomy::test_taxonomy().hierarchy_config();
        let labels = vec![format!("type:{}", test_type(4))];

        let issues = detect_validation_issues(&config, &crate::test_utils::issue_id(68), &labels);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_validate_orphans_membership_namespace_differs_from_type_name() {
        use super::validate_orphans;

        let types = HashMap::from([("release".to_string(), 1), ("task".to_string(), 2)]);
        let associations = HashMap::from([("release".to_string(), "milestone".to_string())]);
        let config = HierarchyConfig::new(types, associations).unwrap();

        let orphaned = |labels: &[&str]| {
            let mut issue = crate::test_utils::fixture_issue("leaf".into(), String::new());
            issue.labels = std::iter::once("type:task")
                .chain(labels.iter().copied())
                .map(String::from)
                .collect();
            !validate_orphans(&config, &issue).is_empty()
        };

        assert!(
            !orphaned(&["milestone:v1"]),
            "membership namespace must count as a parent"
        );
        assert!(
            orphaned(&["release:v1"]),
            "type name must not count as a parent"
        );
    }

    // Note: Removed test_detect_invalid_hierarchy_dep - dependencies are not restricted by type
    // Type hierarchy is orthogonal to DAG. Any issue can depend on any other issue.

    #[test]
    fn test_generate_fixes_for_unknown_type() {
        use super::{generate_fixes, ValidationFix, ValidationIssue};

        let issues = vec![ValidationIssue::UnknownType {
            issue_id: crate::test_utils::issue_id(68),
            unknown_type: "acton".to_string(),
            suggested_fix: Some(test_type(4)),
        }];

        let fixes = generate_fixes(&issues);

        assert_eq!(fixes.len(), 1);
        assert_eq!(
            fixes[0],
            ValidationFix::ReplaceType {
                issue_id: crate::test_utils::issue_id(68),
                old_type: "acton".to_string(),
                new_type: test_type(4),
            }
        );
    }

    // Note: Removed test_generate_fixes_for_invalid_hierarchy
    // We no longer generate fixes for dependencies - type hierarchy is orthogonal to DAG

    #[test]
    fn test_generate_fixes_no_suggestion() {
        use super::{generate_fixes, ValidationIssue};

        let issues = vec![ValidationIssue::UnknownType {
            issue_id: crate::test_utils::issue_id(68),
            unknown_type: "completely_unknown".to_string(),
            suggested_fix: None,
        }];

        let fixes = generate_fixes(&issues);
        assert!(fixes.is_empty()); // No fix generated without suggestion
    }
}
