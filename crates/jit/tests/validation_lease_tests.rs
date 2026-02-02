//! Integration tests for lease validation functionality
//!
//! Tests `jit validate --leases` command with real git worktrees and claims.

use chrono::{Duration, Utc};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use ulid::Ulid;

/// Get path to jit binary
fn jit_binary() -> PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop(); // Remove test binary name
    path.pop(); // Remove 'deps'
    path.push("jit");
    path
}

/// Setup a git repository with jit initialized
fn setup_git_repo() -> TempDir {
    let temp = TempDir::new().unwrap();

    // Initialize git
    Command::new("git")
        .args(["init"])
        .current_dir(temp.path())
        .status()
        .unwrap();

    // Configure git (required for commits)
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp.path())
        .status()
        .unwrap();

    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(temp.path())
        .status()
        .unwrap();

    // Initialize jit
    Command::new(jit_binary())
        .args(["init"])
        .current_dir(temp.path())
        .status()
        .unwrap();

    // Create initial commit (needed for worktrees)
    Command::new("git")
        .args(["add", "."])
        .current_dir(temp.path())
        .status()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(temp.path())
        .status()
        .unwrap();

    temp
}

/// Create a git worktree with jit initialized
fn create_worktree(base_repo: &Path, worktree_name: &str) -> PathBuf {
    // Create worktree inside the temp directory (as sibling to base repo)
    let parent_dir = base_repo.parent().unwrap();
    let worktree_path = parent_dir.join(worktree_name);

    // Clean up if it already exists (from failed test run)
    if worktree_path.exists() {
        let _ = std::fs::remove_dir_all(&worktree_path);
    }

    // Create worktree
    Command::new("git")
        .args([
            "worktree",
            "add",
            worktree_path.to_str().unwrap(),
            "-b",
            worktree_name,
        ])
        .current_dir(base_repo)
        .status()
        .unwrap();

    // Initialize jit in worktree (creates .jit/worktree.json)
    Command::new(jit_binary())
        .args(["worktree", "info"])
        .current_dir(&worktree_path)
        .output()
        .unwrap();

    worktree_path
}

/// Get worktree ID from a worktree directory
fn get_worktree_id(worktree_path: &Path) -> String {
    let output = Command::new(jit_binary())
        .args(["worktree", "info", "--json"])
        .current_dir(worktree_path)
        .output()
        .unwrap();

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    json["data"]["worktree_id"].as_str().unwrap().to_string()
}

/// Write a claims index file
fn write_claims_index(repo_path: &Path, leases: Vec<Value>) {
    let git_jit_dir = repo_path.join(".git/jit");
    fs::create_dir_all(&git_jit_dir).unwrap();

    let claims_index = serde_json::json!({
        "schema_version": 1,
        "generated_at": Utc::now(),
        "last_seq": leases.len(),
        "stale_threshold_secs": 3600,
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
    let repo = setup_git_repo();

    // Run validation without claims index
    let output = Command::new(jit_binary())
        .args(["validate", "--leases", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Should succeed with no claims index"
    );

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["valid"], true);
}

#[test]
fn test_validate_leases_detects_expired_lease() {
    let repo = setup_git_repo();
    let worktree_name = format!("wt-{}", Ulid::new().to_string().to_lowercase());
    let worktree_path = create_worktree(repo.path(), &worktree_name);
    let worktree_id = get_worktree_id(&worktree_path);

    // Create an issue
    let create_output = Command::new(jit_binary())
        .args(["issue", "create", "-t", "Test Issue", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let create_json: Value = serde_json::from_slice(&create_output.stdout).unwrap();
    let issue_id = create_json["data"]["id"].as_str().unwrap();

    // Create expired lease (acquired 2 days ago, 1 hour TTL)
    let acquired_at = Utc::now() - Duration::days(2);
    let expired_lease = create_test_lease(
        "lease-expired-123",
        issue_id,
        &worktree_id,
        3600, // 1 hour TTL
        acquired_at,
    );

    write_claims_index(repo.path(), vec![expired_lease]);

    // Run validation
    let output = Command::new(jit_binary())
        .args(["validate", "--leases"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    assert!(!output.status.success(), "Should fail with expired lease");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Expired"),
        "Error should mention expiration"
    );
    assert!(
        stderr.contains("lease-expired-123") || stderr.contains(&issue_id[..8]),
        "Error should reference the lease or issue"
    );
}

#[test]
fn test_validate_leases_detects_missing_worktree() {
    let repo = setup_git_repo();

    // Create worktree, get its ID, then remove it
    let worktree_path = create_worktree(repo.path(), "doomed-wt");
    let worktree_id = get_worktree_id(&worktree_path);

    // Create an issue
    let create_output = Command::new(jit_binary())
        .args(["issue", "create", "-t", "Test Issue", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let create_json: Value = serde_json::from_slice(&create_output.stdout).unwrap();
    let issue_id = create_json["data"]["id"].as_str().unwrap();

    // Create valid lease for the worktree
    let lease = create_test_lease(
        "lease-valid-123",
        issue_id,
        &worktree_id,
        7200, // 2 hours TTL
        Utc::now(),
    );

    write_claims_index(repo.path(), vec![lease]);

    // Remove the worktree (use --force since it has .jit directory)
    Command::new("git")
        .args(["worktree", "remove", "--force", "doomed-wt"])
        .current_dir(repo.path())
        .status()
        .unwrap();

    // Run validation
    let output = Command::new(jit_binary())
        .args(["validate", "--leases"])
        .current_dir(repo.path())
        .output()
        .unwrap();

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
    let repo = setup_git_repo();
    let worktree_name = format!("wt-{}", Ulid::new().to_string().to_lowercase());
    let worktree_path = create_worktree(repo.path(), &worktree_name);
    let worktree_id = get_worktree_id(&worktree_path);

    // Create lease for non-existent issue
    let fake_issue_id = "00000000-0000-0000-0000-000000000000";
    let lease = create_test_lease(
        "lease-orphan-123",
        fake_issue_id,
        &worktree_id,
        7200,
        Utc::now(),
    );

    write_claims_index(repo.path(), vec![lease]);

    // Run validation
    let output = Command::new(jit_binary())
        .args(["validate", "--leases"])
        .current_dir(repo.path())
        .output()
        .unwrap();

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
    let repo = setup_git_repo();
    let worktree_path = create_worktree(repo.path(), "valid-wt");
    let worktree_id = get_worktree_id(&worktree_path);

    // Create an issue
    let create_output = Command::new(jit_binary())
        .args(["issue", "create", "-t", "Valid Issue", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let create_json: Value = serde_json::from_slice(&create_output.stdout).unwrap();
    let issue_id = create_json["data"]["id"].as_str().unwrap();

    // Create valid lease (not expired, worktree exists, issue exists)
    let lease = create_test_lease(
        "lease-valid-456",
        issue_id,
        &worktree_id,
        7200, // 2 hours TTL
        Utc::now(),
    );

    write_claims_index(repo.path(), vec![lease]);

    // Run validation
    let output = Command::new(jit_binary())
        .args(["validate", "--leases", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Should succeed with all valid leases"
    );

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["valid"], true);
}

#[test]
fn test_validate_leases_json_output_on_failure() {
    let repo = setup_git_repo();
    let worktree_name = format!("wt-{}", Ulid::new().to_string().to_lowercase());
    let worktree_path = create_worktree(repo.path(), &worktree_name);
    let worktree_id = get_worktree_id(&worktree_path);

    // Create an issue
    let create_output = Command::new(jit_binary())
        .args(["issue", "create", "-t", "Test Issue", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let create_json: Value = serde_json::from_slice(&create_output.stdout).unwrap();
    let issue_id = create_json["data"]["id"].as_str().unwrap();

    // Create expired lease
    let acquired_at = Utc::now() - Duration::days(1);
    let expired_lease = create_test_lease(
        "lease-json-test",
        issue_id,
        &worktree_id,
        3600, // 1 hour TTL
        acquired_at,
    );

    write_claims_index(repo.path(), vec![expired_lease]);

    // Run validation with JSON output
    let output = Command::new(jit_binary())
        .args(["validate", "--leases", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "Command should exit with error code"
    );

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    // Note: "success" field is always true (command executed), but "valid" is false
    assert_eq!(
        json["data"]["valid"], false,
        "Validation should report invalid"
    );

    // Check that validations array contains lease validation result
    let validations = json["data"]["validations"].as_array().unwrap();
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
