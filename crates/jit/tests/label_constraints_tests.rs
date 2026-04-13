//! Integration tests for the label constraint registry (values / pattern /
//! required) and the drift surfacing that `jit validate` does on top of it.
//!
//! Tests exercise the CommandExecutor against a tempdir-backed JsonFileStorage
//! and a real `config.toml`, so the full load-validate path runs.

use jit::commands::CommandExecutor;
use jit::config::JitConfig;
use jit::domain::Priority;
use jit::storage::{IssueStore, JsonFileStorage};
use std::fs;
use tempfile::TempDir;

fn setup_repo(config_toml: &str) -> (TempDir, CommandExecutor<JsonFileStorage>) {
    std::env::set_var("JIT_TEST_MODE", "1");
    let temp = TempDir::new().unwrap();
    let jit_dir = temp.path().join(".jit");
    fs::create_dir(&jit_dir).unwrap();
    fs::write(jit_dir.join("config.toml"), config_toml).unwrap();
    let storage = JsonFileStorage::new(&jit_dir);
    storage.init().unwrap();
    let executor = CommandExecutor::new(storage);
    (temp, executor)
}

fn create_labeled(exec: &CommandExecutor<JsonFileStorage>, title: &str, labels: &[&str]) -> String {
    let (id, _) = exec
        .create_issue(
            title.to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            labels.iter().map(|s| s.to_string()).collect(),
        )
        .unwrap();
    id
}

// ------------------------------------------------------------------
// Config schema
// ------------------------------------------------------------------

#[test]
fn test_namespace_config_round_trips_new_fields() {
    let toml = r#"
[namespaces.type]
description = "Issue type"
unique = true
required = true
values = ["task", "bug"]

[namespaces.milestone]
description = "Release"
unique = false
pattern = '^v\d+\.\d+$'
"#;
    let cfg: JitConfig = toml::from_str(toml).unwrap();
    let ns = cfg.namespaces.unwrap();

    let t = &ns["type"];
    assert_eq!(
        t.values.as_deref(),
        Some(&["task".to_string(), "bug".to_string()][..])
    );
    assert_eq!(t.required, Some(true));

    let m = &ns["milestone"];
    assert_eq!(m.pattern.as_deref(), Some(r"^v\d+\.\d+$"));
    assert!(m.values.is_none());
}

// ------------------------------------------------------------------
// Enum (values) enforcement
// ------------------------------------------------------------------

#[test]
fn test_validate_rejects_value_outside_enum() {
    let cfg = r#"
[namespaces.type]
description = "Issue type"
unique = true
values = ["task", "bug", "story"]
"#;
    let (_tmp, exec) = setup_repo(cfg);
    create_labeled(&exec, "wrong", &["type:taks"]);

    let err = exec.validate_silent().unwrap_err().to_string();
    assert!(
        err.contains("not in allowed set") && err.contains("taks"),
        "unexpected error: {}",
        err
    );
}

#[test]
fn test_validate_accepts_value_in_enum() {
    let cfg = r#"
[namespaces.type]
description = "Issue type"
unique = true
values = ["task", "bug"]
"#;
    let (_tmp, exec) = setup_repo(cfg);
    create_labeled(&exec, "ok", &["type:task"]);
    exec.validate_silent().expect("validate should pass");
}

// ------------------------------------------------------------------
// Pattern enforcement
// ------------------------------------------------------------------

#[test]
fn test_validate_rejects_value_not_matching_pattern() {
    let cfg = r#"
[namespaces.milestone]
description = "Release"
unique = false
pattern = '^v\d+\.\d+$'
"#;
    let (_tmp, exec) = setup_repo(cfg);
    create_labeled(&exec, "bad", &["milestone:1.2"]);

    let err = exec.validate_silent().unwrap_err().to_string();
    assert!(err.contains("does not match pattern"), "{}", err);
}

#[test]
fn test_validate_accepts_pattern_match() {
    let cfg = r#"
[namespaces.milestone]
description = "Release"
unique = false
pattern = '^v\d+\.\d+$'
"#;
    let (_tmp, exec) = setup_repo(cfg);
    create_labeled(&exec, "good", &["milestone:v1.0"]);
    exec.validate_silent().unwrap();
}

#[test]
fn test_validate_surfaces_invalid_pattern_as_config_error() {
    let cfg = r#"
[namespaces.broken]
description = "Bad regex"
unique = false
pattern = "["
"#;
    let (_tmp, exec) = setup_repo(cfg);
    // Even one issue triggers validate_labels.
    create_labeled(&exec, "x", &["broken:foo"]);

    let err = exec.validate_silent().unwrap_err().to_string();
    assert!(err.contains("Invalid regex pattern"), "{}", err);
    assert!(err.contains("broken"), "{}", err);
}

// ------------------------------------------------------------------
// Required namespace
// ------------------------------------------------------------------

#[test]
fn test_validate_flags_missing_required_namespace() {
    let cfg = r#"
[namespaces.type]
description = "Issue type"
unique = true
required = true

[namespaces.component]
description = "Component"
unique = false
"#;
    let (_tmp, exec) = setup_repo(cfg);
    create_labeled(&exec, "orphan", &["component:core"]);

    let err = exec.validate_silent().unwrap_err().to_string();
    assert!(err.contains("missing a required label"), "{}", err);
    assert!(err.contains("type"), "{}", err);
}

#[test]
fn test_validate_allows_when_required_namespace_present() {
    let cfg = r#"
[namespaces.type]
description = "Issue type"
unique = true
required = true
"#;
    let (_tmp, exec) = setup_repo(cfg);
    create_labeled(&exec, "fine", &["type:task"]);
    exec.validate_silent().unwrap();
}

// ------------------------------------------------------------------
// Near-duplicate namespace hint
// ------------------------------------------------------------------

#[test]
fn test_validate_hints_closest_namespace() {
    let cfg = r#"
[namespaces.type]
description = "Issue type"
unique = true

[namespaces.component]
description = "Component"
unique = false
"#;
    let (_tmp, exec) = setup_repo(cfg);
    create_labeled(&exec, "typo", &["typo:foo"]);

    let err = exec.validate_silent().unwrap_err().to_string();
    assert!(err.contains("Did you mean 'type'"), "{}", err);
}
