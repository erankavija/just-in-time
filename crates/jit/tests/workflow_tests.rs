//! End-to-end workflow integration tests
//!
//! These tests verify complete user/agent workflows by running actual CLI commands.

use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/jit")
        .to_string_lossy()
        .to_string()
}

fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let jit = jit_binary();
    Command::new(&jit)
        .args(["init"])
        .current_dir(temp.path())
        .status()
        .unwrap();
    temp
}

fn run_jit(temp: &TempDir, args: &[&str]) -> std::process::Output {
    Command::new(jit_binary())
        .args(args)
        .current_dir(temp.path())
        .output()
        .unwrap()
}

fn extract_id(output: &str) -> String {
    output.split_whitespace().last().unwrap().trim().to_string()
}

// ============================================================================
// Workflow Test: Simple Task Completion
// ============================================================================

#[test]
fn test_workflow_simple_task_completion() {
    let temp = setup_test_repo();

    // 1. Create a task (auto-transitions to Ready since no blockers)
    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Fix bug", "--priority", "high"],
    );
    assert!(output.status.success());
    let id = extract_id(&String::from_utf8_lossy(&output.stdout));

    // 2. Verify it's in ready state (auto-transitioned)
    let output = run_jit(&temp, &["issue", "show", &id]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("State: Ready"));

    // 3. Claim the task
    let output = run_jit(&temp, &["issue", "claim", &id, "agent:worker-1"]);
    assert!(
        output.status.success(),
        "Failed to claim: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // 4. Verify assignment
    let output = run_jit(&temp, &["issue", "show", &id]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("agent:worker-1"),
        "Should contain assignee, got: {}",
        stdout
    );
    assert!(stdout.contains("State: InProgress"));

    // 5. Complete the task
    let output = run_jit(&temp, &["issue", "update", &id, "--state", "done"]);
    assert!(output.status.success());

    // 6. Verify completion
    let output = run_jit(&temp, &["issue", "show", &id]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("State: Done"));
}

// ============================================================================
// Workflow Test: Agent Claiming Next Available Task
// ============================================================================

#[test]
fn test_workflow_agent_claim_next() {
    let temp = setup_test_repo();

    // Create multiple ready tasks with different priorities
    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Low priority", "--priority", "low"],
    );
    let low_id = extract_id(&String::from_utf8_lossy(&output.stdout));
    run_jit(&temp, &["issue", "update", &low_id, "--state", "ready"]);

    let output = run_jit(
        &temp,
        &[
            "issue",
            "create",
            "-t",
            "High priority",
            "--priority",
            "high",
        ],
    );
    let high_id = extract_id(&String::from_utf8_lossy(&output.stdout));
    run_jit(&temp, &["issue", "update", &high_id, "--state", "ready"]);

    let output = run_jit(
        &temp,
        &[
            "issue",
            "create",
            "-t",
            "Critical",
            "--priority",
            "critical",
        ],
    );
    let critical_id = extract_id(&String::from_utf8_lossy(&output.stdout));
    run_jit(
        &temp,
        &["issue", "update", &critical_id, "--state", "ready"],
    );

    // Agent claims next (should get critical)
    let output = run_jit(&temp, &["issue", "claim-next", "agent:worker-1"]);
    assert!(
        output.status.success(),
        "claim-next failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&critical_id));

    // Verify critical task is claimed
    let output = run_jit(&temp, &["issue", "show", &critical_id]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("agent:worker-1"));

    // Agent claims next again (should get high)
    let output = run_jit(&temp, &["issue", "claim-next", "agent:worker-2"]);
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
    let temp = setup_test_repo();

    // Create a chain: task3 depends on task2 depends on task1
    let output = run_jit(&temp, &["issue", "create", "-t", "Task 1"]);
    let task1 = extract_id(&String::from_utf8_lossy(&output.stdout));

    let output = run_jit(&temp, &["issue", "create", "-t", "Task 2"]);
    let task2 = extract_id(&String::from_utf8_lossy(&output.stdout));

    let output = run_jit(&temp, &["issue", "create", "-t", "Task 3"]);
    let task3 = extract_id(&String::from_utf8_lossy(&output.stdout));

    // Create dependencies
    run_jit(&temp, &["dep", "add", &task2, &task1]);
    run_jit(&temp, &["dep", "add", &task3, &task2]);

    // Query ready - should only see task1
    let output = run_jit(&temp, &["query", "ready"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&task1));
    assert!(!stdout.contains(&task2));
    assert!(!stdout.contains(&task3));

    // Complete task1
    run_jit(&temp, &["issue", "update", &task1, "--state", "done"]);

    // Query ready - should now see task2
    let output = run_jit(&temp, &["query", "ready"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(&task1)); // done, not ready
    assert!(stdout.contains(&task2));
    assert!(!stdout.contains(&task3));

    // Complete task2
    run_jit(&temp, &["issue", "update", &task2, "--state", "done"]);

    // Query ready - should now see task3
    let output = run_jit(&temp, &["query", "ready"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&task3));
}

// ============================================================================
// Workflow Test: Gate Workflow (Manual and Auto)
// ============================================================================

#[test]
fn test_workflow_gates() {
    let temp = setup_test_repo();

    // Add gate definitions
    run_jit(
        &temp,
        &[
            "registry",
            "add",
            "review",
            "--title",
            "Code Review",
            "--desc",
            "Manual review",
        ],
    );

    run_jit(
        &temp,
        &[
            "registry",
            "add",
            "tests",
            "--title",
            "Tests",
            "--desc",
            "Unit tests",
            "--auto",
        ],
    );

    // Create issue with both gates
    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Feature", "--gate", "review,tests"],
    );
    let id = extract_id(&String::from_utf8_lossy(&output.stdout));

    // Issue should NOT be blocked (gates don't block ready state)
    let output = run_jit(&temp, &["query", "blocked"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(&id));

    // Issue should be ready
    let output = run_jit(&temp, &["query", "ready"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&id));

    // Attempt to complete - should transition to Gated
    run_jit(&temp, &["issue", "update", &id, "--state", "done"]);
    let output = run_jit(&temp, &["issue", "show", &id]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Gated"));

    // Pass tests gate
    run_jit(&temp, &["gate", "pass", &id, "tests"]);

    // Still in Gated (review not passed)
    let output = run_jit(&temp, &["issue", "show", &id]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Gated"));

    // Pass review gate
    run_jit(&temp, &["gate", "pass", &id, "review"]);

    // Now should auto-transition to Done
    let output = run_jit(&temp, &["issue", "show", &id]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Done"));
}

// ============================================================================
// Workflow Test: Task Release and Reassignment
// ============================================================================

#[test]
fn test_workflow_task_release_and_reassignment() {
    let temp = setup_test_repo();

    // Create and claim a task (auto-transitions to Ready)
    let output = run_jit(&temp, &["issue", "create", "-t", "Task"]);
    let id = extract_id(&String::from_utf8_lossy(&output.stdout));
    run_jit(&temp, &["issue", "claim", &id, "agent:worker-1"]);

    // Verify claimed
    let output = run_jit(&temp, &["query", "assignee", "agent:worker-1"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&id));

    // Release with reason
    let output = run_jit(&temp, &["issue", "release", &id, "timeout"]);
    assert!(output.status.success());

    // Verify released (back to ready)
    let output = run_jit(&temp, &["query", "ready"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&id));

    // Different agent can claim it
    let output = run_jit(&temp, &["issue", "claim", &id, "agent:worker-2"]);
    assert!(output.status.success());

    let output = run_jit(&temp, &["query", "assignee", "agent:worker-2"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&id));
}

// ============================================================================
// Workflow Test: Complex Epic with Multiple Dependencies and Gates
// ============================================================================

#[test]
fn test_workflow_complex_epic() {
    let temp = setup_test_repo();

    // Setup gates
    run_jit(
        &temp,
        &[
            "registry", "add", "tests", "--title", "Tests", "--desc", "Tests", "--auto",
        ],
    );
    run_jit(
        &temp,
        &[
            "registry", "add", "review", "--title", "Review", "--desc", "Review",
        ],
    );

    // Create feature tasks
    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Backend API", "--gate", "tests"],
    );
    let backend = extract_id(&String::from_utf8_lossy(&output.stdout));

    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Frontend UI", "--gate", "tests"],
    );
    let frontend = extract_id(&String::from_utf8_lossy(&output.stdout));

    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Documentation", "--gate", "review"],
    );
    let docs = extract_id(&String::from_utf8_lossy(&output.stdout));

    // Create epic that depends on all
    let output = run_jit(
        &temp,
        &[
            "issue",
            "create",
            "-t",
            "Release v1.0",
            "--priority",
            "critical",
        ],
    );
    let epic = extract_id(&String::from_utf8_lossy(&output.stdout));

    run_jit(&temp, &["dep", "add", &epic, &backend]);
    run_jit(&temp, &["dep", "add", &epic, &frontend]);
    run_jit(&temp, &["dep", "add", &epic, &docs]);

    // Epic should be blocked
    let output = run_jit(&temp, &["query", "blocked"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&epic));

    // Complete backend
    run_jit(&temp, &["gate", "pass", &backend, "tests"]);
    run_jit(&temp, &["issue", "update", &backend, "--state", "done"]);

    // Complete frontend
    run_jit(&temp, &["gate", "pass", &frontend, "tests"]);
    run_jit(&temp, &["issue", "update", &frontend, "--state", "done"]);

    // Complete docs
    run_jit(&temp, &["gate", "pass", &docs, "review"]);
    run_jit(&temp, &["issue", "update", &docs, "--state", "done"]);

    // Epic should now be unblocked and ready
    let output = run_jit(&temp, &["query", "ready"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&epic));
}

// ============================================================================
// Workflow Test: Error Handling
// ============================================================================

#[test]
fn test_workflow_error_scenarios() {
    let temp = setup_test_repo();

    // Try to show non-existent issue
    let output = run_jit(&temp, &["issue", "show", "nonexistent"]);
    assert!(!output.status.success());

    // Try to create cycle
    let output = run_jit(&temp, &["issue", "create", "-t", "Task 1"]);
    let id1 = extract_id(&String::from_utf8_lossy(&output.stdout));

    let output = run_jit(&temp, &["issue", "create", "-t", "Task 2"]);
    let id2 = extract_id(&String::from_utf8_lossy(&output.stdout));

    run_jit(&temp, &["dep", "add", &id2, &id1]);

    let output = run_jit(&temp, &["dep", "add", &id1, &id2]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cycle") || stderr.contains("Cycle"));

    // Try to claim already claimed issue
    let output = run_jit(&temp, &["issue", "create", "-t", "Task"]);
    let id = extract_id(&String::from_utf8_lossy(&output.stdout));
    run_jit(&temp, &["issue", "update", &id, "--state", "ready"]);
    run_jit(&temp, &["issue", "claim", &id, "agent:worker-1"]);

    let output = run_jit(&temp, &["issue", "claim", &id, "agent:worker-2"]);
    assert!(!output.status.success());
}

// ============================================================================
// Workflow Test: Graph Visualization
// ============================================================================

#[test]
fn test_workflow_graph_visualization() {
    let temp = setup_test_repo();

    // Create dependency graph
    let output = run_jit(&temp, &["issue", "create", "-t", "Foundation"]);
    let foundation = extract_id(&String::from_utf8_lossy(&output.stdout));

    let output = run_jit(&temp, &["issue", "create", "-t", "Feature A"]);
    let feature_a = extract_id(&String::from_utf8_lossy(&output.stdout));

    let output = run_jit(&temp, &["issue", "create", "-t", "Feature B"]);
    let feature_b = extract_id(&String::from_utf8_lossy(&output.stdout));

    let output = run_jit(&temp, &["issue", "create", "-t", "Integration"]);
    let integration = extract_id(&String::from_utf8_lossy(&output.stdout));

    run_jit(&temp, &["dep", "add", &feature_a, &foundation]);
    run_jit(&temp, &["dep", "add", &feature_b, &foundation]);
    run_jit(&temp, &["dep", "add", &integration, &feature_a]);
    run_jit(&temp, &["dep", "add", &integration, &feature_b]);

    // Show graph for foundation
    let output = run_jit(&temp, &["graph", "show", &foundation]);
    assert!(
        output.status.success(),
        "graph show failed: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&foundation));

    // Show roots (should be foundation)
    let output = run_jit(&temp, &["graph", "roots"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&foundation));
    assert!(!stdout.contains(&integration));

    // Show downstream of foundation
    let output = run_jit(&temp, &["graph", "downstream", &foundation]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&feature_a));
    assert!(stdout.contains(&feature_b));
    assert!(stdout.contains(&integration));
}

// ============================================================================
// Workflow Test: Auto-transition to Ready on Rejected dependency
// ============================================================================

#[test]
fn test_auto_transition_when_dependency_rejected() {
    let temp = setup_test_repo();

    // Create two issues: B depends on A
    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Task A", "--priority", "normal"],
    );
    assert!(output.status.success());
    let task_a = extract_id(&String::from_utf8_lossy(&output.stdout));

    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Task B", "--priority", "normal"],
    );
    assert!(output.status.success());
    let task_b = extract_id(&String::from_utf8_lossy(&output.stdout));

    // Add dependency: B depends on A
    let output = run_jit(&temp, &["dep", "add", &task_b, &task_a]);
    assert!(output.status.success());

    // Verify B is in Backlog (blocked by A)
    let output = run_jit(&temp, &["issue", "show", &task_b]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("State: Backlog"),
        "Task B should be blocked"
    );

    // Reject task A (terminal state, should unblock B)
    let output = run_jit(&temp, &["issue", "update", &task_a, "--state", "rejected"]);
    assert!(output.status.success());

    // Verify B auto-transitioned to Ready
    let output = run_jit(&temp, &["issue", "show", &task_b]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("State: Ready"),
        "Task B should auto-transition to Ready when dependency is rejected"
    );
}

// ============================================================================
// Workflow Test: Auto-transition with multiple dependencies
// ============================================================================

#[test]
fn test_auto_transition_when_multiple_dependencies_complete() {
    let temp = setup_test_repo();

    // Create three issues: C depends on both A and B
    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Task A", "--priority", "normal"],
    );
    assert!(output.status.success());
    let task_a = extract_id(&String::from_utf8_lossy(&output.stdout));

    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Task B", "--priority", "normal"],
    );
    assert!(output.status.success());
    let task_b = extract_id(&String::from_utf8_lossy(&output.stdout));

    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Task C", "--priority", "normal"],
    );
    assert!(output.status.success());
    let task_c = extract_id(&String::from_utf8_lossy(&output.stdout));

    // Add dependencies: C depends on both A and B
    let output = run_jit(&temp, &["dep", "add", &task_c, &task_a]);
    assert!(output.status.success());
    let output = run_jit(&temp, &["dep", "add", &task_c, &task_b]);
    assert!(output.status.success());

    // Verify C is in Backlog (blocked by A and B)
    let output = run_jit(&temp, &["issue", "show", &task_c]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("State: Backlog"),
        "Task C should be blocked"
    );

    // Complete task A
    let output = run_jit(&temp, &["issue", "update", &task_a, "--state", "done"]);
    assert!(output.status.success());

    // C should still be in Backlog (B not done yet)
    let output = run_jit(&temp, &["issue", "show", &task_c]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("State: Backlog"),
        "Task C should still be blocked by B"
    );

    // Complete task B (now all dependencies done)
    let output = run_jit(&temp, &["issue", "update", &task_b, "--state", "done"]);
    assert!(output.status.success());

    // Verify C auto-transitioned to Ready
    let output = run_jit(&temp, &["issue", "show", &task_c]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("State: Ready"),
        "Task C should auto-transition to Ready when both dependencies are done"
    );
}

// ============================================================================
// Workflow Test: Auto-transition with mixed terminal states
// ============================================================================

#[test]
fn test_auto_transition_with_mixed_terminal_states() {
    let temp = setup_test_repo();

    // Create three issues: C depends on both A and B
    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Task A", "--priority", "normal"],
    );
    assert!(output.status.success());
    let task_a = extract_id(&String::from_utf8_lossy(&output.stdout));

    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Task B", "--priority", "normal"],
    );
    assert!(output.status.success());
    let task_b = extract_id(&String::from_utf8_lossy(&output.stdout));

    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Task C", "--priority", "normal"],
    );
    assert!(output.status.success());
    let task_c = extract_id(&String::from_utf8_lossy(&output.stdout));

    // Add dependencies: C depends on both A and B
    let output = run_jit(&temp, &["dep", "add", &task_c, &task_a]);
    assert!(output.status.success());
    let output = run_jit(&temp, &["dep", "add", &task_c, &task_b]);
    assert!(output.status.success());

    // Complete A with Done, reject B (both terminal)
    let output = run_jit(&temp, &["issue", "update", &task_a, "--state", "done"]);
    assert!(output.status.success());

    let output = run_jit(&temp, &["issue", "update", &task_b, "--state", "rejected"]);
    assert!(output.status.success());

    // Verify C auto-transitioned to Ready (both dependencies in terminal states)
    let output = run_jit(&temp, &["issue", "show", &task_c]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("State: Ready"),
        "Task C should auto-transition to Ready when dependencies are in terminal states (Done/Rejected)"
    );
}

// ============================================================================
// Workflow Test: Auto-transition to Ready on Done dependency
// ============================================================================

#[test]
fn test_auto_transition_when_dependency_done() {
    let temp = setup_test_repo();

    // Create two issues: B depends on A
    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Task A", "--priority", "normal"],
    );
    assert!(output.status.success());
    let task_a = extract_id(&String::from_utf8_lossy(&output.stdout));

    let output = run_jit(
        &temp,
        &["issue", "create", "-t", "Task B", "--priority", "normal"],
    );
    assert!(output.status.success());
    let task_b = extract_id(&String::from_utf8_lossy(&output.stdout));

    // Add dependency: B depends on A
    let output = run_jit(&temp, &["dep", "add", &task_b, &task_a]);
    assert!(output.status.success());

    // Verify B is in Backlog
    let output = run_jit(&temp, &["issue", "show", &task_b]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("State: Backlog"));

    // Complete task A (terminal state, should unblock B)
    let output = run_jit(&temp, &["issue", "update", &task_a, "--state", "done"]);
    assert!(output.status.success());

    // Verify B auto-transitioned to Ready
    let output = run_jit(&temp, &["issue", "show", &task_b]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("State: Ready"),
        "Task B should auto-transition to Ready when dependency is done"
    );
}
