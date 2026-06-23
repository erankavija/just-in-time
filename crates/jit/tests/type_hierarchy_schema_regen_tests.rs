//! Integration tests for T1 (jit:c78168d8): regenerating the baked write-path
//! type-hierarchy schema from `[type_hierarchy]` so a config-declared type passes
//! write-path validation without hand-editing
//! `.jit/schemas/default-type-hierarchy-known.json` (design doc risk R5).
//!
//! Exercises the full disk-based path: a real `.jit/` scaffolded by
//! `scaffold_default_rules` (so `rules.toml` + the baked schema exist), a
//! `config.toml` edited to add a new type, then `CommandExecutor::create_issue`
//! against `JsonFileStorage`. `default:type-hierarchy-known` is `enforce = false`,
//! so an unknown type never blocks the write — it surfaces as a WARNING; the
//! deliverable is that the warning disappears once the schema is regenerated.

use jit::commands::CommandExecutor;
use jit::domain::Priority;
use jit::storage::{IssueStore, JsonFileStorage};
use std::fs;
use tempfile::TempDir;

/// The fully-formed scaffold a freshly-`jit init`ed repo carries: a `config.toml`
/// with a 4-level hierarchy plus the scaffolded `rules.toml` + baked schemas.
fn setup_initialized_repo() -> (TempDir, std::path::PathBuf) {
    std::env::set_var("JIT_TEST_MODE", "1");
    let temp = TempDir::new().unwrap();
    let jit_dir = temp.path().join(".jit");
    fs::create_dir(&jit_dir).unwrap();
    let config_toml = r#"
[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4 }

[namespaces.type]
description = "Issue type"
unique = true
"#;
    fs::write(jit_dir.join("config.toml"), config_toml).unwrap();
    let storage = JsonFileStorage::new(&jit_dir);
    storage.init().unwrap();
    // Materialize rules.toml + the baked schemas, mirroring `jit init`.
    let executor = CommandExecutor::new(storage);
    executor.scaffold_default_rules().unwrap();
    (temp, jit_dir)
}

/// Add `planning`/`breakdown` types to `[type_hierarchy].types` in an existing
/// config.toml.
fn add_planning_type(jit_dir: &std::path::Path) {
    let config_toml = r#"
[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, planning = 3, breakdown = 3, task = 4 }

[namespaces.type]
description = "Issue type"
unique = true
"#;
    fs::write(jit_dir.join("config.toml"), config_toml).unwrap();
}

fn create_typed(jit_dir: &std::path::Path, type_label: &str) -> Vec<String> {
    // A FRESH executor so the OnceLock config/rules caches reflect the edited
    // config.toml and the (possibly) regenerated schema on disk.
    let executor = CommandExecutor::new(JsonFileStorage::new(jit_dir));
    let (_id, warnings) = executor
        .create_issue(
            "issue".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec![type_label.to_string()],
            None,
            false,
        )
        .unwrap();
    warnings
}

#[test]
fn test_new_type_warns_on_write_before_schema_regenerated() {
    // BASELINE (the R5 bug): adding `planning` to config but NOT refreshing the
    // baked schema leaves the write-path `default:type-hierarchy-known` rule
    // reading the frozen enum, so a `type:planning` issue warns.
    let (_temp, jit_dir) = setup_initialized_repo();
    add_planning_type(&jit_dir);

    let warnings = create_typed(&jit_dir, "type:planning");
    assert!(
        warnings.iter().any(|w| w.contains("type-hierarchy-known")),
        "expected a stale type-hierarchy-known warning before regeneration, got {warnings:?}"
    );
}

#[test]
fn test_new_type_passes_write_path_after_schema_regenerated() {
    // DELIVERABLE: regenerating the baked schema from `[type_hierarchy]` makes the
    // write-path rule recognize `planning`, so creating a `type:planning` issue
    // emits NO type-hierarchy-known warning.
    let (_temp, jit_dir) = setup_initialized_repo();
    add_planning_type(&jit_dir);

    // Apply the config change: regenerate the baked schema.
    let admin = CommandExecutor::new(JsonFileStorage::new(&jit_dir));
    let wrote = admin.regenerate_type_hierarchy_schema().unwrap();
    assert!(wrote, "an existing baked layout must be refreshed");

    let warnings = create_typed(&jit_dir, "type:planning");
    assert!(
        !warnings.iter().any(|w| w.contains("type-hierarchy-known")),
        "no type-hierarchy-known warning after regeneration, got {warnings:?}"
    );

    // The pre-existing types still validate cleanly too.
    let warnings = create_typed(&jit_dir, "type:epic");
    assert!(
        !warnings.iter().any(|w| w.contains("type-hierarchy-known")),
        "existing types must keep passing, got {warnings:?}"
    );
}

#[test]
fn test_reinit_refreshes_type_hierarchy_schema() {
    // Re-running the scaffold (the idempotent `jit init` apply path) refreshes the
    // baked schema from the edited config even though rules.toml already exists.
    let (_temp, jit_dir) = setup_initialized_repo();
    add_planning_type(&jit_dir);

    let executor = CommandExecutor::new(JsonFileStorage::new(&jit_dir));
    // rules.toml already exists, so scaffold returns false (no clobber) but still
    // regenerates the baked type-hierarchy schema.
    let scaffolded = executor.scaffold_default_rules().unwrap();
    assert!(
        !scaffolded,
        "re-scaffold must not clobber an existing rules.toml"
    );

    let schema = fs::read_to_string(
        jit_dir
            .join("schemas")
            .join("default-type-hierarchy-known.json"),
    )
    .unwrap();
    assert!(
        schema.contains("planning") && schema.contains("breakdown"),
        "re-init must bake the newly-declared types into the schema: {schema}"
    );

    let warnings = create_typed(&jit_dir, "type:breakdown");
    assert!(
        !warnings.iter().any(|w| w.contains("type-hierarchy-known")),
        "no warning for a declared type after re-init, got {warnings:?}"
    );
}
