//! Integration tests for the namespace registry and the validation drift it
//! surfaces, after the backward-compat hard removal (issue d4188154).
//!
//! The per-namespace `values` / `pattern` / `required` constraints were removed
//! from config-derived defaults: validation is rule-driven, and a repo wanting
//! those constraints authors them in `.jit/rules.toml` directly. What the
//! registry still drives is the `namespace-registry` rule (unknown namespaces)
//! and the `namespace-unique-<ns>` rules (uniqueness); both carry `origin =
//! "default"`.
//!
//! Tests exercise the CommandExecutor against a tempdir-backed JsonFileStorage
//! and a real `config.toml`, so the full load-validate path runs.

use jit::commands::CommandExecutor;
use jit::domain::identity::IssueId;
use jit::domain::Priority;
use jit::storage::JsonFileStorage;
use jit::test_cli::JitCli;
use jit::test_taxonomy::{
    membership_namespace, setup_test_repo_with_taxonomy, type_label, TaxonomyRepo,
};
use jit::test_utils::setup_test_repo_with_config;

fn create_labeled(
    exec: &CommandExecutor<JsonFileStorage>,
    title: &str,
    labels: &[&str],
) -> IssueId {
    let id = exec
        .issues()
        .create_issue(
            title.to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            labels.iter().map(|s| s.to_string()).collect(),
            None,
            None,
            false,
        )
        .unwrap()
        .issue
        .id;
    id
}

// ------------------------------------------------------------------
// Namespace registry: unknown namespaces fail validate (enforce=false)
// ------------------------------------------------------------------

#[test]
fn test_validate_flags_unregistered_namespace() {
    let cfg = r#"
[namespaces.type]
description = "Issue type"
unique = true

[namespaces.component]
description = "Component"
unique = false
"#;
    let (_tmp, exec) = setup_test_repo_with_config(cfg).unwrap();
    create_labeled(&exec, "typo", &["typo:foo"]);

    // An unregistered namespace is caught by the `namespace-registry` rule
    // (origin = "default"). It fails `jit validate` but does NOT block the
    // write (enforce=false).
    let err = exec.validation().validate_silent().unwrap_err().to_string();
    assert!(err.contains("namespace-registry"), "{}", err);
    assert!(err.contains("typo:foo"), "{}", err);
}

#[test]
fn test_validate_accepts_registered_namespace() {
    let cfg = r#"
[namespaces.type]
description = "Issue type"
unique = true
"#;
    let (_tmp, exec) = setup_test_repo_with_config(cfg).unwrap();
    create_labeled(&exec, "ok", &["type:task"]);
    exec.validation()
        .validate_silent()
        .expect("validate should pass");
}

// ------------------------------------------------------------------
// Uniqueness: a unique namespace blocks a duplicate on write
// ------------------------------------------------------------------

#[test]
fn test_unique_namespace_blocks_duplicate_on_write() {
    let cfg = r#"
[namespaces.priority]
description = "Priority"
unique = true
"#;
    let (_tmp, exec) = setup_test_repo_with_config(cfg).unwrap();

    let result = exec.issues().create_issue(
        "dup".to_string(),
        String::new(),
        Priority::Normal,
        vec![],
        vec!["priority:high".to_string(), "priority:low".to_string()],
        None,
        None,
        false,
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("namespace-unique-priority"),
        "duplicate unique label must block the write: {err}"
    );
}

// ------------------------------------------------------------------
// CLI: `jit config show --json` surfaces the namespace registry, and
// The applied taxonomy fixture contributes only the fixed default rules (no
// value/pattern/required rules)
// ------------------------------------------------------------------

#[test]
fn test_config_show_json_includes_namespace_registry() {
    let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
    let cli = JitCli::with_repository(temp);

    let parsed = cli.run_json(["config", "show"]).unwrap();
    let namespaces = parsed
        .get("namespaces")
        .expect("namespaces key present in config show --json output");
    // The registry is still exposed (description/unique stay in config.toml).
    let type_label = type_label(&taxonomy, 4);
    let type_namespace = type_label
        .split_once(':')
        .map(|(namespace, _)| namespace)
        .expect("type label has a namespace");
    let membership = membership_namespace(&taxonomy, 1);
    let type_ns = namespaces
        .get(type_namespace)
        .expect("type namespace exposed");
    assert_eq!(type_ns["unique"], serde_json::json!(true));
    assert!(namespaces.get(membership).is_some());

    // The applied taxonomy fixture's rules.toml carries the FIXED default rules:
    // canonical format, the namespace registry, type-hierarchy-known, the
    // per-unique-namespace uniqueness rules, and the two graph warnings. The
    // removed `values` / `pattern` / `required` rules are NOT present.
    let rules_toml = std::fs::read_to_string(cli.root().join(".jit/rules.toml"))
        .expect("rules.toml written by the fixture");
    assert!(
        rules_toml.contains("namespace-unique-type"),
        "uniqueness rule must be registry-derived: {rules_toml}"
    );
    assert!(
        !rules_toml.contains("namespace-values-"),
        "namespace-values rules must NOT be registry-derived: {rules_toml}"
    );
    assert!(
        !rules_toml.contains("namespace-required-"),
        "namespace-required rules must NOT be registry-derived"
    );
    assert!(
        !rules_toml.contains("namespace-pattern-"),
        "namespace-pattern rules must NOT be registry-derived"
    );
}
