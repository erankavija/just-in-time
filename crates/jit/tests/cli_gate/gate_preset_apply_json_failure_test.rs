//! JSON preset application reports a partial batch as an error envelope.

use crate::create_titled_issue;
use assert_cmd::prelude::*;
use jit::test_cli::JitCli;

/// The project-defined preset every case below applies.
const PRESET: &str = "review";

/// Declare one gate in `temp` and capture it into the project's own preset
/// [`PRESET`], which is the only place a preset comes from. Returns the
/// issue the preset was captured from.
fn define_team_review_preset(temp: &JitCli) -> String {
    temp.command([
        "gate",
        "define",
        "team-review",
        "--title",
        "Team review",
        "--description",
        "A reviewer signs off",
    ])
    .assert()
    .success();
    let reference = create_titled_issue(temp, "Preset source");
    temp.command(["gate", "add", &reference, "team-review"])
        .assert()
        .success();
    temp.command(["gate", "preset", "create", &reference, PRESET])
        .assert()
        .success();
    reference
}

#[test]
fn test_gate_preset_apply_json_partial_batch_emits_error_envelope() {
    let temp = JitCli::initialised().unwrap();
    define_team_review_preset(&temp);
    let valid_issue = create_titled_issue(&temp, "Preset target");
    let missing_issue = "00000000-0000-0000-0000-000000000000";

    let output = temp
        .command([
            "gate",
            "preset",
            "apply",
            PRESET,
            &valid_issue,
            missing_issue,
            "--json",
        ])
        .output()
        .expect("preset application should run");

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("failure must emit JSON");
    let error = response["error"]
        .as_object()
        .expect("standard error envelope");
    assert_eq!(error["code"], "PRESET_ERROR");
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| message.contains(&format!("Failed to apply preset '{PRESET}'"))),
        "failure message should identify the preset: {response}"
    );

    let details = error["details"]
        .as_object()
        .expect("error envelope must retain batch details");
    assert_eq!(details["preset"], PRESET);
    assert!(
        details["success"]
            .as_array()
            .is_some_and(|success| success.iter().any(|entry| entry["issue_id"] == valid_issue)),
        "successful target detail should remain readable: {response}"
    );
    assert!(
        details["errors"]
            .as_array()
            .is_some_and(|errors| errors.iter().any(|entry| {
                entry["issue_id"] == missing_issue
                    && entry["error"]
                        .as_str()
                        .is_some_and(|message| message.contains("Issue not found"))
            })),
        "failing target detail should remain readable: {response}"
    );
}

#[test]
fn test_gate_preset_apply_plain_partial_batch_fails_with_error_summary() {
    let temp = JitCli::initialised().unwrap();
    define_team_review_preset(&temp);

    let output = temp
        .command([
            "gate",
            "preset",
            "apply",
            PRESET,
            "00000000-0000-0000-0000-000000000000",
        ])
        .output()
        .expect("preset application should run");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Errors (1)"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Issue not found"));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        format!("Failed to apply preset '{PRESET}' to 1 issue(s)\n")
    );
}

#[test]
fn test_gate_preset_apply_json_all_success_retains_success_payload() {
    let temp = JitCli::initialised().unwrap();
    define_team_review_preset(&temp);
    let issue = create_titled_issue(&temp, "Preset target");

    let output = temp
        .command(["gate", "preset", "apply", PRESET, &issue, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let response: serde_json::Value =
        serde_json::from_slice(&output).expect("success must emit JSON");

    assert_eq!(response["preset"], PRESET);
    assert!(
        response.get("error").is_none(),
        "success must not be an error envelope"
    );
    assert!(
        response["success"]
            .as_array()
            .is_some_and(|success| success.iter().any(|entry| entry["issue_id"] == issue)),
        "success payload shape must remain unchanged: {response}"
    );
    assert_eq!(response["errors"], serde_json::json!([]));
}
