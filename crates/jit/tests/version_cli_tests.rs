use assert_cmd::Command;
use serde_json::Value;

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
