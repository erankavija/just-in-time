//! Integration tests: `jit validate` reports enforcement drift as a BUILT-IN
//! pass — with NO opt-in `.jit/rules.toml` rule (REQ-01/REQ-02).
//!
//! These prove the reviewer's named gap is closed: a repository that simply
//! declares `.jit/invariants.toml` gets both drift directions from a plain
//! `jit validate`, while a repository with NO invariants registry sees zero new
//! drift findings (the live repo stays clean — graceful degradation).

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

/// Run `jit validate --json` and return (exit_code, parsed_json).
fn run_validate(temp: &TempDir) -> (Option<i32>, Value) {
    let output = Command::new(jit_binary())
        .args(["validate", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "invalid validate JSON: {e}\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    (output.status.code(), json)
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
    let temp = setup_test_repo();
    // NO rules.toml authored: drift is a built-in pass, not an opt-in rule.
    // INV-01 binds to a MISSING rule/gate (declared-but-unenforced).
    std::fs::write(
        temp.path().join(".jit/invariants.toml"),
        "[[invariants]]\nid = \"INV-01\"\nstatement = \"Acyclic deps.\"\nkind = \"enforced\"\n\
         enforced-by = \"ghost-rule\"\n",
    )
    .unwrap();

    let (_code, json) = run_validate(&temp);
    let messages = finding_messages(&json);

    // declared-but-unenforced: INV-01 -> ghost-rule.
    assert!(
        messages
            .iter()
            .any(|m| m.contains("declared-but-unenforced") && m.contains("ghost-rule")),
        "validate did not report declared-but-unenforced drift: {messages:?}"
    );
    // enforced-but-undeclared: the built-in default rules exist but no invariant
    // claims them, so at least one undeclared finding appears.
    assert!(
        messages
            .iter()
            .any(|m| m.contains("enforced-but-undeclared")),
        "validate did not report enforced-but-undeclared drift: {messages:?}"
    );
}

#[test]
fn test_validate_fails_on_declared_but_unenforced_drift() {
    let temp = setup_test_repo();
    // A dangling binding is Error severity -> `jit validate` exits non-zero.
    std::fs::write(
        temp.path().join(".jit/invariants.toml"),
        "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"ghost-rule\"\n",
    )
    .unwrap();

    let output = Command::new(jit_binary())
        .arg("validate")
        .current_dir(temp.path())
        .output()
        .unwrap();
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
    let temp = setup_test_repo();
    std::fs::write(
        temp.path().join(".jit/rules.toml"),
        "[[rules]]\nname = \"bad-rule\"\nseverity = \"error\"\n\
         assert = { this-is-not-a-valid-kind = { foo = 1 } }\n",
    )
    .unwrap();
    std::fs::write(
        temp.path().join(".jit/invariants.toml"),
        "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"bad-rule\"\n",
    )
    .unwrap();

    let (code, json) = run_validate(&temp);
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
    assert_ne!(
        code,
        Some(0),
        "validate must fail when the ruleset is unloadable"
    );
}

#[test]
fn test_validate_clean_when_no_invariants_registry() {
    let temp = setup_test_repo();
    // No .jit/invariants.toml at all -> the drift pass is dormant. `jit validate`
    // must report ZERO enforcement-drift findings (proves the live repo, which has
    // no invariants registry, is unaffected).
    assert!(!temp.path().join(".jit/invariants.toml").exists());

    let (code, json) = run_validate(&temp);
    let messages = finding_messages(&json);
    assert!(
        !messages.iter().any(|m| m.contains("enforcement drift")
            || m.contains("declared-but-unenforced")
            || m.contains("enforced-but-undeclared")),
        "validate reported drift with no invariants registry: {messages:?}"
    );
    // A pristine `jit init` repo with no invariants validates cleanly (exit 0).
    assert_eq!(code, Some(0), "pristine repo should validate clean: {json}");
}

#[test]
fn test_validate_clean_when_invariants_fully_consistent() {
    let temp = setup_test_repo();
    // Author a rules.toml with exactly one rule, claimed by exactly one invariant;
    // and a second invariant claiming the one gate. No rule/gate is unclaimed and
    // no binding dangles -> no drift, validate clean.
    std::fs::write(
        temp.path().join(".jit/rules.toml"),
        "[[rules]]\nname = \"only-rule\"\nseverity = \"warn\"\n\
         assert = { require-section = { heading = \"Goal\" } }\n",
    )
    .unwrap();
    // Define the gate the second invariant claims.
    let gate = Command::new(jit_binary())
        .args([
            "gate",
            "define",
            "only-gate",
            "--title",
            "Only",
            "--description",
            "gate for INV-02",
            "--mode",
            "manual",
        ])
        .env("JIT_TEST_MODE", "1")
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        gate.status.success(),
        "gate define failed: {}",
        String::from_utf8_lossy(&gate.stderr)
    );
    std::fs::write(
        temp.path().join(".jit/invariants.toml"),
        "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"only-rule\"\n\
         [[invariants]]\nid = \"INV-02\"\nstatement = \"s\"\nkind = \"enforced\"\n\
         enforced-by = \"only-gate\"\n",
    )
    .unwrap();

    let (code, json) = run_validate(&temp);
    let messages = finding_messages(&json);
    assert!(
        !messages.iter().any(|m| m.contains("enforcement drift")),
        "consistent registry should produce no drift: {messages:?}"
    );
    assert_eq!(
        code,
        Some(0),
        "consistent repo should validate clean: {json}"
    );
}
