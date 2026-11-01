//! Integration tests for lease validation functionality
//!
//! Tests `jit validate --leases` command with real git worktrees and claims.

use chrono::{Duration, Utc};
use jit::test_cli::JitCli;
use serde_json::Value;
use std::fs;
use std::path::Path;

/// Write a claims index file
fn write_claims_index(repo_path: &Path, leases: Vec<Value>) {
    let git_jit_dir = repo_path.join(".git/jit");
    fs::create_dir_all(&git_jit_dir).unwrap();

    let claims_index = serde_json::json!({
        "schema_version": 1,
        "generated_at": Utc::now(),
        "last_seq": leases.len(),
        "leases": leases
    });

    let claims_path = git_jit_dir.join("claims.index.json");
    fs::write(
        claims_path,
        serde_json::to_string_pretty(&claims_index).unwrap(),
    )
    .unwrap();
}

/// Create a test lease JSON object
fn create_test_lease(
    lease_id: &str,
    issue_id: &str,
    worktree_id: &str,
    ttl_secs: i64,
    acquired_at: chrono::DateTime<Utc>,
) -> Value {
    let expires_at = if ttl_secs > 0 {
        Some(acquired_at + Duration::seconds(ttl_secs))
    } else {
        None
    };

    serde_json::json!({
        "lease_id": lease_id,
        "issue_id": issue_id,
        "agent_id": "agent:test",
        "worktree_id": worktree_id,
        "branch": Some("test-branch"),
        "ttl_secs": ttl_secs,
        "acquired_at": acquired_at,
        "expires_at": expires_at,
        "last_beat": acquired_at
    })
}

#[test]
fn test_validate_leases_no_claims_index() {
    let (primary, _linked) = JitCli::linked_checkouts().unwrap();

    let json = primary.run_json(["validate", "--leases"]).unwrap();

    assert_eq!(json["valid"], true);
}

#[test]
fn test_validate_repository_detects_invalid_claims_index() {
    let (primary, _linked) = JitCli::linked_checkouts().unwrap();
    let claims_dir = primary.root().join(".git/jit");
    fs::create_dir_all(&claims_dir).unwrap();
    fs::write(
        claims_dir.join("claims.index.json"),
        serde_json::json!({
            "schema_version": 2,
            "generated_at": Utc::now(),
            "last_seq": 0,
            "leases": []
        })
        .to_string(),
    )
    .unwrap();

    let output = primary
        .command(["validate"])
        .env_remove("JIT_TEST_MODE")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        combined.contains("Claims index validation failed"),
        "{combined}"
    );
    assert!(combined.contains("Invalid schema version"), "{combined}");

    let skipped = primary
        .command_with_env(["validate"], [("JIT_TEST_MODE", "1")])
        .output()
        .unwrap();
    assert!(
        skipped.status.success(),
        "JIT_TEST_MODE must preserve the established claims-check bypass: {}",
        String::from_utf8_lossy(&skipped.stderr)
    );
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
    write_claims_index(primary.root(), vec![expired_lease]);

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
    write_claims_index(primary.root(), vec![lease]);

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
    write_claims_index(primary.root(), vec![lease]);

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
    write_claims_index(primary.root(), vec![lease]);

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
    write_claims_index(primary.root(), vec![expired_lease]);

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
    assert_eq!(json["error"]["code"], "GENERIC_ERROR");
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
