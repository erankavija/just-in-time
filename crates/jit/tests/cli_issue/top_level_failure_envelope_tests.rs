//! Structural machine-readable failures rendered by the top-level CLI boundary.

use jit::output::ErrorCode;
use jit::test_cli::{created_issue_id, JitCli};
use std::fs;
use std::process::Output;
use std::str::FromStr;

fn parse_single_error(output: &Output) -> (serde_json::Value, ErrorCode) {
    let stdout = std::str::from_utf8(&output.stdout).expect("stdout is UTF-8");
    let envelope: serde_json::Value =
        serde_json::from_str(stdout).expect("stdout contains exactly one JSON document");
    assert_eq!(envelope.as_object().map(|object| object.len()), Some(1));
    let code = ErrorCode::from_str(
        envelope["error"]["code"]
            .as_str()
            .expect("error envelope carries a code"),
    )
    .expect("top-level error code is registered");
    (envelope, code)
}

#[test]
fn test_propagated_config_parse_failure_emits_registered_envelope_and_empty_stderr() {
    let repository = JitCli::initialised().unwrap();
    fs::write(
        repository.root().join(".jit/config.toml"),
        "this is not valid TOML [[[",
    )
    .expect("corrupt repository config");

    let output = repository
        .command(["label", "namespaces", "--json"])
        .output()
        .expect("run propagated failure fixture");

    let (envelope, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::ParseError);
    assert!(!output.status.success());
    assert!(envelope["error"]["message"].is_string());
    assert!(
        output.stderr.is_empty(),
        "JSON failure has no stderr diagnostic: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_propagated_config_parse_failure_without_json_writes_one_stderr_diagnostic() {
    let repository = JitCli::initialised().unwrap();
    fs::write(
        repository.root().join(".jit/config.toml"),
        "this is not valid TOML [[[",
    )
    .expect("corrupt repository config");

    let output = repository
        .command(["label", "namespaces"])
        .output()
        .expect("run propagated human-mode failure fixture");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "human failure leaked to stdout");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.lines().count(),
        1,
        "expected one diagnostic: {stderr}"
    );
    assert!(
        stderr.starts_with("Error: "),
        "unexpected diagnostic: {stderr}"
    );
}

#[test]
fn test_stored_record_parse_failure_uses_typed_registered_code() {
    let repository = JitCli::initialised().unwrap();
    fs::write(repository.root().join(".jit/index.json"), "{")
        .expect("corrupt stored repository index");

    let output = repository
        .command(["query", "all", "--json"])
        .output()
        .expect("run stored-record failure fixture");

    let (_, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::ParseError);
    assert!(!output.status.success());
}

#[test]
fn test_command_specific_error_keeps_one_envelope_and_its_details() {
    let repository = JitCli::initialised().unwrap();

    let output = repository
        .command(["issue", "show", "missing", "--json"])
        .output()
        .expect("run command-specific failure fixture");

    let (envelope, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::IssueNotFound);
    assert!(!output.status.success());
    assert_eq!(envelope["error"]["details"]["issue_id"], "missing");
    assert!(
        output.stderr.is_empty(),
        "handler-owned error stays unchanged"
    );
}

#[test]
fn test_validate_json_success_keeps_the_validation_report() {
    let repository = JitCli::initialised().unwrap();

    let output = repository
        .command(["validate", "--json"])
        .output()
        .expect("run successful validation fixture");

    assert!(output.status.success(), "validation failed: {output:?}");
    assert!(
        output.stderr.is_empty(),
        "successful JSON stays stream-pure"
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("stdout contains exactly one validation report document");
    assert_eq!(report["valid"], true);
    assert!(report.get("error").is_none());
}

#[test]
fn test_validate_json_integrity_failure_emits_one_registered_envelope() {
    let repository = JitCli::initialised().unwrap();
    let created = repository
        .command(["issue", "create", "--title", "broken graph", "--json"])
        .output()
        .expect("create validation fixture");
    assert!(created.status.success(), "issue create failed: {created:?}");
    let created_json: serde_json::Value =
        serde_json::from_slice(&created.stdout).expect("created issue is JSON");
    let issue_id = created_issue_id(&created_json).expect("created issue id");
    let issue_path = repository
        .root()
        .join(".jit/issues")
        .join(format!("{issue_id}.json"));
    let mut issue: serde_json::Value =
        serde_json::from_slice(&fs::read(&issue_path).expect("read created issue record"))
            .expect("created issue record is JSON");
    issue["dependencies"] = serde_json::json!(["00000000-0000-4000-8000-00000000dead"]);
    fs::write(
        issue_path,
        serde_json::to_vec_pretty(&issue).expect("serialize broken issue record"),
    )
    .expect("write broken issue record");

    let output = repository
        .command(["validate", "--json"])
        .output()
        .expect("run handler-owned validation failure fixture");

    let (envelope, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::ValidationFailed);
    assert!(!output.status.success());
    assert_eq!(envelope["error"]["details"]["valid"], false);
    assert!(envelope["error"]["details"]["integrity_error"]
        .as_str()
        .is_some_and(|message| message.contains("00000000-0000-4000-8000-00000000dead")));
    assert!(
        output.stderr.is_empty(),
        "machine-readable validation failure has no diagnostic pollution: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_validate_json_rule_failure_emits_one_registered_envelope() {
    let repository = JitCli::initialised().unwrap();
    let rules_path = repository.root().join(".jit/rules.toml");
    let mut rules = fs::read_to_string(&rules_path).expect("read default rules");
    rules.push_str(
        "\n[[rules]]\nname = \"epic-needs-req\"\nwhen = { type = \"epic\" }\n\
         severity = \"error\"\nenforce = false\n\
         assert = { require-label = { label = \"req:*\", min = 1 } }\n",
    );
    fs::write(rules_path, rules).expect("install rule fixture");
    let created = repository
        .command([
            "issue",
            "create",
            "--title",
            "An epic",
            "--label",
            "type:epic",
            "--label",
            "epic:validation-envelope",
        ])
        .output()
        .expect("create rule failure fixture");
    assert!(created.status.success(), "issue create failed: {created:?}");

    let output = repository
        .command(["validate", "--json"])
        .output()
        .expect("run rule failure fixture");

    let (envelope, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::ValidationFailed);
    assert!(!output.status.success());
    assert_eq!(envelope["error"]["details"]["valid"], false);
    assert!(envelope["error"]["details"]["integrity_error"].is_null());
    assert!(envelope["error"]["details"]["rule_findings"]
        .as_array()
        .is_some_and(|findings| findings.iter().any(|finding| {
            finding["rule"] == "epic-needs-req" && finding["severity"] == "error"
        })));
    assert!(
        output.stderr.is_empty(),
        "machine-readable validation failure has no diagnostic pollution: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_validate_json_branch_drift_failure_emits_one_registered_envelope() {
    let repository = JitCli::initialised().unwrap();

    let output = repository
        .command(["validate", "--branch-drift", "--json"])
        .output()
        .expect("run branch-drift failure fixture");

    let (envelope, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::ValidationFailed);
    assert!(!output.status.success());
    assert_eq!(envelope["error"]["details"]["valid"], false);
    assert_eq!(
        envelope["error"]["details"]["validations"][0]["validation"],
        "branch_drift"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn test_startup_repository_not_found_json_emits_envelope() {
    let cli = JitCli::uninitialised().unwrap();

    let output = cli.command(["query", "all", "--json"]).output().unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("startup failure should emit JSON on stdout under --json");
    assert_eq!(json["error"]["code"], "REPOSITORY_NOT_FOUND");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not found"));
}

#[test]
fn test_startup_repository_not_found_without_json_leaves_stdout_empty() {
    let cli = JitCli::uninitialised().unwrap();

    let output = cli.command(["query", "all"]).output().unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "non-json stdout must stay empty");
    assert!(String::from_utf8_lossy(&output.stderr).contains(".jit"));
}

#[test]
fn test_startup_repository_format_unsupported_json_emits_envelope() {
    let cli = JitCli::initialised().unwrap();
    let index_path = cli.root().join(".jit").join("index.json");
    let mut index: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&index_path).unwrap()).unwrap();
    index["schema_version"] = serde_json::json!(9999);
    fs::write(&index_path, serde_json::to_string_pretty(&index).unwrap()).unwrap();

    let output = cli.command(["query", "all", "--json"]).output().unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("unsupported-format failure should emit JSON on stdout under --json");
    assert_eq!(json["error"]["code"], "REPOSITORY_FORMAT_UNSUPPORTED");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not the version this binary"));
}
