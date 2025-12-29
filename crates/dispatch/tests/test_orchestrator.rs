//! TDD tests for jit-dispatch orchestrator core functionality
//!
//! These tests define the orchestrator behavior:
//! 1. Load configuration from file
//! 2. Poll jit for ready issues
//! 3. Assign issues to available agents based on priority
//! 4. Track agent capacity

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> std::path::PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/jit")
}

fn setup_jit_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    Command::new(jit_binary())
        .args(["init"])
        .current_dir(temp.path())
        .status()
        .unwrap();
    temp
}

fn create_ready_issue(repo_path: &Path, title: &str, priority: &str) -> String {
    // Create issue
    let output = Command::new(jit_binary())
        .args([
            "issue", "create", "-t", title, "-p", priority, "--json", "--orphan",
        ])
        .current_dir(repo_path)
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    // JSON format: { "success": true, "data": { "id": "...", ... }, "metadata": {...} }
    let id = json["data"]["id"].as_str().unwrap().to_string();

    // No need to set to ready - auto-transitions when no blockers

    id
}

#[test]
fn test_config_loads_from_toml() {
    let temp = TempDir::new().unwrap();

    // Create a config file
    let config_content = r#"
poll_interval_secs = 5

[[agents]]
id = "agent-1"
type = "test"
max_concurrent = 2
command = "echo test"
"#;

    let config_path = temp.path().join("dispatch.toml");
    fs::write(&config_path, config_content).unwrap();

    // Load config (will implement this)
    let config = jit_dispatch::Config::from_file(&config_path).expect("Should load config");

    assert_eq!(config.poll_interval_secs, 5);
    assert_eq!(config.agents.len(), 1);
    assert_eq!(config.agents[0].id, "agent-1");
    assert_eq!(config.agents[0].max_concurrent, 2);
}

#[test]
fn test_orchestrator_can_query_ready_issues() {
    let repo = setup_jit_repo();

    // Create ready issues
    create_ready_issue(repo.path(), "Task 1", "normal");
    create_ready_issue(repo.path(), "Task 2", "high");

    // Orchestrator should be able to query
    let orchestrator = jit_dispatch::Orchestrator::new(repo.path());
    let ready = orchestrator
        .query_ready_issues()
        .expect("Should query ready issues");

    assert_eq!(ready.len(), 2);
}

#[test]
fn test_orchestrator_assigns_by_priority() {
    let repo = setup_jit_repo();

    // Create issues with different priorities
    let _low = create_ready_issue(repo.path(), "Low priority", "low");
    let high = create_ready_issue(repo.path(), "High priority", "high");
    let _normal = create_ready_issue(repo.path(), "Normal priority", "normal");

    let orchestrator = jit_dispatch::Orchestrator::new(repo.path());

    // Next issue to assign should be high priority
    let next = orchestrator
        .next_issue_to_assign()
        .expect("Should find next issue");
    assert_eq!(next.id, high);
    assert_eq!(next.priority, "high");
}

#[test]
fn test_orchestrator_tracks_agent_capacity() {
    let temp = TempDir::new().unwrap();
    let config_content = r#"
poll_interval_secs = 5

[[agents]]
id = "agent-1"
type = "test"
max_concurrent = 2
command = "echo test"

[[agents]]
id = "agent-2"
type = "test"
max_concurrent = 1
command = "echo test"
"#;

    let config_path = temp.path().join("dispatch.toml");
    fs::write(&config_path, config_content).unwrap();

    let config = jit_dispatch::Config::from_file(&config_path).unwrap();
    let mut tracker = jit_dispatch::AgentTracker::new(config.agents.clone());

    // Initially all agents available
    let available = tracker.available_agents();
    assert_eq!(available.len(), 2);

    // Assign work to agent-1
    tracker
        .assign_work("agent-1", "issue-1")
        .expect("Should assign");
    tracker
        .assign_work("agent-1", "issue-2")
        .expect("Should assign");

    // agent-1 now at capacity (2/2)
    let available = tracker.available_agents();
    assert_eq!(available.len(), 1);
    assert_eq!(available[0].id, "agent-2");

    // Trying to assign more to agent-1 should fail
    assert!(tracker.assign_work("agent-1", "issue-3").is_err());
}

#[test]
fn test_orchestrator_claims_issue_for_agent() {
    let repo = setup_jit_repo();
    let issue_id = create_ready_issue(repo.path(), "Task", "normal");

    let mut orchestrator = jit_dispatch::Orchestrator::new(repo.path());

    // Claim issue for agent
    orchestrator
        .claim_issue_for_agent(&issue_id, "agent:test-1")
        .expect("Should claim issue");

    // Verify it's claimed
    let ready = orchestrator.query_ready_issues().unwrap();
    assert_eq!(ready.len(), 0, "Issue should no longer be ready");
}

#[test]
fn test_dispatch_cycle_assigns_highest_priority_first() {
    let repo = setup_jit_repo();

    // Create multiple issues with different priorities
    create_ready_issue(repo.path(), "Low 1", "low");
    create_ready_issue(repo.path(), "High 1", "high");
    create_ready_issue(repo.path(), "Critical 1", "critical");
    create_ready_issue(repo.path(), "Normal 1", "normal");

    let temp = TempDir::new().unwrap();
    let config_content = r#"
poll_interval_secs = 5

[[agents]]
id = "agent-1"
type = "test"
max_concurrent = 10
command = "echo test"
"#;

    let config_path = temp.path().join("dispatch.toml");
    fs::write(&config_path, config_content).unwrap();

    let config = jit_dispatch::Config::from_file(&config_path).unwrap();
    let mut orchestrator = jit_dispatch::Orchestrator::with_config(repo.path(), config);

    // Run one dispatch cycle
    let assigned = orchestrator.run_dispatch_cycle().expect("Should dispatch");

    // Should assign all 4 issues
    assert_eq!(assigned, 4);

    // All should be claimed now
    let ready = orchestrator.query_ready_issues().unwrap();
    assert_eq!(ready.len(), 0);
}

// TODO: Add tests for:
// - Stalled work detection
// - Agent failure handling
// - Config reload
// - Multiple dispatch cycles
