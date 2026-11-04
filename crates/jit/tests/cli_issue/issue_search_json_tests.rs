//! TDD tests for `jit issue search` JSON output compaction
//!
//! Tests verify that search returns compact results by default (MinimalIssue)
//! and full results only when --full flag is provided.

use jit::test_cli::JitCli;

fn seed_search_issues(cli: &JitCli) {
    cli.create_issue([
        "--title",
        "Test authentication feature",
        "--description",
        "This is a detailed description about implementing authentication with JWT tokens and middleware.",
        "--priority",
        "high",
        "--label",
        "type:task",
        "--label",
        "epic:auth",
        "--label",
        "component:api",
    ])
        .unwrap();

    // Create second test issue
    cli.create_issue([
        "--title",
        "Add authentication middleware",
        "--description",
        "Implement middleware for JWT validation",
        "--label",
        "type:task",
        "--label",
        "epic:auth",
    ])
    .unwrap();
}

#[test]
fn test_search_returns_compact_by_default() {
    let temp = JitCli::initialised().unwrap();
    seed_search_issues(&temp);

    let data = temp
        .run_json(["issue", "search", "authentication"])
        .unwrap();
    let issues = data["issues"].as_array().unwrap();

    assert!(!issues.is_empty(), "Should find authentication issues");

    // Check first issue has ONLY compact fields
    let first_issue = &issues[0];

    // Should have these fields (MinimalIssue)
    assert!(first_issue.get("id").is_some());
    assert!(first_issue.get("short_id").is_some());
    assert!(first_issue.get("title").is_some());
    assert!(first_issue.get("state").is_some());
    assert!(first_issue.get("priority").is_some());
    assert!(first_issue.get("assignee").is_some());
    assert!(first_issue.get("labels").is_some());

    // Should NOT have these fields (full Issue)
    assert!(
        first_issue.get("description").is_none(),
        "Compact mode should not include description"
    );
    assert!(
        first_issue.get("dependencies").is_none(),
        "Compact mode should not include dependencies"
    );
    assert!(
        first_issue.get("gates_required").is_none(),
        "Compact mode should not include gates_required"
    );
    assert!(
        first_issue.get("gates_status").is_none(),
        "Compact mode should not include gates_status"
    );
    assert!(
        first_issue.get("documents").is_none(),
        "Compact mode should not include documents"
    );
    assert!(
        first_issue.get("context").is_none(),
        "Compact mode should not include context"
    );
}

#[test]
fn test_search_returns_full_with_flag() {
    let temp = JitCli::initialised().unwrap();
    seed_search_issues(&temp);
    let data = temp
        .run_json(["issue", "search", "authentication", "--full"])
        .unwrap();
    let issues = data["issues"].as_array().unwrap();

    assert!(!issues.is_empty(), "Should find authentication issues");

    // Check first issue has ALL fields
    let first_issue = &issues[0];

    // Should have all Issue fields
    assert!(first_issue.get("id").is_some());
    assert!(first_issue.get("title").is_some());
    assert!(
        first_issue.get("description").is_some(),
        "--full should include description"
    );
    assert!(first_issue.get("state").is_some());
    assert!(first_issue.get("priority").is_some());
    assert!(first_issue.get("assignee").is_some());
    assert!(first_issue.get("labels").is_some());
    assert!(
        first_issue.get("dependencies").is_some(),
        "--full should include dependencies"
    );
    assert!(
        first_issue.get("gates_required").is_some(),
        "--full should include gates_required"
    );
    assert!(
        first_issue.get("gates_status").is_some(),
        "--full should include gates_status"
    );
    assert!(
        first_issue.get("documents").is_some(),
        "--full should include documents"
    );
    assert!(
        first_issue.get("context").is_some(),
        "--full should include context"
    );
}

#[test]
fn test_search_compact_has_short_id() {
    let temp = JitCli::initialised().unwrap();
    seed_search_issues(&temp);
    let data = temp
        .run_json(["issue", "search", "authentication"])
        .unwrap();
    let issues = data["issues"].as_array().unwrap();

    assert!(!issues.is_empty());

    let first_issue = &issues[0];
    let short_id = first_issue["short_id"].as_str().unwrap();
    let full_id = first_issue["id"].as_str().unwrap();

    // short_id should be first 8 chars of full ID
    assert_eq!(short_id.len(), 8);
    assert!(full_id.starts_with(short_id));
}

#[test]
fn test_search_compact_returns_every_matching_issue() {
    let temp = JitCli::initialised().unwrap();
    seed_search_issues(&temp);
    let data = temp
        .run_json(["issue", "search", "authentication"])
        .unwrap();
    let issues = data["issues"].as_array().unwrap();

    // We created 2 issues with "authentication"
    assert_eq!(issues.len(), 2);
}
