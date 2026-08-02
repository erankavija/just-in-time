//! TDD tests for repeatable, AND-combined `--label` filters across the query
//! family (`jit query all/available/blocked/strategic/closed`), `jit issue
//! list`, the top-level `jit list` alias, and the bare `jit query` form.
//!
//! Companion to `issue_search_label_filter_tests.rs`, which already covers
//! `jit issue search`'s repeatable AND semantics — that command is the
//! reference implementation these tests hold every other label-filtered
//! command to (jit:dc3bef62). Each command here previously accepted only a
//! single `--label` occurrence; this file locks in:
//! - Multiple `--label` flags AND together (issue must carry ALL).
//! - A single `--label` still behaves exactly as before (regression).
//! - A multi-label filter with no matching issue returns an empty result,
//!   not an error.

use serde_json::Value;
use std::process::Command;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_test_repo() -> crate::TaxonomyRepo {
    crate::setup_test_repo_with_taxonomy()
}

/// Create an issue with the given labels, returning its id.
fn create_issue<L: AsRef<str>>(dir: &std::path::Path, title: &str, labels: &[L]) -> String {
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
        args.push(label.as_ref().into());
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
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

fn set_state(dir: &std::path::Path, id: &str, state: &str) {
    let out = Command::new(jit_binary())
        .current_dir(dir)
        .args(["issue", "update", id, "--state", state])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "state update to {state} failed for {id}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn add_dep(dir: &std::path::Path, from_id: &str, to_id: &str) {
    let out = Command::new(jit_binary())
        .current_dir(dir)
        .args(["dep", "add", from_id, to_id])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "dep add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn run_json<A: AsRef<std::ffi::OsStr> + std::fmt::Debug>(
    dir: &std::path::Path,
    args: &[A],
) -> Value {
    let out = Command::new(jit_binary())
        .current_dir(dir)
        .args(args.iter().map(AsRef::as_ref))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "command failed for args {args:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn ids(json: &Value) -> Vec<String> {
    json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

// ── jit query all ────────────────────────────────────────────────────────

#[test]
fn test_query_all_repeatable_label_ands() {
    let temp = setup_test_repo();
    let both = create_issue(temp.path(), "Both", &["epic:auth", "component:api"]);
    create_issue(temp.path(), "EpicOnly", &["epic:auth"]);
    create_issue(temp.path(), "ComponentOnly", &["component:api"]);

    let json = run_json(
        temp.path(),
        &[
            "query",
            "all",
            "--label",
            "epic:auth",
            "--label",
            "component:api",
            "--json",
        ],
    );
    assert_eq!(ids(&json), vec![both]);
}

#[test]
fn test_query_all_repeatable_label_zero_match() {
    let temp = setup_test_repo();
    create_issue(temp.path(), "EpicOnly", &["epic:auth"]);
    create_issue(temp.path(), "ComponentOnly", &["component:api"]);

    let json = run_json(
        temp.path(),
        &[
            "query",
            "all",
            "--label",
            "epic:auth",
            "--label",
            "component:api",
            "--json",
        ],
    );
    assert_eq!(json["count"], 0);
    assert!(ids(&json).is_empty());
}

#[test]
fn test_query_all_single_label_regression() {
    let temp = setup_test_repo();
    let tagged = create_issue(temp.path(), "Tagged", &["epic:auth"]);
    create_issue(temp.path(), "Untagged", &[] as &[&str]);

    let json = run_json(
        temp.path(),
        &["query", "all", "--label", "epic:auth", "--json"],
    );
    assert_eq!(ids(&json), vec![tagged]);
}

// ── jit query available ──────────────────────────────────────────────────

#[test]
fn test_query_available_repeatable_label_ands() {
    let temp = setup_test_repo();
    let both = create_issue(temp.path(), "Both", &["epic:auth", "component:api"]);
    create_issue(temp.path(), "EpicOnly", &["epic:auth"]);

    let json = run_json(
        temp.path(),
        &[
            "query",
            "available",
            "--label",
            "epic:auth",
            "--label",
            "component:api",
            "--json",
        ],
    );
    assert_eq!(ids(&json), vec![both]);
}

#[test]
fn test_query_available_single_label_regression() {
    let temp = setup_test_repo();
    let tagged = create_issue(temp.path(), "Tagged", &["epic:auth"]);
    create_issue(temp.path(), "Untagged", &[] as &[&str]);

    let json = run_json(
        temp.path(),
        &["query", "available", "--label", "epic:auth", "--json"],
    );
    assert_eq!(ids(&json), vec![tagged]);
}

// ── jit query blocked ─────────────────────────────────────────────────────

#[test]
fn test_query_blocked_repeatable_label_ands() {
    let temp = setup_test_repo();
    let parent = create_issue(temp.path(), "Parent", &[] as &[&str]);

    let both = create_issue(temp.path(), "Both", &["epic:auth", "component:api"]);
    add_dep(temp.path(), &both, &parent);

    let epic_only = create_issue(temp.path(), "EpicOnly", &["epic:auth"]);
    add_dep(temp.path(), &epic_only, &parent);

    let json = run_json(
        temp.path(),
        &[
            "query",
            "blocked",
            "--label",
            "epic:auth",
            "--label",
            "component:api",
            "--json",
        ],
    );
    assert_eq!(ids(&json), vec![both]);
}

#[test]
fn test_query_blocked_single_label_regression() {
    let temp = setup_test_repo();
    let parent = create_issue(temp.path(), "Parent", &[] as &[&str]);

    let tagged = create_issue(temp.path(), "Tagged", &["epic:auth"]);
    add_dep(temp.path(), &tagged, &parent);

    let untagged = create_issue(temp.path(), "Untagged", &[] as &[&str]);
    add_dep(temp.path(), &untagged, &parent);

    let json = run_json(
        temp.path(),
        &["query", "blocked", "--label", "epic:auth", "--json"],
    );
    assert_eq!(ids(&json), vec![tagged]);
}

// ── jit query strategic ──────────────────────────────────────────────────

#[test]
fn test_query_strategic_repeatable_label_ands() {
    let temp = setup_test_repo();
    let strategic = crate::type_label(&temp.taxonomy, 2);
    let component = crate::membership_label(&temp.taxonomy, 3, "api");
    let team = crate::membership_label(&temp.taxonomy, 1, "backend");
    let both = create_issue(
        temp.path(),
        "Both",
        &[strategic.clone(), component.clone(), team.clone()],
    );
    create_issue(
        temp.path(),
        "ComponentOnly",
        &[strategic, component.clone()],
    );

    let args = vec![
        "query".to_string(),
        "strategic".to_string(),
        "--label".to_string(),
        component,
        "--label".to_string(),
        team,
        "--json".to_string(),
    ];
    let json = run_json(temp.path(), &args);
    assert_eq!(ids(&json), vec![both]);
}

#[test]
fn test_query_strategic_single_label_regression() {
    let temp = setup_test_repo();
    let strategic = crate::type_label(&temp.taxonomy, 2);
    let component = crate::membership_label(&temp.taxonomy, 3, "api");
    let tagged = create_issue(
        temp.path(),
        "Tagged",
        &[strategic.clone(), component.clone()],
    );
    create_issue(temp.path(), "Untagged", &[strategic]);

    let args = vec![
        "query".to_string(),
        "strategic".to_string(),
        "--label".to_string(),
        component,
        "--json".to_string(),
    ];
    let json = run_json(temp.path(), &args);
    assert_eq!(ids(&json), vec![tagged]);
}

// ── jit query closed ─────────────────────────────────────────────────────

#[test]
fn test_query_closed_repeatable_label_ands() {
    let temp = setup_test_repo();
    let both = create_issue(temp.path(), "Both", &["epic:auth", "component:api"]);
    set_state(temp.path(), &both, "done");
    let epic_only = create_issue(temp.path(), "EpicOnly", &["epic:auth"]);
    set_state(temp.path(), &epic_only, "done");

    let json = run_json(
        temp.path(),
        &[
            "query",
            "closed",
            "--label",
            "epic:auth",
            "--label",
            "component:api",
            "--json",
        ],
    );
    assert_eq!(ids(&json), vec![both]);
}

#[test]
fn test_query_closed_single_label_regression() {
    let temp = setup_test_repo();
    let tagged = create_issue(temp.path(), "Tagged", &["epic:auth"]);
    set_state(temp.path(), &tagged, "done");
    let untagged = create_issue(temp.path(), "Untagged", &[] as &[&str]);
    set_state(temp.path(), &untagged, "done");

    let json = run_json(
        temp.path(),
        &["query", "closed", "--label", "epic:auth", "--json"],
    );
    assert_eq!(ids(&json), vec![tagged]);
}

// ── jit issue list ───────────────────────────────────────────────────────

#[test]
fn test_issue_list_repeatable_label_ands() {
    let temp = setup_test_repo();
    let both = create_issue(temp.path(), "Both", &["epic:auth", "component:api"]);
    create_issue(temp.path(), "EpicOnly", &["epic:auth"]);

    let json = run_json(
        temp.path(),
        &[
            "issue",
            "list",
            "--label",
            "epic:auth",
            "--label",
            "component:api",
            "--json",
        ],
    );
    assert_eq!(ids(&json), vec![both]);
}

#[test]
fn test_issue_list_single_label_regression() {
    let temp = setup_test_repo();
    let tagged = create_issue(temp.path(), "Tagged", &["epic:auth"]);
    create_issue(temp.path(), "Untagged", &[] as &[&str]);

    let json = run_json(
        temp.path(),
        &["issue", "list", "--label", "epic:auth", "--json"],
    );
    assert_eq!(ids(&json), vec![tagged]);
}

// ── jit list (top-level alias) ───────────────────────────────────────────

#[test]
fn test_top_level_list_repeatable_label_ands() {
    let temp = setup_test_repo();
    let both = create_issue(temp.path(), "Both", &["epic:auth", "component:api"]);
    create_issue(temp.path(), "EpicOnly", &["epic:auth"]);

    let json = run_json(
        temp.path(),
        &[
            "list",
            "--label",
            "epic:auth",
            "--label",
            "component:api",
            "--json",
        ],
    );
    assert_eq!(ids(&json), vec![both]);
}

#[test]
fn test_top_level_list_single_label_regression() {
    let temp = setup_test_repo();
    let tagged = create_issue(temp.path(), "Tagged", &["epic:auth"]);
    create_issue(temp.path(), "Untagged", &[] as &[&str]);

    let json = run_json(temp.path(), &["list", "--label", "epic:auth", "--json"]);
    assert_eq!(ids(&json), vec![tagged]);
}

// ── bare `jit query` ─────────────────────────────────────────────────────

#[test]
fn test_query_bare_repeatable_label_ands() {
    let temp = setup_test_repo();
    let both = create_issue(temp.path(), "Both", &["epic:auth", "component:api"]);
    create_issue(temp.path(), "EpicOnly", &["epic:auth"]);

    let json = run_json(
        temp.path(),
        &[
            "query",
            "--label",
            "epic:auth",
            "--label",
            "component:api",
            "--json",
        ],
    );
    assert_eq!(ids(&json), vec![both]);
}

#[test]
fn test_query_bare_single_label_regression() {
    let temp = setup_test_repo();
    let tagged = create_issue(temp.path(), "Tagged", &["epic:auth"]);
    create_issue(temp.path(), "Untagged", &[] as &[&str]);

    let json = run_json(temp.path(), &["query", "--label", "epic:auth", "--json"]);
    assert_eq!(ids(&json), vec![tagged]);
}
