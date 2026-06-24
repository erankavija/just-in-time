//! Integration tests for `jit gate pass-all` (issue df7f934c).
//!
//! Covers: all-pass -> exit 0 with per-gate JSON; fail-fast at the first
//! non-passing gate (a later gate's checker never runs) -> exit 4; runner error
//! -> exit 10; and inheritance of the skip-if-passed-at-HEAD behaviour from
//! `gate pass` (already-passed gates are not re-run).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn jit() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
}

fn git(args: &[&str], dir: &Path) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A jit repo inside a fresh git repo with one commit (HEAD resolves).
fn setup_git_jit_repo() -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    git(&["init"], &root);
    git(&["config", "user.name", "Test User"], &root);
    git(&["config", "user.email", "test@example.com"], &root);
    jit().current_dir(&root).arg("init").output().unwrap();
    fs::write(root.join("README.md"), "repo").unwrap();
    git(&["add", "-A"], &root);
    git(&["commit", "-m", "init"], &root);
    (temp, root)
}

/// Define an auto gate `key` whose checker runs `command` and appends a line to
/// `<key>.log` (a side-effect proving the checker ran).
fn define_gate(root: &Path, key: &str, command: &str) {
    let checker = format!("echo run >> {}.log; {}", key, command);
    jit()
        .current_dir(root)
        .args([
            "gate",
            "define",
            key,
            "--title",
            key,
            "--description",
            "test",
            "--mode",
            "auto",
            "--checker-command",
            &checker,
            "--timeout",
            "10",
        ])
        .output()
        .unwrap();
}

/// Create an issue requiring the listed gates (in order). Returns its id.
fn create_issue_with_gates(root: &Path, gates: &[&str]) -> String {
    let mut args = vec!["issue", "create", "--title", "Work"];
    for g in gates {
        args.push("--gate");
        args.push(g);
    }
    args.push("--json");
    let out = jit().current_dir(root).args(&args).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    json["id"].as_str().unwrap().to_string()
}

/// How many times gate `key`'s checker ran (lines in its side-effect log).
fn run_count(root: &Path, key: &str) -> usize {
    fs::read_to_string(root.join(format!("{}.log", key)))
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

#[test]
fn test_pass_all_all_gates_pass_exit_0() {
    let (_temp, root) = setup_git_jit_repo();
    define_gate(&root, "g1", "true");
    define_gate(&root, "g2", "true");
    let id = create_issue_with_gates(&root, &["g1", "g2"]);

    let out = jit()
        .current_dir(&root)
        .args(["gate", "pass-all", &id, "--json"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["verdict"], "pass");
    let gates = json["gates"].as_array().unwrap();
    assert_eq!(gates.len(), 2);
    assert_eq!(gates[0]["gate_key"], "g1");
    assert_eq!(gates[0]["already_passed"], false);
    assert_eq!(gates[1]["gate_key"], "g2");
    assert_eq!(run_count(&root, "g1"), 1);
    assert_eq!(run_count(&root, "g2"), 1);
}

#[test]
fn test_pass_all_fail_fast_stops_at_first_failure_exit_4() {
    let (_temp, root) = setup_git_jit_repo();
    define_gate(&root, "g1", "false"); // first gate fails (checker exit 1)
    define_gate(&root, "g2", "true"); // must never run
    let id = create_issue_with_gates(&root, &["g1", "g2"]);

    let out = jit()
        .current_dir(&root)
        .args(["gate", "pass-all", &id, "--json"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(4),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["error"]["code"], "GATE_FAILED");
    assert_eq!(json["error"]["details"]["verdict"], "fail");
    assert_eq!(json["error"]["details"]["gate_key"], "g1");

    // Fail-fast: the first gate ran, the later gate's checker never did.
    assert_eq!(run_count(&root, "g1"), 1);
    assert_eq!(
        run_count(&root, "g2"),
        0,
        "later gate must NOT run after a fail-fast"
    );
}

#[test]
fn test_pass_all_runner_error_exit_10() {
    let (_temp, root) = setup_git_jit_repo();
    // First gate's checker kills itself -> no exit code -> runner error.
    define_gate(&root, "g1", "kill -9 $$");
    define_gate(&root, "g2", "true");
    let id = create_issue_with_gates(&root, &["g1", "g2"]);

    let out = jit()
        .current_dir(&root)
        .args(["gate", "pass-all", &id, "--json"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(10),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["error"]["code"], "IO_ERROR");
    assert_eq!(json["error"]["details"]["verdict"], "error");
    assert_eq!(run_count(&root, "g2"), 0, "later gate must not run");
}

#[test]
fn test_pass_all_skips_already_passed_gate() {
    let (_temp, root) = setup_git_jit_repo();
    define_gate(&root, "g1", "true");
    define_gate(&root, "g2", "true");
    let id = create_issue_with_gates(&root, &["g1", "g2"]);

    // Pre-pass g1 at HEAD so pass-all should skip it.
    jit()
        .current_dir(&root)
        .args(["gate", "pass", &id, "g1"])
        .output()
        .unwrap();
    assert_eq!(run_count(&root, "g1"), 1);

    let out = jit()
        .current_dir(&root)
        .args(["gate", "pass-all", &id, "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let gates = json["gates"].as_array().unwrap();
    assert_eq!(gates.len(), 2);
    assert_eq!(gates[0]["gate_key"], "g1");
    assert_eq!(
        gates[0]["already_passed"], true,
        "g1 already passed at HEAD; must be skipped"
    );
    assert_eq!(gates[1]["gate_key"], "g2");
    assert_eq!(gates[1]["already_passed"], false);

    // g1's checker did NOT re-run; g2's ran once.
    assert_eq!(run_count(&root, "g1"), 1, "g1 must not re-run");
    assert_eq!(run_count(&root, "g2"), 1);
}

#[test]
fn test_pass_all_no_required_gates_exit_0() {
    let (_temp, root) = setup_git_jit_repo();
    let id = create_issue_with_gates(&root, &[]);

    let out = jit()
        .current_dir(&root)
        .args(["gate", "pass-all", &id, "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["gates"].as_array().unwrap().len(), 0);
}

#[test]
fn test_pass_all_issue_not_found_exit_3() {
    let (_temp, root) = setup_git_jit_repo();

    let out = jit()
        .current_dir(&root)
        .args(["gate", "pass-all", "nonexistent", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(3));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["error"]["code"], "ISSUE_NOT_FOUND");
}
