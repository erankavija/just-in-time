use jit::schema::CommandSchema;
use serde_json::Value;

#[test]
fn test_schema_has_version() {
    let schema = CommandSchema::generate();
    assert_eq!(schema.version, "0.2.0");
}

#[test]
fn test_schema_has_all_top_level_commands() {
    let schema = CommandSchema::generate();

    // Top-level commands
    assert!(schema.commands.contains_key("init"));
    assert!(schema.commands.contains_key("issue"));
    assert!(schema.commands.contains_key("dep"));
    assert!(schema.commands.contains_key("gate"));
    assert!(schema.commands.contains_key("registry"));
    assert!(schema.commands.contains_key("events"));
    assert!(schema.commands.contains_key("graph"));
    assert!(schema.commands.contains_key("query"));
    assert!(schema.commands.contains_key("status"));
    assert!(schema.commands.contains_key("validate"));
}

#[test]
fn test_schema_issue_subcommands() {
    let schema = CommandSchema::generate();
    let issue_cmd = schema.commands.get("issue").unwrap();

    let subcommands = issue_cmd.subcommands.as_ref().unwrap();
    assert!(subcommands.contains_key("create"));
    assert!(subcommands.contains_key("list"));
    assert!(subcommands.contains_key("show"));
    assert!(subcommands.contains_key("update"));
    assert!(subcommands.contains_key("delete"));
    assert!(subcommands.contains_key("claim"));
    assert!(subcommands.contains_key("unassign"));
}

#[test]
fn test_schema_create_issue_args() {
    let schema = CommandSchema::generate();
    let issue_cmd = schema.commands.get("issue").unwrap();
    let create_cmd = issue_cmd
        .subcommands
        .as_ref()
        .unwrap()
        .get("create")
        .unwrap();

    // Should have title, description, priority, gate flags (not args, since they use --flag syntax)
    let flags = &create_cmd.flags;
    assert!(flags.iter().any(|f| f.name == "title"));
    assert!(flags.iter().any(|f| f.name == "description"));
    assert!(flags.iter().any(|f| f.name == "priority"));
    assert!(flags.iter().any(|f| f.name == "gate"));

    // Title flag should exist
    let title_flag = flags.iter().find(|f| f.name == "title").unwrap();
    assert_eq!(title_flag.flag_type, "string");

    // Priority flag should exist
    let priority_flag = flags.iter().find(|f| f.name == "priority").unwrap();
    assert_eq!(priority_flag.flag_type, "string");
}

#[test]
fn test_schema_has_json_flags() {
    let schema = CommandSchema::generate();

    // Check that commands with --json flag have it in flags
    let issue_list = schema
        .commands
        .get("issue")
        .unwrap()
        .subcommands
        .as_ref()
        .unwrap()
        .get("list")
        .unwrap();
    assert!(issue_list.flags.iter().any(|f| f.name == "json"));

    let status = schema.commands.get("status").unwrap();
    assert!(status.flags.iter().any(|f| f.name == "json"));
}

#[test]
fn test_schema_serializes_to_valid_json() {
    let schema = CommandSchema::generate();
    let json = serde_json::to_string_pretty(&schema).unwrap();

    // Should parse back
    let _parsed: Value = serde_json::from_str(&json).unwrap();

    // Should contain expected fields
    assert!(json.contains("\"version\""));
    assert!(json.contains("\"commands\""));
    assert!(json.contains("\"types\""));
}

#[test]
fn test_schema_type_definitions() {
    let schema = CommandSchema::generate();

    // Should define core types
    assert!(schema.types.contains_key("Issue"));
    assert!(schema.types.contains_key("State"));
    assert!(schema.types.contains_key("Priority"));
    assert!(schema.types.contains_key("ErrorResponse"));

    // State should be an enum with variants
    let state = schema.types.get("State").unwrap();
    let state_obj = state.as_object().unwrap();
    assert!(state_obj.contains_key("enum"));

    let variants = state_obj.get("enum").unwrap().as_array().unwrap();
    assert!(!variants.is_empty());
}

#[test]
fn test_schema_exit_codes_documented() {
    let schema = CommandSchema::generate();

    // Should have exit_codes field
    assert!(!schema.exit_codes.is_empty());

    // Should have standard exit codes
    assert!(schema.exit_codes.iter().any(|e| e.code == 0));
    assert!(schema.exit_codes.iter().any(|e| e.code == 1));
    assert!(schema.exit_codes.iter().any(|e| e.code == 3));
    assert!(schema.exit_codes.iter().any(|e| e.code == 4));
}
