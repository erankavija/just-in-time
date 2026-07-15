//! Integration tests for `jit gate pass` skipping the checker when the gate
//! already passed at the current HEAD commit (issue 9bfcc474).
//!
//! Scenarios:
//!   (a) pass, then pass again at same HEAD -> exit 0, `already_passed: true`,
//!       checker did NOT re-run (asserted via a checker side-effect file).
//!   (b) `--force` re-runs even when already passed.
//!   (c) HEAD advanced / no prior passing run -> checker runs normally.
//!   (d) not a git repo (no HEAD) -> never skips, runs normally.

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

/// Define an auto gate whose checker appends a line to `runs.log` (a checker
/// side-effect) every time it executes, then passes. Counting the lines tells
/// us how many times the checker actually ran. Create an issue requiring it and
/// return its id.
fn define_counting_gate_and_issue(root: &Path) -> String {
    // Append to runs.log relative to the checker working dir (repo root).
    jit()
        .current_dir(root)
        .args([
            "gate",
            "define",
            "counting",
            "--title",
            "Counting Gate",
            "--description",
            "Counts runs",
            "--mode",
            "auto",
            "--checker-command",
            "echo run >> runs.log",
            "--timeout",
            "10",
        ])
        .output()
        .unwrap();

    let create = jit()
        .current_dir(root)
        .args([
            "issue", "create", "--title", "Work", "--gate", "counting", "--json",
        ])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&create.stdout).unwrap();
    json["id"].as_str().unwrap().to_string()
}

fn run_count(root: &Path) -> usize {
    fs::read_to_string(root.join("runs.log"))
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

#[test]
fn test_gate_pass_skips_when_already_passed_at_head() {
    let (_temp, root) = setup_git_jit_repo();
    let id = define_counting_gate_and_issue(&root);

    // First pass: checker runs once.
    let first = jit()
        .current_dir(&root)
        .args(["gate", "pass", &id, "counting", "--json"])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(json["verdict"], "pass");
    assert_eq!(json["already_passed"], false);
    assert_eq!(run_count(&root), 1, "checker should have run once");

    // Second pass at the SAME HEAD: must skip, checker count unchanged.
    let second = jit()
        .current_dir(&root)
        .args(["gate", "pass", &id, "counting", "--json"])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(json["already_passed"], true);
    assert_eq!(json["verdict"], "pass");
    assert_eq!(
        run_count(&root),
        1,
        "checker must NOT re-run on the skip path"
    );
}

#[test]
fn test_gate_pass_force_reruns_even_when_already_passed() {
    let (_temp, root) = setup_git_jit_repo();
    let id = define_counting_gate_and_issue(&root);

    jit()
        .current_dir(&root)
        .args(["gate", "pass", &id, "counting"])
        .output()
        .unwrap();
    assert_eq!(run_count(&root), 1);

    // --force must re-run the checker even though it already passed at HEAD.
    let forced = jit()
        .current_dir(&root)
        .args(["gate", "pass", &id, "counting", "--force", "--json"])
        .output()
        .unwrap();
    assert_eq!(forced.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&forced.stdout).unwrap();
    assert_eq!(json["already_passed"], false);
    assert_eq!(json["verdict"], "pass");
    assert_eq!(run_count(&root), 2, "--force must re-run the checker");
}

#[test]
fn test_gate_pass_reruns_when_head_advanced() {
    let (_temp, root) = setup_git_jit_repo();
    let id = define_counting_gate_and_issue(&root);

    jit()
        .current_dir(&root)
        .args(["gate", "pass", &id, "counting"])
        .output()
        .unwrap();
    assert_eq!(run_count(&root), 1);

    // Advance HEAD with a new commit; the prior pass no longer applies.
    fs::write(root.join("change.txt"), "edit").unwrap();
    git(&["add", "-A"], &root);
    git(&["commit", "-m", "advance"], &root);

    let again = jit()
        .current_dir(&root)
        .args(["gate", "pass", &id, "counting", "--json"])
        .output()
        .unwrap();
    assert_eq!(again.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&again.stdout).unwrap();
    assert_eq!(json["already_passed"], false);
    assert_eq!(run_count(&root), 2, "checker must run again at new HEAD");
}

#[test]
fn test_gate_pass_does_not_skip_without_git() {
    // No git repo: HEAD is None, so the skip path can never engage even after a
    // prior pass. The checker re-runs every time.
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    jit().current_dir(&root).arg("init").output().unwrap();
    let id = define_counting_gate_and_issue(&root);

    jit()
        .current_dir(&root)
        .args(["gate", "pass", &id, "counting"])
        .output()
        .unwrap();
    assert_eq!(run_count(&root), 1);

    let second = jit()
        .current_dir(&root)
        .args(["gate", "pass", &id, "counting", "--json"])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(
        json["already_passed"], false,
        "without git HEAD we cannot prove a prior pass; must not skip"
    );
    assert_eq!(
        run_count(&root),
        2,
        "checker must re-run when HEAD is unknown"
    );
}

/// Current `status` of a gate from `issue show --json`.
fn gate_status(root: &Path, id: &str, key: &str) -> String {
    let out = jit()
        .current_dir(root)
        .args(["issue", "show", id, "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    json["gates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["key"] == key)
        .unwrap()["status"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn test_gate_pass_reruns_when_status_reset_despite_passing_run_at_head() {
    // Regression: a passing GateRunResult lingers at HEAD, but the gate's CURRENT
    // status was reset to Pending (remove + re-add). The skip must NOT engage on
    // the stale historical run alone, or the gate would stay Pending while the
    // command falsely reports success.
    let (_temp, root) = setup_git_jit_repo();
    let id = define_counting_gate_and_issue(&root);

    // Pass once: records a passing run at HEAD and sets status Passed.
    jit()
        .current_dir(&root)
        .args(["gate", "pass", &id, "counting"])
        .output()
        .unwrap();
    assert_eq!(run_count(&root), 1);
    assert_eq!(gate_status(&root, &id, "counting"), "passed");

    // Remove then re-add the gate -> current status resets to Pending, while the
    // historical passing run at HEAD remains on disk.
    jit()
        .current_dir(&root)
        .args(["issue", "update", &id, "--remove-gate", "counting"])
        .output()
        .unwrap();
    jit()
        .current_dir(&root)
        .args(["gate", "add", &id, "counting"])
        .output()
        .unwrap();
    assert_eq!(gate_status(&root, &id, "counting"), "pending");

    // Pass again at the SAME HEAD: must RUN the checker (not skip on stale run).
    let again = jit()
        .current_dir(&root)
        .args(["gate", "pass", &id, "counting", "--json"])
        .output()
        .unwrap();
    assert_eq!(again.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&again.stdout).unwrap();
    assert_eq!(
        json["already_passed"], false,
        "must not skip when current status was reset to Pending"
    );
    assert_eq!(
        run_count(&root),
        2,
        "checker must re-run to re-establish a passed status"
    );
    assert_eq!(
        gate_status(&root, &id, "counting"),
        "passed",
        "gate status must be Passed again after the re-run"
    );
}
