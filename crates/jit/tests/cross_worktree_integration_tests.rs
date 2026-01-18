//! Integration tests for cross-worktree issue visibility.
//!
//! These tests verify that issues can be read across git worktrees using
//! the 3-tier fallback chain: local .jit → git HEAD → main worktree .jit

use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Setup a git repository with jit initialized
fn setup_git_repo() -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().to_path_buf();

    // Initialize git repository
    Command::new("git")
        .args(["init"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Initial commit to establish HEAD
    fs::write(repo_path.join("README.md"), "Test repo").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    (temp_dir, repo_path)
}

/// Run jit command and return stdout
fn run_jit(dir: &PathBuf, args: &[&str]) -> Result<String> {
    let output = Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(args)
        .current_dir(dir)
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        eprintln!("Command failed: jit {:?}", args);
        eprintln!("Stdout: {}", stdout);
        eprintln!("Stderr: {}", stderr);
        anyhow::bail!("Command failed: {}", stderr);
    }

    Ok(stdout)
}

#[test]
fn test_issue_show_reads_from_git_in_secondary_worktree() {
    let (_temp_dir, repo_path) = setup_git_repo();

    // Initialize jit in main worktree
    run_jit(&repo_path, &["init"]).unwrap();

    // Create and commit an issue
    let output = run_jit(&repo_path, &["issue", "create", "--title", "Issue in Git", "--json"]).unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let issue_id = json["data"]["id"].as_str().unwrap();

    Command::new("git")
        .args(["add", ".jit"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Add issue"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Create secondary worktree with unique name
    let secondary_path = repo_path.parent().unwrap().join(format!("secondary-{}", issue_id));
    Command::new("git")
        .args(["worktree", "add", "-b", &format!("feature-{}", issue_id), secondary_path.to_str().unwrap()])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Initialize jit in secondary worktree
    run_jit(&secondary_path, &["init"]).unwrap();

    // Should be able to read issue from git
    let output = run_jit(&secondary_path, &["issue", "show", issue_id, "--json"]).unwrap();
    let loaded: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(loaded["data"]["title"].as_str().unwrap(), "Issue in Git");
}

#[test]
fn test_issue_show_reads_from_main_worktree_uncommitted() {
    let (_temp_dir, repo_path) = setup_git_repo();

    // Initialize jit in main worktree
    run_jit(&repo_path, &["init"]).unwrap();

    // Create issue but DON'T commit it
    let output = run_jit(&repo_path, &["issue", "create", "--title", "Uncommitted Issue", "--json"]).unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let issue_id = json["data"]["id"].as_str().unwrap();

    // Create secondary worktree with unique name
    let secondary_path = repo_path.parent().unwrap().join(format!("secondary-main-{}", issue_id));
    Command::new("git")
        .args(["worktree", "add", "-b", &format!("feature-main-{}", issue_id), secondary_path.to_str().unwrap()])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Initialize jit in secondary worktree
    run_jit(&secondary_path, &["init"]).unwrap();

    // Should be able to read uncommitted issue from main worktree
    let output = run_jit(&secondary_path, &["issue", "show", issue_id, "--json"]).unwrap();
    let loaded: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(loaded["data"]["title"].as_str().unwrap(), "Uncommitted Issue");
}

#[test]
fn test_query_all_shows_issues_from_all_sources() {
    let (_temp_dir, repo_path) = setup_git_repo();

    // Initialize jit in main worktree
    run_jit(&repo_path, &["init"]).unwrap();

    // Create issue A and commit it
    let output_a = run_jit(&repo_path, &["issue", "create", "--title", "Issue A", "--json"]).unwrap();
    let json_a: serde_json::Value = serde_json::from_str(&output_a).unwrap();
    let id_a = json_a["data"]["id"].as_str().unwrap();

    Command::new("git")
        .args(["add", ".jit"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Add issue A"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Create issue B but don't commit
    let output_b = run_jit(&repo_path, &["issue", "create", "--title", "Issue B", "--json"]).unwrap();
    let json_b: serde_json::Value = serde_json::from_str(&output_b).unwrap();
    let id_b = json_b["data"]["id"].as_str().unwrap();

    // Create secondary worktree with unique name
    let secondary_path = repo_path.parent().unwrap().join(format!("secondary-query-{}", id_a));
    Command::new("git")
        .args(["worktree", "add", "-b", &format!("feature-query-{}", id_a), secondary_path.to_str().unwrap()])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Initialize jit in secondary worktree
    run_jit(&secondary_path, &["init"]).unwrap();

    // Create issue C locally in secondary
    let output_c = run_jit(&secondary_path, &["issue", "create", "--title", "Issue C", "--json"]).unwrap();
    let json_c: serde_json::Value = serde_json::from_str(&output_c).unwrap();
    let id_c = json_c["data"]["id"].as_str().unwrap();

    // Query all from secondary - should see A (git), B (main), C (local)
    let output = run_jit(&secondary_path, &["query", "all", "--json"]).unwrap();
    let result: serde_json::Value = serde_json::from_str(&output).unwrap();
    let issues_array = result["data"]["issues"].as_array().unwrap();

    assert_eq!(issues_array.len(), 3, "Should see all 3 issues");

    let ids: Vec<&str> = issues_array
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();

    assert!(ids.contains(&id_a), "Should include issue A from git");
    assert!(ids.contains(&id_b), "Should include issue B from main worktree");
    assert!(ids.contains(&id_c), "Should include issue C from local");
}

#[test]
fn test_graph_show_works_across_worktrees() {
    let (_temp_dir, repo_path) = setup_git_repo();

    // Initialize jit in main worktree
    run_jit(&repo_path, &["init"]).unwrap();

    // Create two issues with dependency and commit
    let output1 = run_jit(&repo_path, &["issue", "create", "--title", "Dependency", "--json"]).unwrap();
    let json1: serde_json::Value = serde_json::from_str(&output1).unwrap();
    let id1 = json1["data"]["id"].as_str().unwrap();

    let output2 = run_jit(&repo_path, &["issue", "create", "--title", "Dependent", "--json"]).unwrap();
    let json2: serde_json::Value = serde_json::from_str(&output2).unwrap();
    let id2 = json2["data"]["id"].as_str().unwrap();

    run_jit(&repo_path, &["dep", "add", id2, id1]).unwrap();

    Command::new("git")
        .args(["add", ".jit"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Add issues with dependency"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Create secondary worktree with unique name
    let secondary_path = repo_path.parent().unwrap().join(format!("secondary-graph-{}", id1));
    Command::new("git")
        .args(["worktree", "add", "-b", &format!("feature-graph-{}", id1), secondary_path.to_str().unwrap()])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Initialize jit in secondary worktree
    run_jit(&secondary_path, &["init"]).unwrap();

    // Graph show should work - verify we can query from secondary
    let output = run_jit(&secondary_path, &["query", "all", "--json"]).unwrap();
    let query: serde_json::Value = serde_json::from_str(&output).unwrap();

    // Should see both issues in the query
    let issues = query["data"]["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 2, "Should see both issues in query");

    let titles: Vec<&str> = issues
        .iter()
        .map(|i| i["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Dependency"));
    assert!(titles.contains(&"Dependent"));
}

#[test]
fn test_partial_id_resolution_across_worktrees() {
    let (_temp_dir, repo_path) = setup_git_repo();

    // Initialize jit in main worktree
    run_jit(&repo_path, &["init"]).unwrap();

    // Create and commit an issue
    let output = run_jit(&repo_path, &["issue", "create", "--title", "Find Me", "--json"]).unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let full_id = json["data"]["id"].as_str().unwrap();
    let partial_id = &full_id[..8];

    Command::new("git")
        .args(["add", ".jit"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Add issue"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Create secondary worktree with unique name
    let secondary_path = repo_path.parent().unwrap().join(format!("secondary-partial-{}", full_id));
    Command::new("git")
        .args(["worktree", "add", "-b", &format!("feature-partial-{}", partial_id), secondary_path.to_str().unwrap()])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Initialize jit in secondary worktree
    run_jit(&secondary_path, &["init"]).unwrap();

    // Should resolve partial ID from git
    let output = run_jit(&secondary_path, &["issue", "show", partial_id, "--json"]).unwrap();
    let loaded: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(loaded["data"]["id"].as_str().unwrap(), full_id);
    assert_eq!(loaded["data"]["title"].as_str().unwrap(), "Find Me");
}

#[test]
fn test_local_overrides_git_and_main() {
    let (_temp_dir, repo_path) = setup_git_repo();

    // Initialize jit in main worktree
    run_jit(&repo_path, &["init"]).unwrap();

    // Create and commit an issue
    let output = run_jit(&repo_path, &["issue", "create", "--title", "Original", "--json"]).unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let issue_id = json["data"]["id"].as_str().unwrap();

    Command::new("git")
        .args(["add", ".jit"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    Command::new("git")
        .args(["commit", "-m", "Add original issue"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Create secondary worktree with unique name
    let secondary_path = repo_path.parent().unwrap().join(format!("secondary-override-{}", issue_id));
    Command::new("git")
        .args(["worktree", "add", "-b", &format!("feature-override-{}", issue_id), secondary_path.to_str().unwrap()])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Initialize jit in secondary worktree
    run_jit(&secondary_path, &["init"]).unwrap();

    // Modify issue in secondary (creates local copy)
    run_jit(&secondary_path, &["issue", "update", issue_id, "--title", "Modified in Secondary"]).unwrap();

    // Should read modified version (local overrides git)
    let output = run_jit(&secondary_path, &["issue", "show", issue_id, "--json"]).unwrap();
    let loaded: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(loaded["data"]["title"].as_str().unwrap(), "Modified in Secondary");

    // Main worktree should still see original
    let output = run_jit(&repo_path, &["issue", "show", issue_id, "--json"]).unwrap();
    let loaded_main: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(loaded_main["data"]["title"].as_str().unwrap(), "Original");
}
