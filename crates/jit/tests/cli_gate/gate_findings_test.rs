//! Structured gate findings in machine output (issue 27338abc).
//!
//! An automated checker may append a machine-readable block to its stdout:
//!
//! ```text
//! <<<JIT-FINDINGS-JSON
//! {"verdict":"fail","summary":"...","findings":[{"id":...,"severity":...,"summary":...}]}
//! JIT-FINDINGS-JSON>>>
//! ```
//!
//! jit parses that block at gate-run record time and surfaces it as structured
//! data across the gate views, while keeping raw stdout available. A checker
//! that emits no block degrades gracefully (no findings field, no error).

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn jit() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
}

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    jit()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    temp
}

fn define_auto_gate(temp: &TempDir, key: &str, checker_command: &str) {
    jit()
        .current_dir(temp.path())
        .args([
            "gate",
            "define",
            key,
            "--title",
            key,
            "--description",
            "Test gate for structured findings",
            "--mode",
            "auto",
            "--checker-command",
            checker_command,
            "--timeout",
            "10",
        ])
        .assert()
        .success();
}

fn create_issue(temp: &TempDir, gate_keys: &[&str]) -> String {
    let mut args: Vec<String> = vec![
        "issue".into(),
        "create".into(),
        "--title".into(),
        "Test issue".into(),
    ];
    for k in gate_keys {
        args.push("--gate".into());
        args.push((*k).into());
    }
    let out = jit()
        .current_dir(temp.path())
        .args(&args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&out);
    s.lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

fn run_gate(temp: &TempDir, issue_id: &str, gate_key: &str) {
    // Records a fresh run by executing the checker, regardless of verdict.
    jit()
        .current_dir(temp.path())
        .args(["gate", "pass", issue_id, gate_key, "--force"])
        .assert();
}

/// A checker command that emits a conforming findings block, then exits 1.
fn findings_checker() -> &'static str {
    concat!(
        "printf '%s\\n' ",
        "'<<<JIT-FINDINGS-JSON' ",
        r#"'{"verdict":"fail","summary":"1 issue found","findings":[{"id":"F1","severity":"high","disposition":"blocking","origin":"issue-impact","summary":"missing error context","file":"src/x.rs","line":42}]}' "#,
        "'JIT-FINDINGS-JSON>>>'; exit 1"
    )
}

// ---------------------------------------------------------------------------
// REQ-01: structured findings surface in the latest-run JSON view
// ---------------------------------------------------------------------------

#[test]
fn test_findings_surface_in_latest_run_json() {
    let temp = setup_repo();
    define_auto_gate(&temp, "code-review", findings_checker());
    let id = create_issue(&temp, &["code-review"]);
    run_gate(&temp, &id, "code-review");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "status", &id, "code-review", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let findings = &json["findings"];
    assert_eq!(findings["verdict"], "fail");
    assert_eq!(findings["summary"], "1 issue found");
    let arr = findings["findings"].as_array().expect("findings array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "F1");
    assert_eq!(arr[0]["severity"], "high");
    assert_eq!(arr[0]["disposition"], "blocking");
    assert_eq!(arr[0]["origin"], "issue-impact");
    assert_eq!(arr[0]["summary"], "missing error context");
    assert_eq!(arr[0]["file"], "src/x.rs");
    assert_eq!(arr[0]["line"], 42);
    // Raw stdout remains available alongside the structured findings.
    assert!(json["stdout"]
        .as_str()
        .unwrap()
        .contains("JIT-FINDINGS-JSON"));
}

#[test]
fn test_passing_structured_result_preserves_advisory_pre_existing_finding() {
    let temp = setup_repo();
    let checker = concat!(
        "printf '%s\\n' ",
        "'<<<JIT-FINDINGS-JSON' ",
        r#"'{"verdict":"pass","summary":"issue docs are complete; 1 advisory","findings":[{"id":"A1","severity":"low","disposition":"advisory","origin":"pre-existing","summary":"unrelated stale example","file":"docs/old.md","line":9}]}' "#,
        "'JIT-FINDINGS-JSON>>>'; exit 0"
    );
    define_auto_gate(&temp, "doc-review", checker);
    let id = create_issue(&temp, &["doc-review"]);
    run_gate(&temp, &id, "doc-review");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "status", &id, "doc-review", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["findings"]["verdict"], "pass");
    assert_eq!(json["findings"]["findings"][0]["disposition"], "advisory");
    assert_eq!(json["findings"]["findings"][0]["origin"], "pre-existing");
}

// ---------------------------------------------------------------------------
// REQ-01: findings surface in the history view too
// ---------------------------------------------------------------------------

#[test]
fn test_findings_surface_in_history_json() {
    let temp = setup_repo();
    define_auto_gate(&temp, "code-review", findings_checker());
    let id = create_issue(&temp, &["code-review"]);
    run_gate(&temp, &id, "code-review");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "status", &id, "--all", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["findings"]["verdict"], "fail");
    assert_eq!(results[0]["findings"]["findings"][0]["id"], "F1");
}

// ---------------------------------------------------------------------------
// REQ-01: findings surface (and survive lean projection) in status-all
// ---------------------------------------------------------------------------

#[test]
fn test_findings_survive_lean_projection_in_status_all() {
    let temp = setup_repo();
    // A PASSING checker whose stdout is dropped by the lean projection, but
    // whose structured findings must be retained.
    let checker = concat!(
        "printf '%s\\n' ",
        "'<<<JIT-FINDINGS-JSON' ",
        r#"'{"verdict":"pass","summary":"all clear","findings":[]}' "#,
        "'JIT-FINDINGS-JSON>>>'; exit 0"
    );
    define_auto_gate(&temp, "code-review", checker);
    let id = create_issue(&temp, &["code-review"]);
    run_gate(&temp, &id, "code-review");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "status-all", &id, "--json"])
        .assert()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let results = json["results"].as_array().unwrap();
    let review = results
        .iter()
        .find(|r| r["key"] == "code-review")
        .expect("code-review run present");
    // Lean form drops stdout for a passing run...
    assert!(review["stdout"].is_null());
    // ...but the structured findings survive.
    assert_eq!(review["findings"]["verdict"], "pass");
}

// ---------------------------------------------------------------------------
// REQ-03: --findings text view prints only findings + verdict
// ---------------------------------------------------------------------------

#[test]
fn test_findings_text_view() {
    let temp = setup_repo();
    define_auto_gate(&temp, "code-review", findings_checker());
    let id = create_issue(&temp, &["code-review"]);
    run_gate(&temp, &id, "code-review");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "status", &id, "code-review", "--findings"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&out);
    // Header line carries verdict + count; no run metadata decoration.
    assert!(s.contains("verdict: fail"), "verdict in header: {s}");
    assert!(s.contains("findings: 1"), "count in header: {s}");
    // One finding per line, greppable by severity and classifications.
    assert!(
        s.contains("F1 [high] [blocking] [issue-impact] missing error context"),
        "finding line: {s}"
    );
    assert!(s.contains("(src/x.rs:42)"), "locator on finding: {s}");
    // Findings view omits the full run details (no duration decoration).
    assert!(!s.contains("Duration:"), "no run detail decoration: {s}");
}

#[test]
fn test_findings_text_view_json() {
    let temp = setup_repo();
    define_auto_gate(&temp, "code-review", findings_checker());
    let id = create_issue(&temp, &["code-review"]);
    run_gate(&temp, &id, "code-review");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "status", &id, "code-review", "--findings", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["has_findings"], true);
    assert_eq!(json["verdict"], "fail");
    assert_eq!(json["findings"][0]["id"], "F1");
}

// ---------------------------------------------------------------------------
// REQ-04: non-conforming checker degrades gracefully
// ---------------------------------------------------------------------------

#[test]
fn test_non_conforming_checker_has_no_findings() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "echo just plain output; exit 0");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "status", &id, "tests", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    // No block emitted -> findings field absent, no error, stdout unchanged.
    assert!(json["findings"].is_null(), "no findings field: {json}");
    assert!(json["stdout"]
        .as_str()
        .unwrap()
        .contains("just plain output"));
}

#[test]
fn test_findings_view_degrades_for_plaintext_checker() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "echo just plain output; exit 0");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");

    // Text form: single header line noting the absence, no error.
    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "status", &id, "tests", "--findings"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&out);
    assert!(
        s.contains("no machine-readable findings block"),
        "graceful text: {s}"
    );

    // JSON form: has_findings=false, empty findings array.
    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "status", &id, "tests", "--findings", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["has_findings"], false);
    assert!(json["findings"].as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Findings ride along in the gate-blocked transition JSON error envelope
// ---------------------------------------------------------------------------

#[test]
fn test_findings_in_gate_failed_error_envelope() {
    let temp = setup_repo();
    define_auto_gate(&temp, "code-review", findings_checker());
    let id = create_issue(&temp, &["code-review"]);

    // `gate pass` (no --force) runs the checker; it exits 1 -> GATE_FAILED.
    let output = jit()
        .current_dir(temp.path())
        .args(["gate", "pass", &id, "code-review", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["details"]["verdict"], "fail");
    // The embedded checker_result carries the structured findings.
    let findings = &json["error"]["details"]["checker_result"]["findings"];
    assert_eq!(findings["verdict"], "fail");
    assert_eq!(findings["findings"][0]["id"], "F1");
}

// ---------------------------------------------------------------------------
// Mutual exclusion of the findings view with other views
// ---------------------------------------------------------------------------

#[test]
fn test_findings_and_history_conflict_json_error() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "echo x; exit 0");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");

    jit()
        .current_dir(temp.path())
        .args([
            "gate",
            "status",
            &id,
            "tests",
            "--findings",
            "--all",
            "--json",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("mutually exclusive"));
}
