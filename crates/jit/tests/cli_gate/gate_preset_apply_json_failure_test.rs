//! JSON preset application reports a partial batch as an error envelope.

use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn jit(temp: &TempDir) -> Command {
    let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    command.current_dir(temp.path());
    command
}

fn setup_repo() -> TempDir {
    let temp = TempDir::new().expect("temporary repository");
    jit(&temp).arg("init").assert().success();
    temp
}

fn create_issue(temp: &TempDir, title: &str) -> String {
    let output = jit(temp)
        .args(["issue", "create", "--title", title, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let response: serde_json::Value =
        serde_json::from_slice(&output).expect("issue creation must emit JSON");
    response["id"]
        .as_str()
        .expect("issue creation response must contain an id")
        .to_string()
}

#[test]
fn test_gate_preset_apply_json_partial_batch_emits_error_envelope() {
    let temp = setup_repo();
    let valid_issue = create_issue(&temp, "Preset target");
    let missing_issue = "00000000-0000-0000-0000-000000000000";

    let output = jit(&temp)
        .args([
            "gate",
            "preset",
            "apply",
            "plan-review",
            &valid_issue,
            missing_issue,
            "--json",
        ])
        .output()
        .expect("preset application should run");

    assert_eq!(output.status.code(), Some(3));
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("failure must emit JSON");
    let error = response["error"]
        .as_object()
        .expect("standard error envelope");
    assert_eq!(error["code"], "PRESET_ERROR");
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| message.contains("Failed to apply preset 'plan-review'")),
        "failure message should identify the preset: {response}"
    );

    let details = error["details"]
        .as_object()
        .expect("error envelope must retain batch details");
    assert_eq!(details["preset"], "plan-review");
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
fn test_gate_preset_apply_json_all_success_retains_success_payload() {
    let temp = setup_repo();
    let issue = create_issue(&temp, "Preset target");

    let output = jit(&temp)
        .args(["gate", "preset", "apply", "plan-review", &issue, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let response: serde_json::Value =
        serde_json::from_slice(&output).expect("success must emit JSON");

    assert_eq!(response["preset"], "plan-review");
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
