use jit::test_cli::JitCli;
use std::fs;

#[test]
fn test_issue_not_found_error_json() {
    let cli = JitCli::initialised().unwrap();

    // Try to show non-existent issue
    let output = cli
        .command(["issue", "show", "nonexistent", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify error structure
    assert!(json["error"]["code"]
        .as_str()
        .unwrap()
        .contains("NOT_FOUND"));
    assert!(json["error"]["message"].is_string());
    assert!(json["error"]["suggestions"].is_array());
}

#[test]
fn test_cycle_detected_error_json() {
    let cli = JitCli::initialised().unwrap();

    // Create two issues
    let id1 = cli.create_issue(["-t", "Task A", "-d", "First"]).unwrap();
    let id2 = cli.create_issue(["-t", "Task B", "-d", "Second"]).unwrap();

    // Add A depends on B
    cli.command(["dep", "add", &id1, &id2]).output().unwrap();

    // Try to add B depends on A (creates cycle)
    let output = cli
        .command(["dep", "add", &id2, &id1, "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify error structure
    assert!(json["error"]["code"]
        .as_str()
        .unwrap()
        .contains("CYCLE_DETECTED"));
    assert!(json["error"]["message"].as_str().unwrap().contains("cycle"));
    assert!(json["error"]["suggestions"].is_array());
}

#[test]
fn test_deletion_not_confirmed_error_json() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["-t", "Doomed"]).unwrap();

    // Delete without JIT_ALLOW_DELETION=1: refused (REQ-02).
    let output = cli
        .command(["issue", "delete", &id, "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["error"]["code"], "DELETION_NOT_CONFIRMED");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("discouraged"));
    let suggestions = json["error"]["suggestions"].as_array().unwrap();
    assert!(
        suggestions
            .iter()
            .any(|s| s.as_str().unwrap().contains("JIT_ALLOW_DELETION=1")),
        "suggestions must carry the confirmation hint, got: {suggestions:?}"
    );

    // The issue must still exist: the refusal must not have deleted it.
    let show = cli.command(["issue", "show", &id]).output().unwrap();
    assert!(show.status.success(), "issue must survive a refused delete");
}

#[test]
fn test_deletion_not_confirmed_without_json_hints_on_stderr_and_leaves_stdout_empty() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["-t", "Doomed"]).unwrap();

    let output = cli.command(["issue", "delete", &id]).output().unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("JIT_ALLOW_DELETION=1"), "stderr: {stderr}");
    assert!(output.stdout.is_empty(), "refusal must leave stdout empty");
}

#[test]
fn test_invalid_state_error_json() {
    let cli = JitCli::initialised().unwrap();

    // Try to query with invalid state
    let output = cli
        .command(["query", "all", "--state", "invalid_state", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("the payload stream must carry the JSON error envelope");
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(
        json["error"]["message"]
            .as_str()
            .is_some_and(|message| !message.trim().is_empty()),
        "the JSON error envelope must carry a non-empty message: {json}"
    );
}

#[test]
fn test_gate_operation_error_json() {
    let cli = JitCli::initialised().unwrap();

    // Create an issue
    let id = cli.create_issue(["-t", "Task", "-d", "Test"]).unwrap();

    // Try to evaluate a gate that wasn't added to the issue
    let output = cli
        .command(["gate", "evaluate", &id, "nonexistent-gate", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("the payload stream must carry the JSON error envelope");
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(
        json["error"]["message"]
            .as_str()
            .is_some_and(|message| !message.trim().is_empty()),
        "the JSON error envelope must carry a non-empty message: {json}"
    );
}

#[test]
fn test_blocked_by_gates_human_diverts_to_gated_then_completes_on_pass() {
    let cli = JitCli::initialised().unwrap();
    cli.run([
        "gate",
        "define",
        "--title",
        "Tests",
        "--description",
        "Tests",
        "tests",
    ])
    .unwrap();
    let id = cli
        .create_issue(["--title", "Test issue", "--gate", "tests"])
        .unwrap();
    cli.run(["issue", "update", &id, "--state", "ready"])
        .unwrap();

    let output = cli
        .command(["issue", "update", &id, "--state", "done"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("gate") || stderr.contains("tests"));
    assert!(stderr.contains("gated") || stderr.contains("not passed"));
    assert!(stderr.contains("jit gate evaluate"));
    assert!(stderr.contains(&id));
    assert_eq!(
        cli.run_json(["issue", "show", &id]).unwrap()["state"],
        "gated"
    );

    cli.run(["gate", "evaluate", &id, "tests", "--by", "human:reviewer"])
        .unwrap();
    assert_eq!(
        cli.run_json(["issue", "show", &id]).unwrap()["state"],
        "done"
    );
}

#[test]
fn test_blocked_by_gates_json_details_and_remediation() {
    let cli = JitCli::initialised().unwrap();
    cli.run([
        "gate",
        "define",
        "--title",
        "Tests",
        "--description",
        "Tests",
        "tests",
    ])
    .unwrap();
    let id = cli
        .create_issue(["--title", "Test issue", "--gate", "tests"])
        .unwrap();

    let output = cli
        .command(["issue", "update", &id, "--state", "done", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let code = json["error"]["code"].as_str().unwrap();
    assert!(code.contains("GATE") || code.contains("VALIDATION"));
    let error_msg = json["error"]["message"].as_str().unwrap();
    assert!(error_msg.contains("gate") || error_msg.contains("tests"));

    let details = &json["error"]["details"];
    assert_eq!(details["issue_id"], id.as_str());
    assert_eq!(details["requested_state"], "done");
    assert_eq!(details["actual_state"], "gated");
    assert_eq!(details["blockers"][0]["type"], "gate");
    assert_eq!(details["blockers"][0]["key"], "tests");
    assert_eq!(details["blockers"][0]["status"], "pending");

    let remediation = details["remediation"].as_array().unwrap();
    let suggests = |expected: String| {
        remediation
            .iter()
            .any(|cmd| cmd.as_str().unwrap().contains(&expected))
    };
    assert!(suggests(format!("jit gate evaluate {id} tests")));
    assert!(suggests(format!("jit gate status-all {id}")));
    assert!(remediation.iter().any(|cmd| {
        let cmd = cmd.as_str().unwrap();
        cmd.contains(&format!("jit gate status {id} tests")) && cmd.contains("--all")
    }));
}

#[test]
fn test_blocked_by_dependencies_json_details_and_remediation() {
    let cli = JitCli::initialised().unwrap();
    let dependency = cli
        .create_issue(["--title", "Blocked prerequisite"])
        .unwrap();
    let dependent = cli.create_issue(["--title", "Blocked work"]).unwrap();
    cli.run(["dep", "add", &dependent, &dependency]).unwrap();

    let output = cli
        .command(["issue", "update", &dependent, "--state", "ready", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "BLOCKED");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("dependencies"));

    let details = &json["error"]["details"];
    assert_eq!(details["issue_id"], dependent.as_str());
    assert_eq!(details["requested_state"], "ready");
    assert_eq!(details["actual_state"], "backlog");
    assert_eq!(details["blockers"][0]["type"], "dependency");
    assert_eq!(details["blockers"][0]["issue_id"], dependency.as_str());
    assert_eq!(details["blockers"][0]["title"], "Blocked prerequisite");
    assert_eq!(details["blockers"][0]["state"], "ready");

    let remediation = details["remediation"].as_array().unwrap();
    assert!(remediation.iter().any(|cmd| cmd
        .as_str()
        .unwrap()
        .contains(&format!("jit graph deps {dependent}"))));
    assert!(remediation.iter().any(|cmd| cmd
        .as_str()
        .unwrap()
        .contains(&format!("jit issue show {dependency}"))));
}

#[test]
fn test_blocked_by_dependencies_human_names_blocker_and_remediation() {
    let cli = JitCli::initialised().unwrap();
    let dependency = cli
        .create_issue(["--title", "Blocked prerequisite"])
        .unwrap();
    let dependent = cli.create_issue(["--title", "Blocked work"]).unwrap();
    cli.run(["dep", "add", &dependent, &dependency]).unwrap();

    let output = cli
        .command(["issue", "update", &dependent, "--state", "ready"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Blocked prerequisite"));
    assert!(stderr.contains(&dependency[..8]));
    assert!(stderr.contains("jit graph deps"));
    assert!(stderr.contains("jit issue show"));
}

#[test]
fn test_blocked_by_missing_dependency_json_reports_missing_state() {
    let cli = JitCli::initialised().unwrap();
    let dependent = cli.create_issue(["--title", "Dangling work"]).unwrap();

    let missing_id = "00000000-0000-4000-8000-00000000dead";
    let issue_path = cli
        .root()
        .join(".jit")
        .join("issues")
        .join(format!("{dependent}.json"));
    let mut issue_data: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&issue_path).unwrap()).unwrap();
    issue_data["state"] = serde_json::json!("backlog");
    issue_data["dependencies"] = serde_json::json!([missing_id]);
    fs::write(
        &issue_path,
        serde_json::to_string_pretty(&issue_data).unwrap(),
    )
    .unwrap();

    let output = cli
        .command(["issue", "update", &dependent, "--state", "ready", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "BLOCKED");
    let blocker = &json["error"]["details"]["blockers"][0];
    assert_eq!(blocker["type"], "dependency");
    assert_eq!(blocker["issue_id"], missing_id);
    assert_eq!(blocker["state"], "missing");
}

#[test]
fn test_issue_update_classified_failure_reports_its_code_in_json_and_human_modes() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["-t", "Gated"]).unwrap();
    let update = |json: bool| {
        let mut args = vec!["issue", "update", &id, "--add-gate", "unregistered-gate"];
        args.extend(json.then_some("--json"));
        cli.command(args).output().unwrap()
    };
    let expected = jit::output::ErrorCode::GateNotFound;

    let json_run = update(true);
    let human_run = update(false);

    let envelope: serde_json::Value = serde_json::from_slice(&json_run.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], String::from(expected));
    assert_eq!(json_run.status.code(), Some(expected.exit_code().code()));
    assert_eq!(human_run.status.code(), json_run.status.code());
}
