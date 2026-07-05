//! CLI-level tests for the `jit dep add` write-time redundancy guard (jit:7a50e021).
//!
//! Verifies the actual exit codes: a redundant add is rejected with a nonzero
//! exit by default, and `--reduce` makes it succeed while leaving `jit validate`
//! clean.

use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    Command::new(jit_binary())
        .args(["init"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    temp
}

fn create_issue(dir: &std::path::Path, title: &str) -> String {
    let out = Command::new(jit_binary())
        .args(["issue", "create", "-t", title])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

fn dep_add(dir: &std::path::Path, from: &str, to: &str, extra: &[&str]) -> std::process::Output {
    let mut args = vec!["dep", "add", from, to];
    args.extend_from_slice(extra);
    Command::new(jit_binary())
        .args(&args)
        .current_dir(dir)
        .output()
        .unwrap()
}

/// REQ-01: a redundant `jit dep add` exits nonzero and names the offending pair.
#[test]
fn test_cli_dep_add_redundant_edge_rejected_nonzero() {
    let temp = setup_test_repo();
    let dir = temp.path();
    let x = create_issue(dir, "X");
    let y = create_issue(dir, "Y");
    let z = create_issue(dir, "Z");

    assert!(dep_add(dir, &x, &y, &[]).status.success());
    assert!(dep_add(dir, &x, &z, &[]).status.success());

    // Adding Y → Z makes the pre-existing X → Z redundant: rejected, nonzero.
    let out = dep_add(dir, &y, &z, &[]);
    assert!(!out.status.success(), "redundant add must exit nonzero");
    assert_eq!(
        out.status.code(),
        Some(4),
        "redundant add maps to ValidationFailed (exit 4)"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&x[..8]) && stderr.contains(&z[..8]),
        "error must name the shadowed edge X→Z, got: {stderr}"
    );

    // The write did not happen: validate is still clean.
    let val = Command::new(jit_binary())
        .args(["validate"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        val.status.success(),
        "no partial write should have occurred"
    );
}

/// REQ-01 (JSON): the rejection surfaces as a VALIDATION_FAILED JSON error, exit 4.
#[test]
fn test_cli_dep_add_redundant_edge_json_error() {
    let temp = setup_test_repo();
    let dir = temp.path();
    let x = create_issue(dir, "X");
    let y = create_issue(dir, "Y");
    let z = create_issue(dir, "Z");

    assert!(dep_add(dir, &x, &y, &[]).status.success());
    assert!(dep_add(dir, &x, &z, &[]).status.success());

    let out = dep_add(dir, &y, &z, &["--json"]);
    assert_eq!(out.status.code(), Some(4));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("VALIDATION_FAILED"),
        "json error must carry VALIDATION_FAILED, got: {stdout}"
    );
}

/// REQ-02 / REQ-03: `--reduce` succeeds (exit 0), drops the shadowed edge, and
/// leaves `jit validate` clean.
#[test]
fn test_cli_dep_add_reduce_succeeds_and_validate_clean() {
    let temp = setup_test_repo();
    let dir = temp.path();
    let x = create_issue(dir, "X");
    let y = create_issue(dir, "Y");
    let z = create_issue(dir, "Z");

    assert!(dep_add(dir, &x, &y, &[]).status.success());
    assert!(dep_add(dir, &x, &z, &[]).status.success());

    let out = dep_add(dir, &y, &z, &["--reduce"]);
    assert!(out.status.success(), "--reduce add must succeed");

    let val = Command::new(jit_binary())
        .args(["validate"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        val.status.success(),
        "validate must be clean after --reduce add: {}",
        String::from_utf8_lossy(&val.stderr)
    );
}

/// jit:c8518f2a REQ-01 (variadic): a redundant edge among several targets
/// fails the whole `jit dep add` atomically — the sibling edge that would
/// have succeeded on its own must NOT be persisted either. This is the
/// combination case: A -> C is only redundant once A -> B is ALSO in this
/// same call (neither edge is redundant on its own against the pre-existing
/// graph), so the would-be-final-graph check must consider both edges
/// together, not one at a time.
#[test]
fn test_cli_dep_add_variadic_redundant_among_valid_exits_nonzero() {
    let temp = setup_test_repo();
    let dir = temp.path();
    let a = create_issue(dir, "A");
    let b = create_issue(dir, "B");
    let c = create_issue(dir, "C");

    // Pre-existing B -> C, so a later A -> C is redundant once A -> B is added.
    assert!(dep_add(dir, &b, &c, &[]).status.success());

    // Variadic: A -> B (valid alone) and A -> C (redundant only in
    // combination with A -> B).
    let out = Command::new(jit_binary())
        .args(["dep", "add", &a, &b, &c])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "a redundant edge among the targets must fail the command; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.status.code(),
        Some(4),
        "redundant-edge rejection maps to ValidationFailed (exit 4)"
    );

    // All-or-nothing: A -> B must NOT have been persisted either, even though
    // it would have succeeded on its own.
    let show = Command::new(jit_binary())
        .args(["issue", "show", &a, "--json"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(show.status.success());
    let json: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    let deps = json["dependencies"].as_array().unwrap();
    assert!(
        deps.is_empty(),
        "no edge should have been added when a sibling edge is rejected: {:?}",
        deps
    );

    // Nothing was written, so the pre-existing B -> C edge is exactly as it
    // was: still direct, not shadowed by any A edge.
    let show_b = Command::new(jit_binary())
        .args(["issue", "show", &b, "--json"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(show_b.status.success());
    let json_b: serde_json::Value = serde_json::from_slice(&show_b.stdout).unwrap();
    let deps_b: Vec<&str> = json_b["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();
    assert_eq!(deps_b, vec![c.as_str()]);
}

/// jit:c8518f2a: a batch mixing a usage/resolution failure (too-short id
/// prefix) with a graph-validation failure (a self-redundant edge) exits with
/// the usage error's code (2), not the validation error's (4) — resolution
/// runs before graph validation, so it wins the batch's dominant
/// classification. Both edges are still named in the error output.
#[test]
fn test_cli_dep_add_mixed_error_classes_prefix_wins_over_redundant() {
    let temp = setup_test_repo();
    let dir = temp.path();
    let a = create_issue(dir, "A");
    let b = create_issue(dir, "B");
    let c = create_issue(dir, "C");

    // A -> B -> C exist, so a direct A -> C is self-redundant.
    assert!(dep_add(dir, &a, &b, &[]).status.success());
    assert!(dep_add(dir, &b, &c, &[]).status.success());

    // Text mode: exit 2 (usage/prefix), not 4 (validation).
    let out = Command::new(jit_binary())
        .args(["dep", "add", &a, &c, "ab"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(
        out.status.code(),
        Some(2),
        "a bad id prefix must dominate a sibling redundant-edge failure: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("at least 4 characters"),
        "must name the bad prefix: {stderr}"
    );
    assert!(
        stderr.contains(&c[..8]),
        "must ALSO name the redundant edge, not only the dominant failure: {stderr}"
    );

    // JSON mode: same exit code, INVALID_ID_PREFIX at top level, both edges
    // present in details.rejected.
    let out_json = Command::new(jit_binary())
        .args(["dep", "add", &a, &c, "ab", "--json"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert_eq!(out_json.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&out_json.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ID_PREFIX");

    let rejected = json["error"]["details"]["rejected"].as_array().unwrap();
    assert_eq!(rejected.len(), 2, "both edges must be named: {rejected:?}");
    let codes: Vec<&str> = rejected
        .iter()
        .map(|r| r["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"INVALID_ID_PREFIX"));
    assert!(codes.contains(&"VALIDATION_FAILED"));

    // Nothing was written.
    let show = Command::new(jit_binary())
        .args(["issue", "show", &a, "--json"])
        .current_dir(dir)
        .output()
        .unwrap();
    let json_a: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    let deps: Vec<&str> = json_a["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();
    assert_eq!(deps, vec![b.as_str()]);
}
