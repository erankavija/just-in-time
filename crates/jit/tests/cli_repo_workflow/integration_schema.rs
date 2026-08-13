//! Integration tests for the --schema flag

use assert_cmd::prelude::*;
use predicates::prelude::*;
use serde_json::Value;
use std::process::Command;

// Helper macro using the new cargo_bin! approach
macro_rules! cmd {
    () => {
        Command::new(assert_cmd::cargo::cargo_bin!(env!("CARGO_PKG_NAME")))
    };
}

#[test]
fn test_schema_flag_outputs_json() {
    let mut cmd = cmd!();
    cmd.arg("--schema");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("version"))
        .stdout(predicate::str::contains("commands"))
        .stdout(predicate::str::contains("types"))
        .stdout(predicate::str::contains("exit_codes"));
}

#[test]
fn test_schema_is_valid_json() {
    let output = cmd!().arg("--schema").output().unwrap();
    assert!(output.status.success());

    let json_str = String::from_utf8(output.stdout).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();

    assert!(parsed.is_object());
    assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn test_schema_has_all_commands() {
    let output = cmd!().arg("--schema").output().unwrap();

    let json_str = String::from_utf8(output.stdout).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();

    let commands = parsed["commands"].as_object().unwrap();

    // Top-level commands
    assert!(commands.contains_key("init"));
    assert!(commands.contains_key("issue"));
    assert!(commands.contains_key("dep"));
    assert!(commands.contains_key("gate"));
    assert!(commands.contains_key("profile"));
    assert!(commands.contains_key("status"));
    assert!(commands.contains_key("validate"));
}

#[test]
fn test_schema_exposes_profile_commands_and_typed_outputs() {
    let output = cmd!().arg("--schema").output().unwrap();
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    let profile = parsed["commands"]["profile"]["subcommands"]
        .as_object()
        .unwrap();

    assert_eq!(
        profile
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            "add".to_string(),
            "apply".to_string(),
            "capture".to_string(),
            "diff".to_string(),
            "list".to_string(),
            "pack".to_string(),
            "reconfigure".to_string(),
            "show".to_string(),
            "upgrade".to_string(),
            "validate".to_string(),
        ])
    );
    for command in [
        "list",
        "show",
        "validate",
        "diff",
        "apply",
        "capture",
        "pack",
        "add",
        "reconfigure",
        "upgrade",
    ] {
        assert!(
            profile[command]["output"]["success_schema"].is_object(),
            "profile {command} must expose a success schema"
        );
    }
    for command in ["apply", "reconfigure", "upgrade"] {
        assert!(profile[command]["flags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|flag| flag["name"] == "dry-run"));
        for name in ["set", "values-file"] {
            assert!(
                profile[command]["flags"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|flag| flag["name"] == name),
                "profile {command} must expose --{name}"
            );
        }
    }
}

#[test]
fn test_schema_exposes_only_repeatable_profile_selectors() {
    let output = cmd!().arg("--schema").output().unwrap();
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    let profile = &parsed["commands"]["profile"]["subcommands"];

    for command in ["show", "apply", "reconfigure", "upgrade"] {
        let flags = profile[command]["flags"].as_array().unwrap();
        let selector = flags
            .iter()
            .find(|flag| flag["name"] == "profile")
            .unwrap_or_else(|| panic!("profile {command} must expose --profile"));
        assert_eq!(selector["type"], "array<string>");
        assert_eq!(selector["required"], true);
        assert!(!flags.iter().any(|flag| flag["name"] == "from"));
        assert!(profile[command]["args"].as_array().unwrap().is_empty());
    }

    let show_schema = &profile["show"]["output"]["success_schema"];
    let show_properties = show_schema
        .get("properties")
        .or_else(|| show_schema.pointer("/definitions/ProfileShowResult/properties"))
        .expect("profile show collection properties");
    assert!(show_properties["count"].is_object());
    assert_eq!(show_properties["profiles"]["type"], "array");
    assert!(show_properties.get("manifest").is_none());

    let init_flags = parsed["commands"]["init"]["flags"].as_array().unwrap();
    let selector = init_flags
        .iter()
        .find(|flag| flag["name"] == "profile")
        .expect("init must expose --profile");
    assert_eq!(selector["type"], "array<string>");
    assert_eq!(selector["required"], false);
    assert!(!init_flags.iter().any(|flag| flag["name"] == "from"));
    for name in ["set", "values-file"] {
        assert!(
            init_flags.iter().any(|flag| flag["name"] == name),
            "init must expose --{name}"
        );
    }
}

#[test]
fn test_schema_exposes_only_dependency_aware_archive_family() {
    let output = cmd!().arg("--schema").output().unwrap();
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();

    let document_commands = parsed["commands"]["doc"]["subcommands"]
        .as_object()
        .unwrap();
    assert!(
        !document_commands.contains_key("archive"),
        "the retired document subcommand must not remain in the schema"
    );

    let archive_commands = parsed["commands"]["archive"]["subcommands"]
        .as_object()
        .unwrap();
    assert_eq!(
        archive_commands
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            "candidates".to_string(),
            "container".to_string(),
            "document".to_string(),
        ])
    );
}

#[test]
fn test_schema_issue_create_details() {
    let output = cmd!().arg("--schema").output().unwrap();
    let json_str = String::from_utf8(output.stdout).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();

    let create = &parsed["commands"]["issue"]["subcommands"]["create"];

    assert_eq!(create["description"], "Create a new issue");

    // issue create now has one positional arg: the title (REQ-01).
    let args = create["args"].as_array().unwrap();
    assert_eq!(
        args.len(),
        1,
        "issue create should have exactly one positional arg (title)"
    );
    assert!(
        args.iter().any(|a| a["name"] == "positional_title"),
        "positional title arg must be named 'positional_title' in schema"
    );

    // Check flags: --title preserved alongside positional; --type added (REQ-02).
    let flags = create["flags"].as_array().unwrap();
    assert!(flags.iter().any(|f| f["name"] == "title"));
    assert!(flags.iter().any(|f| f["name"] == "priority"));
    assert!(flags.iter().any(|f| f["name"] == "json"));
    assert!(
        flags
            .iter()
            .any(|f| f["name"] == "description-file" || f["name"] == "description_file"),
        "issue create must expose --description-file in --schema"
    );
    assert!(
        flags
            .iter()
            .any(|f| f["name"] == "type" || f["name"] == "issue_type"),
        "issue create must expose a --type flag in --schema"
    );
}

/// The `--content-format` flag must surface in the auto-generated `--schema`
/// for BOTH `issue create` and `issue update` so the MCP server (which derives
/// its tools from this schema, DR §9.3) exposes the new authorable surface.
#[test]
fn test_schema_issue_create_and_update_expose_content_format() {
    let output = cmd!().arg("--schema").output().unwrap();
    let json_str = String::from_utf8(output.stdout).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();

    let create_flags = parsed["commands"]["issue"]["subcommands"]["create"]["flags"]
        .as_array()
        .unwrap();
    assert!(
        create_flags
            .iter()
            .any(|f| f["name"] == "content-format" || f["name"] == "content_format"),
        "issue create must expose a content-format flag in --schema, got: {create_flags:?}"
    );

    let update_flags = parsed["commands"]["issue"]["subcommands"]["update"]["flags"]
        .as_array()
        .unwrap();
    assert!(
        update_flags
            .iter()
            .any(|f| f["name"] == "content-format" || f["name"] == "content_format"),
        "issue update must expose a content-format flag in --schema, got: {update_flags:?}"
    );
}

#[test]
fn test_schema_includes_exit_codes() {
    let output = cmd!().arg("--schema").output().unwrap();

    let json_str = String::from_utf8(output.stdout).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();

    let exit_codes = parsed["exit_codes"].as_array().unwrap();

    assert!(exit_codes.iter().any(|e| e["code"] == 0));
    assert!(exit_codes.iter().any(|e| e["code"] == 1));
    assert!(exit_codes.iter().any(|e| e["code"] == 3));
    assert!(exit_codes.iter().any(|e| e["code"] == 4));
}

#[test]
fn test_schema_includes_type_definitions() {
    let output = cmd!().arg("--schema").output().unwrap();
    let json_str = String::from_utf8(output.stdout).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();

    let types = parsed["types"].as_object().unwrap();

    assert!(types.contains_key("State"));
    assert!(types.contains_key("Priority"));
    assert!(types.contains_key("Issue"));
    assert!(types.contains_key("ErrorResponse"));

    // Check State enum
    let state = &types["State"];
    assert_eq!(state["type"], "enum");
    let state_values = state["enum"].as_array().unwrap();
    assert!(state_values.iter().any(|v| v == "backlog"));
    assert!(state_values.iter().any(|v| v == "ready"));
    assert!(state_values.iter().any(|v| v == "done"));
}

#[test]
fn test_no_command_with_schema_works() {
    // --schema should work without a subcommand
    cmd!().arg("--schema").assert().success();
}
