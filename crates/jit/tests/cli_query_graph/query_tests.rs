use jit::test_cli::JitCli;
use jit::test_taxonomy::setup_test_repo_with_taxonomy;

#[test]
fn test_query_blocked_wiring() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let parent_id = cli.create_issue(["-t", "Parent"]).unwrap();
    let child_id = cli.create_issue(["-t", "Child"]).unwrap();
    cli.run(["dep", "add", &child_id, &parent_id]).unwrap();

    let json = cli.run_json(["query", "blocked"]).unwrap();

    assert_eq!(json["issues"][0]["id"], child_id);
}

#[test]
fn test_query_all_assignee_flag() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id = cli.create_issue(["-t", "Assigned"]).unwrap();
    cli.create_issue(["-t", "Unassigned"]).unwrap();
    cli.run(["issue", "assign", &id, "agent:worker-1"]).unwrap();

    let json = cli
        .run_json(["query", "all", "--assignee", "agent:worker-1", "--full"])
        .unwrap();

    assert_eq!(json["issues"].as_array().unwrap().len(), 1);
    assert_eq!(json["issues"][0]["id"], id);
    assert_eq!(json["issues"][0]["assignee"], "agent:worker-1");
}

#[test]
fn test_query_all_state_flag() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    cli.create_issue(["-t", "Open"]).unwrap();
    let id = cli.create_issue(["-t", "Done"]).unwrap();
    cli.run(["issue", "update", &id, "--state", "done"])
        .unwrap();

    let json = cli.run_json(["query", "all", "--state", "done"]).unwrap();

    assert_eq!(json["issues"].as_array().unwrap().len(), 1);
    assert_eq!(json["issues"][0]["id"], id);
}

#[test]
fn test_query_all_priority_flag() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id = cli
        .create_issue(["-t", "Urgent", "--priority", "critical"])
        .unwrap();
    cli.create_issue(["-t", "Normal"]).unwrap();

    let json = cli
        .run_json(["query", "all", "--priority", "critical"])
        .unwrap();

    assert_eq!(json["issues"].as_array().unwrap().len(), 1);
    assert_eq!(json["issues"][0]["id"], id);
}

#[test]
fn test_query_closed_wiring() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id = cli.create_issue(["-t", "Completed"]).unwrap();
    cli.create_issue(["-t", "Open"]).unwrap();
    cli.run(["issue", "update", &id, "--state", "done"])
        .unwrap();

    let json = cli.run_json(["query", "closed"]).unwrap();

    assert_eq!(json["issues"].as_array().unwrap().len(), 1);
    assert_eq!(json["issues"][0]["id"], id);
}

#[test]
fn test_query_available_wiring() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id = cli.create_issue(["-t", "Available"]).unwrap();
    let assigned = cli.create_issue(["-t", "Assigned"]).unwrap();
    cli.run(["issue", "assign", &assigned, "agent:worker-1"])
        .unwrap();

    let json = cli.run_json(["query", "available"]).unwrap();

    assert_eq!(json["issues"].as_array().unwrap().len(), 1);
    assert_eq!(json["issues"][0]["id"], id);
}

#[test]
fn test_query_all_wiring() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id = cli.create_issue(["-t", "Task"]).unwrap();

    let json = cli.run_json(["query", "all"]).unwrap();

    assert_eq!(json["issues"][0]["id"], id);
}

#[test]
fn test_query_strategic_wiring() {
    let repo = setup_test_repo_with_taxonomy();
    let label = format!("type:{}", repo.taxonomy.strategic_types[0]);
    let cli = JitCli::with_repository(repo.temp);
    let id = cli
        .create_issue(["-t", "Strategic", "--label", &label])
        .unwrap();
    cli.create_issue(["-t", "Task"]).unwrap();

    let json = cli.run_json(["query", "strategic"]).unwrap();

    assert_eq!(json["issues"].as_array().unwrap().len(), 1);
    assert_eq!(json["issues"][0]["id"], id);
}

#[test]
fn test_query_bare_wiring() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id = cli.create_issue(["-t", "Task"]).unwrap();

    let json = cli.run_json(["query"]).unwrap();

    assert_eq!(json["issues"][0]["id"], id);
}

#[test]
fn test_query_ready_alias_matches_available() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id = cli.create_issue(["-t", "Ready"]).unwrap();

    let available = cli.run_json(["query", "available"]).unwrap();
    let ready = cli.run_json(["query", "ready"]).unwrap();

    assert_eq!(ready["issues"][0]["id"], id);
    assert_eq!(ready["issues"], available["issues"]);
}

/// A parent-level filter placed before a query subcommand must be rejected with
/// an actionable error, not silently ignored. `jit query -s ready available`
/// (and the `--state`/`--priority`/etc. long forms) would otherwise drop the
/// filter and return wrong results to `--json` consumers.
#[test]
fn test_query_subcommand_rejects_parent_filter() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    // Filter supplied BEFORE the subcommand binds to the parent `query` and is
    // not honored by the subcommand — this must error rather than run.
    let output = cli
        .command(["query", "-s", "ready", "available"])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "parent filter before subcommand should be rejected, but it succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--state") && stderr.contains("available"),
        "error should name the ignored filter and the subcommand, got: {stderr}"
    );
    assert!(
        stderr.contains("bare form") || stderr.contains("after the subcommand"),
        "error should be actionable, got: {stderr}"
    );
}

/// `--json` is global, so placing it before a query subcommand selects JSON
/// rendering rather than tripping the misplaced-parent-filter guard.
#[test]
fn test_query_accepts_global_json_before_subcommand() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id = cli.create_issue(["-t", "Task"]).unwrap();

    let leading = cli
        .run(["query", "--json", "available"])
        .expect("`jit query --json available` must succeed");
    let envelope: serde_json::Value = serde_json::from_str(&leading).unwrap();
    assert_eq!(envelope["issues"][0]["id"], id);

    let trailing = cli.run(["query", "available", "--json"]).unwrap();
    assert_eq!(leading, trailing);
}
