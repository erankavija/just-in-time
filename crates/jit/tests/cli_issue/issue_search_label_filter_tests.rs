//! TDD tests for `jit issue search --label` filtering and optional positional query.
//!
//! Verifies:
//! - `--label ns:val` (no positional query) returns only issues carrying that label.
//! - Multiple `--label` flags AND together (issue must carry ALL).
//! - `--label` combined with `--state` ANDs correctly.
//! - A positional query + `--label` both narrow the result set.
//! - No query and no filter is a clear bad-args error.
//! - A plain positional `search foo` still works (regression).

use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn create_issue(dir: &std::path::Path, title: &str, labels: &[&str], extra: &[&str]) {
    let mut args: Vec<String> = vec![
        "issue".into(),
        "create".into(),
        "--title".into(),
        title.into(),
        "--description".into(),
        format!("Description for {title}"),
    ];
    for label in labels {
        args.push("--label".into());
        args.push((*label).into());
    }
    for e in extra {
        args.push((*e).into());
    }
    let out = Command::new(jit_binary())
        .current_dir(dir)
        .args(&args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "create failed for {title}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    Command::new(jit_binary())
        .current_dir(temp.path())
        .arg("init")
        .output()
        .unwrap();

    // Issue A: type:epic, area:auth
    create_issue(
        temp.path(),
        "Authentication epic",
        &["type:epic", "area:auth"],
        &[],
    );
    // Issue B: type:task, area:auth
    create_issue(
        temp.path(),
        "Auth task one",
        &["type:task", "area:auth"],
        &[],
    );
    // Issue C: type:task, area:billing
    create_issue(
        temp.path(),
        "Billing task",
        &["type:task", "area:billing"],
        &[],
    );

    temp
}

fn search_json(dir: &std::path::Path, args: &[&str]) -> Value {
    let mut full: Vec<&str> = vec!["issue", "search"];
    full.extend_from_slice(args);
    full.push("--json");
    let out = Command::new(jit_binary())
        .current_dir(dir)
        .args(&full)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "search failed for args {args:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
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
    let temp = setup_test_repo();
    let json = search_json(temp.path(), &["--label", "type:epic"]);
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
    let temp = setup_test_repo();
    // type:task AND area:auth -> only "Auth task one"
    let json = search_json(
        temp.path(),
        &["--label", "type:task", "--label", "area:auth"],
    );
    let t = titles(&json);
    assert_eq!(
        t,
        vec!["Auth task one".to_string()],
        "AND semantics failed: {t:?}"
    );
}

#[test]
fn test_search_label_combined_with_state() {
    let temp = setup_test_repo();
    // Dependency-free issues default to the ready state. area:auth + state
    // ready = A and B.
    let json = search_json(temp.path(), &["--label", "area:auth", "--state", "ready"]);
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
    let json = search_json(temp.path(), &["--label", "area:auth", "--state", "backlog"]);
    assert_eq!(titles(&json).len(), 0);
}

#[test]
fn test_search_query_and_label_both_narrow() {
    let temp = setup_test_repo();
    // query "task" matches B and C by title; --label area:auth narrows to B only.
    let json = search_json(temp.path(), &["task", "--label", "area:auth"]);
    let t = titles(&json);
    assert_eq!(
        t,
        vec!["Auth task one".to_string()],
        "query+label narrow failed: {t:?}"
    );
}

#[test]
fn test_search_no_query_no_filter_is_error() {
    let temp = setup_test_repo();
    let out = Command::new(jit_binary())
        .current_dir(temp.path())
        .args(["issue", "search"])
        .output()
        .unwrap();
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
    let temp = setup_test_repo();
    let json = search_json(temp.path(), &["Billing"]);
    let t = titles(&json);
    assert_eq!(t, vec!["Billing task".to_string()]);
}

#[test]
fn test_search_label_query_field_null_when_absent() {
    let temp = setup_test_repo();
    let json = search_json(temp.path(), &["--label", "type:epic"]);
    assert!(
        json["query"].is_null(),
        "query field should be null when no positional query given: {}",
        json["query"]
    );
}

#[test]
fn test_search_invalid_label_format_is_error() {
    let temp = setup_test_repo();
    let out = Command::new(jit_binary())
        .current_dir(temp.path())
        .args(["issue", "search", "--label", "notalabel"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "search with malformed label should fail"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        stderr.contains("label"),
        "error should mention label: {stderr}"
    );
}
