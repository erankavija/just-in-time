use jit::config::JitConfig;
use jit::storage::{IssueStore, JsonFileStorage};
use jit::CommandExecutor;
use tempfile::TempDir;

#[test]
fn test_parse_valid_config() {
    let config_toml = r#"
[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4 }

[type_hierarchy.label_associations]
epic = "epic"
milestone = "milestone"

[validation]
strictness = "loose"
warn_orphaned_leaves = true
warn_strategic_consistency = true
"#;

    let config: JitConfig = toml::from_str(config_toml).unwrap();

    let hierarchy = config.type_hierarchy.unwrap();
    assert_eq!(hierarchy.types.get("milestone"), Some(&1));
    assert_eq!(hierarchy.types.get("epic"), Some(&2));
    assert_eq!(hierarchy.types.get("story"), Some(&3));
    assert_eq!(hierarchy.types.get("task"), Some(&4));

    let associations = hierarchy.label_associations.unwrap();
    assert_eq!(associations.get("epic"), Some(&"epic".to_string()));
    assert_eq!(
        associations.get("milestone"),
        Some(&"milestone".to_string())
    );

    let validation = config.validation.unwrap();
    assert_eq!(validation.strictness, Some("loose".to_string()));
    assert_eq!(validation.warn_orphaned_leaves, Some(true));
    assert_eq!(validation.warn_strategic_consistency, Some(true));
}

#[test]
fn test_handle_missing_config_file() {
    let temp_dir = TempDir::new().unwrap();
    let storage = JsonFileStorage::new(temp_dir.path());

    // Config file doesn't exist - should fall back to defaults
    let config = JitConfig::load(storage.root()).unwrap();

    // Should be None (uses defaults)
    assert!(config.type_hierarchy.is_none());
    assert!(config.validation.is_none());
}

#[test]
fn test_config_with_custom_hierarchy() {
    let temp_dir = TempDir::new().unwrap();
    let storage = JsonFileStorage::new(temp_dir.path());

    // Write custom config
    let config_toml = r#"
[type_hierarchy]
types = { theme = 1, feature = 2, task = 3 }

[type_hierarchy.label_associations]
theme = "epic"
feature = "epic"
"#;
    std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

    // Load config
    let config = JitConfig::load(storage.root()).unwrap();
    let hierarchy = config.type_hierarchy.unwrap();

    assert_eq!(hierarchy.types.get("theme"), Some(&1));
    assert_eq!(hierarchy.types.get("feature"), Some(&2));
    assert_eq!(hierarchy.types.get("task"), Some(&3));
    assert!(!hierarchy.types.contains_key("milestone"));
    assert!(!hierarchy.types.contains_key("epic"));
}

#[test]
fn test_warning_toggles_respect_config() {
    let temp_dir = TempDir::new().unwrap();
    let storage = JsonFileStorage::new(temp_dir.path());
    storage.init().unwrap();

    // Write config with warnings disabled
    let config_toml = r#"
[validation]
warn_orphaned_leaves = false
warn_strategic_consistency = false
"#;
    std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

    let executor = CommandExecutor::new(storage);

    // Create a task without parent labels (would normally warn)
    let (issue_id, _) = executor
        .create_issue(
            "Orphaned task".to_string(),
            "No parent labels".to_string(),
            jit::Priority::Normal,
            Vec::new(),
            vec!["type:task".to_string()],
        )
        .unwrap();

    // Check warnings - should be empty because toggles are off
    let warnings = executor.check_warnings(&issue_id).unwrap();
    assert!(
        warnings.is_empty(),
        "Expected no warnings when toggles disabled, got: {:?}",
        warnings
    );
}

#[test]
fn test_malformed_config_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let storage = JsonFileStorage::new(temp_dir.path());

    // Write malformed TOML
    let bad_toml = r#"
[type_hierarchy
types = { broken syntax
"#;
    std::fs::write(storage.root().join("config.toml"), bad_toml).unwrap();

    // Should return error
    let result = JitConfig::load(storage.root());
    assert!(result.is_err());
}
