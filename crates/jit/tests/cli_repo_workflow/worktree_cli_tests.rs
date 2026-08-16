//! Integration tests for worktree CLI commands
//!
//! Tests the full CLI experience for worktree and validate commands.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;
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

    // Create initial commit (required for worktrees)
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

/// Attach a local bare repository as `origin` and publish the current `HEAD`.
fn attach_origin_remote(repo: &Path) -> TempDir {
    let remote = TempDir::new().unwrap();

    Command::new("git")
        .current_dir(remote.path())
        .args(["init", "--bare"])
        .assert()
        .success();

    let remote_path = remote.path().to_str().unwrap();
    Command::new("git")
        .current_dir(repo)
        .args(["remote", "add", "origin", remote_path])
        .assert()
        .success();

    Command::new("git")
        .current_dir(repo)
        .args(["push", "origin", "HEAD:refs/heads/main"])
        .assert()
        .success();

    Command::new("git")
        .current_dir(repo)
        .args(["fetch", "origin"])
        .assert()
        .success();

    remote
}

/// Create a git worktree with a unique name
fn create_worktree(base_repo: &Path, worktree_name: &str) -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};

    // Use timestamp to ensure unique worktree names across test runs
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let unique_name = format!("{}-{}", worktree_name, timestamp);

    let parent_dir = base_repo.parent().unwrap();
    let worktree_path = parent_dir.join(&unique_name);

    // Clean up if exists
    if worktree_path.exists() {
        let _ = fs::remove_dir_all(&worktree_path);
    }

    let status = Command::new("git")
        .args([
            "worktree",
            "add",
            worktree_path.to_str().unwrap(),
            "-b",
            &unique_name,
        ])
        .current_dir(base_repo)
        .status()
        .unwrap();

    assert!(status.success(), "Failed to create worktree");

    worktree_path
}

/// Create an issue in `repo` and return its full id.
fn create_issue(repo: &Path, title: &str) -> String {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(repo)
        .args(["issue", "create", "--title", title, "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    json["id"].as_str().unwrap().to_string()
}

/// Sum of `active_claims` across all worktrees from `jit worktree list --json`.
fn worktree_active_claims_total(repo: &Path) -> u64 {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(repo)
        .args(["worktree", "list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    json["worktrees"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["active_claims"].as_u64().unwrap())
        .sum()
}

/// Number of active leases from `jit claim list --json`.
fn claim_list_count(repo: &Path) -> u64 {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(repo)
        .args(["claim", "list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    json["count"].as_u64().unwrap()
}

// === jit worktree info tests ===

#[test]
fn test_worktree_info_success() {
    let temp = setup_repo();

    // Run worktree info
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["worktree", "info"])
        .assert()
        .success()
        .stdout(predicate::str::contains("wt:"))
        .stdout(predicate::str::contains("main").or(predicate::str::contains("master")));
}

#[test]
fn test_worktree_info_json_output() {
    let temp = setup_repo();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["worktree", "info", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["worktree_id"].is_string());
    assert!(json["branch"].is_string());
    assert!(json["root_path"].is_string());

    // Worktree ID should start with "wt:"
    let wt_id = json["worktree_id"].as_str().unwrap();
    assert!(wt_id.starts_with("wt:"), "ID should start with 'wt:'");
}

#[test]
fn test_worktree_info_follows_overridden_data_root_checkout() {
    let temp = setup_repo();
    let worktree_path = create_worktree(temp.path(), "override-selected-worktree");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_DATA_DIR", worktree_path.join(".jit"))
        .args(["worktree", "info", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "worktree info failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["root_path"].as_str(),
        Some(worktree_path.to_str().unwrap())
    );
    assert_eq!(json["is_main_worktree"], false);
}

#[test]
fn test_init_follows_overridden_linked_data_root_and_replaces_copied_identity() {
    let temp = setup_repo();
    let worktree_path = create_worktree(temp.path(), "override-init-worktree");
    let primary_identity_path = temp.path().join(".jit/worktree.json");
    let linked_identity_path = worktree_path.join(".jit/worktree.json");
    let primary_before = fs::read(&primary_identity_path).unwrap();
    let primary_identity: Value = serde_json::from_slice(&primary_before).unwrap();
    let linked_branch = worktree_path.file_name().unwrap().to_str().unwrap();

    // Reproduce a linked checkout copied from a primary store that carried its
    // machine-local identity. Init must classify the selected linked store,
    // discard this copied identity, and leave the primary file untouched.
    fs::write(&linked_identity_path, &primary_before).unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_DATA_DIR", worktree_path.join(".jit"))
        .args(["init", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "override init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(&primary_identity_path).unwrap(),
        primary_before,
        "initializing the selected linked store must not refresh the primary identity"
    );
    let linked_identity: Value =
        serde_json::from_slice(&fs::read(&linked_identity_path).unwrap()).unwrap();
    assert_eq!(
        linked_identity["root"].as_str(),
        Some(worktree_path.to_str().unwrap())
    );
    assert_eq!(linked_identity["branch"].as_str(), Some(linked_branch));
    assert_ne!(
        linked_identity["branch"], primary_identity["branch"],
        "the linked identity must use the selected checkout's branch"
    );
    assert_ne!(
        linked_identity["worktree_id"], primary_identity["worktree_id"],
        "a copied primary identity must be replaced for the selected linked checkout"
    );
}

#[test]
fn test_worktree_info_creates_identity_file() {
    let temp = setup_repo();

    // Run worktree info to create identity
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["worktree", "info"])
        .assert()
        .success();

    // Verify worktree.json was created
    let wt_file = temp.path().join(".jit/worktree.json");
    assert!(wt_file.exists(), "worktree.json should be created");

    // Verify it's valid JSON
    let content = fs::read_to_string(&wt_file).unwrap();
    let _: Value = serde_json::from_str(&content).unwrap();
}

#[test]
fn test_worktree_info_stable_across_invocations() {
    let temp = setup_repo();

    // First invocation
    let output1 = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["worktree", "info", "--json"])
        .output()
        .unwrap();

    let json1: Value = serde_json::from_slice(&output1.stdout).unwrap();
    let id1 = json1["worktree_id"].as_str().unwrap();

    // Second invocation
    let output2 = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["worktree", "info", "--json"])
        .output()
        .unwrap();

    let json2: Value = serde_json::from_slice(&output2.stdout).unwrap();
    let id2 = json2["worktree_id"].as_str().unwrap();

    assert_eq!(id1, id2, "Worktree ID should be stable");
}

/// Relocation must surface as a typed warning on the `worktree info` command
/// path (regression guard: storage no longer prints it, the output layer does).
/// Previously this condition was an `eprintln!` from storage on every command;
/// it must still surface here, not only via `recover`.
#[test]
fn test_worktree_info_surfaces_relocation_warning() {
    let temp = setup_repo();

    // First run creates worktree.json with root == this repo.
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["worktree", "info"])
        .assert()
        .success();

    // Rewrite the recorded root to a non-existent path. On the next run the
    // worktree is detected at a different (current) path, which is a relocation.
    let wt_file = temp.path().join(".jit/worktree.json");
    let mut identity: Value = serde_json::from_str(&fs::read_to_string(&wt_file).unwrap()).unwrap();
    identity["root"] = Value::String("/nonexistent/old/worktree/path".to_string());
    fs::write(&wt_file, serde_json::to_string_pretty(&identity).unwrap()).unwrap();

    // --json: the relocation must appear in the structured warnings payload,
    // and nothing must leak onto stderr.
    let json_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["worktree", "info", "--json"])
        .output()
        .unwrap();
    assert!(json_output.status.success());
    let json: Value = serde_json::from_slice(&json_output.stdout).unwrap();
    let warnings = json["warnings"].as_array().expect("warnings array present");
    assert!(
        warnings.iter().any(|w| w["kind"] == "worktree_relocated"),
        "worktree info --json must include the relocation warning, got {warnings:?}"
    );
    let json_stderr = String::from_utf8_lossy(&json_output.stderr);
    assert!(
        !json_stderr.contains("relocated"),
        "relocation diagnostic must not leak to stderr under --json, got: {json_stderr}"
    );

    // Re-seed the relocation and check the text path renders it on stderr.
    let mut identity: Value = serde_json::from_str(&fs::read_to_string(&wt_file).unwrap()).unwrap();
    identity["root"] = Value::String("/nonexistent/old/worktree/path".to_string());
    fs::write(&wt_file, serde_json::to_string_pretty(&identity).unwrap()).unwrap();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["worktree", "info"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Worktree relocated"));
}

/// Second command-path guard: `jit claim list` must also surface a relocation
/// as a typed warning (storage previously printed it from every identity load).
#[test]
fn test_claim_list_surfaces_relocation_warning() {
    let temp = setup_repo();

    // First run creates worktree.json with root == this repo.
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list"])
        .assert()
        .success();

    // Point the recorded root at a non-existent path to force a relocation.
    let wt_file = temp.path().join(".jit/worktree.json");
    let mut identity: Value = serde_json::from_str(&fs::read_to_string(&wt_file).unwrap()).unwrap();
    identity["root"] = Value::String("/nonexistent/old/worktree/path".to_string());
    fs::write(&wt_file, serde_json::to_string_pretty(&identity).unwrap()).unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let warnings = json["warnings"].as_array().expect("warnings array present");
    assert!(
        warnings.iter().any(|w| w["kind"] == "worktree_relocated"),
        "claim list --json must include the relocation warning, got {warnings:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("relocated"),
        "relocation diagnostic must not leak to stderr under --json, got: {stderr}"
    );
}

// === jit worktree list tests ===

#[test]
fn test_worktree_list_single_worktree() {
    let temp = setup_repo();

    // List worktrees (only main worktree)
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["worktree", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("main").or(predicate::str::contains("master")));
}

#[test]
fn test_worktree_list_multiple_worktrees() {
    let temp = setup_repo();

    // Create additional worktree within temp directory scope
    let wt_path = create_worktree(temp.path(), "feature-branch");

    // Initialize jit in new worktree - need to run this inline
    let status = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&wt_path)
        .args(["worktree", "info"])
        .status()
        .unwrap();
    assert!(status.success());

    // List from main repo - should see the worktree
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["worktree", "list"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The worktree name starts with "feature-branch-"
    assert!(
        stdout.contains("feature-branch"),
        "Should list the new worktree: {}",
        stdout
    );

    // Cleanup worktree before temp dir is dropped
    let wt_name = wt_path.file_name().unwrap().to_str().unwrap();
    let _ = Command::new("git")
        .current_dir(temp.path())
        .args(["worktree", "remove", "--force", wt_name])
        .status();
}

#[test]
fn test_worktree_list_from_linked_checkout_identifies_primary() {
    let temp = setup_repo();
    let worktree_path = create_worktree(temp.path(), "list-from-linked");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&worktree_path)
        .args(["worktree", "list", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "worktree list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let worktrees = json["worktrees"].as_array().unwrap();
    assert!(worktrees.iter().any(|entry| {
        entry["path"].as_str() == Some(temp.path().to_str().unwrap()) && entry["is_main"] == true
    }));
    assert!(worktrees.iter().any(|entry| {
        entry["path"].as_str() == Some(worktree_path.to_str().unwrap()) && entry["is_main"] == false
    }));
}

#[test]
fn test_worktree_list_json_output() {
    let temp = setup_repo();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["worktree", "list", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["worktrees"].is_array());

    let worktrees = json["worktrees"].as_array().unwrap();
    assert_eq!(json["count"].as_u64(), Some(worktrees.len() as u64));
    assert!(!worktrees.is_empty(), "Should have at least one worktree");

    // Check first worktree has expected fields
    assert!(worktrees[0]["path"].is_string());
    assert!(worktrees[0]["branch"].is_string());
}

/// Regression for the reported bug: `jit worktree list` showed stale
/// `active_claims` after a lease TTL expired, while `jit claim list` (which
/// evicts expired leases) correctly returned 0. The two views must agree.
/// CLI-boundary smoke test that `worktree list` and `claim list` are wired to
/// the same active-lease view for live leases (happy path).
///
/// Expiry-boundary exclusion (the deflaked behavior) is covered deterministically
/// in-process by `commands::worktree::tests::test_worktree_list_excludes_expired_leases`,
/// which drives an injected clock past a lease's TTL instead of sleeping. That
/// trade-off keeps this test free of any wall-clock timing while still exercising
/// the real CLI wiring for the common case.
#[test]
fn test_worktree_list_agrees_with_claim_list_for_live_leases() {
    let temp = setup_repo();

    let short_id = create_issue(temp.path(), "short-lived");
    let long_id = create_issue(temp.path(), "long-lived");

    // Acquire two live leases from the main worktree.
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &short_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test",
        ])
        .assert()
        .success();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &long_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test",
        ])
        .assert()
        .success();

    // Both leases are live, so `worktree list` and `claim list` must agree.
    let active = worktree_active_claims_total(temp.path());
    let claims = claim_list_count(temp.path());
    assert_eq!(active, 2, "both live leases should be counted");
    assert_eq!(
        active, claims,
        "worktree list active_claims must agree with claim list count"
    );
}

// === jit validate tests ===

#[test]
fn test_validate_success() {
    let temp = setup_repo();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["validate"])
        .assert()
        .success();
}

#[test]
fn test_validate_json_output() {
    let temp = setup_repo();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["validate", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["valid"], true);
}

#[test]
fn test_validate_branch_drift_accepts_a_branch_that_matches_origin_main() {
    let temp = setup_repo();
    let _remote = attach_origin_remote(temp.path());

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["validate", "--branch-drift"])
        .assert()
        .success();
}

#[test]
fn test_validate_branch_drift_detects_drifted_branch() {
    let temp = setup_repo();
    let _remote = attach_origin_remote(temp.path());

    let initial_commit = Command::new("git")
        .current_dir(temp.path())
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(initial_commit.status.success());
    let initial_commit = String::from_utf8(initial_commit.stdout).unwrap();
    let initial_commit = initial_commit.trim();

    fs::write(temp.path().join("README.md"), "# Test\nUpdated\n").unwrap();
    Command::new("git")
        .current_dir(temp.path())
        .args(["add", "README.md"])
        .assert()
        .success();
    Command::new("git")
        .current_dir(temp.path())
        .args(["commit", "-m", "Advance main"])
        .assert()
        .success();
    Command::new("git")
        .current_dir(temp.path())
        .args(["push", "origin", "HEAD:refs/heads/main"])
        .assert()
        .success();
    Command::new("git")
        .current_dir(temp.path())
        .args(["fetch", "origin"])
        .assert()
        .success();
    Command::new("git")
        .current_dir(temp.path())
        .args(["checkout", "-b", "stale", initial_commit])
        .assert()
        .success();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["validate", "--branch-drift"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Branch has diverged from origin/main"),
        "branch-drift failure explains the divergence: {stderr}"
    );
    assert!(
        stderr.contains("Fix: git rebase origin/main"),
        "branch-drift failure gives the rebase remedy: {stderr}"
    );
}

#[test]
fn test_validate_divergence_spelling_hints_branch_drift() {
    let temp = setup_repo();

    // `--divergence` names the membership-vs-DAG report, so `jit validate`
    // rejects it with a hint rather than a bare clap error.
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["validate", "--divergence"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("jit validate --branch-drift"))
        .stderr(predicates::str::contains("jit query divergence"));
}

#[test]
fn test_validate_leases_success() {
    let temp = setup_repo();

    // Validate leases (should pass with no leases)
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["validate", "--leases"])
        .assert()
        .success();
}

// === jit recover tests ===

#[test]
fn test_recover_success() {
    let temp = setup_repo();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["recover"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Recovery"));
}

#[test]
fn test_recover_json_output() {
    let temp = setup_repo();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["recover", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let _json: Value = serde_json::from_slice(&output.stdout).unwrap();
    // Envelope removed - just check valid JSON
}

// === jit init in worktrees tests ===

#[test]
fn test_init_in_new_worktree_generates_unique_id() {
    let temp = setup_repo();

    // Create a worktree
    let worktree_path = create_worktree(temp.path(), "test-worktree");

    // Run jit init in the worktree
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&worktree_path)
        .arg("init")
        .assert()
        .success();

    // Get worktree IDs from both locations
    let main_wt_file = temp.path().join(".jit/worktree.json");
    let worktree_wt_file = worktree_path.join(".jit/worktree.json");

    let main_content = fs::read_to_string(&main_wt_file).unwrap();
    let main_identity: Value = serde_json::from_str(&main_content).unwrap();

    let worktree_content = fs::read_to_string(&worktree_wt_file).unwrap();
    let worktree_identity: Value = serde_json::from_str(&worktree_content).unwrap();

    // IDs should be different
    assert_ne!(
        main_identity["worktree_id"], worktree_identity["worktree_id"],
        "Main and worktree should have different IDs"
    );

    // Worktree should have correct root path
    assert_eq!(
        worktree_identity["root"].as_str().unwrap(),
        worktree_path.to_string_lossy().to_string()
    );
}

#[test]
fn test_init_is_idempotent_in_worktree() {
    let temp = setup_repo();

    // Create a worktree
    let worktree_path = create_worktree(temp.path(), "test-worktree-idempotent");

    // Run jit init first time
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&worktree_path)
        .arg("init")
        .assert()
        .success();

    let worktree_wt_file = worktree_path.join(".jit/worktree.json");
    let first_content = fs::read_to_string(&worktree_wt_file).unwrap();
    let first_identity: Value = serde_json::from_str(&first_content).unwrap();
    let first_id = first_identity["worktree_id"].as_str().unwrap();

    // Run jit init second time
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&worktree_path)
        .arg("init")
        .assert()
        .success();

    let second_content = fs::read_to_string(&worktree_wt_file).unwrap();
    let second_identity: Value = serde_json::from_str(&second_content).unwrap();
    let second_id = second_identity["worktree_id"].as_str().unwrap();

    // ID should be unchanged
    assert_eq!(first_id, second_id, "Init should be idempotent");
}

#[test]
fn test_worktree_list_shows_distinct_ids() {
    let temp = setup_repo();

    // Create two worktrees
    let worktree1_path = create_worktree(temp.path(), "test-worktree-1");
    let worktree2_path = create_worktree(temp.path(), "test-worktree-2");

    // Init both worktrees
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&worktree1_path)
        .arg("init")
        .assert()
        .success();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&worktree2_path)
        .arg("init")
        .assert()
        .success();

    // List worktrees from main repo
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["worktree", "list", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let worktrees = json["worktrees"].as_array().unwrap();

    // Collect all worktree IDs
    let mut ids = std::collections::HashSet::new();
    for wt in worktrees {
        if let Some(id) = wt["worktree_id"].as_str() {
            ids.insert(id.to_string());
        }
    }

    // Should have at least 3 distinct IDs (main + 2 worktrees)
    assert!(
        ids.len() >= 3,
        "Should have at least 3 distinct worktree IDs"
    );
}

#[test]
fn test_git_worktree_move_preserves_id() {
    let temp = tempfile::tempdir().unwrap();
    let main_path = temp.path();

    // Initialize a git repo
    Command::new("git")
        .current_dir(main_path)
        .args(["init"])
        .assert()
        .success();

    // Configure git user identity (required for commits)
    Command::new("git")
        .current_dir(main_path)
        .args(["config", "user.email", "test@example.com"])
        .assert()
        .success();

    Command::new("git")
        .current_dir(main_path)
        .args(["config", "user.name", "Test User"])
        .assert()
        .success();

    Command::new("git")
        .current_dir(main_path)
        .args(["commit", "--allow-empty", "-m", "initial"])
        .assert()
        .success();

    // Initialize jit in main worktree and commit the .jit directory
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(main_path)
        .arg("init")
        .assert()
        .success();

    Command::new("git")
        .current_dir(main_path)
        .args(["add", ".jit"])
        .assert()
        .success();

    Command::new("git")
        .current_dir(main_path)
        .args(["commit", "-m", "Add jit tracking"])
        .assert()
        .success();

    // Create a new worktree (this will copy .jit/worktree.json)
    let worktree_path = temp.path().join("feature");
    Command::new("git")
        .current_dir(main_path)
        .args(["worktree", "add", worktree_path.to_str().unwrap()])
        .assert()
        .success();

    // Initialize jit in the worktree (should get unique ID)
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&worktree_path)
        .arg("init")
        .assert()
        .success();

    // Get the worktree ID before move
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&worktree_path)
        .args(["worktree", "info", "--json"])
        .output()
        .unwrap();

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let id_before = json["worktree_id"].as_str().unwrap().to_string();

    // Move the worktree using git worktree move
    let moved_path = temp.path().join("feature-moved");
    Command::new("git")
        .current_dir(main_path)
        .args([
            "worktree",
            "move",
            worktree_path.to_str().unwrap(),
            moved_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    // Get the worktree ID after move
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&moved_path)
        .args(["worktree", "info", "--json"])
        .output()
        .unwrap();

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let id_after = json["worktree_id"].as_str().unwrap().to_string();

    // The ID should be preserved after git worktree move
    assert_eq!(
        id_before, id_after,
        "Worktree ID should be preserved after git worktree move"
    );
}

/// Create an issue in `checkout`, declaring the linked-checkout write stance so
/// a linked checkout writes to its own store rather than refusing.
///
/// Answers the created issue's full id.
fn create_issue_in_own_store(checkout: &Path, title: &str) -> String {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(checkout)
        .env("JIT_WORKTREE_WRITE_POLICY", "allow")
        .args(["issue", "create", "--title", title, "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "issue create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    json["id"].as_str().unwrap().to_string()
}

/// Run `jit worktree store-divergence` in `checkout`, answering its stdout.
fn store_divergence_output(checkout: &Path, json: bool) -> String {
    let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    command
        .current_dir(checkout)
        .args(["worktree", "store-divergence"]);
    if json {
        command.arg("--json");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "worktree store-divergence failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn test_worktree_store_divergence_json_and_rendered_output_name_the_same_findings() {
    let temp = setup_repo();
    let linked = create_worktree(temp.path(), "store-divergence");

    // Diverge the two stores in both directions: one record each store holds
    // alone, each carrying its own creation event into that store's log.
    create_issue(temp.path(), "Held by the primary checkout alone");
    create_issue_in_own_store(&linked, "Held by the linked checkout alone");

    let machine: Value = serde_json::from_str(&store_divergence_output(&linked, true)).unwrap();
    let divergences = machine["divergences"].as_array().unwrap();

    assert_eq!(
        machine["count"].as_u64().unwrap() as usize,
        divergences.len(),
        "the list envelope's count is the length of its collection: {machine}"
    );
    assert!(
        !divergences.is_empty(),
        "the two diverged stores report findings: {machine}"
    );

    let rendered = store_divergence_output(&linked, false);
    for divergence in divergences {
        let record = divergence["record"].as_str().unwrap();
        let class = divergence["class"].as_str().unwrap();
        let id = divergence["id"].as_str().unwrap();
        assert!(
            rendered
                .lines()
                .any(|line| line.contains(record) && line.contains(class) && line.contains(id)),
            "the rendered output names the finding {record}/{class}/{id}:\n{rendered}"
        );
    }
    assert!(
        rendered.contains(&format!("{} divergence(s)", divergences.len())),
        "the rendered output reports the same finding count as the envelope:\n{rendered}"
    );

    let tokens = |field: &str| -> Vec<String> {
        divergences
            .iter()
            .map(|divergence| divergence[field].as_str().unwrap().to_string())
            .collect()
    };
    let classes = tokens("class");
    assert!(
        classes.iter().any(|class| class == "local_only")
            && classes.iter().any(|class| class == "reference_only"),
        "a store divergence is reported in both directions: {machine}"
    );
    let records = tokens("record");
    assert!(
        records.iter().any(|record| record == "issue")
            && records.iter().any(|record| record == "event"),
        "both issue records and event records are compared: {machine}"
    );

    // Cleanup worktree before temp dir is dropped
    let name = linked.file_name().unwrap().to_str().unwrap();
    let _ = Command::new("git")
        .current_dir(temp.path())
        .args(["worktree", "remove", "--force", name])
        .status();
}
