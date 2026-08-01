//! Integration tests for label-based membership validation
//!
//! These tests validate that organizational membership labels (initiative:*, objective:*)
//! reference actual issues with matching types.

use jit::commands::CommandExecutor;
use jit::domain::type_taxonomy::{detect_membership_issues, HierarchyConfig, ValidationIssue};
use jit::storage::json::JsonFileStorage;
use jit::storage::IssueStore;
use std::collections::HashMap;
use tempfile::TempDir;

fn setup_test_repo() -> (TempDir, CommandExecutor<JsonFileStorage>) {
    let temp = TempDir::new().unwrap();
    let storage = JsonFileStorage::new(temp.path().join(".jit"));
    let initial_layout =
        jit::storage::discover_repository_layout(temp.path(), storage.root()).unwrap();
    CommandExecutor::new(storage.clone())
        .with_layout(initial_layout)
        .initialize_fresh_repository(
            temp.path(),
            &jit::hierarchy_templates::HierarchyTemplate::default(),
            None,
        )
        .unwrap();
    let layout = jit::storage::discover_repository_layout(temp.path(), storage.root()).unwrap();
    let executor = CommandExecutor::new(storage).with_layout(layout);
    (temp, executor)
}

fn publish_issue(executor: &CommandExecutor<JsonFileStorage>, issue: &mut jit::domain::Issue) {
    let id = executor
        .create_issue(
            issue.title.clone(),
            issue.description.clone(),
            issue.priority,
            issue.gates_required.clone(),
            issue.labels.clone(),
            None,
            None,
            false,
        )
        .unwrap()
        .0;
    *issue = executor.storage().load_issue(&id).unwrap();
}

#[test]
fn test_valid_initiative_membership() {
    let (_temp, executor) = setup_test_repo();
    let config = jit::test_taxonomy::test_taxonomy().hierarchy_config();

    // Create an initiative
    let mut initiative = crate::fixture_issue("Authentication System".to_string(), String::new());
    initiative.labels = vec!["type:initiative".to_string(), "initiative:auth".to_string()];
    publish_issue(&executor, &mut initiative);

    // Create an action that references the initiative
    let mut action = crate::fixture_issue("Implement login".to_string(), String::new());
    action.labels = vec!["type:action".to_string(), "initiative:auth".to_string()];
    publish_issue(&executor, &mut action);

    // Load all issues for validation
    let all_issues = executor.storage().list_issues().unwrap();

    // Validate the action - should have no issues
    let issues = detect_membership_issues(&config, &action, &all_issues);
    assert!(
        issues.is_empty(),
        "Valid initiative reference should not produce validation issues"
    );
}

#[test]
fn test_invalid_initiative_reference_not_found() {
    let (_temp, executor) = setup_test_repo();
    let config = jit::test_taxonomy::test_taxonomy().hierarchy_config();

    // Create an action that references a non-existent initiative
    let mut action = crate::fixture_issue("Implement login".to_string(), String::new());
    action.labels = vec![
        "type:action".to_string(),
        "initiative:nonexistent".to_string(),
    ];
    publish_issue(&executor, &mut action);

    let all_issues = executor.storage().list_issues().unwrap();

    // Validate - should find the invalid reference
    let issues = detect_membership_issues(&config, &action, &all_issues);
    assert_eq!(
        issues.len(),
        1,
        "Should detect invalid initiative reference"
    );

    match &issues[0] {
        ValidationIssue::InvalidMembershipReference {
            issue_id,
            label,
            namespace,
            value,
            reason,
        } => {
            eprintln!("DEBUG: reason = '{}'", reason);
            assert_eq!(issue_id, &action.id);
            assert_eq!(label, "initiative:nonexistent");
            assert_eq!(namespace, "initiative");
            assert_eq!(value, "nonexistent");
            assert!(
                reason.contains("No issue found"),
                "Expected 'No issue found' in reason, got: '{}'",
                reason
            );
        }
        _ => panic!("Expected InvalidMembershipReference"),
    }
}

#[test]
fn test_invalid_initiative_reference_wrong_type() {
    let (_temp, executor) = setup_test_repo();
    let config = jit::test_taxonomy::test_taxonomy().hierarchy_config();

    // Create an issue with type:action but initiative:backend label
    let mut backend = crate::fixture_issue("Backend Service".to_string(), String::new());
    backend.labels = vec!["type:action".to_string(), "initiative:backend".to_string()];
    publish_issue(&executor, &mut backend);

    // Create an action that references it as an initiative (wrong!)
    let mut action = crate::fixture_issue("Add endpoint".to_string(), String::new());
    action.labels = vec!["type:action".to_string(), "initiative:backend".to_string()];
    publish_issue(&executor, &mut action);

    let all_issues = executor.storage().list_issues().unwrap();

    // Validate - should detect type mismatch
    let issues = detect_membership_issues(&config, &action, &all_issues);
    assert_eq!(issues.len(), 1, "Should detect type mismatch");

    match &issues[0] {
        ValidationIssue::InvalidMembershipReference { reason, .. } => {
            assert!(
                reason.contains("type:initiative") && reason.contains("type:action"),
                "Should explain type mismatch"
            );
        }
        _ => panic!("Expected InvalidMembershipReference"),
    }
}

#[test]
fn test_valid_objective_membership() {
    let (_temp, executor) = setup_test_repo();
    let config = jit::test_taxonomy::test_taxonomy().hierarchy_config();

    // Create objective
    let mut objective = crate::fixture_issue("v1.0 Release".to_string(), String::new());
    objective.labels = vec!["type:objective".to_string(), "objective:v1.0".to_string()];
    publish_issue(&executor, &mut objective);

    // Create action under objective
    let mut action = crate::fixture_issue("Fix critical bug".to_string(), String::new());
    action.labels = vec!["type:action".to_string(), "objective:v1.0".to_string()];
    publish_issue(&executor, &mut action);

    let all_issues = executor.storage().list_issues().unwrap();

    let issues = detect_membership_issues(&config, &action, &all_issues);
    assert!(issues.is_empty(), "Valid objective reference should be OK");
}

#[test]
fn test_multiple_membership_labels() {
    let (_temp, executor) = setup_test_repo();
    let config = jit::test_taxonomy::test_taxonomy().hierarchy_config();

    // Create objective and initiative
    let mut objective = crate::fixture_issue("v1.0".to_string(), String::new());
    objective.labels = vec!["type:objective".to_string(), "objective:v1.0".to_string()];
    publish_issue(&executor, &mut objective);

    let mut initiative = crate::fixture_issue("Auth".to_string(), String::new());
    initiative.labels = vec![
        "type:initiative".to_string(),
        "initiative:auth".to_string(),
        "objective:v1.0".to_string(), // Initiative belongs to objective
    ];
    publish_issue(&executor, &mut initiative);

    // Action belongs to both
    let mut action = crate::fixture_issue("Login".to_string(), String::new());
    action.labels = vec![
        "type:action".to_string(),
        "initiative:auth".to_string(),
        "objective:v1.0".to_string(),
    ];
    publish_issue(&executor, &mut action);

    let all_issues = executor.storage().list_issues().unwrap();

    let issues = detect_membership_issues(&config, &action, &all_issues);
    assert!(
        issues.is_empty(),
        "Valid multiple membership references should be OK"
    );
}

#[test]
fn test_no_membership_labels_is_ok() {
    let (_temp, executor) = setup_test_repo();
    let config = jit::test_taxonomy::test_taxonomy().hierarchy_config();

    // Action with no membership labels (orphan)
    let mut action = crate::fixture_issue("Standalone action".to_string(), String::new());
    action.labels = vec!["type:action".to_string()];
    publish_issue(&executor, &mut action);

    let all_issues = executor.storage().list_issues().unwrap();

    let issues = detect_membership_issues(&config, &action, &all_issues);
    assert!(
        issues.is_empty(),
        "No membership labels should not be an error"
    );
}

#[test]
fn test_initiative_referencing_itself() {
    let (_temp, executor) = setup_test_repo();
    let config = jit::test_taxonomy::test_taxonomy().hierarchy_config();

    // Initiative that references itself (valid but maybe weird)
    let mut initiative = crate::fixture_issue("Auth".to_string(), String::new());
    initiative.labels = vec!["type:initiative".to_string(), "initiative:auth".to_string()];
    publish_issue(&executor, &mut initiative);

    let all_issues = executor.storage().list_issues().unwrap();

    let issues = detect_membership_issues(&config, &initiative, &all_issues);
    // Self-reference should be OK (it's identifying itself)
    assert!(
        issues.is_empty(),
        "Initiative with matching label should be OK (self-identification)"
    );
}

#[test]
fn test_mixed_valid_and_invalid_references() {
    let (_temp, executor) = setup_test_repo();
    let config = jit::test_taxonomy::test_taxonomy().hierarchy_config();

    // Create one valid initiative
    let mut initiative = crate::fixture_issue("Auth".to_string(), String::new());
    initiative.labels = vec!["type:initiative".to_string(), "initiative:auth".to_string()];
    publish_issue(&executor, &mut initiative);

    // Action references one valid, one invalid
    let mut action = crate::fixture_issue("Login".to_string(), String::new());
    action.labels = vec![
        "type:action".to_string(),
        "initiative:auth".to_string(), // Valid
        "objective:v2.0".to_string(),  // Invalid - doesn't exist
    ];
    publish_issue(&executor, &mut action);

    let all_issues = executor.storage().list_issues().unwrap();

    let issues = detect_membership_issues(&config, &action, &all_issues);
    assert_eq!(issues.len(), 1, "Should detect only the invalid reference");

    match &issues[0] {
        ValidationIssue::InvalidMembershipReference { label, .. } => {
            assert_eq!(label, "objective:v2.0");
        }
        _ => panic!("Expected InvalidMembershipReference"),
    }
}

#[test]
fn test_custom_type_names_and_namespaces() {
    let (_temp, executor) = setup_test_repo();

    // Create a custom config: "theme" type uses "theme" namespace
    let mut types = HashMap::new();
    types.insert("theme".to_string(), 1);
    types.insert("feature".to_string(), 2);

    let mut label_associations = HashMap::new();
    label_associations.insert("theme".to_string(), "theme".to_string());
    label_associations.insert("feature".to_string(), "feature".to_string());

    let config = HierarchyConfig::new(types, label_associations).unwrap();

    // Create a theme
    let mut theme = crate::fixture_issue("Dark Mode".to_string(), String::new());
    theme.labels = vec!["type:theme".to_string(), "theme:ui".to_string()];
    publish_issue(&executor, &mut theme);

    // Create a feature that references the theme
    let mut feature = crate::fixture_issue("Dark sidebar".to_string(), String::new());
    feature.labels = vec!["type:feature".to_string(), "theme:ui".to_string()];
    publish_issue(&executor, &mut feature);

    let all_issues = executor.storage().list_issues().unwrap();

    // Validate the feature - should have no issues
    let issues = detect_membership_issues(&config, &feature, &all_issues);
    assert!(
        issues.is_empty(),
        "Valid theme reference should work with custom type names"
    );
}

#[test]
fn test_type_alias_same_namespace() {
    let (_temp, executor) = setup_test_repo();

    // Create config where both "objective" and "release" use "objective" namespace
    let mut types = HashMap::new();
    types.insert("objective".to_string(), 1);
    types.insert("release".to_string(), 1); // Same level
    types.insert("action".to_string(), 2);

    let mut label_associations = HashMap::new();
    label_associations.insert("objective".to_string(), "objective".to_string());
    label_associations.insert("release".to_string(), "objective".to_string()); // Alias!

    let config = HierarchyConfig::new(types, label_associations).unwrap();

    // Create a release (uses objective namespace)
    let mut release = crate::fixture_issue("v2.0".to_string(), String::new());
    release.labels = vec!["type:release".to_string(), "objective:v2.0".to_string()];
    publish_issue(&executor, &mut release);

    // Action references it via objective:v2.0 label
    let mut action = crate::fixture_issue("Prepare release notes".to_string(), String::new());
    action.labels = vec!["type:action".to_string(), "objective:v2.0".to_string()];
    publish_issue(&executor, &mut action);

    let all_issues = executor.storage().list_issues().unwrap();

    // Should validate successfully - release has type:release but objective namespace
    let issues = detect_membership_issues(&config, &action, &all_issues);
    assert!(
        issues.is_empty(),
        "Type alias (release -> objective namespace) should work"
    );
}
