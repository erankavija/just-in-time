//! Verify field projection and multi-id support for `jit issue show`:
//! - `--field <name>` prints a single top-level field as plain text
//! - `--fields a,b` prints those fields as a single compact JSON object
//! - multiple ids with `--json` return the list envelope `{count, issues}`
//! - unknown fields and mutually-exclusive / multi-id projection combos error

use assert_cmd::prelude::*;
use jit::test_cli::JitCli;

fn jit_ok(cli: &JitCli, args: &[&str]) -> Vec<u8> {
    let output = cli.command(args).output().unwrap();
    assert!(output.status.success(), "jit failed: {output:?}");
    output.stdout
}

#[test]
fn test_field_prints_scalar_as_plain_text() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Ready issue", "--description", "Body"])
        .unwrap();
    jit_ok(&temp, &["issue", "update", &id, "--state", "ready"]);

    let out = jit_ok(&temp, &["issue", "show", &id, "--field", "state"]);
    let text = String::from_utf8_lossy(&out);
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

    let out = jit_ok(&temp, &["issue", "show", &id, "--field", "title"]);
    let text = String::from_utf8_lossy(&out);
    assert_eq!(text.trim(), "My Title", "got: {text:?}");
}

#[test]
fn test_field_array_falls_back_to_compact_json() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Labeled", "--description", "Body"])
        .unwrap();
    jit_ok(&temp, &["issue", "update", &id, "--label", "type:task"]);

    let out = jit_ok(&temp, &["issue", "show", &id, "--field", "labels"]);
    let text = String::from_utf8_lossy(&out);
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
    jit_ok(&temp, &["issue", "update", &id, "--state", "ready"]);

    let out = jit_ok(&temp, &["issue", "show", &id, "--fields", "state,title"]);
    let text = String::from_utf8_lossy(&out);
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

    let out = jit_ok(&temp, &["issue", "show", &a, &b, "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["count"].as_u64(), Some(2), "envelope carries count");
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

    let out = jit_ok(&temp, &["issue", "show", &id, "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(json.is_object(), "single-id --json must stay an object");
    assert_eq!(json["id"].as_str(), Some(id.as_str()));
}

#[test]
fn test_unknown_field_errors() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "X", "--description", "Body"])
        .unwrap();

    temp.command(["issue", "show", &id, "--field", "nope"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_unknown_field_in_fields_list_errors() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "X", "--description", "Body"])
        .unwrap();

    temp.command(["issue", "show", &id, "--fields", "state,bogus"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_field_and_fields_mutually_exclusive() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "X", "--description", "Body"])
        .unwrap();

    // clap-level conflict -> exit code 2 (usage error).
    temp.command([
        "issue", "show", &id, "--field", "state", "--fields", "state",
    ])
    .assert()
    .failure()
    .code(2);
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

    temp.command(["issue", "show", &a, &b, "--field", "state"])
        .assert()
        .failure()
        .code(2);
}
