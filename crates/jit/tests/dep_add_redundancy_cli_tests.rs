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

/// REQ-01 (variadic): a redundant edge among several targets must fail the whole
/// `jit dep add` with a nonzero exit, even though a sibling edge was added — a
/// rejected redundant edge must not be masked by a partial success. The valid
/// edge stays persisted and the graph stays transitively reduced.
#[test]
fn test_cli_dep_add_variadic_redundant_among_valid_exits_nonzero() {
    let temp = setup_test_repo();
    let dir = temp.path();
    let a = create_issue(dir, "A");
    let b = create_issue(dir, "B");
    let c = create_issue(dir, "C");

    // Pre-existing B -> C, so a later A -> C is redundant once A -> B is added.
    assert!(dep_add(dir, &b, &c, &[]).status.success());

    // Variadic: A -> B (valid) and A -> C (redundant via A -> B -> C).
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

    // The valid A -> B edge persisted and the redundant A -> C was rejected, so
    // the repository still validates cleanly (no transitive-reduction violation).
    let validate = Command::new(jit_binary())
        .args(["validate"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        validate.status.success(),
        "validate must be clean after the partial add: {}",
        String::from_utf8_lossy(&validate.stderr)
    );
}
