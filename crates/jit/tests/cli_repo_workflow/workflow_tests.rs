//! End-to-end workflow integration tests
//!
//! These tests verify complete user/agent workflows by running actual CLI commands.

use jit::test_cli::JitCli;

// ============================================================================
// Workflow Test: Simple Task Completion
// ============================================================================

#[test]
fn test_workflow_simple_task_completion() {
    let cli = JitCli::initialised().unwrap();

    // 1. Create a task (auto-transitions to Ready since no blockers)
    let id = cli
        .create_issue(["-t", "Fix bug", "--priority", "high"])
        .unwrap();

    // 2. Verify it's in ready state (auto-transitioned)
    let output = cli.command(["issue", "show", &id]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("State: ready"));

    // 3. Claim the task
    let output = cli
        .command(["issue", "claim", &id, "agent:worker-1"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Failed to claim: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // 4. Verify assignment
    let output = cli.command(["issue", "show", &id]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("agent:worker-1"),
        "Should contain assignee, got: {}",
        stdout
    );
    assert!(stdout.contains("State: in_progress"));

    // 5. Complete the task
    let output = cli
        .command(["issue", "update", &id, "--state", "done"])
        .output()
        .unwrap();
    assert!(output.status.success());

    // 6. Verify completion
    let output = cli.command(["issue", "show", &id]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("State: done"));
}

/// Human `issue show` prints the same state and priority tokens as `--json`.
#[test]
fn test_issue_show_human_and_json_render_same_vocabulary_tokens() {
    let cli = JitCli::initialised().unwrap();
    let id = cli
        .create_issue(["-t", "Token parity", "--priority", "high"])
        .unwrap();
    assert!(cli
        .command(["issue", "claim", &id, "agent:worker-1"])
        .output()
        .unwrap()
        .status
        .success());

    let json: serde_json::Value = serde_json::from_slice(
        &cli.command(["issue", "show", &id, "--json"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let human =
        String::from_utf8_lossy(&cli.command(["issue", "show", &id]).output().unwrap().stdout)
            .into_owned();

    let state = json["state"].as_str().unwrap();
    let priority = json["priority"].as_str().unwrap();
    assert_eq!(state, "in_progress");
    assert!(human.contains(&format!("State: {state}")), "got: {human}");
    assert!(
        human.contains(&format!("Priority: {priority}")),
        "got: {human}"
    );
}

// ============================================================================
// Workflow Test: Agent Claiming Next Available Task
// ============================================================================

#[test]
fn test_workflow_agent_claim_next() {
    let cli = JitCli::initialised().unwrap();

    // Create multiple ready tasks with different priorities
    let low_id = cli
        .create_issue(["-t", "Low priority", "--priority", "low"])
        .unwrap();
    cli.command(["issue", "update", &low_id, "--state", "ready"])
        .output()
        .unwrap();

    let high_id = cli
        .create_issue(["-t", "High priority", "--priority", "high"])
        .unwrap();
    cli.command(["issue", "update", &high_id, "--state", "ready"])
        .output()
        .unwrap();

    let critical_id = cli
        .create_issue(["-t", "Critical", "--priority", "critical"])
        .unwrap();
    cli.command(["issue", "update", &critical_id, "--state", "ready"])
        .output()
        .unwrap();

    // Agent claims next (should get critical)
    let output = cli
        .command(["issue", "claim-next", "agent:worker-1"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "claim-next failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&critical_id));

    // Verify critical task is claimed
    let output = cli
        .command(["issue", "show", &critical_id])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("agent:worker-1"));

    // Agent claims next again (should get high)
    let output = cli
        .command(["issue", "claim-next", "agent:worker-2"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "claim-next failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&high_id));
}

// ============================================================================
// Workflow Test: Dependency Chain Unblocking
// ============================================================================

#[test]
fn test_workflow_dependency_chain_unblocking() {
    let cli = JitCli::initialised().unwrap();

    // Create a chain: task3 depends on task2 depends on task1
    let task1 = cli.create_issue(["-t", "Task 1"]).unwrap();

    let task2 = cli.create_issue(["-t", "Task 2"]).unwrap();

    let task3 = cli.create_issue(["-t", "Task 3"]).unwrap();

    // Create dependencies
    cli.command(["dep", "add", &task2, &task1])
        .output()
        .unwrap();
    cli.command(["dep", "add", &task3, &task2])
        .output()
        .unwrap();

    // Query ready - should only see task1
    let output = cli.command(["query", "available"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&task1));
    assert!(!stdout.contains(&task2));
    assert!(!stdout.contains(&task3));

    // Complete task1
    cli.command(["issue", "update", &task1, "--state", "done"])
        .output()
        .unwrap();

    // Query ready - should now see task2
    let output = cli.command(["query", "available"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(&task1)); // done, not ready
    assert!(stdout.contains(&task2));
    assert!(!stdout.contains(&task3));

    // Complete task2
    cli.command(["issue", "update", &task2, "--state", "done"])
        .output()
        .unwrap();

    // Query ready - should now see task3
    let output = cli.command(["query", "available"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&task3));
}

// ============================================================================
// Workflow Test: Gate Workflow (Manual and Auto)
// ============================================================================

#[test]
fn test_workflow_gates() {
    let cli = JitCli::initialised().unwrap();

    // Add gate definitions
    cli.command([
        "gate",
        "define",
        "review",
        "--title",
        "Code Review",
        "--description",
        "Manual review",
    ])
    .output()
    .unwrap();

    cli.command([
        "gate",
        "define",
        "tests",
        "--title",
        "Tests",
        "--description",
        "Unit tests",
        "--auto",
        "--checker-command",
        "true",
    ])
    .output()
    .unwrap();

    // Create issue with both gates
    let id = cli
        .create_issue(["-t", "Feature", "--gate", "review,tests"])
        .unwrap();

    // Issue should NOT be blocked (gates don't block ready state)
    let output = cli.command(["query", "blocked"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(&id));

    // Issue should be ready
    let output = cli.command(["query", "available"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&id));

    // Attempt to complete - should transition to Gated
    cli.command(["issue", "update", &id, "--state", "done"])
        .output()
        .unwrap();
    let output = cli.command(["issue", "show", &id]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("gated"));

    // Evaluate tests gate
    cli.command(["gate", "evaluate", &id, "tests"])
        .output()
        .unwrap();

    // Still in Gated (review not passed)
    let output = cli.command(["issue", "show", &id]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("gated"));

    // Evaluate review gate
    cli.command(["gate", "evaluate", &id, "review", "--by", "human:reviewer"])
        .output()
        .unwrap();

    // Now should auto-transition to Done
    let output = cli.command(["issue", "show", &id]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("done"));
}

// ============================================================================
// Workflow Test: Task Release and Reassignment
// ============================================================================

#[test]
fn test_workflow_task_release_and_reassignment() {
    let cli = JitCli::initialised().unwrap();

    // Create and claim a task (auto-transitions to Ready)
    let id = cli.create_issue(["-t", "Task"]).unwrap();
    cli.command(["issue", "claim", &id, "agent:worker-1"])
        .output()
        .unwrap();

    // Verify claimed
    let output = cli
        .command(["query", "all", "--assignee", "agent:worker-1"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&id));

    // Release with reason
    let output = cli
        .command(["issue", "release", &id, "timeout"])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Verify released (back to ready)
    let output = cli.command(["query", "available"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&id));

    // Different agent can claim it
    let output = cli
        .command(["issue", "claim", &id, "agent:worker-2"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let output = cli
        .command(["query", "all", "--assignee", "agent:worker-2"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&id));
}

// ============================================================================
// Workflow Test: Complex Epic with Multiple Dependencies and Gates
// ============================================================================

#[test]
fn test_workflow_complex_epic() {
    let cli = JitCli::initialised().unwrap();

    // Setup gates
    cli.command([
        "gate",
        "define",
        "tests",
        "--title",
        "Tests",
        "--description",
        "Tests",
        "--auto",
        "--checker-command",
        "true",
    ])
    .output()
    .unwrap();
    cli.command([
        "gate",
        "define",
        "review",
        "--title",
        "Review",
        "--description",
        "Review",
    ])
    .output()
    .unwrap();

    // Create feature tasks
    let backend = cli
        .create_issue(["-t", "Backend API", "--gate", "tests"])
        .unwrap();

    let frontend = cli
        .create_issue(["-t", "Frontend UI", "--gate", "tests"])
        .unwrap();

    let docs = cli
        .create_issue(["-t", "Documentation", "--gate", "review"])
        .unwrap();

    // Create epic that depends on all
    let epic = cli
        .create_issue(["-t", "Release v1.0", "--priority", "critical"])
        .unwrap();

    cli.command(["dep", "add", &epic, &backend])
        .output()
        .unwrap();
    cli.command(["dep", "add", &epic, &frontend])
        .output()
        .unwrap();
    cli.command(["dep", "add", &epic, &docs]).output().unwrap();

    // Epic should be blocked
    let output = cli.command(["query", "blocked"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&epic));

    // Complete backend
    cli.command(["gate", "evaluate", &backend, "tests"])
        .output()
        .unwrap();
    cli.command(["issue", "update", &backend, "--state", "done"])
        .output()
        .unwrap();

    // Complete frontend
    cli.command(["gate", "evaluate", &frontend, "tests"])
        .output()
        .unwrap();
    cli.command(["issue", "update", &frontend, "--state", "done"])
        .output()
        .unwrap();

    // Complete docs
    cli.command([
        "gate",
        "evaluate",
        &docs,
        "review",
        "--by",
        "human:reviewer",
    ])
    .output()
    .unwrap();
    cli.command(["issue", "update", &docs, "--state", "done"])
        .output()
        .unwrap();

    // Epic should now be unblocked and ready
    let output = cli.command(["query", "available"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&epic));
}

// ============================================================================
// Workflow Test: Error Handling
// ============================================================================

#[test]
fn test_workflow_error_scenarios() {
    let cli = JitCli::initialised().unwrap();

    // Try to show non-existent issue
    let output = cli
        .command(["issue", "show", "nonexistent"])
        .output()
        .unwrap();
    assert!(!output.status.success());

    // Try to create cycle
    let id1 = cli.create_issue(["-t", "Task 1"]).unwrap();

    let id2 = cli.create_issue(["-t", "Task 2"]).unwrap();

    cli.command(["dep", "add", &id2, &id1]).output().unwrap();

    let output = cli.command(["dep", "add", &id1, &id2]).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cycle") || stderr.contains("Cycle"));

    // Try to claim already claimed issue
    let id = cli.create_issue(["-t", "Task"]).unwrap();
    cli.command(["issue", "update", &id, "--state", "ready"])
        .output()
        .unwrap();
    cli.command(["issue", "claim", &id, "agent:worker-1"])
        .output()
        .unwrap();

    let output = cli
        .command(["issue", "claim", &id, "agent:worker-2"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // REQ-02 (jit:30a3b5c1): the already-assigned error names the current
    // holder and states that re-claiming as that same holder succeeds.
    assert!(
        stderr.contains("agent:worker-1"),
        "must name the current holder, got: {stderr}"
    );
    assert!(
        stderr.contains("re-claiming as agent:worker-1 succeeds"),
        "must hint that re-claiming as the same holder succeeds, got: {stderr}"
    );
}

// ============================================================================
// Workflow Test: Graph Visualization
// ============================================================================

#[test]
fn test_workflow_graph_visualization() {
    let cli = JitCli::initialised().unwrap();

    // Create dependency graph
    let foundation = cli.create_issue(["-t", "Foundation"]).unwrap();

    let feature_a = cli.create_issue(["-t", "Feature A"]).unwrap();

    let feature_b = cli.create_issue(["-t", "Feature B"]).unwrap();

    let integration = cli.create_issue(["-t", "Integration"]).unwrap();

    cli.command(["dep", "add", &feature_a, &foundation])
        .output()
        .unwrap();
    cli.command(["dep", "add", &feature_b, &foundation])
        .output()
        .unwrap();
    cli.command(["dep", "add", &integration, &feature_a])
        .output()
        .unwrap();
    cli.command(["dep", "add", &integration, &feature_b])
        .output()
        .unwrap();

    // Show graph for foundation (use graph deps instead of removed graph show)
    let output = cli
        .command(["graph", "deps", &foundation])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "graph deps failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Show roots (should be foundation)
    let output = cli.command(["graph", "roots"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&foundation));
    assert!(!stdout.contains(&integration));

    // Show downstream of foundation (--depth 0 opts in to unbounded traversal)
    let output = cli
        .command(["graph", "downstream", &foundation, "--depth", "0"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&feature_a));
    assert!(stdout.contains(&feature_b));
    assert!(stdout.contains(&integration));
}

// ============================================================================
// Workflow Test: Auto-transition to Ready on Done dependency
// ============================================================================

#[test]
fn test_auto_transition_when_dependency_done() {
    let cli = JitCli::initialised().unwrap();

    // Create two issues: B depends on A
    let task_a = cli
        .create_issue(["-t", "Task A", "--priority", "normal"])
        .unwrap();

    let task_b = cli
        .create_issue(["-t", "Task B", "--priority", "normal"])
        .unwrap();

    // Add dependency: B depends on A
    let output = cli
        .command(["dep", "add", &task_b, &task_a])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Verify B is in Backlog
    let output = cli.command(["issue", "show", &task_b]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("State: backlog"));

    // Complete task A (terminal state, should unblock B)
    let output = cli
        .command(["issue", "update", &task_a, "--state", "done"])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Verify B auto-transitioned to Ready
    let output = cli.command(["issue", "show", &task_b]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("State: ready"),
        "Task B should auto-transition to Ready when dependency is done"
    );
}

// ============================================================================
// Workflow Test: Reject convenience command
// ============================================================================

#[test]
fn test_reject_convenience_command() {
    let cli = JitCli::initialised().unwrap();

    // Create an issue
    let task_id = cli
        .create_issue(["-t", "Won't fix task", "--priority", "low"])
        .unwrap();

    // Reject it with reason
    let output = cli
        .command(["issue", "reject", &task_id, "--reason", "wont-fix"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rejected issue"));
    assert!(stdout.contains("wont-fix"));

    // Verify state is Rejected and has resolution label (check with --json)
    let output = cli
        .command(["issue", "show", &task_id, "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"state\":\"rejected\"") || stdout.contains("\"state\": \"rejected\"")
    );
    assert!(stdout.contains("resolution:wont-fix"));
}

#[test]
fn test_reject_without_reason() {
    let cli = JitCli::initialised().unwrap();

    // Create an issue
    let task_id = cli
        .create_issue(["-t", "Task", "--priority", "normal"])
        .unwrap();

    // Reject without reason
    let output = cli.command(["issue", "reject", &task_id]).output().unwrap();
    assert!(output.status.success());

    // Verify state is Rejected but no resolution label
    let output = cli
        .command(["issue", "show", &task_id, "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"state\":\"rejected\"") || stdout.contains("\"state\": \"rejected\"")
    );
    assert!(!stdout.contains("resolution:"));
}

#[test]
fn test_short_hash_with_json_flag() {
    let cli = JitCli::initialised().unwrap();

    // Create an issue
    let full_id = cli
        .create_issue(["-t", "Test short hash", "--priority", "low"])
        .unwrap();
    let short_hash = &full_id[..8];

    // Test update with short hash and --json (previously broken)
    let output = cli
        .command([
            "issue",
            "update",
            short_hash,
            "--json",
            "--state",
            "in_progress",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Short hash with --json should work"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&full_id), "Should return full ID in JSON");
    assert!(stdout.contains("in_progress"));

    // Test other commands with short hash and --json
    let output = cli
        .command(["issue", "show", short_hash, "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let output = cli
        .command(["issue", "assign", short_hash, "agent:test", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let output = cli
        .command(["issue", "unassign", short_hash, "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let output = cli
        .command(["issue", "reject", short_hash, "--reason", "test", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
}
