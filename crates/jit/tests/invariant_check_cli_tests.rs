//! Integration tests for `jit invariant check` (enforcement-drift).
//!
//! Exercises the bidirectional declaration-consistency drift check end-to-end
//! through the real CLI binary: an invariant whose `enforced-by` names a missing
//! rule/gate (declared-but-unenforced) and a rule/gate no invariant claims
//! (enforced-but-undeclared). Asserts both drift directions are emitted, that the
//! command exits non-zero on drift and zero when consistent, and that `--json`
//! produces a valid machine-readable payload.

use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let output = Command::new(jit_binary())
        .arg("init")
        .current_dir(temp.path())
        .output()
        .expect("Failed to run jit init");
    assert!(output.status.success(), "jit init failed");
    temp
}

/// Write a `rules.toml` so the rule namespace is controlled (otherwise the
/// in-memory default ruleset's many rule names appear as undeclared).
fn write_rules(temp: &TempDir, toml: &str) {
    std::fs::write(temp.path().join(".jit/rules.toml"), toml).unwrap();
}

fn write_invariants(temp: &TempDir, toml: &str) {
    std::fs::write(temp.path().join(".jit/invariants.toml"), toml).unwrap();
}

#[test]
fn test_check_reports_both_drift_directions_and_exits_nonzero() {
    let temp = setup_test_repo();
    // One rule named `real-rule`; INV-01 binds to a MISSING `ghost-rule`.
    write_rules(
        &temp,
        "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
         assert = { require-section = { heading = \"Goal\" } }\n",
    );
    write_invariants(
        &temp,
        "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"ghost-rule\"\n",
    );

    let output = Command::new(jit_binary())
        .args(["invariant", "check", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Drift present -> non-zero exit (ValidationFailed == 4).
    assert_eq!(output.status.code(), Some(4), "expected exit 4 on drift");

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let findings = json["findings"].as_array().unwrap();
    // declared-but-unenforced: INV-01 -> ghost-rule.
    assert!(
        findings
            .iter()
            .any(|f| f["direction"] == "declared-but-unenforced"
                && f["invariant_id"] == "INV-01"
                && f["subject"] == "ghost-rule"),
        "missing declared-but-unenforced finding: {json}"
    );
    // enforced-but-undeclared: real-rule is claimed by no invariant.
    assert!(
        findings
            .iter()
            .any(|f| f["direction"] == "enforced-but-undeclared" && f["subject"] == "real-rule"),
        "missing enforced-but-undeclared finding: {json}"
    );
}

#[test]
fn test_check_exits_zero_when_consistent() {
    let temp = setup_test_repo();
    // Exactly one rule, claimed by exactly one invariant: no drift.
    write_rules(
        &temp,
        "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
         assert = { require-section = { heading = \"Goal\" } }\n",
    );
    write_invariants(
        &temp,
        "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"real-rule\"\n",
    );

    let output = Command::new(jit_binary())
        .args(["invariant", "check", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "expected exit 0 when consistent, got {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 0);
    assert!(json["findings"].as_array().unwrap().is_empty());
}

#[test]
fn test_check_human_output_names_both_directions() {
    let temp = setup_test_repo();
    write_rules(
        &temp,
        "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
         assert = { require-section = { heading = \"Goal\" } }\n",
    );
    write_invariants(
        &temp,
        "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"ghost-rule\"\n",
    );

    let output = Command::new(jit_binary())
        .args(["invariant", "check"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("declared-but-unenforced") && stdout.contains("ghost-rule"),
        "human output missing declared direction: {stdout}"
    );
    assert!(
        stdout.contains("enforced-but-undeclared") && stdout.contains("real-rule"),
        "human output missing enforced direction: {stdout}"
    );
}

#[test]
fn test_check_reports_unloadable_rule_source_not_a_parse_error() {
    // REQ-01 "unloadable" half: an invariant binds to `bad-rule` and `.jit/rules.toml`
    // is MALFORMED. `jit invariant check` must report a declared-but-unenforced
    // finding and exit non-zero — NOT print a raw TOML parse error.
    let temp = setup_test_repo();
    write_rules(
        &temp,
        "[[rules]]\nname = \"bad-rule\"\nseverity = \"error\"\n\
         assert = { this-is-not-a-valid-kind = { foo = 1 } }\n",
    );
    write_invariants(
        &temp,
        "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"bad-rule\"\n",
    );

    let output = Command::new(jit_binary())
        .args(["invariant", "check", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Exit non-zero (drift present), and stdout is VALID JSON (not a parse error).
    assert_eq!(output.status.code(), Some(4), "expected exit 4 on drift");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "expected valid JSON, not a raw parse error: {e}\nstdout: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    let findings = json["findings"].as_array().unwrap();
    assert!(
        findings
            .iter()
            .any(|f| f["direction"] == "declared-but-unenforced"
                && f["subject"] == "bad-rule"
                && f["unloadable"] == true),
        "missing unloadable declared-but-unenforced finding: {json}"
    );
}

#[test]
fn test_check_reports_unloadable_gate_registry() {
    // The gate registry is malformed and an invariant binds to a gate that would
    // live there -> declared-but-unenforced (unloadable), exit non-zero.
    let temp = setup_test_repo();
    // Loadable (empty) rules so only the gate source is unloadable.
    write_rules(&temp, "\n");
    std::fs::write(temp.path().join(".jit/gates.json"), "not valid json {").unwrap();
    write_invariants(
        &temp,
        "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"some-gate\"\n",
    );

    let output = Command::new(jit_binary())
        .args(["invariant", "check", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        json["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["direction"] == "declared-but-unenforced"
                && f["subject"] == "some-gate"
                && f["unloadable"] == true),
        "missing unloadable gate finding: {json}"
    );
}
