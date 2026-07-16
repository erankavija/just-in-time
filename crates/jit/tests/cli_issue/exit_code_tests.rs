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
        .args([
            "gate",
            "define",
            "--title",
            "Test gate",
            "--description",
            "Test gate",
            "test-gate",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    // Try to add same gate again - should return exit code 6
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args([
            "gate",
            "define",
            "--title",
            "Test gate",
            "--description",
            "Test gate",
            "test-gate",
        ])
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
        .args([
            "gate",
            "define",
            "--title",
            "Tests",
            "--description",
            "Tests",
            "tests",
        ])
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
    assert!(stderr.contains("jit gate evaluate"));
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

    // Now evaluate the gate and verify transition to done succeeds
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["gate", "evaluate", id, "tests", "--by", "human:reviewer"])
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
        .args([
            "gate",
            "define",
            "--title",
            "Tests",
            "--description",
            "Tests",
            "tests",
        ])
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
    assert_eq!(details["blockers"][0]["key"], "tests");
    assert_eq!(details["blockers"][0]["status"], "pending");

    let remediation = details["remediation"].as_array().unwrap();
    assert!(remediation.iter().any(|cmd| cmd
        .as_str()
        .unwrap()
        .contains(&format!("jit gate evaluate {} tests", id))));
    assert!(remediation.iter().any(|cmd| cmd
        .as_str()
        .unwrap()
        .contains(&format!("jit gate status-all {}", id))));
    // jit:62f3bebd REQ-01 — the gate-blocked transition error also points at
    // the per-gate run-history view, not just the readiness/evaluate commands.
    assert!(remediation.iter().any(|cmd| {
        let cmd = cmd.as_str().unwrap();
        cmd.contains(&format!("jit gate status {} tests", id)) && cmd.contains("--all")
    }));
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
        json["error"]["details"]["blockers"][0]["key"],
        "tdd-reminder"
    );

    let remediation = json["error"]["details"]["remediation"].as_array().unwrap();
    assert!(remediation.iter().any(|cmd| cmd
        .as_str()
        .unwrap()
        .contains(&format!("jit gate evaluate {} tdd-reminder", issue))));
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
        json["error"]["details"]["blockers"][0]["key"],
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
/// `dependent` is in `backlog` and depends on the still-unmet `dependency`.
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

/// A claim blocked by unmet dependencies must name `jit issue assign` in
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
        .args([
            "gate",
            "define",
            "--title",
            "My gate",
            "--description",
            "My gate",
            "mygate"
        ])
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
        .args(["gate", "remove", "mygate"])
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

// ============================================================================
// Prefix-resolution argument errors (issue a05b87ae)
//
// Ambiguous-prefix and too-short-prefix id lookups are argument errors (exit 2),
// carrying a distinguishing `code` in `--json` output. Previously both fell
// through to the generic exit 1.
// ============================================================================

/// Craft two issue files that share the `aaaabbbb` id prefix and register them in
/// the on-disk index, so a lookup of that prefix is genuinely ambiguous. Returns
/// the shared prefix. Uses a real issue's JSON as a template so every required
/// field is present.
fn make_ambiguous_prefix(temp_dir: &TempDir) -> &'static str {
    let template_id = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(temp_dir)
            .args(["issue", "create", "--title", "Template", "--json"])
            .output()
            .unwrap(),
    );

    let issues_dir = temp_dir.path().join(".jit").join("issues");
    let template_path = issues_dir.join(format!("{}.json", template_id));
    let template: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&template_path).unwrap()).unwrap();

    let crafted_ids = [
        "aaaabbbb-1111-4111-8111-111111111111",
        "aaaabbbb-2222-4222-8222-222222222222",
    ];
    for id in crafted_ids {
        let mut issue = template.clone();
        issue["id"] = serde_json::json!(id);
        fs::write(
            issues_dir.join(format!("{}.json", id)),
            serde_json::to_string_pretty(&issue).unwrap(),
        )
        .unwrap();
    }

    // Register the crafted ids in the local index so resolve_issue_id sees them.
    let index_path = temp_dir.path().join(".jit").join("index.json");
    let mut index: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&index_path).unwrap()).unwrap();
    let all_ids = index["all_ids"].as_array_mut().unwrap();
    for id in crafted_ids {
        all_ids.push(serde_json::json!(id));
    }
    fs::write(&index_path, serde_json::to_string_pretty(&index).unwrap()).unwrap();

    "aaaabbbb"
}

#[test]
fn test_exit_code_ambiguous_prefix() {
    let temp_dir = setup_test_env();
    let prefix = make_ambiguous_prefix(&temp_dir);

    // Human path: ambiguous prefix is an argument error (exit 2), preserving the
    // original "Ambiguous ID" phrasing.
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "show", prefix])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Ambiguous ID"), "stderr was: {stderr}");

    // --json path: same exit code, distinguishing AMBIGUOUS_ID code on stdout.
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "show", prefix, "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "AMBIGUOUS_ID");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Ambiguous ID"));
}

#[test]
fn test_exit_code_too_short_prefix() {
    let temp_dir = setup_test_env();

    // Human path: a sub-4-char prefix is an argument error (exit 2), preserving
    // the original phrasing.
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "show", "ab"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("at least 4 characters"),
        "stderr was: {stderr}"
    );

    // --json path: same exit code, distinguishing INVALID_ID_PREFIX code.
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "show", "ab", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ID_PREFIX");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("at least 4 characters"));
}

/// REQ-02: the batch-mode description guard is a usage error (exit 2), matching
/// clap usage errors and the other batch-mode rejections.
#[test]
fn test_exit_code_batch_description_guard() {
    let temp_dir = setup_test_env();

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args([
            "issue",
            "update",
            "--filter",
            "state:backlog",
            "--append-description",
            "a note",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not supported with --filter"));
}

/// A query filter placed before a subcommand (where it would be silently
/// dropped) is a usage error (exit 2), matching the other query-family and
/// batch-mode usage guards. Under --json it carries an INVALID_ARGUMENT
/// envelope; the message text is unchanged.
#[test]
fn test_exit_code_query_parent_filter_before_subcommand() {
    let temp_dir = setup_test_env();

    // Human path: exit 2, original phrasing preserved.
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["query", "--state", "ready", "available"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("were given before the `available` subcommand"));

    // --json path: same exit code, machine-readable envelope on stdout. The
    // parent-level `--json` (and `--state`) precede the subcommand — that is the
    // misplacement the guard rejects, and it also selects JSON rendering.
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["query", "--state", "ready", "--json", "available"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("were given before"));
}

/// REQ-03: `jit dep rm <from> <target>` validates both id arguments identically.
/// A too-short prefix in either position is the same argument error (exit 2),
/// where previously a short `<target>` was silently treated as "not found"
/// (exit 0) while a short `<from>` exited 1.
#[test]
fn test_exit_code_dep_rm_both_args_symmetric() {
    let temp_dir = setup_test_env();

    let dependent = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Dependent", "--json"])
            .output()
            .unwrap(),
    );
    let dependency = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Dependency", "--json"])
            .output()
            .unwrap(),
    );
    assert!(Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", &dependent, &dependency])
        .status()
        .unwrap()
        .success());

    // Short `<from>`: argument error (exit 2).
    let short_from = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "rm", "ab", &dependency])
        .output()
        .unwrap();
    assert_eq!(
        short_from.status.code(),
        Some(2),
        "short <from> should exit 2"
    );
    assert!(String::from_utf8_lossy(&short_from.stderr).contains("at least 4 characters"));

    // Short `<target>`: SAME argument error (exit 2), not a silent no-op.
    let short_target = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "rm", &dependent, "ab"])
        .output()
        .unwrap();
    assert_eq!(
        short_target.status.code(),
        Some(2),
        "short <target> should exit 2 identically to short <from>"
    );
    assert!(String::from_utf8_lossy(&short_target.stderr).contains("at least 4 characters"));
}

/// REQ-01 for `jit dep add`: a too-short id prefix in EITHER the `<from>` or a
/// `<target>` position (including the variadic case) is an argument error (exit
/// 2), carrying the distinguishing INVALID_ID_PREFIX code under --json. Both id
/// arguments resolve inside the per-edge add, so both flow through the same
/// classifier.
#[test]
fn test_exit_code_dep_add_too_short_prefix_both_positions() {
    let temp_dir = setup_test_env();

    let a = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "A", "--json"])
            .output()
            .unwrap(),
    );
    let b = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "B", "--json"])
            .output()
            .unwrap(),
    );

    // Short `<from>`.
    let short_from = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", "ab", &b])
        .output()
        .unwrap();
    assert_eq!(
        short_from.status.code(),
        Some(2),
        "short <from> should exit 2"
    );
    assert!(String::from_utf8_lossy(&short_from.stderr).contains("at least 4 characters"));

    let short_from_json = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", "ab", &b, "--json"])
        .output()
        .unwrap();
    assert_eq!(short_from_json.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&short_from_json.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ID_PREFIX");

    // Short `<target>`.
    let short_target = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", &a, "ab"])
        .output()
        .unwrap();
    assert_eq!(
        short_target.status.code(),
        Some(2),
        "short <target> should exit 2"
    );

    let short_target_json = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", &a, "ab", "--json"])
        .output()
        .unwrap();
    assert_eq!(short_target_json.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&short_target_json.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ID_PREFIX");

    // Variadic: a short target among valid ones fails the whole command (exit 2).
    let variadic_json = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", &a, &b, "ab", "--json"])
        .output()
        .unwrap();
    assert_eq!(variadic_json.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&variadic_json.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ID_PREFIX");
}

/// REQ-01 for `jit dep add`: an ambiguous id prefix in either position is an
/// argument error (exit 2) carrying the AMBIGUOUS_ID code under --json.
#[test]
fn test_exit_code_dep_add_ambiguous_prefix_both_positions() {
    let temp_dir = setup_test_env();
    let prefix = make_ambiguous_prefix(&temp_dir);
    let other = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Other", "--json"])
            .output()
            .unwrap(),
    );

    // Ambiguous `<from>`.
    let from_json = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", prefix, &other, "--json"])
        .output()
        .unwrap();
    assert_eq!(from_json.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&from_json.stdout).unwrap();
    assert_eq!(json["error"]["code"], "AMBIGUOUS_ID");

    // Ambiguous `<target>`.
    let target_json = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", &other, prefix, "--json"])
        .output()
        .unwrap();
    assert_eq!(target_json.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&target_json.stdout).unwrap();
    assert_eq!(json["error"]["code"], "AMBIGUOUS_ID");
}

// ============================================================================
// Startup-failure JSON envelope (issue a05b87ae, REQ-04)
//
// With --json, startup failures that abort before a command handler runs
// (repository not found, repository format too new) emit a structured error
// object on stdout while keeping their exit codes (3 and 10).
// ============================================================================

#[test]
fn test_exit_code_startup_repo_not_found_json_envelope() {
    // Uninitialized directory: `--json` must produce a structured envelope on
    // stdout AND keep exit code 3, with the human line still on stderr.
    let temp_dir = TempDir::new().unwrap();

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["query", "all", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(3));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("startup failure should emit JSON on stdout under --json");
    assert_eq!(json["error"]["code"], "REPOSITORY_NOT_FOUND");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not found"));
    // Human line stays on stderr.
    assert!(String::from_utf8_lossy(&output.stderr).contains(".jit"));
}

#[test]
fn test_exit_code_startup_repo_not_found_non_json_stdout_empty() {
    // Without --json the behavior is unchanged: bare stderr line, empty stdout.
    let temp_dir = TempDir::new().unwrap();

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["query", "all"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty(), "non-json stdout must stay empty");
    assert!(String::from_utf8_lossy(&output.stderr).contains(".jit"));
}

#[test]
fn test_exit_code_startup_format_too_new_json_envelope() {
    let temp_dir = setup_test_env();

    // Bump the on-disk index schema version beyond what this binary supports.
    let index_path = temp_dir.path().join(".jit").join("index.json");
    let mut index: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&index_path).unwrap()).unwrap();
    index["schema_version"] = serde_json::json!(9999);
    fs::write(&index_path, serde_json::to_string_pretty(&index).unwrap()).unwrap();

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["query", "all", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(10));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("format-too-new failure should emit JSON on stdout under --json");
    assert_eq!(json["error"]["code"], "REPOSITORY_FORMAT_TOO_NEW");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("newer than this jit"));
    // Human line stays on stderr.
    assert!(String::from_utf8_lossy(&output.stderr).contains("newer than this jit"));
}

// ============================================================================
// `issue delete` confirmation (jit:0daba57d)
//
// A deletion refused for missing `JIT_ALLOW_DELETION=1` confirmation must exit
// nonzero (REQ-01) so scripts observe the refusal as a failure instead of
// reading exit 0 and assuming the deletion happened. A confirmed deletion's
// exit code and output stay unchanged (REQ-03).
// ============================================================================

#[test]
fn test_exit_code_issue_delete_unconfirmed_text_mode() {
    let temp_dir = setup_test_env();
    let id = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Doomed", "--json"])
            .output()
            .unwrap(),
    );

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .env_remove("JIT_ALLOW_DELETION")
        .args(["issue", "delete", &id])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("JIT_ALLOW_DELETION=1"), "stderr: {stderr}");
    assert!(output.stdout.is_empty(), "non-json stdout must stay empty");
}

#[test]
fn test_exit_code_issue_delete_confirmed_unchanged_text_mode() {
    let temp_dir = setup_test_env();
    let id = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Doomed", "--json"])
            .output()
            .unwrap(),
    );

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .env("JIT_ALLOW_DELETION", "1")
        .args(["issue", "delete", &id])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("Deleted issue: {id}")),
        "stdout: {stdout}"
    );
}

#[test]
fn test_exit_code_issue_delete_confirmed_unchanged_json_mode() {
    let temp_dir = setup_test_env();
    let id = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Doomed", "--json"])
            .output()
            .unwrap(),
    );

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .env("JIT_ALLOW_DELETION", "1")
        .args(["issue", "delete", &id, "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["id"], id);
    assert_eq!(json["deleted"], true);
}
