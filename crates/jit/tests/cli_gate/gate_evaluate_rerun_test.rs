//! Integration tests for `jit gate evaluate` over a prior pass: an automated
//! checker executes again, and a manual gate requires a fresh attestation.

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

/// An automated gate declaring no inputs executes its checker on every
/// evaluation, including a second one at the same `HEAD` over a passing run.
#[test]
fn test_gate_evaluate_reruns_checker_without_declared_inputs_at_unchanged_head() {
    let (_temp, root) = setup_git_jit_repo();
    let id = define_counting_gate_and_issue(&root);

    for expected_runs in 1..=2 {
        let out = jit()
            .current_dir(&root)
            .args(["gate", "evaluate", &id, "counting", "--json"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0));
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(json["verdict"], "pass");
        assert_eq!(run_count(&root), expected_runs);
    }
}

/// The `updated_by` stamped on a gate's current status, read straight from the
/// on-disk issue JSON (no CLI surface exposes it). `None` when never set.
fn gate_updated_by(root: &Path, id: &str, key: &str) -> Option<String> {
    let path = root.join(".jit").join("issues").join(format!("{id}.json"));
    let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    json["gates_status"][key]["updated_by"]
        .as_str()
        .map(str::to_string)
}

#[test]
fn test_manual_redefine_requires_attestation_over_stale_auto_pass() {
    // Regression (jit:1d59070d F1): an auto gate passes at HEAD, then the gate is
    // redefined to manual. The lingering auto-era pass must NOT let a bare
    // `evaluate` short-circuit. Manual evaluation always requires and records a
    // fresh attestation, including when the current status is already Passed.
    let (_temp, root) = setup_git_jit_repo();
    let id = define_counting_gate_and_issue(&root);

    // Auto evaluate at HEAD: checker runs, records a passing run stamped
    // auto:executor and sets the gate status Passed.
    let first = jit()
        .current_dir(&root)
        .args(["gate", "evaluate", &id, "counting", "--json"])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(json["verdict"], "pass");
    assert_eq!(run_count(&root), 1);
    assert_eq!(gate_status(&root, &id, "counting"), "passed");
    assert_eq!(
        gate_updated_by(&root, &id, "counting").as_deref(),
        Some("auto:executor"),
        "the auto pass is stamped auto:executor"
    );

    // Redefine the gate to manual (the checker is dropped); the issue's stale
    // auto-era pass in gates_status is left untouched.
    let redefine = jit()
        .current_dir(&root)
        .args(["gate", "update", "counting", "--mode", "manual"])
        .output()
        .unwrap();
    assert_eq!(redefine.status.code(), Some(0));

    // Bare evaluate: must NOT short-circuit on the stale auto pass. Attestation is
    // required -> ManualGateAttestationRequiredError, exit 2 (InvalidArgument).
    let bare = jit()
        .current_dir(&root)
        .args(["gate", "evaluate", &id, "counting"])
        .output()
        .unwrap();
    assert_eq!(
        bare.status.code(),
        Some(2),
        "stale auto pass must not satisfy manual attestation: {}",
        String::from_utf8_lossy(&bare.stderr)
    );

    // Attest with --by: records a fresh attested pass (updated_by = human:alice).
    let attested = jit()
        .current_dir(&root)
        .args([
            "gate",
            "evaluate",
            &id,
            "counting",
            "--by",
            "human:alice",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(attested.status.code(), Some(0));
    assert_eq!(gate_status(&root, &id, "counting"), "passed");
    assert_eq!(
        gate_updated_by(&root, &id, "counting").as_deref(),
        Some("human:alice"),
        "attested pass records the human attestor"
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
