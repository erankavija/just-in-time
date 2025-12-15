//! Tests for warning validations with custom strategic types

use jit::domain::Issue;
use jit::type_hierarchy::{HierarchyConfig, ValidationWarning};
use std::collections::HashMap;

#[test]
fn test_custom_strategic_type_theme() {
    // Custom hierarchy with "theme" as strategic type
    let mut types = HashMap::new();
    types.insert("theme".to_string(), 1); // Strategic
    types.insert("feature".to_string(), 2);
    types.insert("task".to_string(), 3);

    let mut label_associations = HashMap::new();
    label_associations.insert("theme".to_string(), "theme".to_string());
    label_associations.insert("feature".to_string(), "feature".to_string());

    let config = HierarchyConfig::new(types, label_associations).unwrap();

    // Theme without theme:* label should warn
    let mut theme = Issue::new("UI Theme".to_string(), "Theme description".to_string());
    theme.labels = vec!["type:theme".to_string()];

    let warnings = jit::type_hierarchy::validate_strategic_labels(&config, &theme);

    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        ValidationWarning::MissingStrategicLabel {
            type_name,
            expected_namespace,
            ..
        } => {
            assert_eq!(type_name, "theme");
            assert_eq!(expected_namespace, "theme");
        }
        _ => panic!("Expected MissingStrategicLabel warning"),
    }
}

#[test]
fn test_custom_strategic_type_theme_with_label() {
    let mut types = HashMap::new();
    types.insert("theme".to_string(), 1);
    types.insert("task".to_string(), 2);

    let mut label_associations = HashMap::new();
    label_associations.insert("theme".to_string(), "theme".to_string());

    let config = HierarchyConfig::new(types, label_associations).unwrap();

    // Theme with theme:* label should not warn
    let mut theme = Issue::new("UI Theme".to_string(), "Theme description".to_string());
    theme.labels = vec!["type:theme".to_string(), "theme:design-system".to_string()];

    let warnings = jit::type_hierarchy::validate_strategic_labels(&config, &theme);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_type_alias_release_uses_milestone_namespace() {
    // "release" type uses "milestone" namespace (alias)
    let mut types = HashMap::new();
    types.insert("milestone".to_string(), 1);
    types.insert("release".to_string(), 1); // Same level
    types.insert("task".to_string(), 2);

    let mut label_associations = HashMap::new();
    label_associations.insert("milestone".to_string(), "milestone".to_string());
    label_associations.insert("release".to_string(), "milestone".to_string()); // Alias!

    let config = HierarchyConfig::new(types, label_associations).unwrap();

    // Release without milestone:* label should warn
    let mut release = Issue::new("v2.0".to_string(), "Release description".to_string());
    release.labels = vec!["type:release".to_string()];

    let warnings = jit::type_hierarchy::validate_strategic_labels(&config, &release);

    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        ValidationWarning::MissingStrategicLabel {
            type_name,
            expected_namespace,
            ..
        } => {
            assert_eq!(type_name, "release");
            assert_eq!(expected_namespace, "milestone"); // Uses milestone namespace!
        }
        _ => panic!("Expected MissingStrategicLabel warning"),
    }
}

#[test]
fn test_type_alias_release_with_milestone_label() {
    let mut types = HashMap::new();
    types.insert("milestone".to_string(), 1);
    types.insert("release".to_string(), 1);
    types.insert("task".to_string(), 2);

    let mut label_associations = HashMap::new();
    label_associations.insert("milestone".to_string(), "milestone".to_string());
    label_associations.insert("release".to_string(), "milestone".to_string());

    let config = HierarchyConfig::new(types, label_associations).unwrap();

    // Release with milestone:* label should not warn
    let mut release = Issue::new("v2.0".to_string(), "Release description".to_string());
    release.labels = vec!["type:release".to_string(), "milestone:v2.0".to_string()];

    let warnings = jit::type_hierarchy::validate_strategic_labels(&config, &release);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_orphan_detection_with_custom_hierarchy() {
    // Custom 3-level hierarchy
    let mut types = HashMap::new();
    types.insert("theme".to_string(), 1);
    types.insert("feature".to_string(), 2);
    types.insert("subtask".to_string(), 3); // Leaf level

    let mut label_associations = HashMap::new();
    label_associations.insert("theme".to_string(), "theme".to_string());
    label_associations.insert("feature".to_string(), "feature".to_string());

    let config = HierarchyConfig::new(types, label_associations).unwrap();

    // Subtask without parent labels should warn
    let mut subtask = Issue::new("Fix typo".to_string(), "Subtask description".to_string());
    subtask.labels = vec!["type:subtask".to_string()];

    let warnings = jit::type_hierarchy::validate_orphans(&config, &subtask);

    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        ValidationWarning::OrphanedLeaf { type_name, .. } => {
            assert_eq!(type_name, "subtask");
        }
        _ => panic!("Expected OrphanedLeaf warning"),
    }
}

#[test]
fn test_orphan_with_custom_parent_label() {
    let mut types = HashMap::new();
    types.insert("theme".to_string(), 1);
    types.insert("subtask".to_string(), 2);

    let mut label_associations = HashMap::new();
    label_associations.insert("theme".to_string(), "theme".to_string());

    let config = HierarchyConfig::new(types, label_associations).unwrap();

    // Subtask with theme:* label should not warn
    let mut subtask = Issue::new("Fix typo".to_string(), "Subtask description".to_string());
    subtask.labels = vec![
        "type:subtask".to_string(),
        "theme:design-system".to_string(),
    ];

    let warnings = jit::type_hierarchy::validate_orphans(&config, &subtask);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_non_strategic_type_no_warning() {
    // Only "epic" is strategic, "feature" is not
    let mut types = HashMap::new();
    types.insert("epic".to_string(), 1);
    types.insert("feature".to_string(), 2);
    types.insert("task".to_string(), 3);

    let mut label_associations = HashMap::new();
    label_associations.insert("epic".to_string(), "epic".to_string());
    // Note: "feature" is NOT in label_associations, so it's not strategic

    let config = HierarchyConfig::new(types, label_associations).unwrap();

    // Feature without feature:* label should NOT warn (not strategic)
    let mut feature = Issue::new("Login".to_string(), "Feature description".to_string());
    feature.labels = vec!["type:feature".to_string()];

    let warnings = jit::type_hierarchy::validate_strategic_labels(&config, &feature);

    assert_eq!(warnings.len(), 0);
}
