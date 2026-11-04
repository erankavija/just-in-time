use jit::schema::CommandSchema;
use serde_json::Value;

#[test]
fn test_schema_has_version() {
    let schema = CommandSchema::generate();
    assert_eq!(schema.version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn test_schema_has_all_top_level_commands() {
    let schema = CommandSchema::generate();

    // Top-level commands
    assert!(schema.commands.contains_key("init"));
    assert!(schema.commands.contains_key("issue"));
    assert!(schema.commands.contains_key("dep"));
    assert!(schema.commands.contains_key("gate"));
    assert!(schema.commands.contains_key("events"));
    assert!(schema.commands.contains_key("graph"));
    assert!(schema.commands.contains_key("query"));
    assert!(schema.commands.contains_key("version"));
    assert!(schema.commands.contains_key("status"));
    assert!(schema.commands.contains_key("validate"));
}

#[test]
fn test_schema_issue_subcommands() {
    let schema = CommandSchema::generate();
    let issue_cmd = schema.commands.get("issue").unwrap();

    let subcommands = issue_cmd.subcommands.as_ref().unwrap();
    assert!(subcommands.contains_key("create"));
    assert!(subcommands.contains_key("show"));
    assert!(subcommands.contains_key("update"));
    assert!(subcommands.contains_key("delete"));
    assert!(subcommands.contains_key("claim"));
    assert!(subcommands.contains_key("unassign"));
    // Note: 'list' was removed - use 'jit query all' instead
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
    fn json_flag_paths(
        prefix: &str,
        commands: &std::collections::HashMap<String, jit::schema::Command>,
    ) -> Vec<String> {
        commands
            .iter()
            .flat_map(|(name, command)| {
                let path = format!("{prefix}{name}");
                let own = command
                    .flags
                    .iter()
                    .any(|f| f.name == "json")
                    .then(|| path.clone());
                let nested = command
                    .subcommands
                    .as_ref()
                    .map(|children| json_flag_paths(&format!("{path} "), children))
                    .unwrap_or_default();
                own.into_iter().chain(nested)
            })
            .collect()
    }

    let schema = CommandSchema::generate();

    // `--json` is one global option, inherited by every command rather than
    // declared on any of them.
    assert_eq!(
        schema
            .global_options
            .iter()
            .filter(|f| f.name == "json")
            .count(),
        1
    );
    assert_eq!(json_flag_paths("", &schema.commands), Vec::<String>::new());
}

#[test]
fn test_schema_has_standardized_json_output_types() {
    let schema = CommandSchema::generate();

    let gate_list = schema
        .commands
        .get("gate")
        .unwrap()
        .subcommands
        .as_ref()
        .unwrap()
        .get("list")
        .unwrap();
    assert_eq!(
        gate_list.output.as_ref().unwrap().success,
        jit::schema::OutputSchema::of::<jit::output::GateListResponse>().success
    );

    let label_namespaces = schema
        .commands
        .get("label")
        .unwrap()
        .subcommands
        .as_ref()
        .unwrap()
        .get("namespaces")
        .unwrap();
    assert_eq!(
        label_namespaces.output.as_ref().unwrap().success,
        "NamespacesResponse"
    );

    let search = schema.commands.get("search").unwrap();
    assert_eq!(search.output.as_ref().unwrap().success, "SearchResponse");

    let worktree_list = schema
        .commands
        .get("worktree")
        .unwrap()
        .subcommands
        .as_ref()
        .unwrap()
        .get("list")
        .unwrap();
    assert_eq!(
        worktree_list.output.as_ref().unwrap().success,
        "WorktreeListResponse"
    );
}

#[test]
fn test_schema_declares_list_envelope_outputs_for_count_bearing_commands() {
    let schema = CommandSchema::generate();
    let declared = |path: &[&str]| {
        let (head, rest) = path.split_first().unwrap();
        let command = rest
            .iter()
            .fold(schema.commands.get(*head).unwrap(), |command, name| {
                command
                    .subcommands
                    .as_ref()
                    .and_then(|subcommands| subcommands.get(*name))
                    .unwrap_or_else(|| panic!("command {path:?} is missing from the schema"))
            });
        command.output.as_ref().unwrap()
    };

    for (path, response) in [
        (&["gate", "preset", "list"][..], "GatePresetListResponse"),
        (&["events", "tail"], "EventListResponse"),
        (&["events", "query"], "EventListResponse"),
        (&["label", "values"], "LabelValuesResponse"),
        (&["claim", "status"], "LeaseListResponse"),
        (&["claim", "list"], "LeaseListResponse"),
        (&["graph", "rdeps"], "GraphDownstreamResponse"),
        (&["rdeps"], "GraphDownstreamResponse"),
    ] {
        let output = declared(path);
        assert_eq!(output.success, response, "output of {path:?}");
        assert!(
            output.success_schema.is_some(),
            "{path:?} must publish a success schema"
        );
    }
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
