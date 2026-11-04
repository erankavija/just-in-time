//! Verify `jit issue show --summary` returns a compact response without the
//! description field, while the default behaviour is unchanged.

use assert_cmd::prelude::*;
use jit::test_cli::JitCli;

#[test]
fn test_show_default_still_returns_full_description() {
    let description = "The full description text MUST appear here.";
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "An issue", "--description", description])
        .unwrap();

    let output = temp
        .command(["issue", "show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        json["description"].as_str(),
        Some(description),
        "default `issue show --json` must keep full description; got: {}",
        json
    );
}

#[test]
fn test_show_summary_omits_description() {
    let description = "B".repeat(4000);
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "An issue", "--description", &description])
        .unwrap();

    let output = temp
        .command(["issue", "show", &id, "--summary", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let data = &json;
    assert!(
        data.get("description").is_none(),
        "--summary must omit description; got: {}",
        data
    );

    let serialized = serde_json::to_string(&json).unwrap();
    assert!(
        !serialized.contains(&description),
        "description content leaked into summary response"
    );
}

#[test]
fn test_show_summary_exposes_gates_array() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "An issue", "--description", "short"])
        .unwrap();

    // Add a gate so the projected `gates` array is non-empty.
    temp.command([
        "gate",
        "define",
        "manual-gate",
        "--title",
        "Manual",
        "--description",
        "Manual gate",
        "--mode",
        "manual",
    ])
    .assert()
    .success();
    temp.command(["issue", "update", &id, "--add-gate", "manual-gate"])
        .assert()
        .success();

    let output = temp
        .command(["issue", "show", &id, "--summary", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let data = &json;
    // Summary shares the `gates` field name with the full `issue show`, projected
    // to `{key, status}` per gate — never the raw storage fields.
    let gates = data["gates"]
        .as_array()
        .unwrap_or_else(|| panic!("--summary must expose a `gates` array; got: {data}"));
    assert_eq!(gates.len(), 1, "one entry per required gate; got: {data}");
    assert_eq!(gates[0]["key"].as_str(), Some("manual-gate"));
    assert!(gates[0]["status"].is_string());
    assert!(
        data.get("gates_required").is_none() && data.get("gates_status").is_none(),
        "--summary must not carry the raw storage gate fields; got: {data}"
    );

    // Confirm summary also includes the minimal-issue fields it should keep.
    assert!(data["id"].is_string());
    assert!(data["short_id"].is_string());
    assert!(data["title"].is_string());
    assert!(data["state"].is_string());
    assert!(data["priority"].is_string());
}
