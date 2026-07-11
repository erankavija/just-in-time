//! REQ-02: bind the exception rows of the `command_exit_codes` projection to
//! real CLI dispatch.
//!
//! The classifier-driven rows are verified against `error_to_exit_code` by a
//! unit test in `main.rs`. The exception rows, however, are emitted by direct
//! `std::process::exit` sites in the dispatch (a completed run signalling
//! findings), so they can only be observed by running the binary. Each case
//! below runs a real command, asserts the observed exit code, and asserts the
//! projection documents that exact `(command, code)` pair as an exception — so a
//! divergence between the projection and the dispatch fails the build.

use jit::schema::CommandSchema;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup() -> TempDir {
    let temp = TempDir::new().unwrap();
    let status = Command::new(jit_binary())
        .current_dir(&temp)
        .arg("init")
        .status()
        .unwrap();
    assert!(status.success());
    temp
}

fn create_issue(temp: &TempDir, title: &str, extra: &[&str]) -> String {
    let mut args = vec!["issue", "create", "--title", title, "--json"];
    args.extend_from_slice(extra);
    let output = Command::new(jit_binary())
        .current_dir(temp)
        .args(&args)
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json["id"].as_str().unwrap().to_string()
}

/// Assert the projection carries an exception row for `command` at `code`.
fn assert_documented_exception(command: &str, code: i32) {
    let schema = CommandSchema::generate();
    let row = schema
        .command_exit_codes
        .iter()
        .find(|c| c.command == command && c.code == code)
        .unwrap_or_else(|| {
            panic!("command_exit_codes projection has no row for `{command}` code {code}")
        });
    assert!(
        row.exception,
        "row for `{command}` code {code} must be flagged as an exception"
    );
}

/// `jit validate` exits 4 on repository-integrity findings — a completed run
/// reporting findings, matching the `validate`/4 exception row.
#[test]
fn validate_findings_exit_matches_projection() {
    let temp = setup();
    let id = create_issue(&temp, "Corruptible", &[]);

    // Point the issue at a non-existent dependency so validation finds a broken
    // reference (an error-severity integrity finding).
    let issue_path = temp
        .path()
        .join(".jit")
        .join("issues")
        .join(format!("{id}.json"));
    let mut issue: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&issue_path).unwrap()).unwrap();
    issue["dependencies"] = serde_json::json!(["nonexistent"]);
    fs::write(&issue_path, serde_json::to_string_pretty(&issue).unwrap()).unwrap();

    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .arg("validate")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    assert_documented_exception("validate", 4);
}

/// `jit gate status-all` exits 4 while any required gate is unpassed — a status
/// report, matching the `gate status-all`/4 exception row.
#[test]
fn gate_status_all_exit_matches_projection() {
    let temp = setup();

    let status = Command::new(jit_binary())
        .current_dir(&temp)
        .args([
            "gate",
            "define",
            "--title",
            "Tests",
            "--description",
            "Tests",
            "tests",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let id = create_issue(&temp, "Gated", &["--gate", "tests"]);

    let output = Command::new(jit_binary())
        .current_dir(&temp)
        .args(["gate", "status-all", &id])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    assert_documented_exception("gate status-all", 4);
}
