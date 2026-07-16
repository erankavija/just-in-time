//! Regression for jit:46657f6f: a gate checker spawned by `jit gate evaluate`
//! can run a *mutating* nested `jit` command against the same repository
//! without deadlocking on the recovery boundary the evaluator holds.
//!
//! The universal mutation recovery boundary (jit:eceffc17) makes every
//! mutating CLI command hold the bootstrap → repository write locks for its
//! whole execution. `jit gate evaluate` is mutating (it records gate runs), so
//! it keeps those locks while the checker runs. A checker that shells out to a
//! nested mutating `jit` (this repo's `docs-mechanical` gate runs `jit
//! invariant render`) would block on those exact locks and die at its lock
//! timeout — the gate becomes unpassable via `jit gate evaluate`.
//!
//! The fix suspends the evaluator's startup recovery session while the external
//! checker runs. Nested and unrelated `jit` processes therefore acquire the
//! ordinary bootstrap → repository lock chain themselves. The evaluator
//! reacquires that chain and runs recovery before persisting the checker result.
//! These tests exercise the end-to-end CLI path where the recovery session is
//! genuinely held, which an in-process command harness cannot reproduce.

use assert_cmd::prelude::*;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

fn jit_bin() -> std::path::PathBuf {
    assert_cmd::cargo::cargo_bin!("jit").to_path_buf()
}

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    Command::new(jit_bin())
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    temp
}

/// Define an auto gate running `checker_command` and create an issue requiring
/// it. Returns the short issue id.
fn define_gate_and_issue(temp: &TempDir, gate: &str, checker_command: &str) -> String {
    Command::new(jit_bin())
        .current_dir(temp.path())
        .args([
            "gate",
            "define",
            gate,
            "--title",
            gate,
            "--description",
            "nested-checker regression gate",
            "--mode",
            "auto",
            "--checker-command",
            checker_command,
            "--timeout",
            "30",
        ])
        .assert()
        .success();

    let output = Command::new(jit_bin())
        .current_dir(temp.path())
        .args(["issue", "create", "--title", "carrier", "--gate", gate])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output_str = String::from_utf8_lossy(&output);
    output_str
        .lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

/// Titles of every issue currently in the repository.
fn issue_titles(temp: &TempDir) -> Vec<String> {
    let output = Command::new(jit_bin())
        .current_dir(temp.path())
        .args(["issue", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["title"].as_str().map(str::to_string))
        .collect()
}

/// REQ-01: a checker that invokes a nested mutating `jit` against the same
/// repository runs to completion and the gate records a pass.
#[test]
fn test_gate_evaluate_allows_nested_mutating_jit() {
    let temp = setup_repo();
    let jit = jit_bin();
    // The checker creates a fresh issue in the SAME repository through a nested
    // `jit` invocation — a mutating command that takes the repository write
    // lock the evaluator is holding.
    let checker = format!("\"{}\" issue create --title nested-mutation", jit.display());
    let issue_id = define_gate_and_issue(&temp, "nested-mut", &checker);

    Command::new(&jit)
        .current_dir(temp.path())
        .args(["gate", "evaluate", &issue_id, "nested-mut"])
        .assert()
        .success();

    // The nested mutation actually landed: proof the child acquired the locks
    // rather than timing out and exiting nonzero.
    let titles = issue_titles(&temp);
    assert!(
        titles.iter().any(|t| t == "nested-mutation"),
        "nested `jit issue create` must have created its issue; titles: {titles:?}"
    );

    // And the gate is recorded as passed.
    let output = Command::new(&jit)
        .current_dir(temp.path())
        .args(["gate", "status", &issue_id, "nested-mut", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        json["status"], "passed",
        "gate must be recorded as passed: {json}"
    );
}

/// REQ-02: an unrelated process still takes the real bootstrap lock. There is
/// no environment-variable or descendant bypass that can turn the lock into an
/// in-process-only guard.
#[test]
fn test_unrelated_process_still_respects_bootstrap_lock() {
    let temp = setup_repo();
    let jit = jit_bin();
    let lock = jit::storage::RepoWriteLock::for_lock_path(
        temp.path().join(".jit-bootstrap.lock"),
        Duration::from_secs(1),
    );
    let _guard = lock.acquire().unwrap();

    Command::new(&jit)
        .current_dir(temp.path())
        .env("JIT_LOCK_TIMEOUT", "1")
        .args(["issue", "create", "--title", "unrelated-write"])
        .assert()
        .failure();

    let titles = issue_titles(&temp);
    assert!(
        !titles.iter().any(|t| t == "unrelated-write"),
        "an unrelated process must not mutate while the bootstrap lock is held; titles: {titles:?}"
    );
}
