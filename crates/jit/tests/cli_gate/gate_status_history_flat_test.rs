//! REQ-08: `gate status` is the unified gate-run inspection surface.
//!
//! Beyond the latest-run view, `gate status` exposes:
//!   - History (`--all` / `--limit <N>`): prior runs newest-first, filterable
//!     by `--gate <key>` and `--status <passed|failed|...>`.
//!   - Flat report text (`--stdout` / `--stderr` / `--tail <N>`): the stored
//!     report text printed verbatim with no wrapping or decoration.
//!
//! Every view supports `--json`, and argument errors are machine-readable.

use super::{create_issue_requiring, define_auto_gate, run_gate_forced};
use assert_cmd::prelude::*;
use jit::test_cli::JitCli;
use predicates::prelude::*;

// ---------------------------------------------------------------------------
// Latest-by-default: unchanged behaviour (regression guard)
// ---------------------------------------------------------------------------

#[test]
fn test_status_latest_by_default_unchanged() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "tests", "echo hello; exit 0");
    let id = create_issue_requiring(&temp, &["tests"]);
    run_gate_forced(&temp, &id, "tests");

    temp.command(["gate", "status", &id, "tests"])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed"));
}

#[test]
fn test_gate_status_reused_verdict_names_source_issue_and_run() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "tests", "exit 0");
    let id = create_issue_requiring(&temp, &["tests"]);
    run_gate_forced(&temp, &id, "tests");
    let summary = temp.run_json(["gate", "status", &id, "tests"]).unwrap();
    let run_id = serde_json::from_value(summary["run_id"].clone()).unwrap();
    let path = temp.root().join(".jit").join(
        jit::repository_state::VirtualPath::gate_run_record(&id.parse().unwrap(), &run_id)
            .relative()
            .as_path(),
    );
    let mut run: jit::domain::GateRunResult =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let source_issue = jit::test_utils::issue_id(160);
    let source_run = jit::test_utils::gate_run_id(1);
    run.origin = jit::domain::GateVerdictOrigin::Reused {
        source_issue: source_issue.clone(),
        source_run: source_run.clone(),
    };
    std::fs::write(path, serde_json::to_vec(&run).unwrap()).unwrap();

    temp.command(["gate", "status", &id, "tests"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains(source_issue.to_string())
                .and(predicate::str::contains(source_run.to_string())),
        );
}

#[test]
fn test_status_latest_run_shows_manual_attestation_with_attestor_and_time() {
    let temp = JitCli::initialised().unwrap();
    temp.command([
        "gate",
        "define",
        "review",
        "--title",
        "review",
        "--description",
        "Review",
        "--mode",
        "manual",
    ])
    .assert()
    .success();
    let id = create_issue_requiring(&temp, &["review"]);
    temp.command(["gate", "evaluate", &id, "review", "--by", "human:me"])
        .assert()
        .success();

    let out = temp
        .command(["gate", "status", &id, "review", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let attested_at = json["started_at"]
        .as_str()
        .expect("the attestation is the gate's latest run");

    temp.command(["gate", "status", &id, "review"])
        .assert()
        .success()
        .stdout(predicate::str::contains("human:me").and(predicate::str::contains(attested_at)));
}

// ---------------------------------------------------------------------------
// History: --all lists prior runs newest-first
// ---------------------------------------------------------------------------

#[test]
fn test_status_history_lists_prior_runs() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "tests", "echo run; exit 0");
    let id = create_issue_requiring(&temp, &["tests"]);
    run_gate_forced(&temp, &id, "tests");
    run_gate_forced(&temp, &id, "tests");
    run_gate_forced(&temp, &id, "tests");

    let out = temp
        .command(["gate", "status", &id, "--all", "--json"])
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
fn test_status_limit_caps_history() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "tests", "echo run; exit 0");
    let id = create_issue_requiring(&temp, &["tests"]);
    run_gate_forced(&temp, &id, "tests");
    run_gate_forced(&temp, &id, "tests");
    run_gate_forced(&temp, &id, "tests");

    let out = temp
        .command(["gate", "status", &id, "--limit", "2", "--json"])
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
fn test_status_history_gate_filter() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "tests", "echo t; exit 0");
    define_auto_gate(&temp, "clippy", "echo c; exit 0");
    let id = create_issue_requiring(&temp, &["tests", "clippy"]);
    run_gate_forced(&temp, &id, "tests");
    run_gate_forced(&temp, &id, "clippy");

    let out = temp
        .command(["gate", "status", &id, "--all", "--gate", "tests", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "only the tests gate run is listed");
    assert_eq!(results[0]["key"], "tests");
}

#[test]
fn test_status_history_status_filter() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "passing", "echo ok; exit 0");
    define_auto_gate(&temp, "failing", "echo boom; exit 1");
    let id = create_issue_requiring(&temp, &["passing", "failing"]);
    run_gate_forced(&temp, &id, "passing");
    run_gate_forced(&temp, &id, "failing");

    let out = temp
        .command([
            "gate", "status", &id, "--all", "--status", "failed", "--json",
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
fn test_status_history_invalid_status_json_error() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "tests", "echo ok; exit 0");
    let id = create_issue_requiring(&temp, &["tests"]);
    run_gate_forced(&temp, &id, "tests");

    for status in ["bogus", "skipped"] {
        let out = temp
            .command(["gate", "status", &id, "--all", "--status", status, "--json"])
            .assert()
            .failure()
            .get_output()
            .stdout
            .clone();
        let json: serde_json::Value =
            serde_json::from_slice(&out).expect("invalid --status must emit machine-readable JSON");
        assert_eq!(json["error"]["code"], "INVALID_ARGUMENT", "{status}");
    }
}

// ---------------------------------------------------------------------------
// Flat report text: --stdout / --stderr / --tail
// ---------------------------------------------------------------------------

#[test]
fn test_status_stdout_flat_verbatim() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "tests", "echo FLAT_MARKER_LINE; exit 0");
    let id = create_issue_requiring(&temp, &["tests"]);
    run_gate_forced(&temp, &id, "tests");

    let out = temp
        .command(["gate", "status", &id, "tests", "--stdout"])
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
fn test_status_stdout_flat_is_byte_exact_no_injected_newline() {
    // The flat view must emit the stored report text byte-for-byte. A checker
    // whose stdout has NO trailing newline must come back with NO trailing
    // newline — `print!`, not `println!` (which would append one).
    let temp = JitCli::initialised().unwrap();
    // `printf` (no `\n`) stores stdout with no trailing newline.
    define_auto_gate(&temp, "tests", "printf NO_TRAILING_NL; exit 0");
    let id = create_issue_requiring(&temp, &["tests"]);
    run_gate_forced(&temp, &id, "tests");

    let out = temp
        .command(["gate", "status", &id, "tests", "--stdout"])
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
fn test_status_stderr_flat_verbatim() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "tests", "echo ERRLINE 1>&2; exit 1");
    let id = create_issue_requiring(&temp, &["tests"]);
    run_gate_forced(&temp, &id, "tests");

    let out = temp
        .command(["gate", "status", &id, "tests", "--stderr"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("ERRLINE"), "stderr text printed verbatim");
}

#[test]
fn test_status_tail_limits_lines() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "tests", "printf 'a\\nb\\nc\\nd\\ne\\n'; exit 0");
    let id = create_issue_requiring(&temp, &["tests"]);
    run_gate_forced(&temp, &id, "tests");

    let out = temp
        .command(["gate", "status", &id, "tests", "--stdout", "--tail", "2"])
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
fn test_status_stdout_flat_json() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "tests", "echo JSONFLAT; exit 0");
    let id = create_issue_requiring(&temp, &["tests"]);
    run_gate_forced(&temp, &id, "tests");

    let out = temp
        .command(["gate", "status", &id, "tests", "--stdout", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["key"], "tests");
    assert!(
        json["stdout"].as_str().unwrap().contains("JSONFLAT"),
        "flat json carries stdout text"
    );
}

// ---------------------------------------------------------------------------
// Mutual exclusion: history vs flat -> machine-readable error
// ---------------------------------------------------------------------------

#[test]
fn test_status_history_and_flat_conflict_json_error() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "tests", "echo ok; exit 0");
    let id = create_issue_requiring(&temp, &["tests"]);
    run_gate_forced(&temp, &id, "tests");

    let out = temp
        .command([
            "gate", "status", &id, "tests", "--all", "--stdout", "--json",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn test_status_flag_requires_history_json_error() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "tests", "echo ok; exit 0");
    let id = create_issue_requiring(&temp, &["tests"]);
    run_gate_forced(&temp, &id, "tests");

    let out = temp
        .command([
            "gate", "status", &id, "tests", "--status", "passed", "--json",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}
