//! Integration tests for label-based membership validation
//!
//! These tests validate that organizational membership labels (epic:*, milestone:*)
//! reference actual issues with matching types.

use jit::commands::CommandExecutor;
use jit::domain::Issue;
use jit::storage::json::JsonFileStorage;
use jit::storage::IssueStore;
use jit::type_hierarchy::{detect_membership_issues, HierarchyConfig, ValidationIssue};
use std::collections::HashMap;
use tempfile::TempDir;

fn setup_test_repo() -> (TempDir, CommandExecutor<JsonFileStorage>) {
    let temp = TempDir::new().unwrap();
    let storage = JsonFileStorage::new(temp.path().join(".jit"));
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap(); // Initialize the repository
    (temp, executor)
}

#[test]
fn test_valid_epic_membership() {
    let (_temp, executor) = setup_test_repo();
    let config = HierarchyConfig::default();

    // Create an epic
    let mut epic = Issue::new("Authentication System".to_string(), String::new());
    epic.labels = vec!["type:epic".to_string(), "epic:auth".to_string()];
    executor.storage().save_issue(epic.clone()).unwrap();

    // Create a task that references the epic
    let mut task = Issue::new("Implement login".to_string(), String::new());
    task.labels = vec!["type:task".to_string(), "epic:auth".to_string()];
    executor.storage().save_issue(task.clone()).unwrap();

    // Load all issues for validation
    let all_issues = executor.storage().list_issues().unwrap();

    // Validate the task - should have no issues
    let issues = detect_membership_issues(&config, &task, &all_issues);
    assert!(
        issues.is_empty(),
        "Valid epic reference should not produce validation issues"
    );
}

#[test]
fn test_invalid_epic_reference_not_found() {
    let (_temp, executor) = setup_test_repo();
    let config = HierarchyConfig::default();

    // Create a task that references a non-existent epic
    let mut task = Issue::new("Implement login".to_string(), String::new());
    task.labels = vec!["type:task".to_string(), "epic:nonexistent".to_string()];
    executor.storage().save_issue(task.clone()).unwrap();

    let all_issues = executor.storage().list_issues().unwrap();

    // Validate - should find the invalid reference
    let issues = detect_membership_issues(&config, &task, &all_issues);
    assert_eq!(issues.len(), 1, "Should detect invalid epic reference");

    match &issues[0] {
        ValidationIssue::InvalidMembershipReference {
            issue_id,
            label,
            namespace,
            value,
            reason,
        } => {
            eprintln!("DEBUG: reason = '{}'", reason);
            assert_eq!(issue_id, &task.id);
            assert_eq!(label, "epic:nonexistent");
            assert_eq!(namespace, "epic");
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
fn test_invalid_epic_reference_wrong_type() {
    let (_temp, executor) = setup_test_repo();
    let config = HierarchyConfig::default();

    // Create an issue with type:task but epic:backend label
    let mut backend = Issue::new("Backend Service".to_string(), String::new());
    backend.labels = vec!["type:task".to_string(), "epic:backend".to_string()];
    executor.storage().save_issue(backend.clone()).unwrap();

    // Create a task that references it as an epic (wrong!)
    let mut task = Issue::new("Add endpoint".to_string(), String::new());
    task.labels = vec!["type:task".to_string(), "epic:backend".to_string()];
    executor.storage().save_issue(task.clone()).unwrap();

    let all_issues = executor.storage().list_issues().unwrap();

    // Validate - should detect type mismatch
    let issues = detect_membership_issues(&config, &task, &all_issues);
    assert_eq!(issues.len(), 1, "Should detect type mismatch");

    match &issues[0] {
        ValidationIssue::InvalidMembershipReference { reason, .. } => {
            assert!(
                reason.contains("type:epic") && reason.contains("type:task"),
                "Should explain type mismatch"
            );
        }
        _ => panic!("Expected InvalidMembershipReference"),
    }
}

#[test]
fn test_valid_milestone_membership() {
    let (_temp, executor) = setup_test_repo();
    let config = HierarchyConfig::default();

    // Create milestone
    let mut milestone = Issue::new("v1.0 Release".to_string(), String::new());
    milestone.labels = vec!["type:milestone".to_string(), "milestone:v1.0".to_string()];
    executor.storage().save_issue(milestone.clone()).unwrap();

    // Create task under milestone
    let mut task = Issue::new("Fix critical bug".to_string(), String::new());
    task.labels = vec!["type:task".to_string(), "milestone:v1.0".to_string()];
    executor.storage().save_issue(task.clone()).unwrap();

    let all_issues = executor.storage().list_issues().unwrap();

    let issues = detect_membership_issues(&config, &task, &all_issues);
    assert!(issues.is_empty(), "Valid milestone reference should be OK");
}

#[test]
fn test_multiple_membership_labels() {
    let (_temp, executor) = setup_test_repo();
    let config = HierarchyConfig::default();

    // Create milestone and epic
    let mut milestone = Issue::new("v1.0".to_string(), String::new());
    milestone.labels = vec!["type:milestone".to_string(), "milestone:v1.0".to_string()];
    executor.storage().save_issue(milestone.clone()).unwrap();

    let mut epic = Issue::new("Auth".to_string(), String::new());
    epic.labels = vec![
        "type:epic".to_string(),
        "epic:auth".to_string(),
        "milestone:v1.0".to_string(), // Epic belongs to milestone
    ];
    executor.storage().save_issue(epic.clone()).unwrap();

    // Task belongs to both
    let mut task = Issue::new("Login".to_string(), String::new());
    task.labels = vec![
        "type:task".to_string(),
        "epic:auth".to_string(),
        "milestone:v1.0".to_string(),
    ];
    executor.storage().save_issue(task.clone()).unwrap();

    let all_issues = executor.storage().list_issues().unwrap();

    let issues = detect_membership_issues(&config, &task, &all_issues);
    assert!(
        issues.is_empty(),
        "Valid multiple membership references should be OK"
    );
}

#[test]
fn test_no_membership_labels_is_ok() {
    let (_temp, executor) = setup_test_repo();
    let config = HierarchyConfig::default();

    // Task with no membership labels (orphan)
    let mut task = Issue::new("Standalone task".to_string(), String::new());
    task.labels = vec!["type:task".to_string()];
    executor.storage().save_issue(task.clone()).unwrap();

    let all_issues = executor.storage().list_issues().unwrap();

    let issues = detect_membership_issues(&config, &task, &all_issues);
    assert!(
        issues.is_empty(),
        "No membership labels should not be an error"
    );
}

#[test]
fn test_epic_referencing_itself() {
    let (_temp, executor) = setup_test_repo();
    let config = HierarchyConfig::default();

    // Epic that references itself (valid but maybe weird)
    let mut epic = Issue::new("Auth".to_string(), String::new());
    epic.labels = vec!["type:epic".to_string(), "epic:auth".to_string()];
    executor.storage().save_issue(epic.clone()).unwrap();

    let all_issues = executor.storage().list_issues().unwrap();

    let issues = detect_membership_issues(&config, &epic, &all_issues);
    // Self-reference should be OK (it's identifying itself)
    assert!(
        issues.is_empty(),
        "Epic with matching label should be OK (self-identification)"
    );
}

#[test]
fn test_mixed_valid_and_invalid_references() {
    let (_temp, executor) = setup_test_repo();
    let config = HierarchyConfig::default();

    // Create one valid epic
    let mut epic = Issue::new("Auth".to_string(), String::new());
    epic.labels = vec!["type:epic".to_string(), "epic:auth".to_string()];
    executor.storage().save_issue(epic.clone()).unwrap();

    // Task references one valid, one invalid
    let mut task = Issue::new("Login".to_string(), String::new());
    task.labels = vec![
        "type:task".to_string(),
        "epic:auth".to_string(),      // Valid
        "milestone:v2.0".to_string(), // Invalid - doesn't exist
    ];
    executor.storage().save_issue(task.clone()).unwrap();

    let all_issues = executor.storage().list_issues().unwrap();

    let issues = detect_membership_issues(&config, &task, &all_issues);
    assert_eq!(issues.len(), 1, "Should detect only the invalid reference");

    match &issues[0] {
        ValidationIssue::InvalidMembershipReference { label, .. } => {
            assert_eq!(label, "milestone:v2.0");
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
    let mut theme = Issue::new("Dark Mode".to_string(), String::new());
    theme.labels = vec!["type:theme".to_string(), "theme:ui".to_string()];
    executor.storage().save_issue(theme.clone()).unwrap();

    // Create a feature that references the theme
    let mut feature = Issue::new("Dark sidebar".to_string(), String::new());
    feature.labels = vec!["type:feature".to_string(), "theme:ui".to_string()];
    executor.storage().save_issue(feature.clone()).unwrap();

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

    // Create config where both "milestone" and "release" use "milestone" namespace
    let mut types = HashMap::new();
    types.insert("milestone".to_string(), 1);
    types.insert("release".to_string(), 1); // Same level
    types.insert("task".to_string(), 2);

    let mut label_associations = HashMap::new();
    label_associations.insert("milestone".to_string(), "milestone".to_string());
    label_associations.insert("release".to_string(), "milestone".to_string()); // Alias!

    let config = HierarchyConfig::new(types, label_associations).unwrap();

    // Create a release (uses milestone namespace)
    let mut release = Issue::new("v2.0".to_string(), String::new());
    release.labels = vec!["type:release".to_string(), "milestone:v2.0".to_string()];
    executor.storage().save_issue(release.clone()).unwrap();

    // Task references it via milestone:v2.0 label
    let mut task = Issue::new("Prepare release notes".to_string(), String::new());
    task.labels = vec!["type:task".to_string(), "milestone:v2.0".to_string()];
    executor.storage().save_issue(task.clone()).unwrap();

    let all_issues = executor.storage().list_issues().unwrap();

    // Should validate successfully - release has type:release but milestone namespace
    let issues = detect_membership_issues(&config, &task, &all_issues);
    assert!(
        issues.is_empty(),
        "Type alias (release -> milestone namespace) should work"
    );
}
