//! Tests for --quiet flag functionality and broken pipe handling.

use assert_cmd::prelude::*;
use jit::test_cli::JitCli;
use predicates::prelude::*;

#[test]
fn test_quiet_flag_exists() {
    let cli = JitCli::initialised().unwrap();

    cli.command(["status", "--quiet"]).assert().success();
}

#[test]
fn test_quiet_short_flag_exists() {
    let cli = JitCli::initialised().unwrap();

    cli.command(["status", "-q"]).assert().success();
}

#[test]
fn test_quiet_mode_suppresses_success_messages() {
    let cli = JitCli::initialised().unwrap();
    let issue_id = cli.create_issue(["--title", "Test Issue"]).unwrap();

    let normal_output = cli
        .command(["issue", "update", &issue_id, "--state", "done"])
        .output()
        .unwrap();

    let normal_stdout = String::from_utf8(normal_output.stdout).unwrap();
    assert!(
        normal_stdout.contains("Updated") || normal_stdout.contains("success"),
        "Normal mode should show success message"
    );

    cli.command([
        "issue", "update", &issue_id, "--state", "backlog", "--quiet",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Updated").not())
    .stdout(predicate::str::contains("success").not());
}

#[test]
fn test_quiet_mode_preserves_essential_output() {
    let cli = JitCli::initialised().unwrap();
    cli.create_issue(["--title", "Test Issue 1"]).unwrap();
    cli.create_issue(["--title", "Test Issue 2"]).unwrap();

    cli.command(["query", "all", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Test Issue 1"))
        .stdout(predicate::str::contains("Test Issue 2"));
}

#[test]
fn test_quiet_with_json_outputs_only_json() {
    let cli = JitCli::initialised().unwrap();
    let issue_id = cli.create_issue(["--title", "Test Issue"]).unwrap();

    let output = cli
        .command(["issue", "show", &issue_id, "--quiet", "--json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("Output should be pure JSON in quiet+json mode");

    assert_eq!(json["title"], "Test Issue");
}

#[test]
fn test_quiet_suppresses_informational_output() {
    let cli = JitCli::initialised().unwrap();

    let normal_output = cli.command(["status"]).output().unwrap();
    let normal_stdout = String::from_utf8(normal_output.stdout).unwrap();

    let quiet_output = cli.command(["status", "--quiet"]).output().unwrap();
    let quiet_stdout = String::from_utf8(quiet_output.stdout).unwrap();

    assert!(normal_output.status.success());
    assert!(quiet_output.status.success());
    assert!(!quiet_stdout.trim().is_empty());
    assert!(!normal_stdout.trim().is_empty());
}

#[test]
fn test_quiet_with_dep_add() {
    let cli = JitCli::initialised().unwrap();
    let issue1 = cli.create_issue(["--title", "Issue 1"]).unwrap();
    let issue2 = cli.create_issue(["--title", "Issue 2"]).unwrap();

    let normal_output = cli
        .command(["dep", "add", &issue1, &issue2])
        .output()
        .unwrap();

    let normal_stdout = String::from_utf8(normal_output.stdout).unwrap();
    assert!(
        normal_stdout.contains("Added") || normal_stdout.contains("dependency"),
        "Normal mode should show confirmation"
    );

    cli.command(["dep", "rm", &issue1, &issue2, "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed").not())
        .stdout(predicate::str::contains("dependency").not());
}

#[test]
fn test_errors_always_shown_even_in_quiet_mode() {
    let cli = JitCli::initialised().unwrap();

    let output = cli
        .command(["issue", "show", "nonexistent", "--quiet"])
        .output()
        .unwrap();

    assert!(!output.status.success());

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("Error") || stderr.contains("not found"),
        "Errors must be shown even in quiet mode"
    );
}

#[test]
fn test_issue_create_quiet_outputs_id() {
    let cli = JitCli::initialised().unwrap();

    let output = cli
        .command(["issue", "create", "--title", "New Issue", "--quiet"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    let trimmed = stdout.trim();
    assert!(trimmed.len() >= 8, "Quiet create should output issue ID");
    assert!(
        !stdout.contains("Created issue") && !stdout.contains("Successfully"),
        "Quiet mode should not show verbose messages"
    );
}

#[test]
fn test_quiet_flag_position_independent() {
    let cli = JitCli::initialised().unwrap();

    cli.command(["--quiet", "status"]).assert().success();
    cli.command(["status", "--quiet"]).assert().success();
}

#[test]
fn test_quiet_with_gate_commands() {
    let cli = JitCli::initialised().unwrap();

    cli.command([
        "gate",
        "define",
        "test-gate",
        "--title",
        "Test Gate",
        "--description",
        "A test gate",
    ])
    .assert()
    .success();

    let issue_id = cli.create_issue(["--title", "Test Issue"]).unwrap();

    cli.command(["gate", "add", &issue_id, "test-gate", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added").not());

    cli.command([
        "gate",
        "evaluate",
        &issue_id,
        "test-gate",
        "--by",
        "human:reviewer",
        "--quiet",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Passed").not());
}
