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
default_type = "task"
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
    assert_eq!(validation.default_type, Some("task".to_string()));
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
fn test_orphan_warning_is_unconditional_in_default_ruleset() {
    // The orphan-leaf / strategic-consistency graph warnings are now UNCONDITIONAL
    // built-in default rules (the former `warn_*` toggles were removed; MF1). With
    // no rules.toml, the in-memory defaults always emit them, so an orphaned task
    // warns. A repo that wants them silenced edits rules.toml.
    let temp_dir = TempDir::new().unwrap();
    let storage = JsonFileStorage::new(temp_dir.path());
    storage.init().unwrap();

    // Remove the scaffolded rules.toml so we exercise the in-memory defaults.
    let rules_path = storage.root().join("rules.toml");
    if rules_path.exists() {
        std::fs::remove_file(&rules_path).unwrap();
    }

    let executor = CommandExecutor::new(storage);

    // Create a task without parent labels (orphaned at a leaf level).
    let (issue_id, _) = executor
        .create_issue(
            "Orphaned task".to_string(),
            "No parent labels".to_string(),
            jit::Priority::Normal,
            Vec::new(),
            vec!["type:task".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let issues = executor.storage().list_issues().unwrap();
    let warnings: Vec<_> = executor
        .evaluate_graph_rules(&issues)
        .unwrap()
        .into_iter()
        .filter(|gf| gf.issue_id.as_deref() == Some(issue_id.as_str()))
        .collect();
    assert!(
        !warnings.is_empty(),
        "Expected an orphan warning from the unconditional default graph rules"
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
