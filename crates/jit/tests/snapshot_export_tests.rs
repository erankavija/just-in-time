//! Integration tests for snapshot export CLI

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_snapshot_export_help() {
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.arg("snapshot").arg("--help");
    
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Snapshot export commands"))
        .stdout(predicate::str::contains("export"));
}

#[test]
fn test_snapshot_export_subcommand_help() {
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.arg("snapshot").arg("export").arg("--help");
    
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Archive a complete snapshot"))
        .stdout(predicate::str::contains("--scope"))
        .stdout(predicate::str::contains("--format"))
        .stdout(predicate::str::contains("--out"));
}

#[test]
fn test_snapshot_export_requires_init() {
    let temp = TempDir::new().unwrap();
    
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .arg("snapshot")
        .arg("export");
    
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_snapshot_export_all_directory() {
    let temp = TempDir::new().unwrap();
    
    // Initialize jit
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    
    // Create a test issue
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .args(&["issue", "create", "--title", "Test Issue"])
        .assert()
        .success();
    
    // Export snapshot
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&["snapshot", "export", "--out", "test-snapshot"]);
    
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Snapshot exported"))
        .stdout(predicate::str::contains("1 issues"));
    
    // Verify snapshot structure
    let snapshot_dir = temp.path().join("test-snapshot");
    assert!(snapshot_dir.exists());
    assert!(snapshot_dir.join("manifest.json").exists());
    assert!(snapshot_dir.join("README.md").exists());
    assert!(snapshot_dir.join("checksums.txt").exists());
    assert!(snapshot_dir.join(".jit").exists());
    assert!(snapshot_dir.join(".jit/issues").exists());
}

#[test]
fn test_snapshot_export_tar_format() {
    let temp = TempDir::new().unwrap();
    
    // Initialize and create issue
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .args(&["issue", "create", "--title", "Test"])
        .assert()
        .success();
    
    // Export as tar
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&["snapshot", "export", "--format", "tar", "--out", "test.tar"]);
    
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Snapshot exported"))
        .stdout(predicate::str::contains("Archive"));
    
    // Verify tar file exists
    let tar_file = temp.path().join("test.tar");
    assert!(tar_file.exists());
    assert!(tar_file.metadata().unwrap().len() > 0);
}

#[test]
fn test_snapshot_export_label_scope() {
    let temp = TempDir::new().unwrap();
    
    // Initialize
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    
    // Create issues with different labels
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .args(&["issue", "create", "--title", "Epic Issue", "--label", "epic:auth"])
        .assert()
        .success();
    
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .args(&["issue", "create", "--title", "Auth Task", "--label", "epic:auth"])
        .assert()
        .success();
    
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .args(&["issue", "create", "--title", "Other Task", "--label", "epic:billing"])
        .assert()
        .success();
    
    // Export only epic:auth scope
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&["snapshot", "export", "--scope", "label:epic:auth", "--out", "auth-snapshot"]);
    
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("2 issues"));
    
    let snapshot_dir = temp.path().join("auth-snapshot");
    assert!(snapshot_dir.exists());
    
    // Verify manifest shows 2 issues
    let manifest_content = fs::read_to_string(snapshot_dir.join("manifest.json")).unwrap();
    assert!(manifest_content.contains("\"count\": 2"));
}

#[test]
fn test_snapshot_export_default_naming() {
    let temp = TempDir::new().unwrap();
    
    // Initialize and create issue
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .args(&["issue", "create", "--title", "Test"])
        .assert()
        .success();
    
    // Export without --out (should use timestamp-based name)
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&["snapshot", "export"]);
    
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("snapshot-"));
}

#[test]
fn test_snapshot_export_json_output() {
    let temp = TempDir::new().unwrap();
    
    // Initialize and create issue
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .args(&["issue", "create", "--title", "Test"])
        .assert()
        .success();
    
    // Export with --json
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&["snapshot", "export", "--out", "test-json", "--json"]);
    
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("{"))
        .stdout(predicate::str::contains("\"path\""));
}

#[test]
fn test_snapshot_export_invalid_scope() {
    let temp = TempDir::new().unwrap();
    
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    
    // Try invalid scope format
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&["snapshot", "export", "--scope", "invalid"]);
    
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Invalid scope"));
}

#[test]
fn test_snapshot_export_output_exists() {
    let temp = TempDir::new().unwrap();
    
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .args(&["issue", "create", "--title", "Test"])
        .assert()
        .success();
    
    // First export
    Command::cargo_bin("jit").unwrap()
        .current_dir(temp.path())
        .args(&["snapshot", "export", "--out", "existing"])
        .assert()
        .success();
    
    // Try to export to same location
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&["snapshot", "export", "--out", "existing"]);
    
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}
