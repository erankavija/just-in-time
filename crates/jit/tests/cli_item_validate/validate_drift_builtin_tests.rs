//! Integration tests: `jit validate` reports enforcement drift as a BUILT-IN
//! pass — with NO opt-in `.jit/rules.toml` rule (REQ-01/REQ-02).
//!
//! These prove the reviewer's named gap is closed: a repository that declares an
//! invariant in `.jit/invariants.toml` gets the declared-but-unenforced drift
//! finding from a plain `jit validate` (an unclaimed rule/gate is not drift,
//! REQ-05), while a repository declaring no invariant sees zero new drift
//! findings (a freshly initialized repository stays clean).

use jit::test_cli::JitCli;
use serde_json::Value;
use std::process::Output;

/// Whether a `jit validate --json` run succeeded, and its parsed stdout.
fn validate_report(output: &Output) -> (bool, Value) {
    let json: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "invalid validate JSON: {e}\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    (output.status.success(), json)
}

/// Collect every finding message anywhere in the validate JSON payload.
fn finding_messages(json: &Value) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                if let Some(Value::String(m)) = map.get("message") {
                    out.push(m.clone());
                }
                for child in map.values() {
                    walk(child, out);
                }
            }
            Value::Array(a) => a.iter().for_each(|c| walk(c, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(json, &mut out);
    out
}

#[test]
fn test_validate_reports_drift_without_opt_in_rule() {
    let cli = JitCli::initialised().unwrap();
    // NO rules.toml authored: drift is a built-in pass, not an opt-in rule.
    // sample-invariant binds to a MISSING rule/gate (declared-but-unenforced).
    std::fs::write(
        cli.root().join(".jit/invariants.toml"),
        "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"Acyclic deps.\"\nkind = \"enforced\"\n\
         enforced-by = \"@/rule/ghost-rule\"\n",
    )
    .unwrap();

    let (_succeeded, json) =
        validate_report(&cli.command(["validate", "--json"]).output().unwrap());
    let messages = finding_messages(&json);

    // declared-but-unenforced: sample-invariant -> ghost-rule.
    assert!(
        messages
            .iter()
            .any(|m| m.contains("declared-but-unenforced") && m.contains("ghost-rule")),
        "validate did not report declared-but-unenforced drift: {messages:?}"
    );
    // The built-in default rules exist but no invariant claims them; that is NOT
    // drift (REQ-05 removed the enforced-but-undeclared direction).
    assert!(
        !messages
            .iter()
            .any(|m| m.contains("enforced-but-undeclared")),
        "validate must not report enforced-but-undeclared drift: {messages:?}"
    );
}

#[test]
fn test_validate_fails_on_declared_but_unenforced_drift() {
    let cli = JitCli::initialised().unwrap();
    // A dangling binding is Error severity -> `jit validate` exits non-zero.
    std::fs::write(
        cli.root().join(".jit/invariants.toml"),
        "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"@/rule/ghost-rule\"\n",
    )
    .unwrap();

    let output = cli.command(["validate"]).output().unwrap();
    assert!(
        !output.status.success(),
        "validate should fail on a declared-but-unenforced binding"
    );
}

#[test]
fn test_validate_reports_unloadable_target_drift_not_parse_error() {
    // REQ-01 "unloadable" half (the named gap): an invariant binds to `bad-rule`
    // and `.jit/rules.toml` is MALFORMED. `jit validate --json` must surface the
    // declared-but-unenforced drift finding (worded as unloadable), not exit with a
    // raw parse error that loses the finding.
    let cli = JitCli::initialised().unwrap();
    std::fs::write(
        cli.root().join(".jit/rules.toml"),
        "[[rules]]\nname = \"bad-rule\"\nseverity = \"error\"\n\
         assert = { this-is-not-a-valid-kind = { foo = 1 } }\n",
    )
    .unwrap();
    std::fs::write(
        cli.root().join(".jit/invariants.toml"),
        "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"@/rule/bad-rule\"\n",
    )
    .unwrap();

    let (succeeded, json) = validate_report(&cli.command(["validate", "--json"]).output().unwrap());
    let messages = finding_messages(&json);

    // The drift finding is present (worded to make the unloadable source clear).
    assert!(
        messages
            .iter()
            .any(|m| m.contains("declared-but-unenforced")
                && m.contains("bad-rule")
                && m.contains("failed to load")),
        "validate did not surface the unloadable-target drift finding: {messages:?}"
    );
    // And validation fails (drift Error + ruleset parse config-error).
    assert!(
        !succeeded,
        "validate must fail when the ruleset is unloadable"
    );
}

#[test]
fn test_validate_clean_when_no_invariant_is_declared() {
    let cli = JitCli::initialised().unwrap();
    // Initialization leaves the registry present and empty, so nothing is
    // declared and the drift pass is dormant. `jit validate` must report ZERO
    // enforcement-drift findings.
    let registry = std::fs::read_to_string(cli.root().join(".jit/invariants.toml")).unwrap();
    assert!(!registry.contains("[[invariants]]"));

    let (succeeded, json) = validate_report(&cli.command(["validate", "--json"]).output().unwrap());
    let messages = finding_messages(&json);
    assert!(
        !messages.iter().any(|m| m.contains("enforcement drift")
            || m.contains("declared-but-unenforced")
            || m.contains("enforced-but-undeclared")),
        "validate reported drift with no invariants registry: {messages:?}"
    );
    // A pristine `jit init` repo with no invariants validates cleanly (exit 0).
    assert!(succeeded, "pristine repo should validate clean: {json}");
}

#[test]
fn test_validate_clean_when_invariants_fully_consistent() {
    let cli = JitCli::initialised().unwrap();
    // Author a rules.toml with one rule claimed by one invariant, and a second
    // invariant claiming a defined gate. Every binding resolves -> no drift,
    // validate clean.
    std::fs::write(
        cli.root().join(".jit/rules.toml"),
        "[[rules]]\nname = \"only-rule\"\nseverity = \"warn\"\n\
         assert = { require-section = { heading = \"Goal\" } }\n",
    )
    .unwrap();
    // Define the gate the second invariant claims.
    cli.run([
        "gate",
        "define",
        "only-gate",
        "--title",
        "Only",
        "--description",
        "gate for second-invariant",
        "--mode",
        "manual",
    ])
    .expect("define the gate");
    std::fs::write(
        cli.root().join(".jit/invariants.toml"),
        "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"@/rule/only-rule\"\n\
         [[invariants]]\nid = \"second-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"@/gate/only-gate\"\n",
    )
    .unwrap();

    let (succeeded, json) = validate_report(&cli.command(["validate", "--json"]).output().unwrap());
    let messages = finding_messages(&json);
    assert!(
        !messages.iter().any(|m| m.contains("enforcement drift")),
        "consistent registry should produce no drift: {messages:?}"
    );
    assert!(succeeded, "consistent repo should validate clean: {json}");
}
