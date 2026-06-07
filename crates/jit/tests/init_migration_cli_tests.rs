//! Phase E CLI tests for `jit init` migration/scaffold (decisions D5/D6/D7).
//!
//! Spawns the built `jit` binary against throwaway repos:
//! - fresh init: no deprecation warning, no migration message, complete rules.toml;
//! - legacy re-init: migrates + strips keys, config reloads clean (no warning);
//! - idempotent double-init;
//! - coexistence: a pre-existing rules.toml is preserved, defaults appended by
//!   name, keys stripped.

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn jit_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_jit"))
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(jit_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn jit")
}

/// Init a fresh repo (git-initialized so worktree detection is happy).
fn fresh_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    // git init (best-effort; jit works without git too).
    let _ = Command::new("git")
        .arg("init")
        .current_dir(dir.path())
        .output();
    dir
}

#[test]
fn cli_fresh_init_emits_no_deprecation_warning_or_migration_message() {
    let dir = fresh_repo();
    let out = run(dir.path(), &["init"]);
    assert!(out.status.success(), "init failed: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        !stderr.contains("deprecated"),
        "fresh init must not warn about deprecated keys: {stderr}"
    );
    assert!(
        !stdout.contains("Migrated") && !stderr.contains("Migrated"),
        "fresh init must not print a migration message: out={stdout} err={stderr}"
    );
    assert!(
        stdout.contains("Scaffolded .jit/rules.toml") || stderr.contains("Scaffolded"),
        "fresh init should report scaffolding: out={stdout} err={stderr}"
    );

    // The complete rules.toml + schemas exist.
    assert!(dir.path().join(".jit/rules.toml").exists());
    let rules = std::fs::read_to_string(dir.path().join(".jit/rules.toml")).unwrap();
    assert!(rules.contains("default:label-format"));
    assert!(rules.contains("default:orphan-leaf"));
    assert!(dir.path().join(".jit/schemas").exists());

    // A subsequent normal command emits NO deprecation warning (config is clean).
    let v = run(dir.path(), &["validate"]);
    let verr = String::from_utf8_lossy(&v.stderr);
    assert!(
        !verr.contains("deprecated"),
        "post-init config must be clean: {verr}"
    );
    assert!(
        !verr.contains("no .jit/rules.toml found"),
        "rules.toml must be present after init: {verr}"
    );
}

#[test]
fn cli_legacy_reinit_migrates_strips_and_reloads_clean() {
    let dir = fresh_repo();
    let jit_dir = dir.path().join(".jit");
    std::fs::create_dir_all(&jit_dir).unwrap();
    // A legacy config with enforcement keys + namespace constraints, NO rules.toml.
    std::fs::write(
        jit_dir.join("config.toml"),
        r#"[version]
schema = 2

[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4 }

[validation]
default_type = "task"
strictness = "loose"
require_type_label = true
label_regex = '^team:[a-z]+$'
reject_malformed_labels = true
enforce_namespace_registry = true
warn_orphaned_leaves = true
warn_strategic_consistency = true

[namespaces.type]
description = "Issue type"
unique = true
required = true
values = ["task", "bug"]

[namespaces.milestone]
description = "Release"
unique = false
pattern = '^v\d+\.\d+$'
"#,
    )
    .unwrap();

    // Re-init migrates.
    let out = run(dir.path(), &["init"]);
    assert!(out.status.success(), "re-init failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Migrated") && stdout.contains("legacy validation key"),
        "legacy re-init must report migration: {stdout}"
    );

    // The six enforcement keys + namespace constraints are gone; default_type +
    // strictness retained.
    let config = std::fs::read_to_string(jit_dir.join("config.toml")).unwrap();
    for gone in [
        "require_type_label",
        "label_regex",
        "reject_malformed_labels",
        "enforce_namespace_registry",
        "warn_orphaned_leaves",
        "warn_strategic_consistency",
    ] {
        assert!(!config.contains(gone), "{gone} must be stripped: {config}");
    }
    assert!(!config.contains("values ="));
    assert!(!config.contains("required ="));
    assert!(!config.contains("pattern ="));
    assert!(config.contains("default_type = \"task\""));
    assert!(config.contains("strictness = \"loose\""));

    // rules.toml carries the complete migrated ruleset.
    let rules = std::fs::read_to_string(jit_dir.join("rules.toml")).unwrap();
    assert!(rules.contains("default:require-type-label"));
    assert!(rules.contains("default:label-format-custom"));
    assert!(rules.contains("default:namespace-values:type"));
    assert!(rules.contains("default:namespace-pattern:milestone"));

    // A normal command now loads cleanly: no deprecation warning, no missing-file.
    let v = run(dir.path(), &["validate"]);
    let verr = String::from_utf8_lossy(&v.stderr);
    assert!(!verr.contains("deprecated"), "config must be clean: {verr}");
    assert!(
        !verr.contains("no .jit/rules.toml found"),
        "rules.toml present: {verr}"
    );
}

#[test]
fn cli_double_init_is_idempotent() {
    let dir = fresh_repo();
    let first = run(dir.path(), &["init"]);
    assert!(first.status.success());
    let rules_after_first = std::fs::read_to_string(dir.path().join(".jit/rules.toml")).unwrap();
    let config_after_first = std::fs::read_to_string(dir.path().join(".jit/config.toml")).unwrap();

    let second = run(dir.path(), &["init"]);
    assert!(second.status.success(), "second init failed: {second:?}");
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        !stdout.contains("Migrated"),
        "idempotent re-init must not migrate again: {stdout}"
    );
    let rules_after_second = std::fs::read_to_string(dir.path().join(".jit/rules.toml")).unwrap();
    let config_after_second = std::fs::read_to_string(dir.path().join(".jit/config.toml")).unwrap();
    assert_eq!(rules_after_first, rules_after_second, "rules.toml stable");
    assert_eq!(
        config_after_first, config_after_second,
        "config.toml stable"
    );
}

#[test]
fn cli_coexistence_preserves_user_rules_and_strips_keys() {
    let dir = fresh_repo();
    let jit_dir = dir.path().join(".jit");
    std::fs::create_dir_all(&jit_dir).unwrap();
    std::fs::write(
        jit_dir.join("config.toml"),
        r#"[version]
schema = 2

[validation]
default_type = "task"
require_type_label = true

[namespaces.type]
description = "Issue type"
unique = true
values = ["task", "bug"]
"#,
    )
    .unwrap();
    // A pre-existing user rules.toml with a custom rule.
    std::fs::write(
        jit_dir.join("rules.toml"),
        r#"[[rules]]
name = "user:epic-goals"
when = { type = "epic" }
assert = { require-section = { heading = "Goals" } }
"#,
    )
    .unwrap();

    let out = run(dir.path(), &["init"]);
    assert!(out.status.success(), "coexistence init failed: {out:?}");

    let rules = std::fs::read_to_string(jit_dir.join("rules.toml")).unwrap();
    // User rule preserved.
    assert!(rules.contains("user:epic-goals"), "user rule must survive");
    // Missing defaults appended by name.
    assert!(rules.contains("default:require-type-label"));
    assert!(rules.contains("default:namespace-values:type"));

    // Legacy keys stripped from config.
    let config = std::fs::read_to_string(jit_dir.join("config.toml")).unwrap();
    assert!(!config.contains("require_type_label"));
    assert!(!config.contains("values ="));
    assert!(config.contains("default_type = \"task\""));

    // The merged rules.toml reloads cleanly (a normal command succeeds).
    let v = run(dir.path(), &["validate"]);
    let verr = String::from_utf8_lossy(&v.stderr);
    assert!(!verr.contains("invalid"), "merged rules must load: {verr}");
}
