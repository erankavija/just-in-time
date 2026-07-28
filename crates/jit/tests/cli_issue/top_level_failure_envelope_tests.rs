//! Structural machine-readable failures rendered by the top-level CLI boundary.

use jit::output::ErrorCode;
use std::fs;
use std::process::{Command, Output};
use std::str::FromStr;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_repository() -> TempDir {
    let repository = TempDir::new().expect("create temporary repository");
    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .arg("init")
        .output()
        .expect("initialize repository");
    assert!(output.status.success(), "init failed: {output:?}");
    repository
}

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
fn test_propagated_config_parse_failure_emits_registered_envelope_and_human_diagnostic() {
    let repository = setup_repository();
    fs::write(
        repository.path().join(".jit/config.toml"),
        "this is not valid TOML [[[",
    )
    .expect("corrupt repository config");

    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .args(["label", "namespaces", "--json"])
        .output()
        .expect("run propagated failure fixture");

    let (envelope, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::ParseError);
    assert_eq!(output.status.code(), Some(code.exit_code().code()));
    assert!(envelope["error"]["message"].is_string());
    assert!(
        String::from_utf8_lossy(&output.stderr).starts_with("Error: "),
        "human diagnostic remains on stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_stored_record_parse_failure_uses_typed_registered_code() {
    let repository = setup_repository();
    fs::write(repository.path().join(".jit/index.json"), "{")
        .expect("corrupt stored repository index");

    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .args(["query", "all", "--json"])
        .output()
        .expect("run stored-record failure fixture");

    let (_, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::ParseError);
    assert_eq!(output.status.code(), Some(code.exit_code().code()));
}

#[test]
fn test_command_specific_error_keeps_one_envelope_and_its_details() {
    let repository = setup_repository();

    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .args(["issue", "show", "missing", "--json"])
        .output()
        .expect("run command-specific failure fixture");

    let (envelope, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::IssueNotFound);
    assert_eq!(output.status.code(), Some(code.exit_code().code()));
    assert_eq!(envelope["error"]["details"]["issue_id"], "missing");
    assert!(
        output.stderr.is_empty(),
        "handler-owned error stays unchanged"
    );
}

#[test]
fn test_validate_json_success_keeps_the_validation_report() {
    let repository = setup_repository();

    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .args(["validate", "--json"])
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
    let repository = setup_repository();
    let created = Command::new(jit_binary())
        .current_dir(repository.path())
        .args(["issue", "create", "--title", "broken graph", "--json"])
        .output()
        .expect("create validation fixture");
    assert!(created.status.success(), "issue create failed: {created:?}");
    let created_json: serde_json::Value =
        serde_json::from_slice(&created.stdout).expect("created issue is JSON");
    let issue_id = created_json["id"].as_str().expect("created issue id");
    let issue_path = repository
        .path()
        .join(".jit/issues")
        .join(format!("{issue_id}.json"));
    let mut issue: serde_json::Value =
        serde_json::from_slice(&fs::read(&issue_path).expect("read created issue record"))
            .expect("created issue record is JSON");
    issue["dependencies"] = serde_json::json!(["missing-dependency"]);
    fs::write(
        issue_path,
        serde_json::to_vec_pretty(&issue).expect("serialize broken issue record"),
    )
    .expect("write broken issue record");

    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .args(["validate", "--json"])
        .output()
        .expect("run handler-owned validation failure fixture");

    assert_eq!(
        output.status.code(),
        Some(jit::output::ExitCode::ValidationFailed.code())
    );
    let (envelope, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::ValidationFailed);
    assert_eq!(output.status.code(), Some(code.exit_code().code()));
    assert_eq!(envelope["error"]["details"]["valid"], false);
    assert!(envelope["error"]["details"]["integrity_error"]
        .as_str()
        .is_some_and(|message| message.contains("missing-dependency")));
    assert!(
        output.stderr.is_empty(),
        "machine-readable validation failure has no diagnostic pollution: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_validate_json_rule_failure_emits_one_registered_envelope() {
    let repository = setup_repository();
    let rules_path = repository.path().join(".jit/rules.toml");
    let mut rules = fs::read_to_string(&rules_path).expect("read default rules");
    rules.push_str(
        "\n[[rules]]\nname = \"epic-needs-req\"\nwhen = { type = \"epic\" }\n\
         severity = \"error\"\nenforce = false\n\
         assert = { require-label = { label = \"req:*\", min = 1 } }\n",
    );
    fs::write(rules_path, rules).expect("install rule fixture");
    let created = Command::new(jit_binary())
        .current_dir(repository.path())
        .args([
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

    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .args(["validate", "--json"])
        .output()
        .expect("run rule failure fixture");

    let (envelope, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::GenericError);
    assert_eq!(output.status.code(), Some(code.exit_code().code()));
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
    let repository = setup_repository();

    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .args(["validate", "--branch-drift", "--json"])
        .output()
        .expect("run branch-drift failure fixture");

    let (envelope, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::GenericError);
    assert_eq!(output.status.code(), Some(code.exit_code().code()));
    assert_eq!(envelope["error"]["details"]["valid"], false);
    assert_eq!(
        envelope["error"]["details"]["validations"][0]["validation"],
        "branch_drift"
    );
    assert!(output.stderr.is_empty());
}
