//! Regression for jit:46657f6f: a gate checker spawned by `jit gate evaluate`
//! can run a *mutating* nested `jit` command against the same repository
//! without deadlocking on the recovery boundary the evaluator holds.
//!
//! The universal mutation recovery boundary (jit:eceffc17) makes every
//! mutating CLI command hold the bootstrap → repository write locks for its
//! whole execution. `jit gate evaluate` is mutating (it records gate runs), so
//! it keeps those locks while the checker runs. A checker that shells out to a
//! nested mutating `jit` (this repo's `docs-mechanical` gate runs `jit
//! project render`) would block on those exact locks and die at its lock
//! timeout — the gate becomes unpassable via `jit gate evaluate`.
//!
//! The fix suspends the evaluator's startup recovery session while the external
//! checker runs. Nested and unrelated `jit` processes therefore acquire the
//! ordinary bootstrap → repository lock chain themselves. The evaluator
//! reacquires that chain and runs recovery before persisting the checker result.
//! These tests exercise the end-to-end CLI path where the recovery session is
//! genuinely held, which an in-process command harness cannot reproduce.

use assert_cmd::prelude::*;
use jit::test_cli::{jit_binary, JitCli};
use std::time::Duration;

fn define_gate_and_issue(cli: &JitCli, gate: &str, checker_command: &str) -> String {
    cli.run([
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
    .unwrap();
    cli.create_issue(["--title", "carrier", "--gate", gate])
        .unwrap()
}

/// Titles of every issue currently in the repository.
fn issue_titles(cli: &JitCli) -> Vec<String> {
    cli.run_json(["issue", "list"]).unwrap()["issues"]
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
    let cli = JitCli::initialised().unwrap();
    let jit = jit_binary();
    // The checker creates a fresh issue in the SAME repository through a nested
    // `jit` invocation — a mutating command that takes the repository write
    // lock the evaluator is holding.
    let checker = format!("\"{}\" issue create --title nested-mutation", jit.display());
    let issue_id = define_gate_and_issue(&cli, "nested-mut", &checker);

    cli.command(["gate", "evaluate", &issue_id, "nested-mut"])
        .assert()
        .success();

    // The nested mutation actually landed: proof the child acquired the locks
    // rather than timing out and exiting nonzero.
    let titles = issue_titles(&cli);
    assert!(
        titles.iter().any(|t| t == "nested-mutation"),
        "nested `jit issue create` must have created its issue; titles: {titles:?}"
    );

    // And the gate is recorded as passed.
    let json = cli
        .run_json(["gate", "status", &issue_id, "nested-mut"])
        .unwrap();
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
    let cli = JitCli::initialised().unwrap();
    let lock = jit::storage::RepoWriteLock::for_lock_path(
        cli.root().join(".jit-bootstrap.lock"),
        Duration::from_secs(1),
    );
    let _guard = lock.acquire().unwrap();

    cli.command_with_env(
        ["issue", "create", "--title", "unrelated-write"],
        [("JIT_LOCK_TIMEOUT", "1")],
    )
    .assert()
    .failure();

    let titles = issue_titles(&cli);
    assert!(
        !titles.iter().any(|t| t == "unrelated-write"),
        "an unrelated process must not mutate while the bootstrap lock is held; titles: {titles:?}"
    );
}
