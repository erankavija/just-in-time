//! Integration tests for label-based membership validation
//!
//! These tests validate that organizational membership labels (initiative:*, objective:*)
//! reference actual issues with matching types.

use jit::commands::CommandExecutor;
use jit::domain::type_taxonomy::{detect_membership_issues, HierarchyConfig, ValidationIssue};
use jit::storage::json::JsonFileStorage;
use jit::storage::IssueStore;
use jit::test_taxonomy::TestTaxonomy;
use std::collections::HashMap;
use tempfile::TempDir;

fn setup_test_repo() -> (TempDir, CommandExecutor<JsonFileStorage>, TestTaxonomy) {
    let (temp, storage, taxonomy) = jit::test_utils::setup_test_repo_with_taxonomy().unwrap();
    let layout = jit::storage::discover_repository_layout(temp.path(), storage.root()).unwrap();
    let executor = CommandExecutor::new(storage).with_layout(layout);
    (temp, executor, taxonomy)
}

fn type_label(taxonomy: &TestTaxonomy, level: u8) -> String {
    format!("type:{}", taxonomy.type_at_level(level))
}

fn membership_label(taxonomy: &TestTaxonomy, type_name: &str, value: &str) -> String {
    format!(
        "{}:{value}",
        taxonomy
            .label_associations
            .get(type_name)
            .expect("the test taxonomy associates this type with a namespace")
    )
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
    let (_temp, executor, taxonomy) = setup_test_repo();
    let config = taxonomy.hierarchy_config();
    let initiative_type = taxonomy.type_at_level(2);

    // Create an initiative
    let mut initiative_issue =
        crate::fixture_issue("Authentication System".to_string(), String::new());
    initiative_issue.labels = vec![
        type_label(&taxonomy, 2),
        membership_label(&taxonomy, initiative_type, "auth"),
    ];
    publish_issue(&executor, &mut initiative_issue);

    // Create an action that references the initiative
    let mut action_issue = crate::fixture_issue("Implement login".to_string(), String::new());
    action_issue.labels = vec![
        type_label(&taxonomy, 4),
        membership_label(&taxonomy, initiative_type, "auth"),
    ];
    publish_issue(&executor, &mut action_issue);

    // Load all issues for validation
    let all_issues = executor.storage().list_issues().unwrap();

    // Validate the action - should have no issues
    let issues = detect_membership_issues(&config, &action_issue, &all_issues);
    assert!(
        issues.is_empty(),
        "Valid initiative reference should not produce validation issues"
    );
}

#[test]
fn test_invalid_initiative_reference_not_found() {
    let (_temp, executor, taxonomy) = setup_test_repo();
    let config = taxonomy.hierarchy_config();
    let initiative_type = taxonomy.type_at_level(2);

    // Create an action that references a non-existent initiative
    let mut action = crate::fixture_issue("Implement login".to_string(), String::new());
    action.labels = vec![
        type_label(&taxonomy, 4),
        membership_label(&taxonomy, initiative_type, "nonexistent"),
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
            assert_eq!(
                label,
                &membership_label(&taxonomy, initiative_type, "nonexistent")
            );
            assert_eq!(
                namespace,
                taxonomy
                    .label_associations
                    .get(initiative_type)
                    .expect("the initiative namespace is declared")
            );
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
    let (_temp, executor, taxonomy) = setup_test_repo();
    let config = taxonomy.hierarchy_config();
    let initiative_type = taxonomy.type_at_level(2);

    // Create an issue with type:action but initiative:backend label
    let mut backend = crate::fixture_issue("Backend Service".to_string(), String::new());
    backend.labels = vec![
        type_label(&taxonomy, 4),
        membership_label(&taxonomy, initiative_type, "backend"),
    ];
    publish_issue(&executor, &mut backend);

    // Create an action that references it as an initiative (wrong!)
    let mut action = crate::fixture_issue("Add endpoint".to_string(), String::new());
    action.labels = vec![
        type_label(&taxonomy, 4),
        membership_label(&taxonomy, initiative_type, "backend"),
    ];
    publish_issue(&executor, &mut action);

    let all_issues = executor.storage().list_issues().unwrap();

    // Validate - should detect type mismatch
    let issues = detect_membership_issues(&config, &action, &all_issues);
    assert_eq!(issues.len(), 1, "Should detect type mismatch");

    match &issues[0] {
        ValidationIssue::InvalidMembershipReference { reason, .. } => {
            assert!(
                reason.contains(&type_label(&taxonomy, 2))
                    && reason.contains(&type_label(&taxonomy, 4)),
                "Should explain type mismatch"
            );
        }
        _ => panic!("Expected InvalidMembershipReference"),
    }
}

#[test]
fn test_valid_objective_membership() {
    let (_temp, executor, taxonomy) = setup_test_repo();
    let config = taxonomy.hierarchy_config();
    let objective_type = taxonomy.type_at_level(1);

    // Create objective
    let mut objective_issue = crate::fixture_issue("v1.0 Release".to_string(), String::new());
    objective_issue.labels = vec![
        type_label(&taxonomy, 1),
        membership_label(&taxonomy, objective_type, "v1.0"),
    ];
    publish_issue(&executor, &mut objective_issue);

    // Create action under objective
    let mut action = crate::fixture_issue("Fix critical bug".to_string(), String::new());
    action.labels = vec![
        type_label(&taxonomy, 4),
        membership_label(&taxonomy, objective_type, "v1.0"),
    ];
    publish_issue(&executor, &mut action);

    let all_issues = executor.storage().list_issues().unwrap();

    let issues = detect_membership_issues(&config, &action, &all_issues);
    assert!(issues.is_empty(), "Valid objective reference should be OK");
}

#[test]
fn test_multiple_membership_labels() {
    let (_temp, executor, taxonomy) = setup_test_repo();
    let config = taxonomy.hierarchy_config();
    let objective_type = taxonomy.type_at_level(1);
    let initiative_type = taxonomy.type_at_level(2);

    // Create objective and initiative
    let mut objective_issue = crate::fixture_issue("v1.0".to_string(), String::new());
    objective_issue.labels = vec![
        type_label(&taxonomy, 1),
        membership_label(&taxonomy, objective_type, "v1.0"),
    ];
    publish_issue(&executor, &mut objective_issue);

    let mut initiative = crate::fixture_issue("Auth".to_string(), String::new());
    initiative.labels = vec![
        type_label(&taxonomy, 2),
        membership_label(&taxonomy, initiative_type, "auth"),
        membership_label(&taxonomy, objective_type, "v1.0"), // Initiative belongs to objective
    ];
    publish_issue(&executor, &mut initiative);

    // Action belongs to both
    let mut action = crate::fixture_issue("Login".to_string(), String::new());
    action.labels = vec![
        type_label(&taxonomy, 4),
        membership_label(&taxonomy, initiative_type, "auth"),
        membership_label(&taxonomy, objective_type, "v1.0"),
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
    let (_temp, executor, taxonomy) = setup_test_repo();
    let config = taxonomy.hierarchy_config();

    // Action with no membership labels (orphan)
    let mut action = crate::fixture_issue("Standalone action".to_string(), String::new());
    action.labels = vec![type_label(&taxonomy, 4)];
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
    let (_temp, executor, taxonomy) = setup_test_repo();
    let config = taxonomy.hierarchy_config();
    let initiative_type = taxonomy.type_at_level(2);

    // Initiative that references itself (valid but maybe weird)
    let mut initiative_issue = crate::fixture_issue("Auth".to_string(), String::new());
    initiative_issue.labels = vec![
        type_label(&taxonomy, 2),
        membership_label(&taxonomy, initiative_type, "auth"),
    ];
    publish_issue(&executor, &mut initiative_issue);

    let all_issues = executor.storage().list_issues().unwrap();

    let issues = detect_membership_issues(&config, &initiative_issue, &all_issues);
    // Self-reference should be OK (it's identifying itself)
    assert!(
        issues.is_empty(),
        "Initiative with matching label should be OK (self-identification)"
    );
}

#[test]
fn test_mixed_valid_and_invalid_references() {
    let (_temp, executor, taxonomy) = setup_test_repo();
    let config = taxonomy.hierarchy_config();
    let initiative_type = taxonomy.type_at_level(2);
    let objective_type = taxonomy.type_at_level(1);

    // Create one valid initiative
    let mut initiative_issue = crate::fixture_issue("Auth".to_string(), String::new());
    initiative_issue.labels = vec![
        type_label(&taxonomy, 2),
        membership_label(&taxonomy, initiative_type, "auth"),
    ];
    publish_issue(&executor, &mut initiative_issue);

    // Action references one valid, one invalid
    let mut action = crate::fixture_issue("Login".to_string(), String::new());
    action.labels = vec![
        type_label(&taxonomy, 4),
        membership_label(&taxonomy, initiative_type, "auth"), // Valid
        membership_label(&taxonomy, objective_type, "v2.0"),  // Invalid - doesn't exist
    ];
    publish_issue(&executor, &mut action);

    let all_issues = executor.storage().list_issues().unwrap();

    let issues = detect_membership_issues(&config, &action, &all_issues);
    assert_eq!(issues.len(), 1, "Should detect only the invalid reference");

    match &issues[0] {
        ValidationIssue::InvalidMembershipReference { label, .. } => {
            assert_eq!(label, &membership_label(&taxonomy, objective_type, "v2.0"));
        }
        _ => panic!("Expected InvalidMembershipReference"),
    }
}

#[test]
fn test_custom_type_names_and_namespaces() {
    let (_temp, executor, _taxonomy) = setup_test_repo();

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
    let (_temp, executor, _taxonomy) = setup_test_repo();

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
