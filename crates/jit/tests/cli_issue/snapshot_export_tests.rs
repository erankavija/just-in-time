//! Integration tests for snapshot export CLI

use assert_cmd::assert::OutputAssertExt;
use predicates::prelude::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn test_snapshot_export_help() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.arg("snapshot").arg("--help");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Snapshot export commands"))
        .stdout(predicate::str::contains("export"));
}

#[test]
fn test_snapshot_export_subcommand_help() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
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

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path()).arg("snapshot").arg("export");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_snapshot_export_all_directory() {
    let temp = TempDir::new().unwrap();

    // Initialize jit
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    // Create a test issue
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "create", "--title", "Test Issue"])
        .assert()
        .success();

    // Export snapshot
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args(["snapshot", "export", "--out", "test-snapshot"]);

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
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "create", "--title", "Test"])
        .assert()
        .success();

    // Export as tar
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args(["snapshot", "export", "--format", "tar", "--out", "test.tar"]);

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
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    // Create issues with different labels
    let epic_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "issue",
            "create",
            "--title",
            "Epic Issue",
            "--label",
            "epic:auth",
        ])
        .output()
        .unwrap();
    let epic_id = String::from_utf8_lossy(&epic_output.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let auth_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "issue",
            "create",
            "--title",
            "Auth Task",
            "--label",
            "epic:auth",
        ])
        .output()
        .unwrap();
    let auth_id = String::from_utf8_lossy(&auth_output.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let other_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "issue",
            "create",
            "--title",
            "Other Task",
            "--label",
            "epic:billing",
        ])
        .output()
        .unwrap();
    let other_id = String::from_utf8_lossy(&other_output.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Create dependencies to connect the issues
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["dep", "add", &auth_id, &epic_id])
        .assert()
        .success();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["dep", "add", &other_id, &epic_id])
        .assert()
        .success();

    // Export only epic:auth scope
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path()).args([
        "snapshot",
        "export",
        "--scope",
        "label:epic:auth",
        "--out",
        "auth-snapshot",
    ]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("2 issues"));

    let snapshot_dir = temp.path().join("auth-snapshot");
    assert!(snapshot_dir.exists());

    // Verify manifest shows 2 issues
    let manifest_content = fs::read_to_string(snapshot_dir.join("manifest.json")).unwrap();
    assert!(manifest_content.contains("\"count\": 2"));
    assert!(manifest_content.contains("\"scope\": \"label:epic:auth\""));
}

#[test]
fn test_snapshot_export_default_naming() {
    let temp = TempDir::new().unwrap();

    // Initialize and create issue
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "create", "--title", "Test"])
        .assert()
        .success();

    // Export without --out (should use timestamp-based name)
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path()).args(["snapshot", "export"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("snapshot-"));
}

#[test]
fn test_snapshot_export_json_output() {
    let temp = TempDir::new().unwrap();

    // Initialize and create issue
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "create", "--title", "Test"])
        .assert()
        .success();

    // Export with --json
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args(["snapshot", "export", "--out", "test-json", "--json"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("{"))
        .stdout(predicate::str::contains("\"path\""));
}

#[test]
fn test_snapshot_export_invalid_scope() {
    let temp = TempDir::new().unwrap();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    // Try invalid scope format
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args(["snapshot", "export", "--scope", "invalid"]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Invalid scope"));
}

#[test]
fn test_snapshot_export_output_exists() {
    let temp = TempDir::new().unwrap();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "create", "--title", "Test"])
        .assert()
        .success();

    // First export
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["snapshot", "export", "--out", "existing"])
        .assert()
        .success();

    // Try to export to same location
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args(["snapshot", "export", "--out", "existing"]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn test_snapshot_export_nested_data_and_external_destinations() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    fs::create_dir(&repo).unwrap();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&repo)
        .arg("init")
        .assert()
        .success();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&repo)
        .args(["issue", "create", "--title", "Test"])
        .assert()
        .success();
    fs::create_dir(repo.join(".jit/exports")).unwrap();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&repo)
        .args(["snapshot", "export", "--out", ".jit/exports/snapshot"])
        .assert()
        .success();
    assert!(repo.join(".jit/exports/snapshot/manifest.json").is_file());

    #[cfg(target_os = "linux")]
    {
        let external_directory = temp.path().join("external-snapshot");
        Command::new(assert_cmd::cargo::cargo_bin!("jit"))
            .current_dir(&repo)
            .args([
                "snapshot",
                "export",
                "--out",
                external_directory.to_str().unwrap(),
            ])
            .assert()
            .success();
        assert!(external_directory.join("manifest.json").is_file());
    }

    let external = temp.path().join("external.tar");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&repo)
        .args([
            "snapshot",
            "export",
            "--format",
            "tar",
            "--out",
            external.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert!(external.is_file());
    let original = fs::read(&external).unwrap();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&repo)
        .args([
            "snapshot",
            "export",
            "--format",
            "tar",
            "--out",
            external.to_str().unwrap(),
        ])
        .assert()
        .code(6);
    assert_eq!(fs::read(&external).unwrap(), original);
    assert!(fs::read_dir(temp.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

#[cfg(unix)]
#[test]
fn test_snapshot_export_rejects_repository_symlink_alias() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    let external = temp.path().join("external");
    fs::create_dir(&repo).unwrap();
    fs::create_dir(&external).unwrap();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&repo)
        .arg("init")
        .assert()
        .success();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&repo)
        .args(["issue", "create", "--title", "Test"])
        .assert()
        .success();
    symlink(&external, repo.join("alias")).unwrap();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&repo)
        .args(["snapshot", "export", "--out", "alias/snapshot"])
        .assert()
        .failure();
    assert!(!external.join("snapshot").exists());
}

#[cfg(unix)]
#[test]
fn test_snapshot_export_rejects_external_symlink_alias_into_repository() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    fs::create_dir(&repo).unwrap();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&repo)
        .arg("init")
        .assert()
        .success();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&repo)
        .args(["issue", "create", "--title", "Test"])
        .assert()
        .success();
    let alias = temp.path().join("alias");
    symlink(&repo, &alias).unwrap();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(&repo)
        .args([
            "snapshot",
            "export",
            "--out",
            alias.join("snapshot").to_str().unwrap(),
        ])
        .assert()
        .failure();
    assert!(!repo.join("snapshot").exists());
}

#[cfg(unix)]
#[test]
fn test_snapshot_export_rejects_symlinked_document_source() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "create", "--title", "Test"])
        .assert()
        .success();
    fs::create_dir(temp.path().join("docs")).unwrap();
    fs::write(temp.path().join("real.md"), "secret").unwrap();
    symlink("../real.md", temp.path().join("docs/link.md")).unwrap();
    let issue_path = fs::read_dir(temp.path().join(".jit/issues"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    let mut issue: serde_json::Value =
        serde_json::from_slice(&fs::read(&issue_path).unwrap()).unwrap();
    issue["documents"] = serde_json::json!([{
        "path": "docs/link.md",
        "commit": null,
        "label": null,
        "doc_type": null,
        "format": "markdown",
        "assets": []
    }]);
    fs::write(&issue_path, serde_json::to_vec_pretty(&issue).unwrap()).unwrap();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "snapshot",
            "export",
            "--working-tree",
            "--force",
            "--out",
            "snapshot",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("Warning"));
    assert!(!temp.path().join("snapshot/docs/link.md").exists());
}

#[cfg(unix)]
#[test]
fn test_snapshot_export_rejects_symlinked_data_source_without_output() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "create", "--title", "Test"])
        .assert()
        .success();
    fs::remove_file(temp.path().join(".jit/config.toml")).unwrap();
    fs::write(temp.path().join("outside.toml"), "secret = true").unwrap();
    symlink("../outside.toml", temp.path().join(".jit/config.toml")).unwrap();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "snapshot",
            "export",
            "--force",
            "--working-tree",
            "--out",
            "snapshot",
        ])
        .assert()
        .failure();
    assert!(!temp.path().join("snapshot").exists());
}
