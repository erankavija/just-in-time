//! Integration tests for claim CLI commands
//!
//! Tests the full claim workflow: acquire → list → release
//! Verifies actual binary execution, exit codes, and output formats.

use assert_cmd::prelude::*;
use jit::output::ErrorCode;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::str::FromStr;
use tempfile::TempDir;

/// Setup a test repository with git and jit initialized
fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();

    // Initialize git (required for worktree detection)
    Command::new("git")
        .current_dir(temp.path())
        .args(["init"])
        .status()
        .unwrap();

    Command::new("git")
        .current_dir(temp.path())
        .args(["config", "user.email", "test@example.com"])
        .status()
        .unwrap();

    Command::new("git")
        .current_dir(temp.path())
        .args(["config", "user.name", "Test User"])
        .status()
        .unwrap();

    // Initialize jit
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    // Initialize control plane (for claims)
    let git_dir = temp.path().join(".git");
    fs::create_dir_all(git_dir.join("jit/locks")).unwrap();
    fs::write(git_dir.join("jit/claims.jsonl"), "").unwrap();

    // Create initial commit (required for claims)
    fs::write(temp.path().join("README.md"), "# Test\n").unwrap();
    Command::new("git")
        .current_dir(temp.path())
        .args(["add", "."])
        .status()
        .unwrap();
    Command::new("git")
        .current_dir(temp.path())
        .args(["commit", "-m", "Initial commit"])
        .status()
        .unwrap();

    temp
}

/// Create a test issue and return its ID
fn create_issue(repo_path: &Path, title: &str) -> String {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(repo_path)
        .args(["issue", "create", "--title", title, "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    json["id"].as_str().unwrap().to_string()
}

fn assert_claim_failure_parity(plain: &Output, json: &Output, expected_code: ErrorCode) {
    let expected_status = Some(expected_code.exit_code().code());
    assert_eq!(plain.status.code(), expected_status);
    assert_eq!(json.status.code(), expected_status);
    assert_eq!(plain.status.code(), json.status.code());

    assert!(plain.stdout.is_empty(), "plain failure leaked to stdout");
    assert!(
        !plain.stderr.is_empty(),
        "plain failure omitted its diagnostic"
    );
    assert!(json.stderr.is_empty(), "JSON failure leaked to stderr");

    let envelope: Value = serde_json::from_slice(&json.stdout)
        .expect("JSON failure stdout contains exactly one document");
    assert_eq!(envelope.as_object().map(|object| object.len()), Some(1));
    let error = envelope["error"]
        .as_object()
        .expect("canonical failure envelope contains an error object");
    assert_eq!(error.len(), 2);
    let actual_code = ErrorCode::from_str(
        error["code"]
            .as_str()
            .expect("canonical failure carries a registered code"),
    )
    .expect("claim failure code belongs to the registered vocabulary");
    assert_eq!(actual_code, expected_code);
    assert!(error["message"].is_string());
}

fn acquire_lease(repo_path: &Path, issue_id: &str, agent_id: &str) -> String {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(repo_path)
        .args([
            "claim",
            "acquire",
            issue_id,
            "--ttl",
            "600",
            "--agent-id",
            agent_id,
            "--json",
        ])
        .output()
        .expect("acquire fixture lease");
    assert!(output.status.success(), "claim acquire failed: {output:?}");
    let json: Value = serde_json::from_slice(&output.stdout).expect("acquire response is JSON");
    json["lease_id"]
        .as_str()
        .expect("acquire response carries lease id")
        .to_string()
}

#[cfg(unix)]
#[test]
fn test_claim_renew_permission_denied_is_identical_in_plain_and_json_forms() {
    use std::os::unix::fs::PermissionsExt;

    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Permission-denied claim");
    let agent = "agent:permission-probe";
    let lease_id = acquire_lease(temp.path(), &issue_id, agent);
    let index_path = temp.path().join(".git/jit/claims.index.json");
    let original_mode = fs::metadata(&index_path).unwrap().permissions().mode();
    fs::set_permissions(&index_path, fs::Permissions::from_mode(0o0)).unwrap();

    let invoke = |json: bool| {
        let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
        command
            .current_dir(temp.path())
            .env("JIT_AGENT_ID", agent)
            .args(["claim", "renew", &lease_id, "--extension", "300"]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    };
    let plain = invoke(false);
    let json = invoke(true);

    fs::set_permissions(&index_path, fs::Permissions::from_mode(original_mode)).unwrap();
    assert_claim_failure_parity(&plain, &json, ErrorCode::PermissionDenied);
}

#[test]
fn test_claim_acquire_external_io_is_identical_in_plain_and_json_forms() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "External-I/O claim");
    let locks_path = temp.path().join(".git/jit/locks");
    fs::remove_dir(&locks_path).unwrap();
    fs::write(&locks_path, "not a directory").unwrap();

    let invoke = |json: bool| {
        let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
        command.current_dir(temp.path()).args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:io-probe",
        ]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    };
    let plain = invoke(false);
    let json = invoke(true);

    assert_claim_failure_parity(&plain, &json, ErrorCode::IoError);
}

#[test]
fn test_claim_acquire_happy_path() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    // Acquire claim
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Acquired lease"));
}

#[test]
fn test_claim_acquire_stamps_claimed_at_and_logs_event() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    let acquire = |agent: &str| {
        Command::new(assert_cmd::cargo::cargo_bin!("jit"))
            .current_dir(temp.path())
            .args([
                "claim",
                "acquire",
                &issue_id,
                "--ttl",
                "600",
                "--agent-id",
                agent,
            ])
            .assert()
            .success();
    };

    acquire("agent:test-1");

    // The lease-acquire path stamped claimed_at on the issue record.
    let show = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "show", &issue_id, "--json"])
        .output()
        .unwrap();
    let show_json: Value = serde_json::from_slice(&show.stdout).unwrap();
    let first_claimed = show_json["claimed_at"]
        .as_str()
        .expect("claimed_at must be set after claim acquire")
        .to_string();

    // @/inv/event-log: an issue_claimed event was appended.
    let events = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["events", "query", "--event-type", "issue_claimed", "--json"])
        .output()
        .unwrap();
    let events_json: Value = serde_json::from_slice(&events.stdout).unwrap();
    assert_eq!(events_json["count"].as_u64().unwrap(), 1);

    // First-occurrence: re-acquiring as the same agent (release then re-acquire)
    // must not move claimed_at.
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "release", &issue_id])
        .assert()
        .success();
    acquire("agent:test-1");

    let show2 = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "show", &issue_id, "--json"])
        .output()
        .unwrap();
    let show2_json: Value = serde_json::from_slice(&show2.stdout).unwrap();
    assert_eq!(show2_json["claimed_at"].as_str().unwrap(), first_claimed);
}

#[test]
fn test_claim_acquire_json_output() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    // Acquire claim with JSON output
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    // success field removed
    assert!(json["lease_id"].is_string());
    assert_eq!(json["issue_id"], issue_id);
    assert_eq!(json["ttl_secs"], 600);
}

#[test]
fn test_claim_acquire_already_claimed_error() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    // First claim succeeds
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
        ])
        .assert()
        .success();

    // Second claim should fail with actionable error
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-2",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already claimed"))
        .stderr(predicate::str::contains("Possible causes:"))
        .stderr(predicate::str::contains("To fix:"))
        .stderr(predicate::str::contains("jit claim status"));
}

#[test]
fn test_claim_list_shows_active_leases() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    // Acquire a claim
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
        ])
        .assert()
        .success();

    // List should show the lease
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&issue_id))
        .stdout(predicate::str::contains("agent:test-1"));
}

#[test]
fn test_claim_list_json_output() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    // Acquire a claim
    let acquire_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["lease_id"].as_str().unwrap();

    // List claims with JSON
    let list_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list", "--json"])
        .output()
        .unwrap();

    assert!(list_output.status.success());

    let list_json: Value = serde_json::from_slice(&list_output.stdout).unwrap();
    // Envelope removed - success field no longer present
    assert!(list_json["leases"].is_array());

    let leases = list_json["leases"].as_array().unwrap();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0]["lease_id"], lease_id);
    assert_eq!(leases[0]["issue_id"], issue_id);
}

#[test]
fn test_claim_release_happy_path() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    // Acquire claim
    let acquire_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let _lease_id = acquire_json["lease_id"].as_str().unwrap();

    // Release claim by ISSUE id (no lease UUID required).
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:test-1")
        .args(["claim", "release", &issue_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Released lease"));
}

#[test]
fn test_claim_release_no_active_lease_error() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Unclaimed Issue");

    // Releasing an existing issue with no active lease should fail with an
    // actionable error that mentions "not found".
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:test")
        .args(["claim", "release", &issue_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no active lease"))
        .stderr(predicate::str::contains("not found"))
        .stderr(predicate::str::contains("Possible causes:"))
        .stderr(predicate::str::contains("To fix:"))
        .stderr(predicate::str::contains("jit claim list"));
}

#[test]
fn test_claim_release_by_issue_succeeds_for_different_owner() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Owned Issue");

    // Agent A acquires the lease.
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:owner",
            "--json",
        ])
        .assert()
        .success();

    // A DIFFERENT agent releases by issue id; the audit reports the prior owner
    // and the acting identity.
    let release_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:stranger")
        .args(["claim", "release", &issue_id, "--json"])
        .output()
        .unwrap();

    assert!(release_output.status.success());
    let release_json: Value = serde_json::from_slice(&release_output.stdout).unwrap();
    assert_eq!(release_json["issue_id"], issue_id);
    assert_eq!(release_json["previous_owner"], "agent:owner");
    assert_eq!(release_json["actor"], "agent:stranger");

    // Lease is gone.
    let list_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list", "--json"])
        .output()
        .unwrap();
    let list_json: Value = serde_json::from_slice(&list_output.stdout).unwrap();
    assert_eq!(list_json["leases"].as_array().unwrap().len(), 0);
}

#[test]
fn test_claim_workflow_end_to_end() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue for Workflow");

    // Step 1: Acquire claim
    let acquire_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:workflow-test",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(acquire_output.status.success());
    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["lease_id"].as_str().unwrap();

    // Step 2: List claims and verify
    let list_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list", "--json"])
        .output()
        .unwrap();

    assert!(list_output.status.success());
    let list_json: Value = serde_json::from_slice(&list_output.stdout).unwrap();
    let leases = list_json["leases"].as_array().unwrap();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0]["lease_id"], lease_id);

    // Step 3: Release claim by issue id
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:workflow-test")
        .args(["claim", "release", &issue_id])
        .assert()
        .success();

    // Step 4: Verify lease is gone
    let list_after_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list", "--json"])
        .output()
        .unwrap();

    let list_after_json: Value = serde_json::from_slice(&list_after_output.stdout).unwrap();
    let leases_after = list_after_json["leases"].as_array().unwrap();
    assert_eq!(
        leases_after.len(),
        0,
        "Lease should be removed after release"
    );
}

#[test]
fn test_claim_status_shows_lease_details() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    // Acquire claim
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
        ])
        .assert()
        .success();

    // Check status
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:test-1")
        .args(["claim", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&issue_id))
        .stdout(predicate::str::contains("agent:test-1"));
}

#[test]
fn test_claim_acquire_nonexistent_issue_error() {
    let temp = setup_repo();

    // Try to claim non-existent issue
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            "00000000-0000-0000-0000-000000000000",
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("Issue")));
}

#[test]
fn test_claim_renew_success() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue for Renew");

    // Acquire claim
    let acquire_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["lease_id"].as_str().unwrap();

    // Renew claim
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:test-1")
        .args(["claim", "renew", lease_id, "--extension", "300"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Renewed lease"));
}

#[test]
fn test_claim_renew_json_output() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue for Renew JSON");

    // Acquire claim
    let acquire_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["lease_id"].as_str().unwrap();

    // Renew with JSON output
    let renew_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:test-1")
        .args(["claim", "renew", lease_id, "--extension", "300", "--json"])
        .output()
        .unwrap();

    assert!(renew_output.status.success());

    let renew_json: Value = serde_json::from_slice(&renew_output.stdout).unwrap();
    // Envelope removed - success field no longer present
    // Check that lease object is returned
    assert!(renew_json["lease"].is_object());
    assert_eq!(renew_json["lease"]["lease_id"], lease_id);
}

#[test]
fn test_claim_renew_not_found_error() {
    let temp = setup_repo();
    let missing_lease = "00000000-0000-0000-0000-000000000000";

    let invoke = |json: bool| {
        let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
        command
            .current_dir(temp.path())
            .env("JIT_AGENT_ID", "agent:test")
            .args(["claim", "renew", missing_lease, "--extension", "300"]);
        if json {
            command.arg("--json");
        }
        command.output().unwrap()
    };

    assert_claim_failure_parity(&invoke(false), &invoke(true), ErrorCode::IssueNotFound);
}

#[test]
fn test_claim_force_evict_success() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue for Force-Evict");

    // Acquire claim
    let acquire_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["lease_id"].as_str().unwrap();

    // Force-evict (admin operation - doesn't need same agent)
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "force-evict", lease_id, "--reason", "test cleanup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Force-evicted"));

    // Verify lease is gone
    let list_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list", "--json"])
        .output()
        .unwrap();

    let list_json: Value = serde_json::from_slice(&list_output.stdout).unwrap();
    let leases = list_json["leases"].as_array().unwrap();
    assert_eq!(leases.len(), 0, "Lease should be evicted");
}

#[test]
fn test_claim_force_evict_requires_reason() {
    let temp = setup_repo();

    // Force-evict without reason should fail (clap validation)
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "force-evict", "01FAKE0000000000000000000"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--reason"));
}

#[test]
fn test_claim_heartbeat_success() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue for Heartbeat");

    // Acquire indefinite claim (TTL=0)
    let acquire_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "0",
            "--agent-id",
            "agent:test-1",
            "--reason",
            "manual review",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        acquire_output.status.success(),
        "Acquire should succeed: {}",
        String::from_utf8_lossy(&acquire_output.stderr)
    );

    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["lease_id"].as_str().unwrap();

    // Heartbeat to update last_beat
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:test-1")
        .args(["claim", "heartbeat", lease_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Heartbeat sent"));
}

#[test]
fn test_claim_heartbeat_json_output() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue for Heartbeat JSON");

    // Acquire indefinite claim
    let acquire_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "0",
            "--agent-id",
            "agent:test-1",
            "--reason",
            "manual review",
            "--json",
        ])
        .output()
        .unwrap();

    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["lease_id"].as_str().unwrap();

    // Heartbeat with JSON output
    let heartbeat_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:test-1")
        .args(["claim", "heartbeat", lease_id, "--json"])
        .output()
        .unwrap();

    assert!(heartbeat_output.status.success());

    let hb_json: Value = serde_json::from_slice(&heartbeat_output.stdout).unwrap();
    // Envelope removed - success field no longer present
    assert_eq!(hb_json["lease_id"], lease_id);
    assert!(hb_json["message"].is_string());
}

#[test]
fn test_claim_heartbeat_not_found_error() {
    let temp = setup_repo();

    // Try to heartbeat non-existent lease
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:test")
        .args(["claim", "heartbeat", "01FAKE0000000000000000000"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_claim_ttl0_workflow_acquire_heartbeat_release() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "TTL=0 Workflow Test");

    // Step 1: Acquire indefinite claim
    let acquire_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "0",
            "--agent-id",
            "agent:workflow-test",
            "--reason",
            "integration test",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(acquire_output.status.success());
    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["lease_id"].as_str().unwrap();
    assert!(
        acquire_json["expires_at"].is_null(),
        "Indefinite lease should have no expiry"
    );
    assert_eq!(acquire_json["ttl_secs"], 0);

    // Step 2: List claims and verify indefinite marker
    let list_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list"])
        .output()
        .unwrap();

    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    assert!(
        list_stdout.contains("indefinite") || list_stdout.contains("never"),
        "List should indicate indefinite lease"
    );

    // Step 3: Heartbeat
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:workflow-test")
        .args(["claim", "heartbeat", lease_id])
        .assert()
        .success();

    // Step 4: Status check
    let status_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:workflow-test")
        .args(["claim", "status", "--json"])
        .output()
        .unwrap();

    let status_json: Value = serde_json::from_slice(&status_output.stdout).unwrap();
    let leases = status_json["leases"].as_array().unwrap();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0]["lease_id"], lease_id);

    // Step 5: Release by issue id
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:workflow-test")
        .args(["claim", "release", &issue_id])
        .assert()
        .success();

    // Verify lease is gone
    let final_list = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list", "--json"])
        .output()
        .unwrap();

    let final_json: Value = serde_json::from_slice(&final_list.stdout).unwrap();
    let final_leases = final_json["leases"].as_array().unwrap();
    assert_eq!(final_leases.len(), 0, "Lease should be released");
}

/// Setup a jit repository WITHOUT git (to test git-requirement errors on claim/lease commands).
fn setup_non_git_jit_repo() -> TempDir {
    let temp = TempDir::new().unwrap();

    // Initialize jit only — no git init.
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    temp
}

#[test]
fn test_claim_acquire_outside_git_repo_emits_git_requirement_error() {
    let temp = setup_non_git_jit_repo();
    let issue_id = create_issue(temp.path(), "Test Issue Outside Git");

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("git repository"))
        .stderr(predicate::str::contains("claim").or(predicate::str::contains("lease")))
        // REQ-04 (jit:30a3b5c1): no git repository at all hints `git init`.
        .stderr(predicate::str::contains("git init"));
}

#[test]
fn test_claim_list_outside_git_repo_emits_git_requirement_error() {
    let temp = setup_non_git_jit_repo();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("git repository"))
        .stderr(predicate::str::contains("claim").or(predicate::str::contains("lease")));
}

/// The `--json` claim path must honor the same exit-10 contract as the human
/// path: running `claim acquire --json` outside a git repository emits a JSON
/// error naming the git requirement and exits with code 10.
#[test]
fn test_claim_acquire_json_outside_git_repo_exits_10_with_git_requirement() {
    let temp = setup_non_git_jit_repo();
    let issue_id = create_issue(temp.path(), "Test Issue Outside Git JSON");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(10),
        "claim acquire --json outside git must exit 10 (external dependency)"
    );

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "CLAIM_REQUIRES_GIT");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("git repository"),
        "message must name the git requirement, got: {message}"
    );
}

/// `claim list --json` outside a git repository must also exit 10 with a JSON
/// error naming the git requirement.
#[test]
fn test_claim_list_json_outside_git_repo_exits_10_with_git_requirement() {
    let temp = setup_non_git_jit_repo();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list", "--json"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(10),
        "claim list --json outside git must exit 10 (external dependency)"
    );

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "CLAIM_REQUIRES_GIT");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("git repository"),
        "message must name the git requirement, got: {message}"
    );
}

/// REQ-01 (jit:30a3b5c1): the already-claimed error is an
/// [`jit::errors::ActionableError`] rendered through the top-level CLI
/// printer; the printer must add the "Error: " prefix exactly once, never a
/// doubled "Error: Error: ".
#[test]
fn test_claim_acquire_already_claimed_error_has_single_error_prefix() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
        ])
        .assert()
        .success();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-2",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.matches("Error:").count(),
        1,
        "expected exactly one 'Error:' prefix, got: {stderr}"
    );
}

/// Setup a jit repository inside a git repository that has been `git init`ed
/// but has no commits yet, to test the "git repo without commits" branch of
/// the claims-require-git failure (REQ-04, jit:30a3b5c1). Distinct from
/// [`setup_non_git_jit_repo`], which has no `.git` directory at all.
fn setup_git_repo_without_commits() -> TempDir {
    let temp = TempDir::new().unwrap();

    Command::new("git")
        .current_dir(temp.path())
        .args(["init"])
        .status()
        .unwrap();

    // Initialize jit only — deliberately no commit yet, so HEAD never resolves.
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    temp
}

/// REQ-04 (jit:30a3b5c1): a git repository with zero commits fails
/// `--abbrev-ref HEAD` exactly like a directory with no git repository at
/// all, but the hint must point at making a commit rather than `git init`
/// (the repository already exists).
#[test]
fn test_claim_acquire_in_git_repo_without_commits_hints_commit_not_git_init() {
    let temp = setup_git_repo_without_commits();
    let issue_id = create_issue(temp.path(), "Test Issue No Commits");

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
        ])
        .assert()
        .failure()
        .code(10)
        .stderr(predicate::str::contains("git repository"))
        .stderr(predicate::str::contains("commit"))
        .stderr(predicate::str::contains("git init").not());
}

/// JSON-mode counterpart of
/// [`test_claim_acquire_in_git_repo_without_commits_hints_commit_not_git_init`]:
/// same exit code and error code as the no-repository case, but the message
/// hints a commit instead of `git init`.
#[test]
fn test_claim_acquire_json_in_git_repo_without_commits_hints_commit_not_git_init() {
    let temp = setup_git_repo_without_commits();
    let issue_id = create_issue(temp.path(), "Test Issue No Commits JSON");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(10));

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "CLAIM_REQUIRES_GIT");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("commit"),
        "message must hint at making a commit, got: {message}"
    );
    assert!(
        !message.contains("git init"),
        "must not suggest `git init` for a repository that already exists, got: {message}"
    );
}

// ============================================================================
// REQ-03 (jit:30a3b5c1): assignment-command help and lease-command help each
// cross-reference the other, since `issue claim`/`release` and `claim
// acquire`/`release` share verbs but are different mechanisms (assignee
// bookkeeping vs. an exclusive, time-boxed lease).
// ============================================================================

fn help_text(args: &[&str]) -> String {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .args(args)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn test_issue_assign_help_cross_references_lease_commands() {
    let help = help_text(&["issue", "assign", "--help"]);
    assert!(
        help.contains("jit claim acquire"),
        "issue assign --help should cross-reference jit claim acquire, got: {help}"
    );
}

#[test]
fn test_issue_claim_help_cross_references_lease_commands() {
    let help = help_text(&["issue", "claim", "--help"]);
    assert!(
        help.contains("jit claim acquire"),
        "issue claim --help should cross-reference jit claim acquire, got: {help}"
    );
}

// ============================================================================
// REQ-01 (jit:1a63ef75): `issue claim --help` documents the idempotent
// same-assignee re-claim and the in_progress promotion, replacing the stale
// "Claim an unassigned issue" wording (same-assignee re-claim was already
// idempotent by the time of this sweep — jit:30a3b5c1 landed the behavior).
// ============================================================================

#[test]
fn test_issue_claim_help_documents_promotion_and_reclaim_idempotency() {
    let help = help_text(&["issue", "claim", "--help"]);
    assert!(
        help.contains("in_progress"),
        "issue claim --help should document promotion to in_progress, got: {help}"
    );
    assert!(
        help.to_lowercase().contains("re-claiming"),
        "issue claim --help should document idempotent same-assignee re-claim, got: {help}"
    );
    assert!(
        !help.contains("Claim an unassigned issue"),
        "issue claim --help should not claim the target must be unassigned, got: {help}"
    );
}

#[test]
fn test_issue_release_help_cross_references_lease_commands() {
    let help = help_text(&["issue", "release", "--help"]);
    assert!(
        help.contains("jit claim acquire"),
        "issue release --help should cross-reference jit claim acquire, got: {help}"
    );
}

#[test]
fn test_issue_unassign_help_cross_references_lease_commands() {
    let help = help_text(&["issue", "unassign", "--help"]);
    assert!(
        help.contains("jit claim acquire"),
        "issue unassign --help should cross-reference jit claim acquire, got: {help}"
    );
}

#[test]
fn test_claim_acquire_help_cross_references_assignment_commands() {
    let help = help_text(&["claim", "acquire", "--help"]);
    assert!(
        help.contains("jit issue claim"),
        "claim acquire --help should cross-reference jit issue claim, got: {help}"
    );
}

#[test]
fn test_claim_release_help_cross_references_assignment_commands() {
    let help = help_text(&["claim", "release", "--help"]);
    assert!(
        help.contains("jit issue claim"),
        "claim release --help should cross-reference jit issue claim, got: {help}"
    );
}
