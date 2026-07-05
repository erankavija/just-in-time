//! End-to-end coverage for the compact issue status projection (jit:cc42a69b):
//! - `jit issue status <id>` prints one greppable text line (state + gates +
//!   unmet deps)
//! - `jit issue status <id> --json` prints one small object
//! - multiple ids emit one row/object per issue in argument order (envelope
//!   under `--json`)
//! - the unmet-dependency set follows readiness semantics (terminal deps are
//!   met; a Rejected dep is NOT unmet)
//! - the full `issue show --json` additionally exposes `unmet_dependencies`
//! - projection flags (`--field`/`--fields`) are not accepted on `issue status`

use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn jit_ok(temp: &TempDir, args: &[&str]) -> Vec<u8> {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone()
}

fn create_issue(temp: &TempDir, title: &str) -> String {
    let stdout = jit_ok(
        temp,
        &["issue", "create", "--title", title, "--description", "Body"],
    );
    let stdout = String::from_utf8_lossy(&stdout);
    stdout
        .lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

fn setup() -> TempDir {
    let temp = TempDir::new().unwrap();
    jit_ok(&temp, &["init"]);
    temp
}

fn short(id: &str) -> &str {
    &id[0..8]
}

/// A single-id status line carries state, gates and unmet deps in one greppable
/// line with the documented field order.
#[test]
fn test_status_single_id_text_one_liner() {
    let temp = setup();
    let id = create_issue(&temp, "Solo task");

    let out = jit_ok(&temp, &["issue", "status", &id]);
    let text = String::from_utf8_lossy(&out);
    let line = text.trim();

    assert!(
        line.starts_with(short(&id)),
        "line starts with short id: {line:?}"
    );
    assert!(line.contains("[ready]"), "state in brackets: {line:?}");
    // No gates and no deps -> both sections read `none`.
    assert!(
        line.contains("gates: none"),
        "empty gates render none: {line:?}"
    );
    assert!(
        line.contains("unmet: none"),
        "empty unmet render none: {line:?}"
    );
    assert!(line.ends_with("title: Solo task"), "title tail: {line:?}");
    // Exactly one line of output.
    assert_eq!(text.lines().count(), 1, "one line per issue: {text:?}");
}

/// The gate section lists each required gate as `key=status` in snake_case.
#[test]
fn test_status_text_lists_gate_key_status() {
    let temp = setup();
    let id = create_issue(&temp, "Gated task");
    jit_ok(
        &temp,
        &[
            "gate",
            "define",
            "tests",
            "--title",
            "Tests",
            "--description",
            "Run tests",
        ],
    );
    jit_ok(&temp, &["issue", "update", &id, "--add-gate", "tests"]);

    let out = jit_ok(&temp, &["issue", "status", &id]);
    let line = String::from_utf8_lossy(&out).trim().to_string();
    assert!(
        line.contains("gates: tests=pending"),
        "gate renders as key=status: {line:?}"
    );
}

/// Single-id `--json` is a bare compact object with the documented keys.
#[test]
fn test_status_single_id_json_small_object() {
    let temp = setup();
    let id = create_issue(&temp, "Solo task");
    jit_ok(
        &temp,
        &[
            "gate",
            "define",
            "tests",
            "--title",
            "Tests",
            "--description",
            "Run tests",
        ],
    );
    jit_ok(&temp, &["issue", "update", &id, "--add-gate", "tests"]);

    let out = jit_ok(&temp, &["issue", "status", &id, "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();

    assert!(
        json.is_object(),
        "single id --json is a bare object: {json}"
    );
    assert_eq!(json["short_id"].as_str(), Some(short(&id)));
    assert_eq!(json["state"].as_str(), Some("ready"));
    assert_eq!(json["title"].as_str(), Some("Solo task"));
    // gates: [{key,status}]
    let gates = json["gates"].as_array().expect("gates array");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0]["key"].as_str(), Some("tests"));
    assert_eq!(gates[0]["status"].as_str(), Some("pending"));
    // unmet_dependencies: [short_ids...]
    assert!(
        json["unmet_dependencies"].as_array().unwrap().is_empty(),
        "no deps -> empty unmet array: {json}"
    );
}

/// An issue with no gates emits an empty `gates` array (not null/absent).
#[test]
fn test_status_json_no_gates_is_empty_array() {
    let temp = setup();
    let id = create_issue(&temp, "No gates");

    let out = jit_ok(&temp, &["issue", "status", &id, "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(json["gates"].is_array());
    assert_eq!(json["gates"].as_array().unwrap().len(), 0);
}

/// Multi-id `--json` wraps one object per id in the `{count, issues}` envelope,
/// in argument order.
#[test]
fn test_status_multi_id_json_envelope() {
    let temp = setup();
    let a = create_issue(&temp, "First");
    let b = create_issue(&temp, "Second");

    let out = jit_ok(&temp, &["issue", "status", &a, &b, "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();

    assert_eq!(json["count"].as_u64(), Some(2), "envelope carries count");
    let arr = json["issues"]
        .as_array()
        .expect("multi-id --json wraps in the list envelope");
    assert_eq!(arr.len(), 2);
    // Argument order preserved.
    assert_eq!(arr[0]["short_id"].as_str(), Some(short(&a)));
    assert_eq!(arr[0]["title"].as_str(), Some("First"));
    assert_eq!(arr[1]["short_id"].as_str(), Some(short(&b)));
    assert_eq!(arr[1]["title"].as_str(), Some("Second"));
}

/// Multi-id text prints one line per id in argument order.
#[test]
fn test_status_multi_id_text_one_line_each() {
    let temp = setup();
    let a = create_issue(&temp, "First");
    let b = create_issue(&temp, "Second");

    let out = jit_ok(&temp, &["issue", "status", &a, &b]);
    let text = String::from_utf8_lossy(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "one line per id: {text:?}");
    assert!(lines[0].starts_with(short(&a)));
    assert!(lines[1].starts_with(short(&b)));
}

/// A backlog issue with mixed dependency states lists only the not-yet-terminal
/// deps as unmet, and a fully-satisfied issue lists none.
#[test]
fn test_status_unmet_follows_readiness_semantics() {
    let temp = setup();
    let done_dep = create_issue(&temp, "Done dep");
    let open_dep = create_issue(&temp, "Open dep");
    let rejected_dep = create_issue(&temp, "Rejected dep");
    let main = create_issue(&temp, "Downstream");

    jit_ok(&temp, &["dep", "add", &main, &done_dep]);
    jit_ok(&temp, &["dep", "add", &main, &open_dep]);
    jit_ok(&temp, &["dep", "add", &main, &rejected_dep]);

    // Terminal deps: Done and Rejected both count as *met* (readiness unblocks
    // on either), so only the open dep remains unmet.
    jit_ok(&temp, &["issue", "update", &done_dep, "--state", "done"]);
    jit_ok(
        &temp,
        &["issue", "update", &rejected_dep, "--state", "rejected"],
    );

    let out = jit_ok(&temp, &["issue", "status", &main, "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let unmet: Vec<String> = json["unmet_dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        unmet,
        vec![short(&open_dep).to_string()],
        "only the non-terminal dep is unmet (Done and Rejected are met): {json}"
    );

    // Text form lists the same single short id.
    let text_out = jit_ok(&temp, &["issue", "status", &main]);
    let line = String::from_utf8_lossy(&text_out).trim().to_string();
    assert!(
        line.contains(&format!("unmet: {}", short(&open_dep))),
        "text unmet section lists the open dep: {line:?}"
    );
}

/// When every dependency is terminal, the unmet list is empty.
#[test]
fn test_status_all_deps_done_empty_unmet() {
    let temp = setup();
    let dep = create_issue(&temp, "Prereq");
    let main = create_issue(&temp, "Downstream");
    jit_ok(&temp, &["dep", "add", &main, &dep]);
    jit_ok(&temp, &["issue", "update", &dep, "--state", "done"]);

    let out = jit_ok(&temp, &["issue", "status", &main, "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(
        json["unmet_dependencies"].as_array().unwrap().is_empty(),
        "all deps terminal -> empty unmet: {json}"
    );
}

/// REQ-02: the FULL `issue show --json` exposes `unmet_dependencies` as objects
/// (subset shape of the dependency entries), following readiness semantics.
#[test]
fn test_show_json_exposes_unmet_dependencies_objects() {
    let temp = setup();
    let done_dep = create_issue(&temp, "Done dep");
    let open_dep = create_issue(&temp, "Open dep");
    let main = create_issue(&temp, "Downstream");
    jit_ok(&temp, &["dep", "add", &main, &done_dep]);
    jit_ok(&temp, &["dep", "add", &main, &open_dep]);
    jit_ok(&temp, &["issue", "update", &done_dep, "--state", "done"]);

    let out = jit_ok(&temp, &["issue", "show", &main, "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();

    let unmet = json["unmet_dependencies"]
        .as_array()
        .expect("unmet_dependencies present on full show --json");
    assert_eq!(unmet.len(), 1, "only the open dep is unmet: {json}");
    let entry = &unmet[0];
    assert_eq!(entry["id"].as_str(), Some(open_dep.as_str()));
    assert_eq!(entry["short_id"].as_str(), Some(short(&open_dep)));
    assert_eq!(entry["title"].as_str(), Some("Open dep"));
    assert_eq!(entry["state"].as_str(), Some("ready"));
}

/// With no dependencies, full show still carries an (empty) `unmet_dependencies`
/// array — never null or absent.
#[test]
fn test_show_json_unmet_dependencies_empty_array_when_none() {
    let temp = setup();
    let id = create_issue(&temp, "Lonely");
    let out = jit_ok(&temp, &["issue", "show", &id, "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(json["unmet_dependencies"].is_array());
    assert_eq!(json["unmet_dependencies"].as_array().unwrap().len(), 0);
}

/// `issue status` does not accept the single-issue projection flags; passing one
/// is a clap usage error (exit 2). Pins the "separate subcommand, no
/// interaction with --field/--fields" decision.
#[test]
fn test_status_rejects_field_projection_flags() {
    let temp = setup();
    let id = create_issue(&temp, "X");

    for flag in [["--field", "state"], ["--fields", "state,title"]] {
        Command::new(assert_cmd::cargo::cargo_bin!("jit"))
            .current_dir(temp.path())
            .args(["issue", "status", &id])
            .args(flag)
            .assert()
            .failure()
            .code(2);
    }
}

/// An unknown id is reported as a failure (does not silently print an empty row).
#[test]
fn test_status_unknown_id_fails() {
    let temp = setup();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "status", "deadbeef"])
        .assert()
        .failure();
}

/// Under `--json`, a non-matching (but long-enough) id yields the JSON error
/// envelope with `ISSUE_NOT_FOUND` and exit code 3 — the same contract as
/// `issue show`, routed through `handle_json_error!`. Pins that id failures do
/// not escape to bare human stderr when `--json` is requested.
#[test]
fn test_status_unknown_id_json_error_envelope() {
    let temp = setup();
    let assert = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "status", "deadbeef", "--json"])
        .assert()
        .failure()
        .code(3);
    let out = assert.get_output().stdout.clone();
    let json: serde_json::Value = serde_json::from_slice(&out)
        .unwrap_or_else(|_| panic!("--json failure must be a JSON envelope: {out:?}"));
    assert_eq!(json["error"]["code"].as_str(), Some("ISSUE_NOT_FOUND"));
}

/// A too-short prefix (< 4 chars) refines to `INVALID_ID_PREFIX` with exit code
/// 2, distinct from the not-found code — proving `refine_id_error` runs on the
/// status path exactly as on `issue show`.
#[test]
fn test_status_short_prefix_json_error_envelope() {
    let temp = setup();
    let assert = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "status", "ab", "--json"])
        .assert()
        .failure()
        .code(2);
    let out = assert.get_output().stdout.clone();
    let json: serde_json::Value = serde_json::from_slice(&out)
        .unwrap_or_else(|_| panic!("--json failure must be a JSON envelope: {out:?}"));
    assert_eq!(json["error"]["code"].as_str(), Some("INVALID_ID_PREFIX"));
}

/// Multi-id `--json` where one id is bad fails fast on the whole command with a
/// JSON error envelope (no partial `{count, issues}` success), matching
/// `issue show`'s multi-id semantics.
#[test]
fn test_status_multi_id_one_bad_fails_fast_json() {
    let temp = setup();
    let good = create_issue(&temp, "Good");

    let assert = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "status", &good, "deadbeef", "--json"])
        .assert()
        .failure()
        .code(3);
    let out = assert.get_output().stdout.clone();
    let text = String::from_utf8_lossy(&out);
    let json: serde_json::Value = serde_json::from_slice(&out)
        .unwrap_or_else(|_| panic!("--json failure must be a JSON envelope: {text:?}"));
    assert_eq!(json["error"]["code"].as_str(), Some("ISSUE_NOT_FOUND"));
    // Fail-fast: no partial success envelope for the good id leaked out.
    assert!(
        !text.contains("\"count\""),
        "must not emit a partial list envelope: {text:?}"
    );
}

/// Non-`--json` multi-id with one bad id also fails the whole command (human
/// stderr path), so text and JSON modes agree on fail-fast.
#[test]
fn test_status_multi_id_one_bad_fails_fast_text() {
    let temp = setup();
    let good = create_issue(&temp, "Good");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "status", &good, "deadbeef"])
        .assert()
        .failure();
}
