//! Tests for the jit recover command

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn jit_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
}

fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();

    // Initialize git repo first (required for recover)
    std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["init"])
        .output()
        .expect("Failed to init git repo");

    // Configure git user for commits
    std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["config", "user.email", "test@test.com"])
        .output()
        .expect("Failed to configure git email");
    std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["config", "user.name", "Test"])
        .output()
        .expect("Failed to configure git name");

    // Create initial commit so we have a branch
    std::fs::write(temp.path().join("README.md"), "# Test").unwrap();
    std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["add", "."])
        .output()
        .expect("Failed to git add");
    std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["commit", "-m", "Initial commit"])
        .output()
        .expect("Failed to git commit");

    // Initialize jit
    jit_cmd()
        .current_dir(temp.path())
        .args(["init"])
        .assert()
        .success();

    temp
}

#[test]
fn test_recover_command_exists() {
    let temp = setup_test_repo();

    jit_cmd()
        .current_dir(temp.path())
        .args(["recover"])
        .assert()
        .success();
}

#[test]
fn test_recover_with_json_output() {
    let temp = setup_test_repo();

    jit_cmd()
        .current_dir(temp.path())
        .args(["recover", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\""));
}

#[test]
fn test_recover_reports_actions_taken() {
    let temp = setup_test_repo();

    // Run recover - should succeed even with nothing to recover
    jit_cmd()
        .current_dir(temp.path())
        .args(["recover"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Recovery complete"));
}

#[test]
fn test_recover_help_describes_purpose() {
    jit_cmd()
        .args(["recover", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("recovery"))
        .stdout(predicate::str::contains("stale"));
}
