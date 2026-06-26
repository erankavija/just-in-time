//! Integration tests for standardized exit codes
//!
//! Tests that the CLI returns appropriate exit codes for different error scenarios.

use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

/// Helper to create a test environment with jit initialized
fn setup_test_env() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .arg("init")
        .status()
        .unwrap();
    assert!(status.success());
    temp_dir
}

fn json_issue_id(output: &std::process::Output) -> String {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json["id"].as_str().expect("id should exist").to_string()
}

#[test]
fn test_exit_code_success() {
    let temp_dir = setup_test_env();

    // Successful command should return exit code 0
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "create", "--title", "Test issue"])
        .status()
        .unwrap();

    assert!(status.success());
    assert_eq!(status.code(), Some(0));
}

#[test]
fn test_exit_code_not_found() {
    let temp_dir = setup_test_env();

    // Issue not found should return exit code 3
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "show", "nonexistent"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));
    // Error message is "Failed to read file" for non-existent issues
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Failed to read file") || stderr.contains("not found"));
}

#[test]
fn test_exit_code_validation_failed_cycle() {
    let temp_dir = setup_test_env();

    // Create two issues
    let output1 = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "create", "--title", "Task A", "--json"])
        .output()
        .unwrap();
    assert!(output1.status.success());
    let json1: serde_json::Value = serde_json::from_slice(&output1.stdout).unwrap();
    let id1 = json1["id"].as_str().expect("id1 should exist");

    let output2 = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "create", "--title", "Task B", "--json"])
        .output()
        .unwrap();
    assert!(output2.status.success());
    let json2: serde_json::Value = serde_json::from_slice(&output2.stdout).unwrap();
    let id2 = json2["id"].as_str().expect("id2 should exist");

    // Add dependency A -> B
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", id1, id2])
        .status()
        .unwrap();
    assert!(status.success());

    // Try to add B -> A (would create cycle) - should return exit code 4
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", id2, id1])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cycle"));
}

#[test]
fn test_exit_code_gate_not_found_in_registry() {
    let temp_dir = setup_test_env();

    // Create an issue to attach the gate to.
    let issue = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Gated work", "--json"])
            .output()
            .unwrap(),
    );

    // Adding an undefined gate is a not-found condition (exit code 3), now
    // classified by downcasting the typed `GateNotFoundError`.
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["gate", "add", &issue, "undefined-gate"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not found in registry"));

    // The same condition under --json carries the GATE_NOT_FOUND code.
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["gate", "add", &issue, "undefined-gate", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "GATE_NOT_FOUND");
}

#[test]
fn test_exit_code_invalid_argument() {
    let temp_dir = setup_test_env();

    // Invalid priority should return exit code 2
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args([
            "issue",
            "create",
            "--title",
            "Test",
            "--priority",
            "invalid",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn test_exit_code_io_error() {
    // Try to run command in non-initialized directory
    // This should return exit code 3 (not found) because data directory doesn't exist
    let temp_dir = TempDir::new().unwrap();

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["query", "all"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    // File not found is code 3, not 10
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains(".jit"));
}

#[test]
fn test_exit_code_already_exists() {
    let temp_dir = setup_test_env();

    // Add a gate to registry
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["registry", "add", "--title", "Test gate", "test-gate"])
        .status()
        .unwrap();
    assert!(status.success());

    // Try to add same gate again - should return exit code 6
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["registry", "add", "--title", "Test gate", "test-gate"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(6));
    assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
}

#[test]
fn test_exit_code_json_error_format() {
    let temp_dir = setup_test_env();

    // Error with --json flag should still return appropriate exit code
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "show", "nonexistent", "--json"])
        .output()
        .unwrap();

    // Should have exit code 3 (not found)
    assert_eq!(output.status.code(), Some(3));

    // Should have valid JSON error
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    // Check error via exit code or error field
    assert_eq!(json["error"]["code"], "ISSUE_NOT_FOUND");
}

#[test]
fn test_exit_code_validation_command() {
    let temp_dir = setup_test_env();

    // Validation should succeed with exit code 0 when no issues
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["validate"])
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(status.code(), Some(0));

    // Create an issue and manually corrupt the data to cause validation failure
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "create", "--title", "Test", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let id = json["id"].as_str().expect("id should exist");

    // Corrupt the issue file by adding invalid dependency reference
    let issue_path = temp_dir
        .path()
        .join(".jit")
        .join("issues")
        .join(format!("{}.json", id));
    let mut issue_data: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&issue_path).unwrap()).unwrap();
    issue_data["dependencies"] = serde_json::json!(["nonexistent"]);
    fs::write(
        &issue_path,
        serde_json::to_string_pretty(&issue_data).unwrap(),
    )
    .unwrap();

    // Validation should fail with exit code 4
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["validate"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid"));
}

#[test]
fn test_exit_code_help_and_version() {
    // --help should return exit code 0
    let status = Command::new(jit_binary()).arg("--help").status().unwrap();
    assert!(status.success());
    assert_eq!(status.code(), Some(0));

    // Help for subcommand should also return 0
    let status = Command::new(jit_binary())
        .args(["issue", "--help"])
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(status.code(), Some(0));
}

#[test]
fn test_exit_code_state_transition_blocked_by_gates() {
    let temp_dir = setup_test_env();

    // Define a gate
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["registry", "add", "--title", "Tests", "tests"])
        .status()
        .unwrap();
    assert!(status.success());

    // Create issue with gate
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args([
            "issue",
            "create",
            "--title",
            "Test issue",
            "--gate",
            "tests",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let id = json["id"].as_str().expect("id should exist");

    // Mark as ready
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "update", id, "--state", "ready"])
        .status()
        .unwrap();
    assert!(status.success());

    // Try to transition to done without passing gate - should fail with exit code 4
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "update", id, "--state", "done"])
        .output()
        .unwrap();

    // Should return exit code 4 (validation failed)
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));

    // Error message should mention the gate
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("gate") || stderr.contains("tests"));
    assert!(stderr.contains("gated") || stderr.contains("not passed"));
    assert!(stderr.contains("jit gate pass"));
    assert!(stderr.contains(id));

    // Verify issue is in gated state (auto-transition happened)
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "show", id, "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["state"], "gated");

    // Now pass the gate and verify transition to done succeeds
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["gate", "pass", id, "tests"])
        .status()
        .unwrap();
    assert!(status.success());

    // Should auto-transition to done
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "show", id, "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["state"], "done");
}

#[test]
fn test_exit_code_state_transition_blocked_by_gates_json() {
    let temp_dir = setup_test_env();

    // Define a gate
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["registry", "add", "--title", "Tests", "tests"])
        .status()
        .unwrap();
    assert!(status.success());

    // Create issue with gate
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args([
            "issue",
            "create",
            "--title",
            "Test issue",
            "--gate",
            "tests",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let id = json["id"].as_str().expect("id should exist");

    // Try to transition to done with --json flag - should get JSON error
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "update", id, "--state", "done", "--json"])
        .output()
        .unwrap();

    // Should return exit code 4 (validation failed)
    assert_eq!(output.status.code(), Some(4));

    // Should have valid JSON error output
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    // Check error via exit code or error field
    assert!(
        json["error"]["code"].as_str().unwrap().contains("GATE")
            || json["error"]["code"]
                .as_str()
                .unwrap()
                .contains("VALIDATION")
    );

    // Error should mention gate blocking
    let error_msg = json["error"]["message"].as_str().unwrap();
    assert!(error_msg.contains("gate") || error_msg.contains("tests"));

    let details = &json["error"]["details"];
    assert_eq!(details["issue_id"], id);
    assert_eq!(details["requested_state"], "done");
    assert_eq!(details["actual_state"], "gated");
    assert_eq!(details["blockers"][0]["type"], "gate");
    assert_eq!(details["blockers"][0]["gate_key"], "tests");
    assert_eq!(details["blockers"][0]["status"], "pending");

    let remediation = details["remediation"].as_array().unwrap();
    assert!(remediation.iter().any(|cmd| cmd
        .as_str()
        .unwrap()
        .contains(&format!("jit gate pass {} tests", id))));
    assert!(remediation.iter().any(|cmd| cmd
        .as_str()
        .unwrap()
        .contains(&format!("jit gate check-all {}", id))));
}

#[test]
fn test_exit_code_state_transition_blocked_by_dependencies_json() {
    let temp_dir = setup_test_env();

    let dependency = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args([
                "issue",
                "create",
                "--title",
                "Blocked prerequisite",
                "--json",
            ])
            .output()
            .unwrap(),
    );
    let dependent = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Blocked work", "--json"])
            .output()
            .unwrap(),
    );

    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", &dependent, &dependency])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "update", &dependent, "--state", "ready", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "BLOCKED");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("dependencies"));

    let details = &json["error"]["details"];
    assert_eq!(details["issue_id"], dependent);
    assert_eq!(details["requested_state"], "ready");
    assert_eq!(details["actual_state"], "backlog");
    assert_eq!(details["blockers"][0]["type"], "dependency");
    assert_eq!(details["blockers"][0]["issue_id"], dependency);
    assert_eq!(details["blockers"][0]["title"], "Blocked prerequisite");
    assert_eq!(details["blockers"][0]["state"], "ready");

    let remediation = details["remediation"].as_array().unwrap();
    assert!(remediation.iter().any(|cmd| cmd
        .as_str()
        .unwrap()
        .contains(&format!("jit graph deps {}", dependent))));
    assert!(remediation.iter().any(|cmd| cmd
        .as_str()
        .unwrap()
        .contains(&format!("jit issue show {}", dependency))));
}

#[test]
fn test_exit_code_state_transition_blocked_by_dependencies_human_remediation() {
    let temp_dir = setup_test_env();

    let dependency = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args([
                "issue",
                "create",
                "--title",
                "Blocked prerequisite",
                "--json",
            ])
            .output()
            .unwrap(),
    );
    let dependent = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Blocked work", "--json"])
            .output()
            .unwrap(),
    );

    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", &dependent, &dependency])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "update", &dependent, "--state", "ready"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Blocked prerequisite"));
    assert!(stderr.contains(&dependency[..8]));
    assert!(stderr.contains("jit graph deps"));
    assert!(stderr.contains("jit issue show"));
}

#[test]
fn test_exit_code_state_transition_blocked_by_missing_dependency_json() {
    let temp_dir = setup_test_env();
    let dependent = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Dangling work", "--json"])
            .output()
            .unwrap(),
    );

    let missing_id = "missing-dependency";
    let issue_path = temp_dir
        .path()
        .join(".jit")
        .join("issues")
        .join(format!("{}.json", dependent));
    let mut issue_data: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&issue_path).unwrap()).unwrap();
    issue_data["state"] = serde_json::json!("backlog");
    issue_data["dependencies"] = serde_json::json!([missing_id]);
    fs::write(
        &issue_path,
        serde_json::to_string_pretty(&issue_data).unwrap(),
    )
    .unwrap();

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "update", &dependent, "--state", "ready", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "BLOCKED");
    assert_eq!(
        json["error"]["details"]["blockers"][0]["type"],
        "dependency"
    );
    assert_eq!(
        json["error"]["details"]["blockers"][0]["issue_id"],
        missing_id
    );
    assert_eq!(json["error"]["details"]["blockers"][0]["state"], "missing");
}

#[test]
fn test_exit_code_claim_blocked_by_dependencies_json() {
    let temp_dir = setup_test_env();

    let dependency = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Claim prerequisite", "--json"])
            .output()
            .unwrap(),
    );
    let dependent = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Claim work", "--json"])
            .output()
            .unwrap(),
    );

    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", &dependent, &dependency])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "claim", &dependent, "agent:test", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "BLOCKED");
    assert_eq!(json["error"]["details"]["issue_id"], dependent);
    assert_eq!(json["error"]["details"]["requested_state"], "in_progress");
    assert_eq!(json["error"]["details"]["actual_state"], "backlog");
    assert_eq!(
        json["error"]["details"]["blockers"][0]["issue_id"],
        dependency
    );

    let remediation = json["error"]["details"]["remediation"].as_array().unwrap();
    assert!(remediation.iter().any(|cmd| cmd
        .as_str()
        .unwrap()
        .contains(&format!("jit graph deps {}", dependent))));
}

#[test]
fn test_exit_code_claim_blocked_by_precheck_gate_json() {
    let temp_dir = setup_test_env();

    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args([
            "gate",
            "define",
            "tdd-reminder",
            "--title",
            "TDD Reminder",
            "-d",
            "Write tests first",
            "--stage",
            "precheck",
            "--mode",
            "manual",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let issue = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args([
                "issue",
                "create",
                "--title",
                "Precheck work",
                "--gate",
                "tdd-reminder",
                "--json",
            ])
            .output()
            .unwrap(),
    );

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "claim", &issue, "agent:test", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(json["error"]["details"]["requested_state"], "in_progress");
    assert_eq!(json["error"]["details"]["actual_state"], "ready");
    assert_eq!(json["error"]["details"]["blockers"][0]["type"], "gate");
    assert_eq!(
        json["error"]["details"]["blockers"][0]["gate_key"],
        "tdd-reminder"
    );

    let remediation = json["error"]["details"]["remediation"].as_array().unwrap();
    assert!(remediation.iter().any(|cmd| cmd
        .as_str()
        .unwrap()
        .contains(&format!("jit gate pass {} tdd-reminder", issue))));
}

#[test]
fn test_exit_code_claim_next_blocked_by_precheck_gate_json() {
    let temp_dir = setup_test_env();

    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args([
            "gate",
            "define",
            "tdd-reminder",
            "--title",
            "TDD Reminder",
            "-d",
            "Write tests first",
            "--stage",
            "precheck",
            "--mode",
            "manual",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let issue = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args([
                "issue",
                "create",
                "--title",
                "Claim-next precheck work",
                "--gate",
                "tdd-reminder",
                "--json",
            ])
            .output()
            .unwrap(),
    );

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "claim-next", "agent:test", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(json["error"]["details"]["issue_id"], issue);
    assert_eq!(
        json["error"]["details"]["blockers"][0]["gate_key"],
        "tdd-reminder"
    );
}

#[test]
fn test_claim_next_json_skips_non_ready_issues() {
    let temp_dir = setup_test_env();

    let done_issue = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Already done", "--json"])
            .output()
            .unwrap(),
    );
    let ready_issue = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Ready work", "--json"])
            .output()
            .unwrap(),
    );

    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "update", &done_issue, "--state", "done"])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "claim-next", "agent:test", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["id"], ready_issue);
    assert_eq!(json["state"], "in_progress");
    assert_eq!(json["assignee"], "agent:test");

    let done_output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "show", &done_issue, "--json"])
        .output()
        .unwrap();
    assert!(done_output.status.success());
    let done_json: serde_json::Value = serde_json::from_slice(&done_output.stdout).unwrap();
    assert!(done_json["assignee"].is_null());
}

/// Build a dependency-blocked fixture: returns (dependency, dependent) where
/// `dependent` is in `backlog` and depends on the still-incomplete `dependency`.
fn dependency_blocked_fixture(temp_dir: &TempDir) -> (String, String) {
    let dependency = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(temp_dir)
            .args(["issue", "create", "--title", "Prerequisite", "--json"])
            .output()
            .unwrap(),
    );
    let dependent = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(temp_dir)
            .args(["issue", "create", "--title", "Dependent work", "--json"])
            .output()
            .unwrap(),
    );
    let status = Command::new(jit_binary())
        .current_dir(temp_dir)
        .args(["dep", "add", &dependent, &dependency])
        .status()
        .unwrap();
    assert!(status.success());
    (dependency, dependent)
}

/// A claim blocked by incomplete dependencies must name `jit issue assign` in
/// its human-readable error so the operator learns how to assign without
/// starting work.
#[test]
fn test_claim_blocked_by_dependencies_human_names_assign() {
    let temp_dir = setup_test_env();
    let (_dependency, dependent) = dependency_blocked_fixture(&temp_dir);

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "claim", &dependent, "agent:test"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("jit issue assign"),
        "expected human error to name 'jit issue assign', got: {}",
        stderr
    );
    assert!(stderr.contains(&dependent));
}

/// The same hint must appear in `--json` output, in the error's suggestions /
/// remediation list.
#[test]
fn test_claim_blocked_by_dependencies_json_names_assign() {
    let temp_dir = setup_test_env();
    let (_dependency, dependent) = dependency_blocked_fixture(&temp_dir);

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "claim", &dependent, "agent:test", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "BLOCKED");

    let suggestions = json["error"]["suggestions"].as_array().unwrap();
    assert!(
        suggestions.iter().any(|cmd| cmd
            .as_str()
            .unwrap()
            .contains(&format!("jit issue assign {}", dependent))),
        "expected suggestions to name 'jit issue assign', got: {:?}",
        suggestions
    );

    let remediation = json["error"]["details"]["remediation"].as_array().unwrap();
    assert!(remediation.iter().any(|cmd| cmd
        .as_str()
        .unwrap()
        .contains(&format!("jit issue assign {}", dependent))));
}

/// `jit issue claim <id> <assignee> --assign-only` sets the assignee without
/// transitioning the issue's state.
#[test]
fn test_claim_assign_only_sets_assignee_without_transition() {
    let temp_dir = setup_test_env();
    let issue = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Assign-only work", "--json"])
            .output()
            .unwrap(),
    );

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args([
            "issue",
            "claim",
            &issue,
            "agent:test",
            "--assign-only",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "assign-only claim failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let show = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "show", &issue, "--json"])
        .output()
        .unwrap();
    assert!(show.status.success());
    let json: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(json["assignee"], "agent:test");
    // State is unchanged by --assign-only: a fresh, dependency-free issue is
    // `ready`, and crucially it was NOT transitioned to `in_progress`.
    assert_ne!(json["state"], "in_progress");
    assert_eq!(json["state"], "ready");
}

/// Regression: a normal claim on a ready (unblocked) issue still transitions to
/// in_progress.
#[test]
fn test_claim_ready_issue_transitions_to_in_progress() {
    let temp_dir = setup_test_env();
    let issue = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Ready work", "--json"])
            .output()
            .unwrap(),
    );
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "update", &issue, "--state", "ready"])
        .status()
        .unwrap();
    assert!(status.success());

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "claim", &issue, "agent:test", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let show = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "show", &issue, "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(json["assignee"], "agent:test");
    assert_eq!(json["state"], "in_progress");
}

// The following lock exit codes (and the verbatim message) for error origins that
// the typed-error refactor must preserve: a malformed `--label` filter and a
// malformed `--scope` are argument errors (2), and building a gate preset from an
// issue whose required gate is missing from the registry is a not-found error (3).
// Each previously routed through the deleted substring classifier; these pin the
// post-refactor downcast classification at the literal exit code AND message.

#[test]
fn test_exit_code_query_invalid_label_pattern() {
    let temp_dir = setup_test_env();

    // A `--label` pattern with no colon reaches `query_by_label`'s validation and
    // is an argument error (exit 2) with the original phrasing preserved.
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["query", "available", "--label", "badpattern"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Invalid label pattern 'badpattern'"),
        "stderr was: {stderr}"
    );
}

#[test]
fn test_exit_code_document_invalid_scope() {
    let temp_dir = setup_test_env();

    // A `--scope` that is neither `all` nor `issue:ID` is an argument error (2).
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["doc", "check-links", "--scope", "badscope"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Invalid scope 'badscope'. Use 'all' or 'issue:ID'"),
        "stderr was: {stderr}"
    );
}

#[test]
fn test_exit_code_gate_preset_create_missing_registry_gate() {
    let temp_dir = setup_test_env();

    // Register a gate, attach it to an issue, then remove it from the registry so
    // the issue references a gate that no longer exists.
    assert!(Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["registry", "add", "--title", "My gate", "mygate"])
        .status()
        .unwrap()
        .success());

    let created = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "create", "--title", "Has gate", "--json"])
        .output()
        .unwrap();
    let issue = json_issue_id(&created);

    assert!(Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["gate", "add", &issue, "mygate"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["registry", "remove", "mygate"])
        .status()
        .unwrap()
        .success());

    // Building a preset now hits the missing-registry-gate path: not found (3),
    // with the original "Gate not found in registry: <key>" phrasing.
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["gate", "preset", "create", &issue, "mypreset"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Gate not found in registry: mygate"),
        "stderr was: {stderr}"
    );
}
