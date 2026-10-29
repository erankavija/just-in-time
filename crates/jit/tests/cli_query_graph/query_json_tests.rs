use jit::test_cli::JitCli;
use jit::test_taxonomy::setup_test_repo_with_taxonomy;

#[test]
fn test_query_ready_json_output() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    cli.create_issue(["-t", "Ready Task", "-d", "Test"])
        .unwrap();

    let json = cli.run_json(["query", "available"]).unwrap();

    assert!(json["issues"].is_array());
    assert_eq!(json["count"], 1);
}

#[test]
fn test_query_blocked_json_output() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id1 = cli.create_issue(["-t", "Task A", "-d", "First"]).unwrap();
    let id2 = cli.create_issue(["-t", "Task B", "-d", "Second"]).unwrap();
    // id2 depends on id1
    cli.run(["dep", "add", &id2, &id1]).unwrap();

    let json = cli.run_json(["query", "blocked"]).unwrap();

    assert!(json["issues"].is_array());
    assert_eq!(json["count"], 1);
    assert!(json["issues"][0]["blocked_reasons"].is_array());

    // blocked_reasons is now an array of strings, not objects
    let blocked_reasons = json["issues"][0]["blocked_reasons"].as_array().unwrap();
    assert_eq!(blocked_reasons.len(), 1);
    let reason_str = blocked_reasons[0].as_str().unwrap();
    assert!(reason_str.starts_with("dependency:"));
}

#[test]
fn test_query_assignee_json_output() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id = cli.create_issue(["-t", "Task", "-d", "Test"]).unwrap();
    cli.run(["issue", "assign", &id, "copilot:test"]).unwrap();

    let json = cli
        .run_json(["query", "all", "--assignee", "copilot:test"])
        .unwrap();

    assert!(json["issues"].is_array());
    assert_eq!(json["count"], 1);
}

#[test]
fn test_query_state_json_output() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    // Issues default to ready when unblocked
    cli.create_issue(["-t", "Task", "-d", "Test"]).unwrap();

    let json = cli.run_json(["query", "all", "--state", "ready"]).unwrap();

    assert!(json["issues"].is_array());
    assert_eq!(json["count"], 1);
}

#[test]
fn test_query_priority_json_output() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    cli.create_issue(["-t", "Urgent", "-d", "Test", "-p", "high"])
        .unwrap();

    let json = cli
        .run_json(["query", "all", "--priority", "high"])
        .unwrap();

    assert!(json["issues"].is_array());
    assert_eq!(json["count"], 1);
}

/// A query filter placed before a subcommand, where it would be silently
/// dropped, is refused; under `--json` as an `INVALID_ARGUMENT` envelope.
#[test]
fn test_query_parent_filter_before_subcommand_rejected() {
    let cli = JitCli::initialised().unwrap();

    let human = cli
        .command(["query", "--state", "ready", "available"])
        .output()
        .unwrap();
    assert!(!human.status.success());
    assert!(String::from_utf8_lossy(&human.stderr)
        .contains("were given before the `available` subcommand"));

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
