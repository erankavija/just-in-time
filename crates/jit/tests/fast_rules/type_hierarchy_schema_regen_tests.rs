//! The `type-hierarchy-known` default rule derives its enum from captured
//! configuration, not from a stale baked schema projection.

use jit::domain::Priority;
use jit::test_utils::{file_executor, setup_test_repo_with_config};
use std::fs;

const HIERARCHY_CONFIG: &str = r#"
[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4 }

[namespaces.type]
description = "Issue type"
unique = true
"#;

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

fn create_typed(worktree: &std::path::Path, type_label: &str) -> Vec<String> {
    file_executor(worktree)
        .unwrap()
        .issues()
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
        .findings
        .into_iter()
        .map(|finding| finding.rule)
        .collect()
}

#[test]
fn test_new_type_passes_without_schema_regeneration() {
    let (temp, _) = setup_test_repo_with_config(HIERARCHY_CONFIG).unwrap();
    let jit_dir = temp.path().join(".jit");
    add_planning_type(&jit_dir);

    let stale =
        fs::read_to_string(jit_dir.join("schemas/default-type-hierarchy-known.json")).unwrap();
    assert!(!stale.contains("planning"));
    assert!(!create_typed(temp.path(), "type:planning")
        .iter()
        .any(|rule| rule == "type-hierarchy-known"));
    assert!(!create_typed(temp.path(), "type:epic")
        .iter()
        .any(|rule| rule == "type-hierarchy-known"));
}

#[test]
fn test_unknown_type_still_warns() {
    let (temp, _) = setup_test_repo_with_config(HIERARCHY_CONFIG).unwrap();
    assert!(create_typed(temp.path(), "type:nonsense")
        .iter()
        .any(|rule| rule == "type-hierarchy-known"));
}
