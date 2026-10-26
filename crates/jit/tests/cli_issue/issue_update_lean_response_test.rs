//! Verify `jit issue update --json` returns a lightweight confirmation
//! response (id, short_id, state, updated_at) instead of the full issue body.

use jit::test_cli::JitCli;

#[test]
fn test_update_json_omits_description() {
    let big_description = "A".repeat(5000);
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", &big_description])
        .unwrap();

    let json = temp
        .run_json(["issue", "update", &id, "--priority", "high"])
        .unwrap();
    let data = &json;

    assert!(
        data.get("description").is_none(),
        "lean update response must not include description; got: {}",
        data
    );

    let serialized = serde_json::to_string(&json).unwrap();
    assert!(
        !serialized.contains(&big_description),
        "description content leaked into update response"
    );
}

#[test]
fn test_update_json_contains_id_short_id_state_updated_at() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", "short"])
        .unwrap();

    let json = temp
        .run_json(["issue", "update", &id, "--priority", "high"])
        .unwrap();
    let data = &json;

    assert!(data["id"].is_string(), "missing id; got: {}", data);
    assert!(
        data["short_id"].is_string(),
        "missing short_id; got: {}",
        data
    );
    assert!(data["state"].is_string(), "missing state; got: {}", data);
    assert!(
        data["updated_at"].is_string(),
        "missing updated_at; got: {}",
        data
    );
}

#[test]
fn test_update_json_omits_enriched_dependencies() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", "short"])
        .unwrap();

    let json = temp
        .run_json(["issue", "update", &id, "--priority", "high"])
        .unwrap();
    let data = &json;

    assert!(
        data.get("dependencies").is_none() && data.get("labels").is_none(),
        "lean update response must not include full issue fields; got: {}",
        data
    );
}
