//! Tests for type hierarchy warning-level validations

use jit::domain::type_taxonomy::{HierarchyConfig, ValidationWarning};

/// Helper to create a test hierarchy config
fn test_config() -> HierarchyConfig {
    jit::test_taxonomy::test_taxonomy().hierarchy_config()
}

#[test]
fn test_initiative_without_initiative_label_warns() {
    let config = test_config();
    let mut initiative = crate::fixture_issue(
        "Auth System".to_string(),
        "Initiative description".to_string(),
    );
    initiative.labels = vec!["type:initiative".to_string()];

    let warnings = jit::domain::type_taxonomy::validate_strategic_labels(&config, &initiative);

    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        ValidationWarning::MissingStrategicLabel {
            type_name,
            expected_namespace,
            ..
        } => {
            assert_eq!(type_name, "initiative");
            assert_eq!(expected_namespace, "initiative");
        }
        _ => panic!("Expected MissingStrategicLabel warning"),
    }
}

#[test]
fn test_objective_without_objective_label_warns() {
    let config = test_config();
    let mut objective = crate::fixture_issue(
        "v1.0 Release".to_string(),
        "Objective description".to_string(),
    );
    objective.labels = vec!["type:objective".to_string()];

    let warnings = jit::domain::type_taxonomy::validate_strategic_labels(&config, &objective);

    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        ValidationWarning::MissingStrategicLabel {
            type_name,
            expected_namespace,
            ..
        } => {
            assert_eq!(type_name, "objective");
            assert_eq!(expected_namespace, "objective");
        }
        _ => panic!("Expected MissingStrategicLabel warning"),
    }
}

#[test]
fn test_action_non_strategic_no_warning() {
    let config = test_config();
    let mut action =
        crate::fixture_issue("Login API".to_string(), "Action description".to_string());
    action.labels = vec!["type:action".to_string()];

    let warnings = jit::domain::type_taxonomy::validate_strategic_labels(&config, &action);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_initiative_with_initiative_label_no_warning() {
    let config = test_config();
    let mut initiative = crate::fixture_issue(
        "Auth System".to_string(),
        "Initiative description".to_string(),
    );
    initiative.labels = vec!["type:initiative".to_string(), "initiative:auth".to_string()];

    let warnings = jit::domain::type_taxonomy::validate_strategic_labels(&config, &initiative);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_objective_with_objective_label_no_warning() {
    let config = test_config();
    let mut objective = crate::fixture_issue(
        "v1.0 Release".to_string(),
        "Objective description".to_string(),
    );
    objective.labels = vec!["type:objective".to_string(), "objective:v1.0".to_string()];

    let warnings = jit::domain::type_taxonomy::validate_strategic_labels(&config, &objective);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_action_without_parent_labels_warns() {
    let config = test_config();
    let mut action =
        crate::fixture_issue("Login API".to_string(), "Action description".to_string());
    action.labels = vec!["type:action".to_string()];

    let warnings = jit::domain::type_taxonomy::validate_orphans(&config, &action);

    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        ValidationWarning::OrphanedLeaf { type_name, .. } => {
            assert_eq!(type_name, "action");
        }
        _ => panic!("Expected OrphanedLeaf warning"),
    }
}

#[test]
fn test_action_with_initiative_label_no_warning() {
    let config = test_config();
    let mut action =
        crate::fixture_issue("Login API".to_string(), "Action description".to_string());
    action.labels = vec!["type:action".to_string(), "initiative:auth".to_string()];

    let warnings = jit::domain::type_taxonomy::validate_orphans(&config, &action);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_action_with_objective_label_no_warning() {
    let config = test_config();
    let mut action =
        crate::fixture_issue("Login API".to_string(), "Action description".to_string());
    action.labels = vec!["type:action".to_string(), "objective:v1.0".to_string()];

    let warnings = jit::domain::type_taxonomy::validate_orphans(&config, &action);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_initiative_non_leaf_no_warning() {
    let config = test_config();
    let mut initiative = crate::fixture_issue(
        "Auth System".to_string(),
        "Initiative description".to_string(),
    );
    initiative.labels = vec!["type:initiative".to_string()];

    let warnings = jit::domain::type_taxonomy::validate_orphans(&config, &initiative);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_action_with_multiple_parent_labels_no_warning() {
    let config = test_config();
    let mut action =
        crate::fixture_issue("Login API".to_string(), "Action description".to_string());
    action.labels = vec![
        "type:action".to_string(),
        "initiative:auth".to_string(),
        "objective:v1.0".to_string(),
    ];

    let warnings = jit::domain::type_taxonomy::validate_orphans(&config, &action);

    assert_eq!(warnings.len(), 0);
}
