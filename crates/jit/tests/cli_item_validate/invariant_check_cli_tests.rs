//! Integration tests for `jit invariant check` (enforcement-drift).
//!
//! Exercises the declaration-consistency drift check end-to-end through the real
//! CLI binary. The sole drift direction is declared-but-unenforced: an invariant
//! whose `enforced-by` names a missing/unloadable rule or gate. An unclaimed
//! rule/gate is NOT drift (the enforced-but-undeclared direction was removed in
//! REQ-05). Asserts the command exits non-zero on a dangling binding, exits zero
//! when every binding resolves (even with unclaimed rules/gates present), and
//! that `--json` preserves successful reports and wraps drift reports in the
//! canonical machine-readable failure envelope.

use jit::test_cli::JitCli;
use serde_json::Value;

/// Write a `rules.toml` so the rule namespace is controlled (otherwise the
/// in-memory default ruleset's many rule names appear as undeclared).
fn write_rules(cli: &JitCli, toml: &str) {
    std::fs::write(cli.root().join(".jit/rules.toml"), toml).unwrap();
}

fn write_invariants(cli: &JitCli, toml: &str) {
    std::fs::write(cli.root().join(".jit/invariants.toml"), toml).unwrap();
}

#[test]
fn test_check_reports_declared_but_unenforced_and_exits_nonzero() {
    let cli = JitCli::initialised().unwrap();
    // One rule named `real-rule`; sample-invariant binds to a MISSING `ghost-rule`.
    write_rules(
        &cli,
        "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
         assert = { require-section = { heading = \"Goal\" } }\n",
    );
    write_invariants(
        &cli,
        "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"@/rule/ghost-rule\"\n",
    );

    let output = cli
        .command(["invariant", "check", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success(), "expected failure on drift");

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "VALIDATION_FAILED", "{json}");
    let findings = json["error"]["details"]["findings"].as_array().unwrap();
    // The dangling binding sample-invariant -> ghost-rule is the sole finding.
    assert_eq!(findings.len(), 1, "{json}");
    assert!(
        findings
            .iter()
            .any(|f| f["invariant_id"] == "sample-invariant" && f["subject"] == "@/rule/ghost-rule"),
        "missing declared-but-unenforced finding: {json}"
    );
    // The unclaimed `real-rule` is NOT reported (no enforced-but-undeclared).
    assert!(
        !findings.iter().any(|f| f["subject"] == "real-rule"),
        "unclaimed rule must not be reported: {json}"
    );
}

#[test]
fn test_check_exits_zero_with_unclaimed_rules_and_gates() {
    let cli = JitCli::initialised().unwrap();
    // A rule and a gate that NO invariant claims; the one invariant's binding
    // resolves. REQ-05: unclaimed rules/gates are not drift, so check exits 0.
    write_rules(
        &cli,
        "[[rules]]\nname = \"claimed-rule\"\nseverity = \"warn\"\n\
         assert = { require-section = { heading = \"Goal\" } }\n\
         [[rules]]\nname = \"unclaimed-rule\"\nseverity = \"warn\"\n\
         assert = { require-section = { heading = \"Plan\" } }\n",
    );
    write_invariants(
        &cli,
        "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"@/rule/claimed-rule\"\n",
    );

    let output = cli
        .command(["invariant", "check", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "expected exit 0 with unclaimed rules/gates, got {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["findings"].as_array().unwrap().is_empty(), "{json}");
}

#[test]
fn test_check_exits_zero_when_consistent() {
    let cli = JitCli::initialised().unwrap();
    // Exactly one rule, claimed by exactly one invariant: no drift.
    write_rules(
        &cli,
        "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
         assert = { require-section = { heading = \"Goal\" } }\n",
    );
    write_invariants(
        &cli,
        "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"@/rule/real-rule\"\n",
    );

    let output = cli
        .command(["invariant", "check", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "expected exit 0 when consistent, got {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["findings"].as_array().unwrap().is_empty());
    assert_eq!(json["message"], "No enforcement drift", "{json}");
}

#[test]
fn test_check_human_output_names_declared_direction() {
    let cli = JitCli::initialised().unwrap();
    write_rules(
        &cli,
        "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
         assert = { require-section = { heading = \"Goal\" } }\n",
    );
    write_invariants(
        &cli,
        "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"@/rule/ghost-rule\"\n",
    );

    let output = cli.command(["invariant", "check"]).output().unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("declared-but-unenforced") && stdout.contains("ghost-rule"),
        "human output missing declared direction: {stdout}"
    );
    // The unclaimed `real-rule` is not surfaced (no enforced-but-undeclared).
    assert!(
        !stdout.contains("enforced-but-undeclared"),
        "human output must not mention enforced-but-undeclared: {stdout}"
    );
}

#[test]
fn test_check_reports_unloadable_rule_source_not_a_parse_error() {
    // REQ-01 "unloadable" half: an invariant binds to `bad-rule` and `.jit/rules.toml`
    // is MALFORMED. `jit invariant check` must report a declared-but-unenforced
    // finding and exit non-zero — NOT print a raw TOML parse error.
    let cli = JitCli::initialised().unwrap();
    write_rules(
        &cli,
        "[[rules]]\nname = \"bad-rule\"\nseverity = \"error\"\n\
         assert = { this-is-not-a-valid-kind = { foo = 1 } }\n",
    );
    write_invariants(
        &cli,
        "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"@/rule/bad-rule\"\n",
    );

    let output = cli
        .command(["invariant", "check", "--json"])
        .output()
        .unwrap();

    // Drift fails, and stdout is VALID JSON (not a parse error).
    assert!(!output.status.success(), "expected failure on drift");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "expected valid JSON, not a raw parse error: {e}\nstdout: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert_eq!(json["error"]["code"], "VALIDATION_FAILED", "{json}");
    let findings = json["error"]["details"]["findings"].as_array().unwrap();
    assert!(
        findings
            .iter()
            .any(|f| f["subject"] == "@/rule/bad-rule" && f["unloadable"] == true),
        "missing unloadable declared-but-unenforced finding: {json}"
    );
}

#[test]
fn test_check_fails_on_unloadable_gate_registry() {
    let cli = JitCli::initialised().unwrap();
    write_rules(&cli, "\n");
    std::fs::write(cli.root().join(".jit/gates.toml"), "not valid toml {").unwrap();
    write_invariants(
        &cli,
        "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"@/gate/some-gate\"\n",
    );

    let output = cli
        .command(["invariant", "check", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains(".jit/gates.toml"),
        "expected gate registry load error: {json}"
    );
}
