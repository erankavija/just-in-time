//! CLI integration tests for the extended `jit validate` surface (issue
//! b8ba1b10): the positional `[<id>]`, `--explain`, `--json`, error-severity
//! exit codes, the `$JIT_ISSUE_ID` gate-checker wiring, and `--schema` / MCP
//! parity for the new positional + flag.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn bin() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
}

/// Absolute path to the built `jit` binary, for use inside gate checker commands
/// (the gate runs `sh -c <command>` and may not have `jit` on PATH).
fn jit_path() -> String {
    assert_cmd::cargo::cargo_bin!("jit")
        .to_string_lossy()
        .to_string()
}

/// Initialize a repo and write a `.jit/rules.toml`. Also declares a `req`
/// label namespace so `req:*` labels pass the namespace registry check.
fn setup_repo_with_rules(rules_toml: &str) -> TempDir {
    let temp = TempDir::new().unwrap();
    bin()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    fs::write(temp.path().join(".jit").join("rules.toml"), rules_toml).unwrap();

    // Register the `req` namespace used by the test rules.
    let config_path = temp.path().join(".jit").join("config.toml");
    let mut config = fs::read_to_string(&config_path).unwrap();
    config.push_str("\n[namespaces.req]\ndescription = \"Requirement id.\"\nunique = false\n");
    fs::write(config_path, config).unwrap();
    temp
}

/// Create an epic issue, returning its short id.
fn create_epic(temp: &TempDir, with_req: bool) -> String {
    let mut args = vec![
        "issue".to_string(),
        "create".to_string(),
        "--title".to_string(),
        "An epic".to_string(),
        "--label".to_string(),
        "type:epic".to_string(),
    ];
    if with_req {
        args.push("--label".to_string());
        args.push("req:REQ-01".to_string());
    }
    let output = bin()
        .current_dir(temp.path())
        .args(&args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&output);
    s.lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

const EPIC_NEEDS_REQ: &str = r#"
[[rules]]
name = "epic-needs-req"
when = { type = "epic" }
severity = "error"
enforce = false
assert = { require-label = { label = "req:*", min = 1 } }
"#;

#[test]
fn test_validate_id_passes_for_compliant_issue() {
    let temp = setup_repo_with_rules(EPIC_NEEDS_REQ);
    let id = create_epic(&temp, true);

    bin()
        .current_dir(temp.path())
        .args(["validate", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed"));
}

#[test]
fn test_validate_id_fails_and_exits_nonzero() {
    let temp = setup_repo_with_rules(EPIC_NEEDS_REQ);
    let id = create_epic(&temp, false);

    bin()
        .current_dir(temp.path())
        .args(["validate", &id])
        .assert()
        .failure()
        .stdout(predicate::str::contains("epic-needs-req"));
}

#[test]
fn test_validate_id_json_reports_findings_and_exits_nonzero() {
    let temp = setup_repo_with_rules(EPIC_NEEDS_REQ);
    let id = create_epic(&temp, false);

    let assert = bin()
        .current_dir(temp.path())
        .args(["validate", &id, "--json"])
        .assert()
        .failure();
    let out = assert.get_output().stdout.clone();
    let json: Value = serde_json::from_slice(&out).unwrap();
    let findings = json["findings"].as_array().unwrap();
    assert!(findings.iter().any(|f| f["rule"] == "epic-needs-req"));
    assert!(findings.iter().any(|f| f["severity"] == "error"));
}

#[test]
fn test_validate_explain_lists_outcomes() {
    let temp = setup_repo_with_rules(EPIC_NEEDS_REQ);
    let id = create_epic(&temp, false);

    bin()
        .current_dir(temp.path())
        .args(["validate", &id, "--explain"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("epic-needs-req"))
        .stdout(predicate::str::contains("FAIL"))
        .stdout(predicate::str::contains("type=epic"));
}

#[test]
fn test_validate_explain_json_structure() {
    let temp = setup_repo_with_rules(EPIC_NEEDS_REQ);
    let id = create_epic(&temp, true);

    let assert = bin()
        .current_dir(temp.path())
        .args(["validate", &id, "--explain", "--json"])
        .assert()
        .success();
    let out = assert.get_output().stdout.clone();
    let json: Value = serde_json::from_slice(&out).unwrap();
    let outcomes = json["outcomes"].as_array().unwrap();
    let outcome = &outcomes[0];
    assert_eq!(outcome["rule"], "epic-needs-req");
    assert_eq!(outcome["scope"], "local");
    assert_eq!(outcome["passed"], true);
}

#[test]
fn test_validate_explain_requires_id() {
    let temp = setup_repo_with_rules(EPIC_NEEDS_REQ);
    bin()
        .current_dir(temp.path())
        .args(["validate", "--explain"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires an issue id"));
}

#[test]
fn test_gate_checker_via_jit_issue_id_env_var() {
    // A gate whose checker runs `jit validate "$JIT_ISSUE_ID" --json` must pass
    // when the issue is compliant. This proves the JIT_ISSUE_ID wiring end to end
    // (no `{issue}` placeholder is used).
    let temp = setup_repo_with_rules(EPIC_NEEDS_REQ);
    let checker = format!("{} validate \"$JIT_ISSUE_ID\" --json", jit_path());

    bin()
        .current_dir(temp.path())
        .args([
            "gate",
            "define",
            "rules-gate",
            "--title",
            "Rules Gate",
            "--description",
            "Runs jit validate for the issue",
            "--mode",
            "auto",
            "--checker-command",
            &checker,
            "--timeout",
            "30",
        ])
        .assert()
        .success();

    // Compliant epic with the gate -> gate passes.
    let output = bin()
        .current_dir(temp.path())
        .args([
            "issue",
            "create",
            "--title",
            "Good epic",
            "--label",
            "type:epic",
            "--label",
            "req:REQ-01",
            "--gate",
            "rules-gate",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&output);
    let good_id = s
        .lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    bin()
        .current_dir(temp.path())
        .args(["gate", "pass", &good_id, "rules-gate"])
        .assert()
        .success();
}

#[test]
fn test_gate_checker_via_jit_issue_id_fails_for_noncompliant() {
    // The same gate must FAIL when the issue violates an error-severity rule,
    // because `jit validate "$JIT_ISSUE_ID"` exits non-zero.
    let temp = setup_repo_with_rules(EPIC_NEEDS_REQ);
    let checker = format!("{} validate \"$JIT_ISSUE_ID\" --json", jit_path());

    bin()
        .current_dir(temp.path())
        .args([
            "gate",
            "define",
            "rules-gate",
            "--title",
            "Rules Gate",
            "--description",
            "Runs jit validate for the issue",
            "--mode",
            "auto",
            "--checker-command",
            &checker,
            "--timeout",
            "30",
        ])
        .assert()
        .success();

    // Non-compliant epic (no req:* label) with the gate -> gate fails.
    let output = bin()
        .current_dir(temp.path())
        .args([
            "issue",
            "create",
            "--title",
            "Bad epic",
            "--label",
            "type:epic",
            "--gate",
            "rules-gate",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&output);
    let bad_id = s
        .lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    bin()
        .current_dir(temp.path())
        .args(["gate", "pass", &bad_id, "rules-gate"])
        .assert()
        .failure();
}

// ---------------------------------------------------------------------------
// Rework (jit:b8ba1b10): whole-repo --json must surface graph-rule findings;
// id + legacy flags must be rejected; per-issue + --explain must report
// malformed graph rules instead of silently passing.
// ---------------------------------------------------------------------------

/// A `label-coverage` GRAPH rule at error severity. An epic declaring a `[hard]`
/// criterion with no satisfying child (child-link = any) violates it.
const COVERAGE_GRAPH_RULE: &str = r#"
[[rules]]
name = "epic-criteria-covered"
when = { type = "epic" }
severity = "error"
scope = "graph"
assert = { label-coverage = { child-link = "any", marker = "[hard]" } }
"#;

/// A MALFORMED `label-reference` GRAPH rule: the required `to` key is missing, so
/// the rule cannot be applied and must surface as a config-error finding.
const MALFORMED_GRAPH_RULE: &str = r#"
[[rules]]
name = "broken-graph-rule"
when = { type = "epic" }
severity = "error"
scope = "graph"
assert = { label-reference = { from = "satisfies" } }
"#;

/// Create an epic whose body declares a single `[hard]` success criterion.
fn create_epic_with_criterion(temp: &TempDir) -> String {
    let output = bin()
        .current_dir(temp.path())
        .args([
            "issue",
            "create",
            "--title",
            "An epic",
            "--label",
            "type:epic",
            "--description",
            "## Success Criteria\n\n- [hard] REQ-01: do the thing\n",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&output);
    s.lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

#[test]
fn test_validate_whole_repo_json_includes_graph_error_findings_and_exits_nonzero() {
    // Regression for finding #1: a whole-repo `jit validate --json` with an
    // error-severity GRAPH rule violation must EMIT the structured rule report
    // including the graph finding, and exit non-zero — not bail via the generic
    // error path that drops the structured report.
    let temp = setup_repo_with_rules(COVERAGE_GRAPH_RULE);
    let _id = create_epic_with_criterion(&temp);

    let assert = bin()
        .current_dir(temp.path())
        .args(["validate", "--json"])
        .assert()
        .failure();
    let out = assert.get_output().stdout.clone();
    let json: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        json["valid"], false,
        "whole-repo run must be invalid: {json}"
    );
    let findings = json["rule_findings"].as_array().unwrap();
    assert!(
        findings
            .iter()
            .any(|f| f["rule"] == "epic-criteria-covered" && f["severity"] == "error"),
        "graph-rule finding must appear in the structured report: {findings:?}"
    );
}

#[test]
fn test_validate_id_with_fix_is_rejected() {
    // Finding #2: `--fix` is repo-wide and must not be silently scoped to an id.
    let temp = setup_repo_with_rules(EPIC_NEEDS_REQ);
    let id = create_epic(&temp, true);
    bin()
        .current_dir(temp.path())
        .args(["validate", &id, "--fix"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "cannot be combined with a positional",
        ));
}

#[test]
fn test_validate_id_with_divergence_is_rejected() {
    // Finding #2: `--divergence` is repo-wide and incompatible with a positional id.
    let temp = setup_repo_with_rules(EPIC_NEEDS_REQ);
    let id = create_epic(&temp, true);
    bin()
        .current_dir(temp.path())
        .args(["validate", &id, "--divergence"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "cannot be combined with a positional",
        ));
}

#[test]
fn test_validate_id_with_leases_is_rejected() {
    // Finding #2: `--leases` is repo-wide and incompatible with a positional id.
    let temp = setup_repo_with_rules(EPIC_NEEDS_REQ);
    let id = create_epic(&temp, true);
    bin()
        .current_dir(temp.path())
        .args(["validate", &id, "--leases"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "cannot be combined with a positional",
        ));
}

#[test]
fn test_validate_id_reports_malformed_graph_rule_not_passed() {
    // Finding #3: a malformed graph rule whose selector applies to the issue must
    // be REPORTED for a per-issue `jit validate <id>` (a config-error finding),
    // never silently dropped.
    let temp = setup_repo_with_rules(MALFORMED_GRAPH_RULE);
    let id = create_epic(&temp, false);

    let assert = bin()
        .current_dir(temp.path())
        .args(["validate", &id, "--json"])
        .assert()
        .failure();
    let out = assert.get_output().stdout.clone();
    let json: Value = serde_json::from_slice(&out).unwrap();
    let findings = json["findings"].as_array().unwrap();
    assert!(
        findings.iter().any(|f| f["rule"] == "broken-graph-rule"
            && f["message"].as_str().unwrap_or("").contains("config error")),
        "malformed graph rule must be reported per-issue: {findings:?}"
    );
}

#[test]
fn test_validate_explain_marks_malformed_graph_rule_as_failed() {
    // Finding #3 (--explain): a graph rule with a config error for the issue must
    // be shown as FAILED, never PASS.
    let temp = setup_repo_with_rules(MALFORMED_GRAPH_RULE);
    let id = create_epic(&temp, false);

    // Human form: the rule appears and is marked FAIL.
    bin()
        .current_dir(temp.path())
        .args(["validate", &id, "--explain"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("broken-graph-rule"))
        .stdout(predicate::str::contains("FAIL"));

    // JSON form: the outcome for the malformed rule has passed = false.
    let assert = bin()
        .current_dir(temp.path())
        .args(["validate", &id, "--explain", "--json"])
        .assert()
        .failure();
    let out = assert.get_output().stdout.clone();
    let json: Value = serde_json::from_slice(&out).unwrap();
    let outcomes = json["outcomes"].as_array().unwrap();
    let outcome = outcomes
        .iter()
        .find(|o| o["rule"] == "broken-graph-rule")
        .expect("malformed graph rule must appear as an outcome");
    assert_eq!(
        outcome["passed"], false,
        "a config-errored graph rule must be reported as failed, not passed: {outcome}"
    );
}

#[test]
fn test_schema_exposes_validate_positional_and_explain_flag() {
    // MCP parity: the new positional `id` arg and `--explain` flag MUST appear in
    // `jit --schema` so the MCP server auto-generates a working tool (DR §9.3).
    let output = bin().arg("--schema").output().unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let validate = &json["commands"]["validate"];

    let args = validate["args"].as_array().unwrap();
    assert!(
        args.iter().any(|a| a["name"] == "id"),
        "validate must expose positional `id`: {args:?}"
    );

    let flags = validate["flags"].as_array().unwrap();
    assert!(
        flags.iter().any(|f| f["name"] == "explain"),
        "validate must expose --explain flag: {flags:?}"
    );
    assert!(
        flags.iter().any(|f| f["name"] == "json"),
        "validate must expose --json flag"
    );
}

#[test]
fn test_mcp_generates_validate_tool_with_id_and_explain() {
    // The MCP server generates tools from `jit --schema`. Run its tool-generator
    // over the real schema and assert the `jit_validate` tool carries the new
    // `id` and `explain` properties (proves a WORKING tool is generated for the
    // new surface, not just that the schema contains the fields).
    let schema_out = bin().arg("--schema").output().unwrap();
    assert!(schema_out.status.success());
    let schema_json = String::from_utf8(schema_out.stdout).unwrap();

    let mcp_dir = mcp_server_dir();
    // Skip gracefully if node is unavailable in the environment.
    let node = match Command::new("node").arg("--version").output() {
        Ok(o) if o.status.success() => "node",
        _ => {
            eprintln!("node not available; skipping MCP parity assertion");
            return;
        }
    };

    let script = r#"
import { generateTools } from './lib/tool-generator.js';
let data = '';
process.stdin.on('data', c => data += c);
process.stdin.on('end', () => {
  const schema = JSON.parse(data);
  const tools = generateTools(schema, false);
  const tool = tools.find(t => t.name === 'jit_validate');
  if (!tool) { console.error('no jit_validate tool'); process.exit(2); }
  const props = tool.inputSchema.properties || {};
  if (!props.id) { console.error('missing id property'); process.exit(3); }
  if (!props.explain) { console.error('missing explain property'); process.exit(4); }
  console.log('ok');
});
"#;

    let mut child = Command::new(node)
        .current_dir(&mcp_dir)
        .args(["--input-type=module", "-e", script])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn node");
    {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(schema_json.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "MCP tool generation parity failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok"));
}

/// Locate the `mcp-server` directory relative to the crate manifest.
fn mcp_server_dir() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR is crates/jit; mcp-server is two levels up.
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("mcp-server")
}
