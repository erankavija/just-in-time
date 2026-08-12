//! `jit --version` and `jit version` report package identity and the cargo
//! build metadata, and nothing about the git state or clock the build ran
//! under (jit:355565c2 REQ-01).
//!
//! The negative half is the point: the version report is derived from a build
//! script that reads nothing ambient, so a commit hash, a dirty flag, or a
//! build timestamp appearing here would mean the build had started reading git
//! or the clock again.

use assert_cmd::Command;
use serde_json::Value;

/// Field names the version report must never carry again.
const WITHDRAWN_FIELDS: [&str; 4] = [
    "git_commit",
    "git_short_commit",
    "git_dirty",
    "build_timestamp",
];

#[test]
fn test_global_version_flag_reports_package_version_without_build_provenance() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success(), "--version should succeed");

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
    assert!(stdout.contains("profile"));
    for absent in ["commit", "dirty"] {
        assert!(
            !stdout.contains(absent),
            "--version must report no {absent}: {stdout}"
        );
    }
}

#[test]
fn test_version_command_reports_human_readable_metadata_without_repo() {
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
    assert!(stdout.contains("Profile:"));
    assert!(stdout.contains("Target:"));
    for absent in ["Commit:", "Dirty:", "Built:"] {
        assert!(
            !stdout.contains(absent),
            "version must report no {absent} line: {stdout}"
        );
    }
}

#[test]
fn test_version_command_reports_json_metadata_without_repo() {
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
    assert!(json.get("build_profile").is_some());
    assert!(json.get("target").is_some());
    for withdrawn in WITHDRAWN_FIELDS {
        assert!(
            json.get(withdrawn).is_none(),
            "version --json must carry no {withdrawn} field: {json}"
        );
    }
}
