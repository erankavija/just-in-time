//! Verify field projection and multi-id support for `jit issue show`:
//! - `--field <name>` prints a single top-level field as plain text
//! - `--fields a --fields b` prints those fields as a single compact JSON object
//! - multiple ids with `--json` return the list envelope `{count, issues}`
//! - unknown fields and mutually-exclusive / multi-id projection combos error

use assert_cmd::prelude::*;
use jit::test_cli::JitCli;

#[test]
fn test_field_prints_scalar_as_plain_text() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Ready issue", "--description", "Body"])
        .unwrap();
    temp.run(["issue", "update", &id, "--state", "ready"])
        .unwrap();

    let text = temp
        .run(["issue", "show", &id, "--field", "state"])
        .unwrap();
    // Plain text, not JSON-quoted.
    assert_eq!(text.trim(), "ready", "got: {text:?}");
    assert!(
        !text.contains('"'),
        "field output must be unquoted: {text:?}"
    );
}

#[test]
fn test_field_string_is_raw_not_quoted() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "My Title", "--description", "Body"])
        .unwrap();

    let text = temp
        .run(["issue", "show", &id, "--field", "title"])
        .unwrap();
    assert_eq!(text.trim(), "My Title", "got: {text:?}");
}

#[test]
fn test_field_array_falls_back_to_compact_json() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Labeled", "--description", "Body"])
        .unwrap();
    temp.run(["issue", "update", &id, "--label", "type:task"])
        .unwrap();

    let text = temp
        .run(["issue", "show", &id, "--field", "labels"])
        .unwrap();
    let trimmed = text.trim();
    // Array field -> compact JSON array.
    assert!(
        trimmed.starts_with('[') && trimmed.ends_with(']'),
        "got: {text:?}"
    );
    let parsed: serde_json::Value = serde_json::from_str(trimmed).unwrap();
    assert!(parsed.as_array().unwrap().iter().any(|l| l == "type:task"));
}

#[test]
fn test_fields_prints_compact_json_object() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Both", "--description", "Body"])
        .unwrap();
    temp.run(["issue", "update", &id, "--state", "ready"])
        .unwrap();

    let text = temp
        .run([
            "issue", "show", &id, "--fields", "state", "--fields", "title",
        ])
        .unwrap();
    let trimmed = text.trim();
    // Compact (no spaces after separators), single object.
    assert_eq!(
        trimmed, r#"{"state":"ready","title":"Both"}"#,
        "got: {text:?}"
    );
}

#[test]
fn test_two_ids_json_returns_envelope() {
    let temp = JitCli::initialised().unwrap();
    let a = temp
        .create_issue(["--title", "First", "--description", "Body"])
        .unwrap();
    let b = temp
        .create_issue(["--title", "Second", "--description", "Body"])
        .unwrap();

    let out = temp.run(["issue", "show", &a, &b, "--json"]).unwrap();
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    let arr = json["issues"]
        .as_array()
        .expect("two-id --json must wrap issues in the list envelope");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["id"].as_str(), Some(a.as_str()));
    assert_eq!(arr[1]["id"].as_str(), Some(b.as_str()));
    assert_eq!(arr[0]["title"].as_str(), Some("First"));
    assert_eq!(arr[1]["title"].as_str(), Some("Second"));
}

#[test]
fn test_single_id_json_stays_object() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Solo", "--description", "Body"])
        .unwrap();

    let out = temp.run(["issue", "show", &id, "--json"]).unwrap();
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(json.is_object(), "single-id --json must stay an object");
    assert_eq!(json["id"].as_str(), Some(id.as_str()));
}

#[test]
fn test_unknown_field_errors() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "X", "--description", "Body"])
        .unwrap();

    let assert = temp
        .command(["issue", "show", &id, "--field", "nope", "--json"])
        .assert()
        .failure();
    let json: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn test_unknown_field_in_fields_list_errors() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "X", "--description", "Body"])
        .unwrap();

    let assert = temp
        .command([
            "issue", "show", &id, "--fields", "state", "--fields", "bogus", "--json",
        ])
        .assert()
        .failure();
    let json: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn test_field_and_fields_mutually_exclusive() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "X", "--description", "Body"])
        .unwrap();

    let assert = temp
        .command([
            "issue", "show", &id, "--field", "state", "--fields", "state",
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("cannot be used with"), "{stderr}");
}

#[test]
fn test_projection_with_multiple_ids_errors() {
    let temp = JitCli::initialised().unwrap();
    let a = temp
        .create_issue(["--title", "First", "--description", "Body"])
        .unwrap();
    let b = temp
        .create_issue(["--title", "Second", "--description", "Body"])
        .unwrap();

    let assert = temp
        .command(["issue", "show", &a, &b, "--field", "state", "--json"])
        .assert()
        .failure();
    let json: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}
