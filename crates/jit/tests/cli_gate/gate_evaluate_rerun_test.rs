//! Integration tests for `jit gate evaluate` over a prior pass: an automated
//! checker executes again, and a manual gate requires a fresh attestation.

use crate::add_auto_gate_issue;
use jit::test_cli::JitCli;
use std::fs;

/// Auto gate `counting` appends a line to `runs.log` on every execution, so
/// the line count is the number of checker runs. Returns the id of an issue
/// requiring it.
fn define_counting_gate_and_issue(cli: &JitCli) -> String {
    add_auto_gate_issue(cli, "counting", "echo run >> runs.log")
}

fn run_count(cli: &JitCli) -> usize {
    fs::read_to_string(cli.root().join("runs.log"))
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

/// An automated gate declaring no inputs executes its checker on every
/// evaluation, including a second one at the same `HEAD` over a passing run.
#[test]
fn test_gate_evaluate_reruns_checker_without_declared_inputs_at_unchanged_head() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("init").unwrap();
    let id = define_counting_gate_and_issue(&cli);

    for expected_runs in 1..=2 {
        let out = cli
            .command(["gate", "evaluate", &id, "counting", "--json"])
            .output()
            .unwrap();
        assert!(out.status.success());
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(json["verdict"], "pass");
        assert_eq!(run_count(&cli), expected_runs);
    }
}

/// The `updated_by` stamped on a gate's current status, read straight from the
/// on-disk issue JSON (no CLI surface exposes it). `None` when never set.
fn gate_updated_by(cli: &JitCli, id: &str, key: &str) -> Option<String> {
    let path = cli
        .root()
        .join(".jit")
        .join("issues")
        .join(format!("{id}.json"));
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
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("init").unwrap();
    let id = define_counting_gate_and_issue(&cli);

    // Auto evaluate at HEAD: checker runs, records a passing run stamped
    // auto:executor and sets the gate status Passed.
    let first = cli
        .command(["gate", "evaluate", &id, "counting", "--json"])
        .output()
        .unwrap();
    assert!(first.status.success());
    let json: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(json["verdict"], "pass");
    assert_eq!(run_count(&cli), 1);
    assert_eq!(gate_status(&cli, &id, "counting"), "passed");
    assert_eq!(
        gate_updated_by(&cli, &id, "counting").as_deref(),
        Some("auto:executor"),
        "the auto pass is stamped auto:executor"
    );

    // Redefine the gate to manual (the checker is dropped); the issue's stale
    // auto-era pass in gates_status is left untouched.
    let redefine = cli
        .command(["gate", "update", "counting", "--mode", "manual"])
        .output()
        .unwrap();
    assert!(redefine.status.success());

    // Bare evaluate: must NOT short-circuit on the stale auto pass. Attestation is
    // required -> ManualGateAttestationRequiredError.
    let bare = cli
        .command(["gate", "evaluate", &id, "counting"])
        .output()
        .unwrap();
    let bare_stderr = String::from_utf8_lossy(&bare.stderr);
    assert!(
        !bare.status.success() && bare_stderr.contains("--by <attestor>"),
        "stale auto pass must not satisfy manual attestation: {bare_stderr}"
    );

    // Attest with --by: records a fresh attested pass (updated_by = human:alice).
    let attested = cli
        .command([
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
    assert!(attested.status.success());
    assert_eq!(gate_status(&cli, &id, "counting"), "passed");
    assert_eq!(
        gate_updated_by(&cli, &id, "counting").as_deref(),
        Some("human:alice"),
        "attested pass records the human attestor"
    );
}

/// Current `status` of a gate from `issue show --json`.
fn gate_status(cli: &JitCli, id: &str, key: &str) -> String {
    let out = cli
        .command(["issue", "show", id, "--json"])
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
