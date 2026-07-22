//! The `type-hierarchy-known` default rule derives its enum from captured
//! configuration, not from a stale baked schema projection.

use jit::commands::CommandExecutor;
use jit::domain::Priority;
use jit::hierarchy_templates::HierarchyTemplate;
use jit::storage::JsonFileStorage;
use std::fs;
use tempfile::TempDir;

fn executor(jit_dir: &std::path::Path) -> CommandExecutor<JsonFileStorage> {
    let layout =
        jit::storage::discover_repository_layout(jit_dir.parent().unwrap(), jit_dir).unwrap();
    CommandExecutor::new(JsonFileStorage::new(jit_dir)).with_layout(layout)
}

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
    let layout = jit::storage::discover_repository_layout(temp.path(), &jit_dir).unwrap();
    CommandExecutor::new(JsonFileStorage::new(&jit_dir))
        .with_layout(layout)
        .initialize_fresh_repository(temp.path(), &HierarchyTemplate::default(), None)
        .unwrap();
    (temp, jit_dir)
}

fn add_planning_type(jit_dir: &std::path::Path) {
    fs::write(
        jit_dir.join("config.toml"),
        r#"
[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, planning = 3, breakdown = 3, task = 4 }

[namespaces.type]
description = "Issue type"
unique = true
"#,
    )
    .unwrap();
}

fn create_typed(jit_dir: &std::path::Path, type_label: &str) -> Vec<String> {
    executor(jit_dir)
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
        .unwrap()
        .1
}

#[test]
fn test_new_type_passes_without_schema_regeneration() {
    let (_temp, jit_dir) = setup_initialized_repo();
    add_planning_type(&jit_dir);

    let stale =
        fs::read_to_string(jit_dir.join("schemas/default-type-hierarchy-known.json")).unwrap();
    assert!(!stale.contains("planning"));
    assert!(!create_typed(&jit_dir, "type:planning")
        .iter()
        .any(|warning| warning.contains("type-hierarchy-known")));
    assert!(!create_typed(&jit_dir, "type:epic")
        .iter()
        .any(|warning| warning.contains("type-hierarchy-known")));
}

#[test]
fn test_unknown_type_still_warns() {
    let (_temp, jit_dir) = setup_initialized_repo();
    assert!(create_typed(&jit_dir, "type:nonsense")
        .iter()
        .any(|warning| warning.contains("type-hierarchy-known")));
}
