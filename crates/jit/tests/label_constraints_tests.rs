//! Integration tests for the namespace registry and the validation drift it
//! surfaces, after the backward-compat hard removal (issue d4188154).
//!
//! The per-namespace `values` / `pattern` / `required` constraints were removed
//! from config-derived defaults: `.jit/rules.toml` is the sole validation source,
//! and a repo wanting those constraints authors them there directly. What the
//! registry still drives is the `default:namespace-registry` rule (unknown
//! namespaces) and the `default:namespace-unique:<ns>` rules (uniqueness).
//!
//! Tests exercise the CommandExecutor against a tempdir-backed JsonFileStorage
//! and a real `config.toml`, so the full load-validate path runs.

use jit::commands::CommandExecutor;
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
            None,
            None,
            false,
        )
        .unwrap();
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
    let (_tmp, exec) = setup_repo(cfg);
    create_labeled(&exec, "typo", &["typo:foo"]);

    // An unregistered namespace is caught by the `default:namespace-registry`
    // rule. It fails `jit validate` but does NOT block the write (enforce=false).
    let err = exec.validate_silent().unwrap_err().to_string();
    assert!(err.contains("default:namespace-registry"), "{}", err);
    assert!(err.contains("typo:foo"), "{}", err);
}

#[test]
fn test_validate_accepts_registered_namespace() {
    let cfg = r#"
[namespaces.type]
description = "Issue type"
unique = true
"#;
    let (_tmp, exec) = setup_repo(cfg);
    create_labeled(&exec, "ok", &["type:task"]);
    exec.validate_silent().expect("validate should pass");
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
    let (_tmp, exec) = setup_repo(cfg);

    let result = exec.create_issue(
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
        err.contains("default:namespace-unique:priority"),
        "duplicate unique label must block the write: {err}"
    );
}

// ------------------------------------------------------------------
// CLI: `jit config show --json` surfaces the namespace registry, and
// `jit init` scaffolds only the fixed default rules (no value/pattern/required)
// ------------------------------------------------------------------

#[test]
fn test_config_show_json_includes_namespace_registry() {
    // CARGO_BIN_EXE_jit is set by Cargo for integration tests and guarantees
    // we're exercising the just-built binary — no silent skip path.
    let jit_bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_jit"));

    let temp = TempDir::new().unwrap();
    let init = std::process::Command::new(&jit_bin)
        .arg("init")
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(init.status.success(), "jit init failed: {:?}", init);

    let show = std::process::Command::new(&jit_bin)
        .args(["config", "show", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        show.status.success(),
        "jit config show --json failed: stderr={}",
        String::from_utf8_lossy(&show.stderr)
    );
    let stdout = String::from_utf8_lossy(&show.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let namespaces = parsed
        .get("namespaces")
        .expect("namespaces key present in config show --json output");
    // The registry is still exposed (description/unique stay in config.toml).
    let type_ns = namespaces.get("type").expect("type namespace exposed");
    assert_eq!(type_ns["unique"], serde_json::json!(true));
    assert!(namespaces.get("milestone").is_some());

    // The scaffolded rules.toml carries the FIXED default rules only: the
    // canonical format, the namespace registry, type-hierarchy-known, the
    // per-unique-namespace uniqueness rules, and the two graph warnings. The
    // removed `values` / `pattern` / `required` rules are NOT present.
    let rules_toml = std::fs::read_to_string(temp.path().join(".jit/rules.toml"))
        .expect("rules.toml scaffolded");
    assert!(
        rules_toml.contains("default:namespace-unique:type"),
        "uniqueness rule must be scaffolded: {rules_toml}"
    );
    assert!(
        !rules_toml.contains("default:namespace-values:"),
        "namespace-values rules must NOT be scaffolded: {rules_toml}"
    );
    assert!(
        !rules_toml.contains("default:namespace-required:"),
        "namespace-required rules must NOT be scaffolded"
    );
    assert!(
        !rules_toml.contains("default:namespace-pattern:"),
        "namespace-pattern rules must NOT be scaffolded"
    );
}
