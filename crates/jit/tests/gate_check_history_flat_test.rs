//! REQ-08: `gate check` is the unified gate-run inspection surface.
//!
//! Beyond the existing latest-run view, `gate check` now exposes:
//!   - History (`--all` / `--limit <N>`): prior runs newest-first, filterable
//!     by `--gate <key>` and `--status <passed|failed|...>`.
//!   - Flat report text (`--stdout` / `--stderr` / `--tail <N>`): the stored
//!     report text printed verbatim with no wrapping or decoration.
//!
//! Every view supports `--json`, and argument errors are machine-readable.
//!
//! `gate check-all` is retained unchanged (latest-run snapshot across gates).

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

/// Define an auto gate with the given checker command.
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
            "Test gate for REQ-08",
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

/// Create an issue requiring the given gate keys, returning the short id.
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

/// Run `gate pass --force` for a gate (records a fresh run regardless of verdict).
fn run_gate(temp: &TempDir, issue_id: &str, gate_key: &str) {
    // Verdict may be pass or fail; we only need the run recorded.
    jit()
        .current_dir(temp.path())
        .args(["gate", "pass", issue_id, gate_key, "--force"])
        .assert();
}

// ---------------------------------------------------------------------------
// Latest-by-default: unchanged behaviour (regression guard)
// ---------------------------------------------------------------------------

#[test]
fn test_check_latest_by_default_unchanged() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "echo hello; exit 0");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");

    jit()
        .current_dir(temp.path())
        .args(["gate", "check", &id, "tests"])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed").or(predicate::str::contains("Passed")));
}

// ---------------------------------------------------------------------------
// History: --all lists prior runs newest-first
// ---------------------------------------------------------------------------

#[test]
fn test_check_all_history_lists_prior_runs() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "echo run; exit 0");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");
    run_gate(&temp, &id, "tests");
    run_gate(&temp, &id, "tests");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "check", &id, "--all", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let results = json["results"].as_array().expect("history results array");
    assert_eq!(results.len(), 3, "all three runs listed");

    // Newest-first: started_at descending.
    let times: Vec<&str> = results
        .iter()
        .map(|r| r["started_at"].as_str().unwrap())
        .collect();
    let mut sorted = times.clone();
    sorted.sort();
    sorted.reverse();
    assert_eq!(times, sorted, "runs are newest-first");
}

#[test]
fn test_check_limit_caps_history() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "echo run; exit 0");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");
    run_gate(&temp, &id, "tests");
    run_gate(&temp, &id, "tests");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "check", &id, "--limit", "2", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 2, "limit caps the history to 2");
}

// ---------------------------------------------------------------------------
// History: --gate and --status filters
// ---------------------------------------------------------------------------

#[test]
fn test_check_history_gate_filter() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "echo t; exit 0");
    define_auto_gate(&temp, "clippy", "echo c; exit 0");
    let id = create_issue(&temp, &["tests", "clippy"]);
    run_gate(&temp, &id, "tests");
    run_gate(&temp, &id, "clippy");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "check", &id, "--all", "--gate", "tests", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "only the tests gate run is listed");
    assert_eq!(results[0]["gate_key"], "tests");
}

#[test]
fn test_check_history_status_filter() {
    let temp = setup_repo();
    define_auto_gate(&temp, "passing", "echo ok; exit 0");
    define_auto_gate(&temp, "failing", "echo boom; exit 1");
    let id = create_issue(&temp, &["passing", "failing"]);
    run_gate(&temp, &id, "passing");
    run_gate(&temp, &id, "failing");

    let out = jit()
        .current_dir(temp.path())
        .args([
            "gate", "check", &id, "--all", "--status", "failed", "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "only failed runs listed");
    assert_eq!(results[0]["status"], "failed");
}

#[test]
fn test_check_history_invalid_status_json_error() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "echo ok; exit 0");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "check", &id, "--all", "--status", "bogus", "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value =
        serde_json::from_slice(&out).expect("invalid --status must emit machine-readable JSON");
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

// ---------------------------------------------------------------------------
// Flat report text: --stdout / --stderr / --tail
// ---------------------------------------------------------------------------

#[test]
fn test_check_stdout_flat_verbatim() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "echo FLAT_MARKER_LINE; exit 0");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "check", &id, "tests", "--stdout"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&out);
    // Verbatim: the report line is present with no "stdout:" decoration prefix.
    assert!(s.contains("FLAT_MARKER_LINE"), "stdout text printed");
    assert!(
        !s.contains("Gate '") && !s.contains("Duration:"),
        "flat output is undecorated, got: {s}"
    );
}

#[test]
fn test_check_stdout_flat_is_byte_exact_no_injected_newline() {
    // The flat view must emit the stored report text byte-for-byte. A checker
    // whose stdout has NO trailing newline must come back with NO trailing
    // newline — `print!`, not `println!` (which would append one).
    let temp = setup_repo();
    // `printf` (no `\n`) stores stdout with no trailing newline.
    define_auto_gate(&temp, "tests", "printf NO_TRAILING_NL; exit 0");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "check", &id, "tests", "--stdout"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        out,
        b"NO_TRAILING_NL",
        "flat --stdout must be byte-exact (no injected trailing newline), got: {:?}",
        String::from_utf8_lossy(&out)
    );
}

#[test]
fn test_check_stderr_flat_verbatim() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "echo ERRLINE 1>&2; exit 1");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "check", &id, "tests", "--stderr"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("ERRLINE"), "stderr text printed verbatim");
}

#[test]
fn test_check_tail_limits_lines() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "printf 'a\\nb\\nc\\nd\\ne\\n'; exit 0");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "check", &id, "tests", "--stdout", "--tail", "2"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("d") && s.contains("e"), "last two lines present");
    assert!(
        !s.contains("a"),
        "earlier lines trimmed by --tail, got: {s}"
    );
}

#[test]
fn test_check_stdout_flat_json() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "echo JSONFLAT; exit 0");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "check", &id, "tests", "--stdout", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["gate_key"], "tests");
    assert!(
        json["stdout"].as_str().unwrap().contains("JSONFLAT"),
        "flat json carries stdout text"
    );
}

// ---------------------------------------------------------------------------
// Mutual exclusion: history vs flat -> machine-readable error
// ---------------------------------------------------------------------------

#[test]
fn test_check_history_and_flat_conflict_json_error() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "echo ok; exit 0");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "check", &id, "tests", "--all", "--stdout", "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn test_check_status_without_history_json_error() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "echo ok; exit 0");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");

    let out = jit()
        .current_dir(temp.path())
        .args([
            "gate", "check", &id, "tests", "--status", "passed", "--json",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

// ---------------------------------------------------------------------------
// gate check-all is retained
// ---------------------------------------------------------------------------

#[test]
fn test_check_all_command_retained() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests", "echo ok; exit 0");
    let id = create_issue(&temp, &["tests"]);
    run_gate(&temp, &id, "tests");

    jit()
        .current_dir(temp.path())
        .args(["gate", "check-all", &id, "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("results"));
}
