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
use jit::test_cli::{seed_ambiguous_prefix, JitCli};

fn short(id: &str) -> &str {
    &id[0..8]
}

/// A single-id status line carries state, gates and unmet deps in one greppable
/// line with the documented field order.
#[test]
fn test_status_single_id_text_one_liner() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Solo task", "--description", "Body"])
        .unwrap();

    let out = temp.run(["issue", "status", &id]).unwrap().into_bytes();
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
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Gated task", "--description", "Body"])
        .unwrap();
    temp.run([
        "gate",
        "define",
        "tests",
        "--title",
        "Tests",
        "--description",
        "Run tests",
    ])
    .unwrap();
    temp.run(["issue", "update", &id, "--add-gate", "tests"])
        .unwrap();

    let out = temp.run(["issue", "status", &id]).unwrap().into_bytes();
    let line = String::from_utf8_lossy(&out).trim().to_string();
    assert!(
        line.contains("gates: tests=pending"),
        "gate renders as key=status: {line:?}"
    );
}

/// Single-id `--json` is a bare compact object with the documented keys.
#[test]
fn test_status_single_id_json_small_object() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Solo task", "--description", "Body"])
        .unwrap();
    temp.run([
        "gate",
        "define",
        "tests",
        "--title",
        "Tests",
        "--description",
        "Run tests",
    ])
    .unwrap();
    temp.run(["issue", "update", &id, "--add-gate", "tests"])
        .unwrap();

    let out = temp
        .run(["issue", "status", &id, "--json"])
        .unwrap()
        .into_bytes();
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
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "No gates", "--description", "Body"])
        .unwrap();

    let out = temp
        .run(["issue", "status", &id, "--json"])
        .unwrap()
        .into_bytes();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(json["gates"].is_array());
    assert_eq!(json["gates"].as_array().unwrap().len(), 0);
}

/// Multi-id `--json` wraps one object per id in the `{count, issues}` envelope,
/// in argument order.
#[test]
fn test_status_multi_id_json_envelope() {
    let temp = JitCli::initialised().unwrap();
    let a = temp
        .create_issue(["--title", "First", "--description", "Body"])
        .unwrap();
    let b = temp
        .create_issue(["--title", "Second", "--description", "Body"])
        .unwrap();

    let out = temp
        .run(["issue", "status", &a, &b, "--json"])
        .unwrap()
        .into_bytes();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();

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
    let temp = JitCli::initialised().unwrap();
    let a = temp
        .create_issue(["--title", "First", "--description", "Body"])
        .unwrap();
    let b = temp
        .create_issue(["--title", "Second", "--description", "Body"])
        .unwrap();

    let out = temp.run(["issue", "status", &a, &b]).unwrap().into_bytes();
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
    let temp = JitCli::initialised().unwrap();
    let done_dep = temp
        .create_issue(["--title", "Done dep", "--description", "Body"])
        .unwrap();
    let open_dep = temp
        .create_issue(["--title", "Open dep", "--description", "Body"])
        .unwrap();
    let rejected_dep = temp
        .create_issue(["--title", "Rejected dep", "--description", "Body"])
        .unwrap();
    let main = temp
        .create_issue(["--title", "Downstream", "--description", "Body"])
        .unwrap();

    temp.run(["dep", "add", &main, &done_dep]).unwrap();
    temp.run(["dep", "add", &main, &open_dep]).unwrap();
    temp.run(["dep", "add", &main, &rejected_dep]).unwrap();

    // Terminal deps: Done and Rejected both count as *met* (readiness unblocks
    // on either), so only the open dep remains unmet.
    temp.run(["issue", "update", &done_dep, "--state", "done"])
        .unwrap();
    temp.run(["issue", "reject", &rejected_dep]).unwrap();

    let out = temp
        .run(["issue", "status", &main, "--json"])
        .unwrap()
        .into_bytes();
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
    let text_out = temp.run(["issue", "status", &main]).unwrap().into_bytes();
    let line = String::from_utf8_lossy(&text_out).trim().to_string();
    assert!(
        line.contains(&format!("unmet: {}", short(&open_dep))),
        "text unmet section lists the open dep: {line:?}"
    );
}

/// When every dependency is terminal, the unmet list is empty.
#[test]
fn test_status_all_deps_done_empty_unmet() {
    let temp = JitCli::initialised().unwrap();
    let dep = temp
        .create_issue(["--title", "Prereq", "--description", "Body"])
        .unwrap();
    let main = temp
        .create_issue(["--title", "Downstream", "--description", "Body"])
        .unwrap();
    temp.run(["dep", "add", &main, &dep]).unwrap();
    temp.run(["issue", "update", &dep, "--state", "done"])
        .unwrap();

    let out = temp
        .run(["issue", "status", &main, "--json"])
        .unwrap()
        .into_bytes();
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
    let temp = JitCli::initialised().unwrap();
    let done_dep = temp
        .create_issue(["--title", "Done dep", "--description", "Body"])
        .unwrap();
    let open_dep = temp
        .create_issue(["--title", "Open dep", "--description", "Body"])
        .unwrap();
    let main = temp
        .create_issue(["--title", "Downstream", "--description", "Body"])
        .unwrap();
    temp.run(["dep", "add", &main, &done_dep]).unwrap();
    temp.run(["dep", "add", &main, &open_dep]).unwrap();
    temp.run(["issue", "update", &done_dep, "--state", "done"])
        .unwrap();

    let out = temp
        .run(["issue", "show", &main, "--json"])
        .unwrap()
        .into_bytes();
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
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Lonely", "--description", "Body"])
        .unwrap();
    let out = temp
        .run(["issue", "show", &id, "--json"])
        .unwrap()
        .into_bytes();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(json["unmet_dependencies"].is_array());
    assert_eq!(json["unmet_dependencies"].as_array().unwrap().len(), 0);
}

/// `issue status` does not accept the single-issue projection flags; passing one
/// is a clap usage error. Pins the "separate subcommand, no
/// interaction with --field/--fields" decision.
#[test]
fn test_status_rejects_field_projection_flags() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "X", "--description", "Body"])
        .unwrap();

    for flag in [["--field", "state"], ["--fields", "state"]] {
        let assert = temp
            .command(["issue", "status", &id])
            .args(flag)
            .assert()
            .failure();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
        assert!(stderr.contains("unexpected argument"), "{stderr}");
    }
}

/// An unknown id is reported as a failure (does not silently print an empty row).
#[test]
fn test_status_unknown_id_fails() {
    let temp = JitCli::initialised().unwrap();
    temp.command(["issue", "status", "deadbeef"])
        .assert()
        .failure();
}

/// Under `--json`, a non-matching (but long-enough) id yields the
/// `ISSUE_NOT_FOUND` envelope, matching `issue show`.
#[test]
fn test_status_unknown_id_json_error_envelope() {
    let temp = JitCli::initialised().unwrap();
    let assert = temp
        .command(["issue", "status", "deadbeef", "--json"])
        .assert()
        .failure();
    let out = assert.get_output().stdout.clone();
    let json: serde_json::Value = serde_json::from_slice(&out)
        .unwrap_or_else(|_| panic!("--json failure must be a JSON envelope: {out:?}"));
    assert_eq!(json["error"]["code"].as_str(), Some("ISSUE_NOT_FOUND"));
}

/// A too-short prefix (< 4 chars) refines to `INVALID_ID_PREFIX`, distinct from
/// the not-found code — proving classification runs on the
/// status path exactly as on `issue show`.
#[test]
fn test_status_short_prefix_json_error_envelope() {
    let temp = JitCli::initialised().unwrap();
    let assert = temp
        .command(["issue", "status", "ab", "--json"])
        .assert()
        .failure();
    let out = assert.get_output().stdout.clone();
    let json: serde_json::Value = serde_json::from_slice(&out)
        .unwrap_or_else(|_| panic!("--json failure must be a JSON envelope: {out:?}"));
    assert_eq!(json["error"]["code"].as_str(), Some("INVALID_ID_PREFIX"));
}

/// `issue show` refuses a too-short prefix as an invalid prefix: the human
/// message names the minimum length and `--json` carries `INVALID_ID_PREFIX`.
#[test]
fn test_show_short_prefix_is_invalid_prefix_human_and_json() {
    let temp = JitCli::initialised().unwrap();

    let human = temp.command(["issue", "show", "ab"]).output().unwrap();
    assert!(!human.status.success());
    assert!(String::from_utf8_lossy(&human.stderr).contains("at least 4 characters"));

    let machine = temp
        .command(["issue", "show", "ab", "--json"])
        .output()
        .unwrap();
    assert!(!machine.status.success());
    let json: serde_json::Value = serde_json::from_slice(&machine.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ID_PREFIX");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("at least 4 characters"));
}

/// `issue show` refuses an ambiguous prefix: the human message says "Ambiguous
/// ID" and `--json` carries `AMBIGUOUS_ID`.
#[test]
fn test_show_ambiguous_prefix_is_ambiguous_id_human_and_json() {
    let temp = JitCli::initialised().unwrap();
    let (prefix, _) = seed_ambiguous_prefix(&temp).unwrap();

    let human = temp.command(["issue", "show", &prefix]).output().unwrap();
    assert!(!human.status.success());
    let stderr = String::from_utf8_lossy(&human.stderr);
    assert!(stderr.contains("Ambiguous ID"), "stderr was: {stderr}");

    let machine = temp
        .command(["issue", "show", &prefix, "--json"])
        .output()
        .unwrap();
    assert!(!machine.status.success());
    let json: serde_json::Value = serde_json::from_slice(&machine.stdout).unwrap();
    assert_eq!(json["error"]["code"], "AMBIGUOUS_ID");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Ambiguous ID"));
}

/// Multi-id `--json` where one id is bad fails fast on the whole command with a
/// JSON error envelope (no partial `{count, issues}` success), matching
/// `issue show`'s multi-id semantics.
#[test]
fn test_status_multi_id_one_bad_fails_fast_json() {
    let temp = JitCli::initialised().unwrap();
    let good = temp
        .create_issue(["--title", "Good", "--description", "Body"])
        .unwrap();

    let assert = temp
        .command(["issue", "status", &good, "deadbeef", "--json"])
        .assert()
        .failure();
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
    let temp = JitCli::initialised().unwrap();
    let good = temp
        .create_issue(["--title", "Good", "--description", "Body"])
        .unwrap();
    temp.command(["issue", "status", &good, "deadbeef"])
        .assert()
        .failure();
}
