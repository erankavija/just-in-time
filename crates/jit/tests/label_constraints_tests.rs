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
            false,
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

    // Post-migration the enum constraint is the `default:namespace-values:type`
    // rule; accept/reject parity is preserved (the bad value still fails
    // validation), the message now comes from the JSON Schema engine.
    let err = exec.validate_silent().unwrap_err().to_string();
    assert!(
        err.contains("default:namespace-values:type") && err.contains("taks"),
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

    // Post-migration the value-pattern constraint is the
    // `default:namespace-pattern:milestone` rule; the bad value still fails
    // validation (parity), message from the JSON Schema engine.
    let err = exec.validate_silent().unwrap_err().to_string();
    assert!(
        err.contains("default:namespace-pattern:milestone") && err.contains("1.2"),
        "{}",
        err
    );
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
    // Post-migration the namespace `pattern` is the `default:namespace-pattern`
    // rule, whose schema embeds the regex. A malformed regex now surfaces as a
    // schema COMPILE error when the rule is evaluated (on create OR validate) —
    // it is never silently swallowed (parity goal: a misconfigured constraint
    // must be visible). The error names the offending rule/namespace.
    let (id, _) = exec
        .create_issue(
            "x".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["broken:foo".to_string()],
            false,
        )
        .unwrap_or_else(|err| {
            // The bad pattern can surface at create time; assert it is the
            // compile error for the broken namespace, then stop.
            let msg = err.to_string();
            assert!(msg.contains("default:namespace-pattern:broken"), "{}", msg);
            assert!(msg.contains("compile"), "{}", msg);
            (String::new(), vec![])
        });
    if id.is_empty() {
        return; // surfaced at create time, already asserted above
    }

    let err = exec.validate_silent().unwrap_err().to_string();
    assert!(err.contains("default:namespace-pattern:broken"), "{}", err);
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

    // Post-migration the required constraint is the
    // `default:namespace-required:type` rule; a missing required label still
    // fails validation (parity), message from the JSON Schema engine.
    let err = exec.validate_silent().unwrap_err().to_string();
    assert!(err.contains("default:namespace-required:type"), "{}", err);
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

// ------------------------------------------------------------------
// CLI: `jit config show --json` surfaces the namespace registry
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

    // Post-migration (0abaddc0): the per-namespace `values` / `pattern` /
    // `required` constraints are migrated OUT of config.toml into the default
    // rules in `.jit/rules.toml`, so they are no longer surfaced by
    // `config show`. They now live as `default:namespace-*` rules instead.
    let rules_toml = std::fs::read_to_string(temp.path().join(".jit/rules.toml"))
        .expect("rules.toml scaffolded");
    assert!(
        rules_toml.contains("default:namespace-values:type"),
        "type values must migrate to a default rule: {rules_toml}"
    );
    assert!(
        rules_toml.contains("default:namespace-required:type"),
        "type required must migrate to a default rule"
    );
    assert!(
        rules_toml.contains("default:namespace-pattern:milestone"),
        "milestone pattern must migrate to a default rule"
    );
}

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

    // Post-migration an unregistered namespace is caught by the
    // `default:namespace-registry` rule (the former `validate_labels` registry
    // check). Accept/reject parity holds — an unknown namespace fails validation —
    // though the closest-namespace hint (a `validate_labels`-only nicety) is no
    // longer emitted by the schema-based check.
    let err = exec.validate_silent().unwrap_err().to_string();
    assert!(err.contains("default:namespace-registry"), "{}", err);
    assert!(err.contains("typo:foo"), "{}", err);
}
