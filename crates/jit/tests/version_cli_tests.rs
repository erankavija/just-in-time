use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;
use std::process::Command as StdCommand;

#[test]
fn test_global_version_flag_reports_local_provenance() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success(), "--version should succeed");

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
    assert!(stdout.contains("commit"));
    assert!(stdout.contains("profile"));
}

#[test]
fn test_version_command_reports_human_readable_provenance_without_repo() {
    let temp_dir = tempfile::TempDir::new().unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .arg("version")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "version command should not require .jit"
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Version:"));
    assert!(stdout.contains("Commit:"));
    assert!(stdout.contains("Dirty:"));
    assert!(stdout.contains("Profile:"));
    assert!(stdout.contains("Built:"));
    assert!(stdout.contains("Target:"));
}

#[test]
fn test_version_command_reports_json_provenance_without_repo() {
    let temp_dir = tempfile::TempDir::new().unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["version", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "version --json should not require .jit"
    );

    let json: Value = serde_json::from_slice(&output.stdout).expect("version output is JSON");
    assert_eq!(json["package"].as_str(), Some("jit"));
    assert_eq!(json["version"].as_str(), Some(env!("CARGO_PKG_VERSION")));
    assert!(json.get("git_commit").is_some());
    assert!(json.get("git_short_commit").is_some());
    assert!(json.get("git_dirty").is_some());
    assert!(json.get("build_profile").is_some());
    assert!(json.get("build_timestamp").is_some());
    assert!(json.get("target").is_some());
}

#[test]
fn test_version_build_without_git_metadata_reports_unknowns() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let output = Command::new("cargo")
        .current_dir(workspace_root)
        .env("CARGO_TARGET_DIR", temp_dir.path().join("target-no-git"))
        .env("GIT_DIR", temp_dir.path().join("missing-git-dir"))
        .env("SOURCE_DATE_EPOCH", "0")
        .args(["run", "-p", "jit", "--quiet", "--", "version", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "cargo run without Git metadata should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).expect("version output is JSON");
    assert_eq!(json["git_commit"].as_str(), Some("unknown"));
    assert_eq!(json["git_short_commit"].as_str(), Some("unknown"));
    assert!(json["git_dirty"].is_null());
    assert_eq!(json["build_timestamp"].as_str(), Some("0"));
}

#[test]
fn test_version_build_from_clean_git_checkout_reports_not_dirty() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let source_dir = temp_dir.path().join("source");
    std::fs::create_dir(&source_dir).unwrap();

    let archive_status = StdCommand::new("bash")
        .current_dir(workspace_root)
        .arg("-c")
        .arg(
            "tar --exclude=.git --exclude=.jit --exclude=target --exclude=web/node_modules --exclude=mcp-server/node_modules -cf - . | tar -C \"$1\" -xf -",
        )
        .arg("copy-workspace")
        .arg(&source_dir)
        .status()
        .unwrap();
    assert!(archive_status.success(), "workspace copy should succeed");

    for args in [
        vec!["init"],
        vec!["config", "user.email", "jit-test@example.invalid"],
        vec!["config", "user.name", "JIT Test"],
        vec!["add", "."],
        vec!["commit", "-m", "clean source"],
    ] {
        let status = StdCommand::new("git")
            .current_dir(&source_dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git setup command should succeed");
    }

    let output = Command::new("cargo")
        .current_dir(&source_dir)
        .env("CARGO_TARGET_DIR", temp_dir.path().join("target-clean-git"))
        .env("SOURCE_DATE_EPOCH", "0")
        .args(["run", "-p", "jit", "--quiet", "--", "version", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "cargo run from clean Git checkout should succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).expect("version output is JSON");
    assert_eq!(json["git_dirty"].as_bool(), Some(false));
}
