//! Integration tests (jit:af4c901a): the `type-hierarchy-known` default rule
//! derives its allowed-type enum from `[type_hierarchy]` in memory at LOAD, so a
//! config-declared type validates without regenerating
//! `.jit/schemas/default-type-hierarchy-known.json`.
//!
//! Exercises the full disk-based path: a real `.jit/` scaffolded by
//! `scaffold_default_rules` (so `rules.toml` + the baked schema exist), a
//! `config.toml` edited to add a new type, then `CommandExecutor::create_issue`
//! against `JsonFileStorage`. `type-hierarchy-known` (origin = "default") is
//! `enforce = false`, so an unknown type never blocks the write — it would
//! surface as a WARNING; the deliverable is that a declared type emits no warning
//! even while the on-disk projection is stale, because the registry (not the
//! baked file) is the authority. The projection is a write-through copy that
//! `refresh_default_schema_projections` republishes on re-init / config writes.

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
            None,
            false,
        )
        .unwrap();
    warnings
}

#[test]
fn test_new_type_passes_without_schema_regeneration() {
    // DELIVERABLE (af4c901a): adding `planning` to `[type_hierarchy]` and NOT
    // refreshing the baked schema still validates a `type:planning` issue clean —
    // `type-hierarchy-known` derives its enum from config in memory at load, so
    // the stale on-disk projection does not decide validation.
    let (_temp, jit_dir) = setup_initialized_repo();
    add_planning_type(&jit_dir);

    // The on-disk projection is still stale (no regeneration step ran).
    let schema = fs::read_to_string(
        jit_dir
            .join("schemas")
            .join("default-type-hierarchy-known.json"),
    )
    .unwrap();
    assert!(
        !schema.contains("planning"),
        "precondition: the baked projection is still stale: {schema}"
    );

    let warnings = create_typed(&jit_dir, "type:planning");
    assert!(
        !warnings.iter().any(|w| w.contains("type-hierarchy-known")),
        "a declared type must validate at load without regenerating, got {warnings:?}"
    );

    // The pre-existing types still validate cleanly too.
    let warnings = create_typed(&jit_dir, "type:epic");
    assert!(
        !warnings.iter().any(|w| w.contains("type-hierarchy-known")),
        "existing types must keep passing, got {warnings:?}"
    );
}

#[test]
fn test_unknown_type_still_warns() {
    // Boundary: a type NOT declared in `[type_hierarchy]` still surfaces the
    // `type-hierarchy-known` warning (derive-at-load did not blanket-disable it).
    let (_temp, jit_dir) = setup_initialized_repo();

    let warnings = create_typed(&jit_dir, "type:nonsense");
    assert!(
        warnings.iter().any(|w| w.contains("type-hierarchy-known")),
        "an undeclared type must still warn, got {warnings:?}"
    );
}

#[test]
fn test_reinit_refreshes_type_hierarchy_projection() {
    // Re-running the scaffold (the idempotent `jit init` apply path) republishes
    // the baked schema projection from the edited config even though rules.toml
    // already exists. Validation already passed before this (see above); the
    // refresh keeps the file current for external consumers.
    let (_temp, jit_dir) = setup_initialized_repo();
    add_planning_type(&jit_dir);

    let executor = CommandExecutor::new(JsonFileStorage::new(&jit_dir));
    // rules.toml already exists, so scaffold returns false (no clobber) but still
    // republishes the default-schema projections.
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
        "re-init must republish the newly-declared types into the projection: {schema}"
    );
}

#[test]
fn test_refresh_republishes_all_default_projections() {
    // `refresh_default_schema_projections` rewrites EVERY default-origin schema
    // file (label-format, namespace-registry, type-hierarchy-known) from config,
    // returning the names it wrote.
    let (_temp, jit_dir) = setup_initialized_repo();
    add_planning_type(&jit_dir);

    let admin = CommandExecutor::new(JsonFileStorage::new(&jit_dir));
    let mut written = admin.refresh_default_schema_projections().unwrap();
    written.sort();
    assert_eq!(
        written,
        vec![
            "default-label-format.json".to_string(),
            "default-namespace-registry.json".to_string(),
            "default-type-hierarchy-known.json".to_string(),
        ],
        "all three default projections are republished"
    );

    let schema = fs::read_to_string(
        jit_dir
            .join("schemas")
            .join("default-type-hierarchy-known.json"),
    )
    .unwrap();
    assert!(
        schema.contains("planning"),
        "the republished type-hierarchy projection tracks config: {schema}"
    );
}

#[test]
fn test_refresh_is_noop_without_materialized_schemas() {
    // A repo whose `rules.toml`/`schemas/` were never materialized (read path
    // builds defaults in memory) has nothing to republish: refresh writes nothing.
    std::env::set_var("JIT_TEST_MODE", "1");
    let temp = TempDir::new().unwrap();
    let jit_dir = temp.path().join(".jit");
    fs::create_dir(&jit_dir).unwrap();
    fs::write(
        jit_dir.join("config.toml"),
        "[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
    )
    .unwrap();
    let storage = JsonFileStorage::new(&jit_dir);
    storage.init().unwrap();

    let admin = CommandExecutor::new(JsonFileStorage::new(&jit_dir));
    let written = admin.refresh_default_schema_projections().unwrap();
    assert!(written.is_empty(), "no baked layout => nothing republished");
    assert!(!jit_dir.join("schemas").exists());
}
