//! Integration tests for the --schema flag
//!
//! Note: This test uses assert_cmd's `cargo_bin` API which is deprecated in 2.x
//! in favor of a macro-based approach that doesn't exist yet in stable releases.
//! The warnings are expected and acceptable for test code until assert_cmd 3.x.
//!
//! See: https://github.com/assert-rs/assert_cmd/issues/180

#![allow(deprecated)]

use assert_cmd::prelude::*;
use predicates::prelude::*;
use serde_json::Value;
use std::process::Command;

// Helper macro from assert_cmd docs
// Suppresses deprecation warning as cargo_bin is the documented approach
#[allow(deprecated)]
macro_rules! cmd {
    () => {
        Command::cargo_bin(env!("CARGO_PKG_NAME")).unwrap()
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
    assert_eq!(parsed["version"], "0.2.0");
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
    assert!(commands.contains_key("status"));
    assert!(commands.contains_key("validate"));
}

#[test]
fn test_schema_issue_create_details() {
    let output = cmd!().arg("--schema").output().unwrap();
    let json_str = String::from_utf8(output.stdout).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();

    let create = &parsed["commands"]["issue"]["subcommands"]["create"];

    assert_eq!(create["description"], "Create a new issue");

    // Check args (issue create has no positional args)
    let args = create["args"].as_array().unwrap();
    assert_eq!(args.len(), 0, "issue create should have no positional args");

    // Check flags (title, priority are flags with automatic schema generation)
    let flags = create["flags"].as_array().unwrap();
    assert!(flags.iter().any(|f| f["name"] == "title"));
    assert!(flags.iter().any(|f| f["name"] == "priority"));
    assert!(flags.iter().any(|f| f["name"] == "json"));
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
