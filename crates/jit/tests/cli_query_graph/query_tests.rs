use jit::domain::Priority;
use jit::test_cli::JitCli;
use jit::test_taxonomy::setup_test_repo_with_taxonomy;

#[test]
fn test_query_ready_returns_unblocked_issues() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    cli.create_issue(["-t", "Task 1", "-d", "Ready task"])
        .unwrap();
    let id2 = cli
        .create_issue(["-t", "Task 2", "-d", "In progress task"])
        .unwrap();
    cli.run([
        "gate",
        "define",
        "test-gate",
        "--title",
        "Test",
        "--description",
        "Test",
    ])
    .unwrap();
    cli.create_issue(["-t", "Task 3", "-d", "Blocked task", "--gate", "test-gate"])
        .unwrap();

    // id1 is auto-ready (no blockers), id2 move to in_progress, id3 is also ready (gates don't block Ready)
    cli.run(["issue", "update", &id2, "--state", "in_progress"])
        .unwrap();

    let json = cli.run_json(["query", "available"]).unwrap();

    // Should return two ready issues (id1 and id3, gates don't block Ready state)
    assert_eq!(json["count"], 2);
}

#[test]
fn test_query_blocked_returns_issues_with_reasons() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let parent_id = cli
        .create_issue(["-t", "Parent", "-d", "Dependency"])
        .unwrap();
    let child_id = cli
        .create_issue(["-t", "Child", "-d", "Depends on parent"])
        .unwrap();
    cli.run(["dep", "add", &child_id, &parent_id]).unwrap();

    let json = cli.run_json(["query", "blocked"]).unwrap();

    // Child should be blocked
    assert!(json["count"].as_u64().unwrap() >= 1);
    let child_issue = json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == child_id)
        .unwrap();

    assert!(!child_issue["blocked_reasons"]
        .as_array()
        .unwrap()
        .is_empty());
    // blocked_reasons are now strings like "dependency:abc123 (Title:State)"
    let first_reason = child_issue["blocked_reasons"][0].as_str().unwrap();
    assert!(first_reason.starts_with("dependency:"));
}

#[test]
fn test_query_by_assignee() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id1 = cli
        .create_issue(["-t", "Task 1", "-d", "For worker 1"])
        .unwrap();
    let id2 = cli
        .create_issue(["-t", "Task 2", "-d", "For worker 2"])
        .unwrap();
    cli.run(["issue", "claim", &id1, "agent:worker-1"]).unwrap();
    cli.run(["issue", "claim", &id2, "agent:worker-2"]).unwrap();

    // Use --full to get assignee field in response
    let json = cli
        .run_json(["query", "all", "--assignee", "agent:worker-1", "--full"])
        .unwrap();

    assert_eq!(json["count"], 1);
    assert_eq!(json["issues"][0]["id"], id1);
    assert_eq!(json["issues"][0]["assignee"], "agent:worker-1");
}

#[test]
fn test_issue_release() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id = cli
        .create_issue(["-t", "Task", "-d", "Test release"])
        .unwrap();
    cli.run(["issue", "claim", &id, "agent:worker-1"]).unwrap();

    cli.run(["issue", "release", &id, "timeout"]).unwrap();

    let json = cli.run_json(["issue", "show", &id]).unwrap();
    assert!(json["assignee"].is_null());
}

#[test]
fn test_query_by_state() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    cli.create_issue(["-t", "Task 1", "-d", "Open"]).unwrap();
    let id2 = cli.create_issue(["-t", "Task 2", "-d", "Done"]).unwrap();
    cli.run(["issue", "update", &id2, "--state", "done"])
        .unwrap();

    let json = cli.run_json(["query", "all", "--state", "done"]).unwrap();

    assert_eq!(json["count"], 1);
    assert_eq!(json["issues"][0]["id"], id2);
    assert_eq!(json["issues"][0]["state"], "done");
}

#[test]
fn test_query_by_priority() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    cli.create_issue(["-t", "Critical", "-d", "Urgent", "--priority", "critical"])
        .unwrap();
    cli.create_issue(["-t", "Low", "-d", "Not urgent", "--priority", "low"])
        .unwrap();

    let json = cli
        .run_json(["query", "all", "--priority", "critical"])
        .unwrap();

    assert_eq!(json["count"], 1);
    assert_eq!(json["issues"][0]["priority"], "critical");
}

#[test]
fn test_query_closed_returns_done_and_rejected() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id1 = cli
        .create_issue(["-t", "Completed", "-d", "Done task"])
        .unwrap();
    let id2 = cli
        .create_issue(["-t", "Rejected", "-d", "Won't do"])
        .unwrap();
    cli.create_issue(["-t", "Ready", "-d", "Still open"])
        .unwrap();
    cli.run(["issue", "update", &id1, "--state", "done"])
        .unwrap();
    cli.run(["issue", "reject", &id2]).unwrap();

    let json = cli.run_json(["query", "closed"]).unwrap();

    // Should return both Done and Rejected
    assert_eq!(json["count"], 2);

    let issue_ids: Vec<String> = json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect();

    assert!(issue_ids.contains(&id1));
    assert!(issue_ids.contains(&id2));
}

#[test]
fn test_query_available_sorts_by_priority() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    // Create one issue per priority, lowest first.
    let ids: Vec<String> = Priority::ALL
        .iter()
        .map(|priority| {
            cli.create_issue([
                "-t",
                &format!("{priority} priority task"),
                "-d",
                "d",
                "--priority",
                priority.as_str(),
            ])
            .unwrap()
        })
        .collect();

    let json = cli.run_json(["query", "available"]).unwrap();

    assert_eq!(json["count"], Priority::ALL.len());
    let issues = json["issues"].as_array().unwrap();

    // Highest priority first.
    issues
        .iter()
        .zip(Priority::ALL.iter().zip(&ids).rev())
        .for_each(|(issue, (priority, id))| {
            assert_eq!(issue["id"], *id);
            assert_eq!(issue["priority"], priority.as_str());
        });
}

// ── query_all: combined filters ───────────────────────────────────────────────

#[test]
fn test_query_all_no_filters_returns_all() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    for title in ["Task A", "Task B", "Task C"] {
        cli.create_issue(["-t", title, "-d", "desc"]).unwrap();
    }

    let json = cli.run_json(["query", "all"]).unwrap();

    assert_eq!(json["count"], 3);
}

#[test]
fn test_query_all_combined_state_and_priority() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id_high_done = cli
        .create_issue(["-t", "High Done", "-d", "d", "--priority", "high"])
        .unwrap();
    // high + ready (should not appear)
    cli.create_issue(["-t", "High Ready", "-d", "d", "--priority", "high"])
        .unwrap();
    // low + done (should not appear)
    let id_low_done = cli
        .create_issue(["-t", "Low Done", "-d", "d", "--priority", "low"])
        .unwrap();
    for id in [&id_high_done, &id_low_done] {
        cli.run(["issue", "update", id, "--state", "done"]).unwrap();
    }

    let json = cli
        .run_json(["query", "all", "--state", "done", "--priority", "high"])
        .unwrap();

    assert_eq!(json["count"], 1);
    assert_eq!(json["issues"][0]["id"], id_high_done);
}

#[test]
fn test_query_all_combined_state_and_assignee() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id_a = cli.create_issue(["-t", "Task A", "-d", "d"]).unwrap();
    let id_b = cli.create_issue(["-t", "Task B", "-d", "d"]).unwrap();
    // Assign both to the same agent without changing state
    for id in [&id_a, &id_b] {
        cli.run(["issue", "update", id, "--assignee", "agent:worker-1"])
            .unwrap();
    }
    // Move only id_a to in_progress
    cli.run(["issue", "update", &id_a, "--state", "in_progress"])
        .unwrap();

    let json = cli
        .run_json([
            "query",
            "all",
            "--state",
            "in_progress",
            "--assignee",
            "agent:worker-1",
            "--full",
        ])
        .unwrap();

    assert_eq!(json["count"], 1);
    assert_eq!(json["issues"][0]["id"], id_a);
}

#[test]
fn test_query_all_combined_state_and_label() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    let id_tagged = cli
        .create_issue(["-t", "Tagged Ready", "-d", "d", "--label", "component:api"])
        .unwrap();
    // ready but no label — should not appear
    cli.create_issue(["-t", "Untagged Ready", "-d", "d"])
        .unwrap();
    // tagged but done — should not appear
    let id_tagged_done = cli
        .create_issue(["-t", "Tagged Done", "-d", "d", "--label", "component:api"])
        .unwrap();
    cli.run(["issue", "update", &id_tagged_done, "--state", "done"])
        .unwrap();

    let json = cli
        .run_json([
            "query",
            "all",
            "--state",
            "ready",
            "--label",
            "component:api",
        ])
        .unwrap();

    assert_eq!(json["count"], 1);
    assert_eq!(json["issues"][0]["id"], id_tagged);
}

// ── REQ-04: bare `jit query` and `jit query ready` alias ─────────────────────

/// Bare `jit query --json` returns all issues via the query-all path.
#[test]
fn test_query_bare_returns_all_issues() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    for title in ["Alpha", "Beta", "Gamma"] {
        cli.create_issue(["-t", title, "-d", "desc"]).unwrap();
    }

    let json = cli.run_json(["query"]).unwrap();

    assert_eq!(json["count"], 3, "bare query should return all 3 issues");
}

/// `jit query ready` is accepted and returns the same count as `jit query available`.
#[test]
fn test_query_ready_alias_matches_available() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);
    for title in ["Ready One", "Ready Two"] {
        cli.create_issue(["-t", title, "-d", "desc"]).unwrap();
    }

    let json_available = cli.run_json(["query", "available"]).unwrap();
    let json_ready = cli.run_json(["query", "ready"]).unwrap();

    assert_eq!(
        json_ready["count"], json_available["count"],
        "`jit query ready` count should match `jit query available`"
    );
    assert_eq!(
        json_ready["issues"], json_available["issues"],
        "`jit query ready` issues should match `jit query available`"
    );
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

    // The bare form (no subcommand) must still apply the same filter cleanly.
    cli.run(["query", "--state", "ready", "--json"])
        .expect("bare `jit query --state ready` must still work");
}

/// `--json` is global, so placing it before a query subcommand selects JSON
/// rendering rather than tripping the misplaced-parent-filter guard.
#[test]
fn test_query_accepts_global_json_before_subcommand() {
    let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

    let leading = cli
        .run(["query", "--json", "available"])
        .expect("`jit query --json available` must succeed");
    let envelope: serde_json::Value = serde_json::from_str(&leading).unwrap();
    assert_eq!(
        envelope["count"].as_u64(),
        envelope["issues"]
            .as_array()
            .map(|issues| issues.len() as u64),
        "list envelope must carry a count matching its issues: {envelope}"
    );

    let trailing = cli.run(["query", "available", "--json"]).unwrap();
    assert_eq!(leading, trailing);
}
