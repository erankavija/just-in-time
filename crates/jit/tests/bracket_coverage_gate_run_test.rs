//! End-to-end integration test for the bracket coverage-preview gate.
//!
//! The bracket-breakdown helper is a pure bracket-builder: it ATTACHES the
//! coverage-preview gate to the breakdown node `B` (left PENDING) and never runs
//! it. The gate is run separately by the standard gate runner
//! (`jit gate pass <B> coverage-preview`) as a breakdown-workflow step. This test
//! proves that the *attached* gate, when run via the real runner against a
//! manually-built bracket fixture, executes the deterministic
//! `jit validate --scope <C>` checker (the project's `scripts/coverage-preview.sh`)
//! and persists a `GateRunResult` reflecting:
//!   - PASS (exit 0) when the drafted children cover every `[hard]` criterion, and
//!   - FAIL (exit 4) when a `[hard]` criterion is left uncovered.
//!
//! It exercises the real subprocess path end-to-end (built `jit` binary against a
//! temp `.jit` repo). The bracket spine `C → child → B` is built by hand so that
//! `B` (which carries the coverage rule's `type:breakdown` selector and the
//! `brackets:<C>` pointer) is inside `C`'s dependency closure — exactly the shape
//! `bracket_breakdown` produces. The checker (`coverage-preview.sh`) resolves `C`
//! from `B`'s `brackets:` label and shells out to the built `jit` binary, so the
//! gate's PATH is set to the built-binary directory.

use assert_cmd::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// `[planning]` vocabulary plus a hierarchy that declares the bracket types, and
/// the coverage-preview deterministic rule keyed on `type:breakdown`.
const PLANNING_CONFIG_TOML: &str = r#"
[type_hierarchy]
types = { epic = 1, planning = 2, breakdown = 2, task = 3 }

[planning]
breakable_types = ["epic"]
planning_type = "planning"
breakdown_type = "breakdown"
plan_doc_location = "inline"
plan_gate_preset = "plan-review"
coverage_gate_preset = "coverage-preview"
"#;

const COVERAGE_RULES_TOML: &str = r#"
[[rules]]
name = "coverage-preview"
when = { type = "breakdown" }
severity = "error"
enforce = true
assert = { label-coverage = { criteria-section = "success_criteria", marker = "[hard]", id-pattern = "REQ-[0-9]+", satisfies-namespace = "satisfies", child-link = "dependencies", child-type-exclude = ["planning", "breakdown"], container-from-label = "brackets" } }
"#;

/// Absolute path to the project's real coverage-preview gate checker.
fn coverage_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2) // crates/jit -> crates -> repo root
        .expect("repo root above crates/jit")
        .join("scripts")
        .join("coverage-preview.sh")
}

/// Directory containing the built `jit` binary under test, so the gate checker's
/// bare `jit` invocations resolve to *this* build (not a stale installed one).
fn bin_dir() -> PathBuf {
    let mut p = PathBuf::from(assert_cmd::cargo::cargo_bin!("jit"));
    p.pop();
    p
}

fn jit(temp: &TempDir) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path());
    cmd
}

/// Run a jit command and return parsed JSON stdout (asserting success).
fn jit_json(temp: &TempDir, args: &[&str]) -> serde_json::Value {
    let out = jit(temp)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap_or_else(|e| {
        panic!(
            "expected JSON from `jit {}`: {e}\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out)
        )
    })
}

/// Create an issue, returning its full id.
fn create_issue(temp: &TempDir, title: &str, description: &str, labels: &[&str]) -> String {
    let mut args = vec![
        "issue",
        "create",
        "--title",
        title,
        "--description",
        description,
        "--json",
    ];
    for l in labels {
        args.push("--label");
        args.push(l);
    }
    let json = jit_json(temp, &args);
    json["id"].as_str().expect("created issue id").to_string()
}

/// Initialize a temp repo with the planning config, coverage rule, and a
/// `coverage-preview` auto gate wired to the real checker (PATH points at the
/// built binary so the script's bare `jit` resolves to this build).
fn setup_bracket_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    jit(&temp).arg("init").assert().success();

    let jit_dir = temp.path().join(".jit");
    std::fs::write(jit_dir.join("config.toml"), PLANNING_CONFIG_TOML).unwrap();
    std::fs::write(jit_dir.join("rules.toml"), COVERAGE_RULES_TOML).unwrap();

    let script = coverage_script();
    let path_env = format!(
        "{}:{}",
        bin_dir().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    jit(&temp)
        .args([
            "gate",
            "define",
            "coverage-preview",
            "--title",
            "Coverage Preview",
            "--description",
            "Scoped [hard]-criterion coverage check",
            "--mode",
            "auto",
            "--checker-command",
            script.to_str().unwrap(),
            "--timeout",
            "60",
            "--env",
            &format!("PATH={path_env}"),
        ])
        .assert()
        .success();
    temp
}

/// Build a bracket `C → child → B` by hand (the shape `bracket_breakdown`
/// produces) with the coverage-preview gate attached to `B`. `child_labels`
/// carries the child's coverage credits (`satisfies:<id>`), if any. Returns
/// `(container_id, breakdown_id)`.
fn build_bracket(
    temp: &TempDir,
    container_title: &str,
    req_id: &str,
    child_labels: &[&str],
) -> (String, String) {
    let description = format!("## Success Criteria\n\n- [hard] {req_id}: it works\n");
    let c = create_issue(temp, container_title, &description, &["type:epic"]);

    let mut kid_labels = vec!["type:task"];
    kid_labels.extend_from_slice(child_labels);
    let k = create_issue(temp, "Impl child", "", &kid_labels);

    let b = create_issue(
        temp,
        &format!("Breakdown: {container_title}"),
        "",
        &["type:breakdown", &format!("brackets:{c}")],
    );

    // Attach the coverage-preview gate to B (PENDING — as the bracket-builder
    // leaves it).
    jit(temp)
        .args(["gate", "add", &b, "coverage-preview"])
        .assert()
        .success();
    // Spine: source child depends on B; C depends on the sink child. This puts B
    // inside C's dependency closure, where the scoped coverage rule evaluates it.
    jit(temp).args(["dep", "add", &k, &b]).assert().success();
    jit(temp).args(["dep", "add", &c, &k]).assert().success();

    (c, b)
}

/// Assert a persisted gate-run result exists for `B` reflecting `status`.
fn assert_gate_run(temp: &TempDir, breakdown_id: &str, expected_status: &str) {
    // A gate-run result file is persisted on disk.
    let runs_dir = temp.path().join(".jit").join("gate-runs");
    assert!(
        runs_dir.exists(),
        ".jit/gate-runs/ must exist after a real gate run"
    );
    let any_run = std::fs::read_dir(&runs_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .next()
        .is_some();
    assert!(any_run, "a GateRunResult must be persisted for the run");

    // `jit gate check` reflects the recorded run (status + exit code).
    let json = jit_json(
        temp,
        &["gate", "check", breakdown_id, "coverage-preview", "--json"],
    );
    assert_eq!(
        json["status"].as_str(),
        Some(expected_status),
        "recorded gate-run status mismatch: {json}"
    );
    assert!(
        json["run_id"].is_string(),
        "persisted run must carry a run_id: {json}"
    );
}

#[test]
fn test_attached_coverage_gate_runs_and_passes_when_hard_criterion_covered() {
    let temp = setup_bracket_repo();
    // Child credits REQ-01 via satisfies:REQ-01 → coverage complete.
    let (_c, b) = build_bracket(&temp, "Auth epic", "REQ-01", &["satisfies:REQ-01"]);

    // Run the ATTACHED gate via the standard runner. Covered → exit 0.
    jit(&temp)
        .args(["gate", "pass", &b, "coverage-preview"])
        .assert()
        .success();

    assert_gate_run(&temp, &b, "passed");

    // The breakdown node's gate status reflects the real run.
    let issue = jit_json(&temp, &["issue", "show", &b, "--json"]);
    assert_eq!(
        issue["gates_status"]["coverage-preview"]["status"].as_str(),
        Some("passed"),
        "covered run must record B's coverage-preview gate Passed: {issue}"
    );
}

#[test]
fn test_attached_coverage_gate_runs_and_fails_when_hard_criterion_uncovered() {
    let temp = setup_bracket_repo();
    // Child carries NO satisfies label → REQ-77 left uncovered.
    let (_c, b) = build_bracket(&temp, "Pay epic", "REQ-77", &[]);

    // Run the ATTACHED gate via the standard runner. Uncovered → exit 4.
    jit(&temp)
        .args(["gate", "pass", &b, "coverage-preview"])
        .assert()
        .failure()
        .code(4);

    assert_gate_run(&temp, &b, "failed");

    // The persisted run's stdout names the uncovered criterion.
    let json = jit_json(&temp, &["gate", "check", &b, "coverage-preview", "--json"]);
    assert!(
        json["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("REQ-77"),
        "the uncovered criterion REQ-77 must appear in the gate-run output: {json}"
    );

    // The breakdown node's gate status reflects the real failing run.
    let issue = jit_json(&temp, &["issue", "show", &b, "--json"]);
    assert_eq!(
        issue["gates_status"]["coverage-preview"]["status"].as_str(),
        Some("failed"),
        "uncovered run must record B's coverage-preview gate Failed: {issue}"
    );
}
