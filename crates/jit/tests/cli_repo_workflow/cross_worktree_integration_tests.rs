//! Integration tests for cross-worktree issue visibility.
//!
//! These tests verify that issues can be read across git worktrees using
//! the 3-tier fallback chain: local .jit → git HEAD → main worktree .jit

use jit::test_cli::JitCli;

/// Declared by every mutation inside a linked checkout: this suite initializes
/// and mutates jit there as setup for the read-visibility properties it asserts
/// (jit:f52567ed REQ-05). The write policy's own behaviour is covered by
/// `linked_checkout_write_policy_tests`.
#[test]
fn test_issue_show_reads_from_git_in_secondary_worktree() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let issue_id = primary.create_issue(["--title", "Issue in Git"]).unwrap();
    primary.commit("Add issue").unwrap();
    linked.git(["merge", "-q", "--ff-only", "main"]).unwrap();
    assert!(linked
        .command_with_env(["init"], [crate::ALLOW_LINKED_WRITES])
        .status()
        .unwrap()
        .success());

    let loaded = linked.run_json(["issue", "show", &issue_id]).unwrap();

    assert_eq!(loaded["title"].as_str().unwrap(), "Issue in Git");
}

#[test]
fn test_issue_show_reads_from_main_worktree_uncommitted() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    // Created but not committed.
    let issue_id = primary
        .create_issue(["--title", "Uncommitted Issue"])
        .unwrap();
    assert!(linked
        .command_with_env(["init"], [crate::ALLOW_LINKED_WRITES])
        .status()
        .unwrap()
        .success());

    let loaded = linked.run_json(["issue", "show", &issue_id]).unwrap();

    assert_eq!(loaded["title"].as_str().unwrap(), "Uncommitted Issue");
}

#[test]
fn test_query_all_shows_issues_from_all_sources() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let id_a = primary.create_issue(["--title", "Issue A"]).unwrap();
    primary.commit("Add issue A").unwrap();
    linked.git(["merge", "-q", "--ff-only", "main"]).unwrap();
    // Issue B stays uncommitted in the primary checkout.
    let id_b = primary.create_issue(["--title", "Issue B"]).unwrap();
    assert!(linked
        .command_with_env(["init"], [crate::ALLOW_LINKED_WRITES])
        .status()
        .unwrap()
        .success());
    let created_c = linked
        .command_with_env(
            ["issue", "create", "--title", "Issue C", "--json"],
            [crate::ALLOW_LINKED_WRITES],
        )
        .output()
        .unwrap();
    assert!(created_c.status.success(), "{created_c:?}");
    let id_c = jit::test_cli::created_issue_id(&serde_json::from_slice(&created_c.stdout).unwrap())
        .unwrap();

    // Query all from secondary - should see A (git), B (main), C (local)
    let result = linked.run_json(["query", "all"]).unwrap();
    let issues_array = result["issues"].as_array().unwrap();

    assert_eq!(issues_array.len(), 3, "Should see all 3 issues");

    let ids: Vec<&str> = issues_array
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();

    assert!(
        ids.contains(&id_a.as_str()),
        "Should include issue A from git"
    );
    assert!(
        ids.contains(&id_b.as_str()),
        "Should include issue B from main worktree"
    );
    assert!(
        ids.contains(&id_c.as_str()),
        "Should include issue C from local"
    );
}

#[test]
fn test_graph_show_works_across_worktrees() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let id1 = primary.create_issue(["--title", "Dependency"]).unwrap();
    let id2 = primary.create_issue(["--title", "Dependent"]).unwrap();
    primary.run(["dep", "add", &id2, &id1]).unwrap();
    primary.commit("Add issues with dependency").unwrap();
    linked.git(["merge", "-q", "--ff-only", "main"]).unwrap();
    assert!(linked
        .command_with_env(["init"], [crate::ALLOW_LINKED_WRITES])
        .status()
        .unwrap()
        .success());

    // Graph show should work - verify we can query from secondary
    let query = linked.run_json(["query", "all"]).unwrap();

    // Should see both issues in the query
    let issues = query["issues"].as_array().unwrap();
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
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let full_id = primary.create_issue(["--title", "Find Me"]).unwrap();
    let partial_id = &full_id[..8];
    primary.commit("Add issue").unwrap();
    linked.git(["merge", "-q", "--ff-only", "main"]).unwrap();
    assert!(linked
        .command_with_env(["init"], [crate::ALLOW_LINKED_WRITES])
        .status()
        .unwrap()
        .success());

    // Should resolve partial ID from git
    let loaded = linked.run_json(["issue", "show", partial_id]).unwrap();

    assert_eq!(loaded["id"].as_str().unwrap(), full_id);
    assert_eq!(loaded["title"].as_str().unwrap(), "Find Me");
}

#[test]
fn test_local_overrides_git_and_main() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let issue_id = primary.create_issue(["--title", "Original"]).unwrap();
    primary.commit("Add original issue").unwrap();
    linked.git(["merge", "-q", "--ff-only", "main"]).unwrap();
    assert!(linked
        .command_with_env(["init"], [crate::ALLOW_LINKED_WRITES])
        .status()
        .unwrap()
        .success());

    // Modify issue in secondary (creates local copy)
    assert!(linked
        .command_with_env(
            [
                "issue",
                "update",
                &issue_id,
                "--title",
                "Modified in Secondary",
            ],
            [crate::ALLOW_LINKED_WRITES],
        )
        .status()
        .unwrap()
        .success());

    // Should read modified version (local overrides git)
    let loaded = linked.run_json(["issue", "show", &issue_id]).unwrap();

    assert_eq!(loaded["title"].as_str().unwrap(), "Modified in Secondary");

    // Main worktree should still see original
    let loaded_main = primary.run_json(["issue", "show", &issue_id]).unwrap();

    assert_eq!(loaded_main["title"].as_str().unwrap(), "Original");
}

// Deletion inside a secondary worktree is not a visibility property and no
// longer a rule of its own: it is one governed case of the linked-checkout write
// policy, covered under every stance by `linked_checkout_write_policy_tests`
// (jit:1b6925a9).

#[test]
fn test_deletion_requires_env_var() {
    // Phase 3: Deletion in main worktree requires JIT_ALLOW_DELETION=1 to discourage deletion
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["-t", "Test", "-d", "desc"]).unwrap();
    let listed_ids = || {
        cli.run_json(["query", "all"]).unwrap()["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|issue| issue["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };

    // Try to delete WITHOUT env var - should be blocked
    let result = cli.run(["issue", "delete", &issue_id]);
    assert!(
        result.is_err(),
        "Deletion should require JIT_ALLOW_DELETION=1"
    );
    let error = result.unwrap_err().to_string();
    assert!(
        error.contains("JIT_ALLOW_DELETION") || error.contains("discouraged"),
        "Error should mention JIT_ALLOW_DELETION: {}",
        error
    );

    // Verify issue still exists
    assert!(
        listed_ids().contains(&issue_id),
        "Issue should still exist after blocked deletion"
    );

    // Delete WITH env var - should succeed
    let deleted = cli
        .command_with_env(
            ["issue", "delete", &issue_id],
            [("JIT_ALLOW_DELETION", "1")],
        )
        .output()
        .unwrap();
    assert!(
        deleted.status.success(),
        "Deletion should succeed with env var: {deleted:?}"
    );

    // Verify issue is gone
    assert!(
        !listed_ids().contains(&issue_id),
        "Issue should be deleted after env var check passed"
    );
}
