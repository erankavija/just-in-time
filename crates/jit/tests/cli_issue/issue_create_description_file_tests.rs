//! CLI coverage for verbatim issue descriptions supplied outside argv.
//!
//! The file forms deliberately exercise the same source-reading contract as
//! `issue update`: `-` means stdin, and content survives without trimming.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn jit(temp: &TempDir) -> Command {
    let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    command.current_dir(temp.path());
    command
}

fn jit_assert_cmd(temp: &TempDir) -> assert_cmd::Command {
    assert_cmd::Command::from_std(jit(temp))
}

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    jit(&temp).arg("init").assert().success();
    temp
}

fn created_id(output: &std::process::Output) -> String {
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json["id"].as_str().unwrap().to_owned()
}

fn description(temp: &TempDir, id: &str) -> Vec<u8> {
    let output = jit(temp)
        .args(["issue", "show", id, "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json["description"].as_str().unwrap().as_bytes().to_vec()
}

#[test]
fn test_create_description_file_preserves_special_bytes_and_trailing_blank_line() {
    let temp = setup_repo();
    let source = b"# Markdown\n\n`literal` and $HOME\n\n";
    let path = temp.path().join("description.md");
    fs::write(&path, source).unwrap();

    let output = jit(&temp)
        .args([
            "issue",
            "create",
            "--title",
            "From file",
            "--description-file",
            path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(description(&temp, &created_id(&output)), source);
}

#[test]
fn test_create_description_file_dash_preserves_stdin_bytes_and_trailing_blank_line() {
    let temp = setup_repo();
    let source = "## From stdin\n\n`literal` and $HOME\n\n";

    let output = jit_assert_cmd(&temp)
        .args([
            "issue",
            "create",
            "--title",
            "From stdin",
            "--description-file",
            "-",
            "--json",
        ])
        .write_stdin(source)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(description(&temp, &created_id(&output)), source.as_bytes());
}

#[test]
fn test_create_description_and_description_file_are_mutually_exclusive() {
    let temp = setup_repo();
    let path = temp.path().join("description.md");
    fs::write(&path, "from file").unwrap();

    jit(&temp)
        .args([
            "issue",
            "create",
            "--title",
            "Conflicting descriptions",
            "--description",
            "inline",
            "--description-file",
            path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn test_create_missing_description_file_is_usage_error_naming_path() {
    let temp = setup_repo();
    let missing = temp.path().join("missing-description.md");

    let output = jit(&temp)
        .args([
            "issue",
            "create",
            "--title",
            "Missing description",
            "--description-file",
            missing.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(missing.to_str().unwrap()),
        "stderr: {stderr}"
    );
}

#[test]
fn test_create_unreadable_description_path_is_usage_error_naming_path() {
    let temp = setup_repo();
    let directory = temp.path().join("description-directory");
    fs::create_dir(&directory).unwrap();

    let output = jit(&temp)
        .args([
            "issue",
            "create",
            "--title",
            "Unreadable description",
            "--description-file",
            directory.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(directory.to_str().unwrap()),
        "stderr: {stderr}"
    );
}

#[test]
fn test_create_help_documents_file_description_forms() {
    let temp = TempDir::new().unwrap();

    jit(&temp)
        .args(["issue", "create", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--description-file"))
        .stdout(predicate::str::contains("stdin"))
        .stdout(predicate::str::contains("verbatim"));
}
