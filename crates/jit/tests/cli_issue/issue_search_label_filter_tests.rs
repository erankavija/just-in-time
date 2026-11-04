//! TDD tests for `jit issue search --label` filtering and optional positional query.
//!
//! Verifies:
//! - `--label ns:val` (no positional query) returns only issues carrying that label.
//! - Multiple `--label` flags AND together (issue must carry ALL).
//! - `--label` combined with `--state` ANDs correctly.
//! - A positional query + `--label` both narrow the result set.
//! - No query and no filter is a clear bad-args error.
//! - A plain positional `search foo` still works (regression).

use jit::test_cli::JitCli;
use serde_json::Value;

fn seed_search_issues(cli: &JitCli) {
    cli.create_issue([
        "--title",
        "Authentication epic",
        "--description",
        "Description for Authentication epic",
        "--label",
        "type:epic",
        "--label",
        "area:auth",
    ])
    .unwrap();
    cli.create_issue([
        "--title",
        "Auth task one",
        "--description",
        "Description for Auth task one",
        "--label",
        "type:task",
        "--label",
        "area:auth",
    ])
    .unwrap();
    cli.create_issue([
        "--title",
        "Billing task",
        "--description",
        "Description for Billing task",
        "--label",
        "type:task",
        "--label",
        "area:billing",
    ])
    .unwrap();
}

fn titles(json: &Value) -> Vec<String> {
    json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn test_search_label_no_query_returns_only_labeled() {
    let temp = JitCli::initialised().unwrap();
    seed_search_issues(&temp);
    let json = temp
        .run_json(["issue", "search", "--label", "type:epic"])
        .unwrap();
    let t = titles(&json);
    assert_eq!(
        t.len(),
        1,
        "expected exactly one type:epic issue, got {t:?}"
    );
    assert_eq!(t[0], "Authentication epic");
}

#[test]
fn test_search_two_labels_anded() {
    let temp = JitCli::initialised().unwrap();
    seed_search_issues(&temp);
    // type:task AND area:auth -> only "Auth task one"
    let json = temp
        .run_json([
            "issue",
            "search",
            "--label",
            "type:task",
            "--label",
            "area:auth",
        ])
        .unwrap();
    let t = titles(&json);
    assert_eq!(
        t,
        vec!["Auth task one".to_string()],
        "AND semantics failed: {t:?}"
    );
}

#[test]
fn test_search_label_combined_with_state() {
    let temp = JitCli::initialised().unwrap();
    seed_search_issues(&temp);
    // Dependency-free issues default to the ready state. area:auth + state
    // ready = A and B.
    let json = temp
        .run_json([
            "issue",
            "search",
            "--label",
            "area:auth",
            "--state",
            "ready",
        ])
        .unwrap();
    let mut t = titles(&json);
    t.sort();
    assert_eq!(
        t,
        vec![
            "Auth task one".to_string(),
            "Authentication epic".to_string()
        ]
    );

    // area:auth + state backlog -> none (nothing is in backlog).
    let json = temp
        .run_json([
            "issue",
            "search",
            "--label",
            "area:auth",
            "--state",
            "backlog",
        ])
        .unwrap();
    assert_eq!(titles(&json).len(), 0);
}

#[test]
fn test_search_query_and_label_both_narrow() {
    let temp = JitCli::initialised().unwrap();
    seed_search_issues(&temp);
    // query "task" matches B and C by title; --label area:auth narrows to B only.
    let json = temp
        .run_json(["issue", "search", "task", "--label", "area:auth"])
        .unwrap();
    let t = titles(&json);
    assert_eq!(
        t,
        vec!["Auth task one".to_string()],
        "query+label narrow failed: {t:?}"
    );
}

#[test]
fn test_search_no_query_no_filter_is_error() {
    let temp = JitCli::initialised().unwrap();
    seed_search_issues(&temp);
    let out = temp.command(["issue", "search"]).output().unwrap();
    assert!(
        !out.status.success(),
        "search with no query and no filter should fail"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        stderr.contains("query") && stderr.contains("filter"),
        "error should mention query and filter, got: {stderr}"
    );
}

#[test]
fn test_search_plain_positional_query_still_works() {
    let temp = JitCli::initialised().unwrap();
    seed_search_issues(&temp);
    let json = temp.run_json(["issue", "search", "Billing"]).unwrap();
    let t = titles(&json);
    assert_eq!(t, vec!["Billing task".to_string()]);
}

#[test]
fn test_search_label_query_field_null_when_absent() {
    let temp = JitCli::initialised().unwrap();
    seed_search_issues(&temp);
    let json = temp
        .run_json(["issue", "search", "--label", "type:epic"])
        .unwrap();
    assert!(
        json["query"].is_null(),
        "query field should be null when no positional query given: {}",
        json["query"]
    );
}

#[test]
fn test_search_malformed_label_pattern_fails() {
    let temp = JitCli::initialised().unwrap();
    seed_search_issues(&temp);
    let out = temp
        .command(["issue", "search", "--label", "type:", "--json"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_LABEL_PATTERN");
}
