//! Integration tests for lease validation functionality
//!
//! Tests `jit validate --leases` command with real git worktrees and claims.

use chrono::{Duration, Utc};
use jit::storage::claim_coordinator::Lease;
use jit::storage::worktree_paths::WorktreePaths;
use jit::test_cli::JitCli;
use jit::test_lease_store::plant_lease;
use serde_json::Value;

/// Create a lease with an explicit acquisition time.
fn create_test_lease(
    lease_id: &str,
    issue_id: &str,
    worktree_id: &str,
    ttl_secs: i64,
    acquired_at: chrono::DateTime<Utc>,
) -> Lease {
    let expires_at = if ttl_secs > 0 {
        Some(acquired_at + Duration::seconds(ttl_secs))
    } else {
        None
    };

    Lease {
        lease_id: lease_id.parse().unwrap(),
        issue_id: issue_id.parse().unwrap(),
        agent_id: "agent:test".parse().unwrap(),
        worktree_id: worktree_id.parse().unwrap(),
        branch: Some("test-branch".into()),
        ttl_secs: ttl_secs.try_into().unwrap(),
        acquired_at,
        expires_at,
        last_beat: acquired_at,
    }
}

#[test]
fn test_validate_leases_without_lease_records_is_valid() {
    let (primary, _linked) = JitCli::linked_checkouts().unwrap();

    let json = primary.run_json(["validate", "--leases"]).unwrap();

    assert_eq!(json["valid"], true);
}

#[test]
fn test_validate_leases_detects_expired_lease() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let worktree_id = linked.run_json(["worktree", "info"]).unwrap()["worktree_id"]
        .as_str()
        .unwrap()
        .to_string();
    let issue_id = primary.create_issue(["-t", "Test Issue"]).unwrap();

    // Acquired 2 days ago with a 1 hour TTL.
    let expired_lease = create_test_lease(
        "00000000-0000-4000-8000-000000000001",
        &issue_id,
        &worktree_id,
        3600,
        Utc::now() - Duration::days(2),
    );
    let paths =
        WorktreePaths::detect_for_data_root(&primary.root().join(".jit"), primary.root()).unwrap();
    plant_lease(&paths.shared_jit, &expired_lease).unwrap();

    let output = primary.command(["validate", "--leases"]).output().unwrap();

    assert!(!output.status.success(), "Should fail with expired lease");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Expired"),
        "Error should mention expiration"
    );
    assert!(
        stderr.contains("00000000-0000-4000-8000-000000000001") || stderr.contains(&issue_id[..8]),
        "Error should reference the lease or issue"
    );
}

#[test]
fn test_validate_leases_detects_missing_worktree() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let worktree_id = linked.run_json(["worktree", "info"]).unwrap()["worktree_id"]
        .as_str()
        .unwrap()
        .to_string();
    let issue_id = primary.create_issue(["-t", "Test Issue"]).unwrap();
    let lease = create_test_lease(
        "00000000-0000-4000-8000-000000000002",
        &issue_id,
        &worktree_id,
        7200,
        Utc::now(),
    );
    let paths =
        WorktreePaths::detect_for_data_root(&primary.root().join(".jit"), primary.root()).unwrap();
    plant_lease(&paths.shared_jit, &lease).unwrap();

    // `--force`: the linked checkout carries untracked `.jit` state.
    primary
        .git([
            "worktree",
            "remove",
            "--force",
            linked.root().to_str().unwrap(),
        ])
        .unwrap();

    let output = primary.command(["validate", "--leases"]).output().unwrap();

    assert!(
        !output.status.success(),
        "Should fail with missing worktree"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Worktree") && stderr.contains("no longer exists"),
        "Error should mention missing worktree"
    );
    assert!(
        stderr.contains(&worktree_id),
        "Error should reference worktree ID"
    );
}

#[test]
fn test_validate_leases_detects_missing_issue() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let worktree_id = linked.run_json(["worktree", "info"]).unwrap()["worktree_id"]
        .as_str()
        .unwrap()
        .to_string();
    let fake_issue_id = "00000000-0000-0000-0000-000000000000";
    let lease = create_test_lease(
        "00000000-0000-4000-8000-000000000003",
        fake_issue_id,
        &worktree_id,
        7200,
        Utc::now(),
    );
    let paths =
        WorktreePaths::detect_for_data_root(&primary.root().join(".jit"), primary.root()).unwrap();
    plant_lease(&paths.shared_jit, &lease).unwrap();

    let output = primary.command(["validate", "--leases"]).output().unwrap();

    assert!(!output.status.success(), "Should fail with missing issue");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Issue") && stderr.contains("no longer exists"),
        "Error should mention missing issue"
    );
    assert!(
        stderr.contains(&fake_issue_id[..8]),
        "Error should reference issue ID"
    );
}

#[test]
fn test_validate_leases_all_valid_succeeds() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let worktree_id = linked.run_json(["worktree", "info"]).unwrap()["worktree_id"]
        .as_str()
        .unwrap()
        .to_string();
    let issue_id = primary.create_issue(["-t", "Valid Issue"]).unwrap();
    // Not expired, worktree exists, issue exists.
    let lease = create_test_lease(
        "00000000-0000-4000-8000-000000000004",
        &issue_id,
        &worktree_id,
        7200,
        Utc::now(),
    );
    let paths =
        WorktreePaths::detect_for_data_root(&primary.root().join(".jit"), primary.root()).unwrap();
    plant_lease(&paths.shared_jit, &lease).unwrap();

    let json = primary.run_json(["validate", "--leases"]).unwrap();

    assert_eq!(json["valid"], true);
}

#[test]
fn test_validate_leases_json_output_on_failure() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let worktree_id = linked.run_json(["worktree", "info"]).unwrap()["worktree_id"]
        .as_str()
        .unwrap()
        .to_string();
    let issue_id = primary.create_issue(["-t", "Test Issue"]).unwrap();
    let expired_lease = create_test_lease(
        "00000000-0000-4000-8000-000000000005",
        &issue_id,
        &worktree_id,
        3600,
        Utc::now() - Duration::days(1),
    );
    let paths =
        WorktreePaths::detect_for_data_root(&primary.root().join(".jit"), primary.root()).unwrap();
    plant_lease(&paths.shared_jit, &expired_lease).unwrap();

    let output = primary
        .command(["validate", "--leases", "--json"])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "Command should exit with error code"
    );

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json.as_object().map(|object| object.len()), Some(1));
    assert_eq!(json["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(output.status.code(), Some(4));
    assert!(output.stderr.is_empty(), "JSON failure stays stream-pure");
    let details = &json["error"]["details"];
    assert_eq!(details["valid"], false, "Validation should report invalid");

    let validations = details["validations"].as_array().unwrap();
    assert!(!validations.is_empty());

    let lease_validation = &validations[0];
    assert_eq!(lease_validation["validation"], "leases");
    assert_eq!(lease_validation["valid"], false);

    let message = lease_validation["message"].as_str().unwrap();
    assert!(
        message.contains("Expired"),
        "Message should mention expiration"
    );
}
