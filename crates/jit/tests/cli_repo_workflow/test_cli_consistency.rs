//! TDD tests for CLI consistency improvements
//!
//! These tests define the expected behavior for consistent CLI:
//! 1. All commands support --json flag for machine-readable output
//! 2. Consistent argument order (ID first, then flags)
//! 3. Predictable output format

use jit::test_cli::JitCli;
use serde_json::Value;

// ============================================================================
// Test: issue create --json
// ============================================================================

#[test]
fn test_issue_create_supports_json() {
    let cli = JitCli::initialised().unwrap();

    let json = cli
        .run_json(["issue", "create", "-t", "Test task"])
        .expect("Output should be valid JSON");

    // Should contain id field
    assert!(json["id"].is_string(), "Should have 'id' field");
    assert!(json["title"].is_string(), "Should have 'title' field");
    assert_eq!(json["title"].as_str().unwrap(), "Test task");
}

#[test]
fn test_issue_create_without_json_is_human_readable() {
    let cli = JitCli::initialised().unwrap();

    let stdout = cli.run(["issue", "create", "-t", "Test task"]).unwrap();

    // Should contain human-readable text (not JSON)
    assert!(stdout.contains("Created issue:"));

    // Should NOT be valid JSON
    assert!(serde_json::from_str::<Value>(&stdout).is_err());
}

// ============================================================================
// Test: issue update --json
// ============================================================================

#[test]
fn test_issue_update_supports_json() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["-t", "Test"]).unwrap();

    // Update with JSON output
    let json = cli
        .run_json(["issue", "update", &id, "--title", "Updated"])
        .expect("Output should be valid JSON");

    // `issue update --json` returns a lean confirmation (id, short_id,
    // state, updated_at). Verify the mutation took effect via `issue show`.
    assert_eq!(json["id"].as_str().unwrap(), id);

    let show_json = cli.run_json(["issue", "show", &id]).unwrap();
    assert_eq!(show_json["title"].as_str().unwrap(), "Updated");
}

// ============================================================================
// Test: issue claim with consistent argument order
// ============================================================================

#[test]
fn test_issue_claim_takes_id_first() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["-t", "Task"]).unwrap();
    cli.run(["issue", "update", &id, "--state", "ready"])
        .unwrap();

    // Claim with ID first (NEW ORDER)
    let json = cli
        .run_json(["issue", "claim", &id, "agent:test"])
        .expect("Claim with ID-first order should work");

    assert_eq!(json["id"].as_str().unwrap(), id);
    assert_eq!(json["assignee"].as_str().unwrap(), "agent:test");
}

#[test]
fn test_issue_claim_supports_json() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["-t", "Task"]).unwrap();
    cli.run(["issue", "update", &id, "--state", "ready"])
        .unwrap();

    // Claim without --json should be human-readable
    let stdout = cli.run(["issue", "claim", &id, "agent:test"]).unwrap();

    assert!(stdout.contains("Claimed issue:") || stdout.contains("claimed"));
}

// ============================================================================
// Test: issue delete --json
// ============================================================================

#[test]
fn test_issue_delete_supports_json() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["-t", "Task"]).unwrap();

    let output = cli
        .command_with_env(
            ["issue", "delete", &id, "--json"],
            [("JIT_ALLOW_DELETION", "1")],
        )
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).expect("Should return JSON");

    assert_eq!(json["id"].as_str().unwrap(), id);
    assert!(json["deleted"].as_bool().unwrap(), "deleted should be true");
}

// ============================================================================
// Test: issue release --json
// ============================================================================

#[test]
fn test_issue_release_supports_json() {
    let cli = JitCli::initialised().unwrap();

    // Create and claim issue
    let id = cli.create_issue(["-t", "Task"]).unwrap();
    cli.run(["issue", "update", &id, "--state", "ready"])
        .unwrap();
    cli.run(["issue", "claim", &id, "agent:test"]).unwrap();

    // Release with JSON
    let json = cli
        .run_json(["issue", "release", &id, "Test"])
        .expect("Should return JSON");

    assert_eq!(json["id"].as_str().unwrap(), id);
    assert!(json["assignee"].is_null(), "Assignee should be cleared");
}

// ============================================================================
// Test: Consistent JSON structure
// ============================================================================

#[test]
fn test_json_output_has_consistent_structure() {
    let cli = JitCli::initialised().unwrap();

    // All mutation commands should return similar structure with at least:
    // - id field
    // - relevant issue data

    let json = cli.run_json(["issue", "create", "-t", "Test"]).unwrap();

    // Check for expected fields
    assert!(json.is_object(), "Root should be an object");
    assert!(json["id"].is_string(), "Should have id");
    assert!(json["title"].is_string(), "Should have title");
    assert!(json["state"].is_string(), "Should have state");
    assert!(json["priority"].is_string(), "Should have priority");
}
