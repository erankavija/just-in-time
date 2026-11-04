use jit::test_cli::JitCli;
use jit::test_taxonomy::setup_test_repo_with_taxonomy;

#[test]
fn test_query_blocked_json_output() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id1 = cli.create_issue(["-t", "Task A", "-d", "First"]).unwrap();
    let id2 = cli.create_issue(["-t", "Task B", "-d", "Second"]).unwrap();
    // id2 depends on id1
    cli.run(["dep", "add", &id2, &id1]).unwrap();

    let json = cli.run_json(["query", "blocked"]).unwrap();

    assert_eq!(json["issues"].as_array().unwrap().len(), 1);
    let blocked_reasons = json["issues"][0]["blocked_reasons"].as_array().unwrap();
    assert_eq!(blocked_reasons.len(), 1);
    let reason_str = blocked_reasons[0].as_str().unwrap();
    assert!(reason_str.starts_with("dependency:"));
}

#[test]
fn test_query_assignee_filter_rejects_malformed_value() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    cli.create_issue(["-t", "Task", "-d", "Test"]).unwrap();

    let output = cli
        .command(["query", "all", "--assignee", "malformed", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("type:identifier")));
}

#[test]
fn test_issue_assign_rejects_malformed_assignee() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id = cli.create_issue(["-t", "Task", "-d", "Test"]).unwrap();

    let output = cli
        .command(["issue", "assign", &id, "malformed", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("type:identifier")));
}

#[test]
fn test_issue_update_filter_rejects_malformed_assignee() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    cli.create_issue(["-t", "Task", "-d", "Test"]).unwrap();

    let output = cli
        .command([
            "issue",
            "update",
            "--filter",
            "assignee:malformed",
            "--priority",
            "high",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("Invalid assignee")));
}

#[test]
fn test_query_parent_filter_before_subcommand_is_an_invalid_argument_envelope() {
    let cli = JitCli::initialised().unwrap();

    let machine = cli
        .command(["query", "--state", "ready", "--json", "available"])
        .output()
        .unwrap();
    assert!(!machine.status.success());
    let json: serde_json::Value = serde_json::from_slice(&machine.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("were given before"));
}
