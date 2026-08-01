//! Tests for type hierarchy warning-level validations
//!
//! Every type name here is read from the vocabulary the fixture declares, so a
//! warning fires because the mechanism resolved the declared hierarchy, not
//! because a literal happened to match a shipped name.

use jit::domain::type_taxonomy::{HierarchyConfig, ValidationWarning};
use jit::test_taxonomy::{test_taxonomy, TestTaxonomy};

/// The declared vocabulary and the hierarchy it configures.
fn declared() -> (TestTaxonomy, HierarchyConfig) {
    let taxonomy = test_taxonomy();
    let config = taxonomy.hierarchy_config();
    (taxonomy, config)
}

/// The `type:<name>` label for a declared type.
fn type_label(name: &str) -> String {
    format!("type:{name}")
}

/// The membership namespace the declared vocabulary gives a type.
fn membership_namespace<'a>(taxonomy: &'a TestTaxonomy, type_name: &str) -> &'a str {
    taxonomy
        .label_associations
        .get(type_name)
        .expect("the declared vocabulary associates a membership namespace with this type")
}

/// The membership label a declared type's own namespace carries.
fn membership_label(taxonomy: &TestTaxonomy, type_name: &str, value: &str) -> String {
    format!("{}:{value}", membership_namespace(taxonomy, type_name))
}

#[test]
fn test_validate_strategic_labels_warns_for_strategic_type_without_membership_label() {
    let (taxonomy, config) = declared();
    let strategic = taxonomy.type_at_level(2);
    let mut issue = crate::fixture_issue("Auth System".to_string(), "Description".to_string());
    issue.labels = vec![type_label(strategic)];

    let warnings = jit::domain::type_taxonomy::validate_strategic_labels(&config, &issue);

    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        ValidationWarning::MissingStrategicLabel {
            type_name,
            expected_namespace,
            ..
        } => {
            assert_eq!(type_name, strategic);
            assert_eq!(
                expected_namespace,
                membership_namespace(&taxonomy, strategic)
            );
        }
        _ => panic!("Expected MissingStrategicLabel warning"),
    }
}

#[test]
fn test_validate_strategic_labels_warns_for_top_level_type_without_membership_label() {
    let (taxonomy, config) = declared();
    let top = taxonomy.type_at_level(1);
    let mut issue = crate::fixture_issue("v1.0 Release".to_string(), "Description".to_string());
    issue.labels = vec![type_label(top)];

    let warnings = jit::domain::type_taxonomy::validate_strategic_labels(&config, &issue);

    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        ValidationWarning::MissingStrategicLabel {
            type_name,
            expected_namespace,
            ..
        } => {
            assert_eq!(type_name, top);
            assert_eq!(expected_namespace, membership_namespace(&taxonomy, top));
        }
        _ => panic!("Expected MissingStrategicLabel warning"),
    }
}

#[test]
fn test_validate_strategic_labels_stays_silent_for_leaf_type() {
    let (taxonomy, config) = declared();
    let mut issue = crate::fixture_issue("Login API".to_string(), "Description".to_string());
    issue.labels = vec![type_label(taxonomy.type_at_level(4))];

    let warnings = jit::domain::type_taxonomy::validate_strategic_labels(&config, &issue);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_validate_strategic_labels_stays_silent_when_membership_label_present() {
    let (taxonomy, config) = declared();
    let strategic = taxonomy.type_at_level(2);
    let mut issue = crate::fixture_issue("Auth System".to_string(), "Description".to_string());
    issue.labels = vec![
        type_label(strategic),
        membership_label(&taxonomy, strategic, "auth"),
    ];

    let warnings = jit::domain::type_taxonomy::validate_strategic_labels(&config, &issue);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_validate_strategic_labels_stays_silent_for_top_level_type_with_membership_label() {
    let (taxonomy, config) = declared();
    let top = taxonomy.type_at_level(1);
    let mut issue = crate::fixture_issue("v1.0 Release".to_string(), "Description".to_string());
    issue.labels = vec![type_label(top), membership_label(&taxonomy, top, "v1.0")];

    let warnings = jit::domain::type_taxonomy::validate_strategic_labels(&config, &issue);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_validate_orphans_warns_for_leaf_without_membership_label() {
    let (taxonomy, config) = declared();
    let leaf = taxonomy.type_at_level(4);
    let mut issue = crate::fixture_issue("Login API".to_string(), "Description".to_string());
    issue.labels = vec![type_label(leaf)];

    let warnings = jit::domain::type_taxonomy::validate_orphans(&config, &issue);

    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        ValidationWarning::OrphanedLeaf { type_name, .. } => {
            assert_eq!(type_name, leaf);
        }
        _ => panic!("Expected OrphanedLeaf warning"),
    }
}

#[test]
fn test_validate_orphans_stays_silent_for_leaf_under_a_container() {
    let (taxonomy, config) = declared();
    let container = taxonomy.type_at_level(2);
    let mut issue = crate::fixture_issue("Login API".to_string(), "Description".to_string());
    issue.labels = vec![
        type_label(taxonomy.type_at_level(4)),
        membership_label(&taxonomy, container, "auth"),
    ];

    let warnings = jit::domain::type_taxonomy::validate_orphans(&config, &issue);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_validate_orphans_stays_silent_for_leaf_under_a_top_level_container() {
    let (taxonomy, config) = declared();
    let top = taxonomy.type_at_level(1);
    let mut issue = crate::fixture_issue("Login API".to_string(), "Description".to_string());
    issue.labels = vec![
        type_label(taxonomy.type_at_level(4)),
        membership_label(&taxonomy, top, "v1.0"),
    ];

    let warnings = jit::domain::type_taxonomy::validate_orphans(&config, &issue);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_validate_orphans_stays_silent_for_non_leaf_type() {
    let (taxonomy, config) = declared();
    let mut issue = crate::fixture_issue("Auth System".to_string(), "Description".to_string());
    issue.labels = vec![type_label(taxonomy.type_at_level(2))];

    let warnings = jit::domain::type_taxonomy::validate_orphans(&config, &issue);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_validate_orphans_stays_silent_for_leaf_with_several_membership_labels() {
    let (taxonomy, config) = declared();
    let top = taxonomy.type_at_level(1);
    let container = taxonomy.type_at_level(2);
    let mut issue = crate::fixture_issue("Login API".to_string(), "Description".to_string());
    issue.labels = vec![
        type_label(taxonomy.type_at_level(4)),
        membership_label(&taxonomy, container, "auth"),
        membership_label(&taxonomy, top, "v1.0"),
    ];

    let warnings = jit::domain::type_taxonomy::validate_orphans(&config, &issue);

    assert_eq!(warnings.len(), 0);
}
