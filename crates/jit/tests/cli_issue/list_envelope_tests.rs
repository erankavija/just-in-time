//! Contract tests for the uniform JSON list envelope (jit:74fbdb69).
//!
//! Every list- and query-family command emits `--json` output as an object
//! carrying a numeric top-level `count` and a plural, collection-typed key
//! holding the array (`issues`, `gates`, `events`, `documents`, `leases`,
//! `namespaces`, `values`, `presets`, `assets`, `items`, `worktrees`, `roots`,
//! `dependents`, `findings`, `templates`, `results`). `count` always equals the
//! length of that array, so agents can parse a single shape without defensive
//! dual-path guards or bare-array fallbacks.

use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

/// Run `jit <args>` in `dir`, asserting success, and parse stdout as JSON.
fn run_json(dir: &TempDir, args: &[&str]) -> Value {
    let output = Command::new(jit_binary())
        .current_dir(dir.path())
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "command {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("command {:?} did not emit JSON: {e}", args))
}

/// Assert that `value` is an envelope object with a numeric `count` and a
/// `collection` array whose length equals `count`.
fn assert_envelope(value: &Value, collection: &str, args: &[&str]) {
    assert!(
        value.is_object(),
        "command {:?} must emit a JSON object, not {}",
        args,
        value
    );
    let count = value
        .get("count")
        .unwrap_or_else(|| panic!("command {:?} missing top-level `count`: {}", args, value));
    let count = count
        .as_u64()
        .unwrap_or_else(|| panic!("command {:?} `count` must be a number: {}", args, count));
    let array = value
        .get(collection)
        .unwrap_or_else(|| {
            panic!(
                "command {:?} missing `{collection}` collection: {}",
                args, value
            )
        })
        .as_array()
        .unwrap_or_else(|| panic!("command {:?} `{collection}` must be an array", args));
    assert_eq!(
        count as usize,
        array.len(),
        "command {:?} `count` must equal `{collection}` length; got count={count}, len={}",
        args,
        array.len()
    );
}

/// Initialise a git-backed jit repo with two issues and one linked document.
fn setup_repo() -> (TempDir, String) {
    let temp = TempDir::new().unwrap();
    let git = |args: &[&str]| {
        Command::new("git")
            .current_dir(temp.path())
            .args(args)
            .output()
            .unwrap();
    };
    git(&["init"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "Test"]);
    // Claim/worktree commands need a HEAD commit for worktree identity.
    git(&["commit", "--allow-empty", "-m", "init"]);

    Command::new(jit_binary())
        .current_dir(temp.path())
        .arg("init")
        .output()
        .unwrap();
    let created = run_json(
        &temp,
        &[
            "issue",
            "create",
            "--title",
            "First authentication task",
            "--description",
            "Body about auth",
            "--label",
            "type:task",
            "--json",
        ],
    );
    let id = created["id"].as_str().unwrap().to_string();
    run_json(
        &temp,
        &[
            "issue",
            "create",
            "--title",
            "Second authentication task",
            "--description",
            "More auth body",
            "--label",
            "type:task",
            "--json",
        ],
    );

    // A linked document so `doc list` / `doc assets list` have material.
    std::fs::write(temp.path().join("design.md"), "# Design\n\nBody.\n").unwrap();
    run_json(
        &temp,
        &[
            "doc",
            "add",
            &id,
            "design.md",
            "--doc-type",
            "design",
            "--json",
        ],
    );

    (temp, id)
}

#[test]
fn test_issue_list_envelope() {
    let (temp, _) = setup_repo();
    assert_envelope(
        &run_json(&temp, &["issue", "list", "--json"]),
        "issues",
        &["issue", "list"],
    );
}

#[test]
fn test_query_family_envelopes() {
    let (temp, _) = setup_repo();
    for sub in ["all", "available", "blocked", "strategic", "closed"] {
        let args = ["query", sub, "--json"];
        assert_envelope(&run_json(&temp, &args), "issues", &args);
    }
    // `ready` is a visible alias of `available`.
    assert_envelope(
        &run_json(&temp, &["query", "ready", "--json"]),
        "issues",
        &["query", "ready"],
    );
}

#[test]
fn test_issue_search_envelope() {
    let (temp, _) = setup_repo();
    assert_envelope(
        &run_json(&temp, &["issue", "search", "authentication", "--json"]),
        "issues",
        &["issue", "search"],
    );
}

#[test]
fn test_content_search_envelope() {
    let (temp, _) = setup_repo();
    assert_envelope(
        &run_json(&temp, &["search", "authentication", "--json"]),
        "results",
        &["search"],
    );
}

#[test]
fn test_gate_list_envelope() {
    let (temp, _) = setup_repo();
    assert_envelope(
        &run_json(&temp, &["gate", "list", "--json"]),
        "gates",
        &["gate", "list"],
    );
}

#[test]
fn test_gate_preset_list_envelope() {
    let (temp, _) = setup_repo();
    assert_envelope(
        &run_json(&temp, &["gate", "preset", "list", "--json"]),
        "presets",
        &["gate", "preset", "list"],
    );
}

#[test]
fn test_events_envelopes() {
    let (temp, _) = setup_repo();
    assert_envelope(
        &run_json(&temp, &["events", "tail", "--json"]),
        "events",
        &["events", "tail"],
    );
    assert_envelope(
        &run_json(&temp, &["events", "query", "--json"]),
        "events",
        &["events", "query"],
    );
}

#[test]
fn test_doc_list_envelope() {
    let (temp, id) = setup_repo();
    assert_envelope(
        &run_json(&temp, &["doc", "list", &id, "--json"]),
        "documents",
        &["doc", "list"],
    );
}

#[test]
fn test_doc_assets_list_envelope() {
    let (temp, id) = setup_repo();
    assert_envelope(
        &run_json(
            &temp,
            &["doc", "assets", "list", &id, "design.md", "--json"],
        ),
        "assets",
        &["doc", "assets", "list"],
    );
}

#[test]
fn test_claim_list_envelope() {
    let (temp, _) = setup_repo();
    assert_envelope(
        &run_json(&temp, &["claim", "list", "--json"]),
        "leases",
        &["claim", "list"],
    );
}

#[test]
fn test_label_envelopes() {
    let (temp, _) = setup_repo();
    assert_envelope(
        &run_json(&temp, &["label", "namespaces", "--json"]),
        "namespaces",
        &["label", "namespaces"],
    );
    assert_envelope(
        &run_json(&temp, &["label", "values", "type", "--json"]),
        "values",
        &["label", "values"],
    );
}

#[test]
fn test_item_envelopes() {
    let (temp, _) = setup_repo();
    assert_envelope(
        &run_json(&temp, &["item", "list", "--json"]),
        "items",
        &["item", "list"],
    );
    assert_envelope(
        &run_json(&temp, &["item", "search", "task", "--json"]),
        "items",
        &["item", "search"],
    );
}

#[test]
fn test_worktree_list_envelope() {
    let (temp, _) = setup_repo();
    assert_envelope(
        &run_json(&temp, &["worktree", "list", "--json"]),
        "worktrees",
        &["worktree", "list"],
    );
}

#[test]
fn test_worktree_store_divergence_envelope() {
    let (temp, _) = setup_repo();
    assert_envelope(
        &run_json(&temp, &["worktree", "store-divergence", "--json"]),
        "divergences",
        &["worktree", "store-divergence"],
    );
}

#[test]
fn test_graph_envelopes() {
    let (temp, id) = setup_repo();
    assert_envelope(
        &run_json(&temp, &["graph", "roots", "--json"]),
        "roots",
        &["graph", "roots"],
    );
    assert_envelope(
        &run_json(&temp, &["graph", "rdeps", &id, "--json"]),
        "dependents",
        &["graph", "rdeps"],
    );
}

#[test]
fn test_invariant_check_envelope() {
    let (temp, _) = setup_repo();
    assert_envelope(
        &run_json(&temp, &["invariant", "check", "--json"]),
        "findings",
        &["invariant", "check"],
    );
}

#[test]
fn test_issue_show_multi_id_envelope() {
    let (temp, id) = setup_repo();
    // Multiple ids must return an envelope, not a bare array.
    let value = run_json(&temp, &["issue", "show", &id, &id, "--json"]);
    assert_envelope(&value, "issues", &["issue", "show", "<id>", "<id>"]);
}

/// Run `jit <args>` and parse stdout as JSON regardless of exit code (some
/// readiness commands emit their envelope while exiting nonzero).
fn run_json_any_exit(dir: &TempDir, args: &[&str]) -> Value {
    let output = Command::new(jit_binary())
        .current_dir(dir.path())
        .args(args)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("command {:?} did not emit JSON: {e}", args))
}

#[test]
fn test_graph_deps_envelope() {
    let (temp, id) = setup_repo();
    // Make `id` depend on the other issue so the tree has a node.
    let all = run_json(&temp, &["query", "all", "--json"]);
    let other = all["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["id"].as_str())
        .find(|s| *s != id.as_str())
        .expect("a second issue")
        .to_string();
    Command::new(jit_binary())
        .current_dir(temp.path())
        .args(["dep", "add", &id, &other])
        .output()
        .unwrap();

    let value = run_json(&temp, &["graph", "deps", &id, "--json"]);
    assert_envelope(&value, "nodes", &["graph", "deps"]);
    assert!(
        value["count"].as_u64().unwrap() >= 1,
        "graph deps count should be >= 1 after adding a dependency"
    );
}

#[test]
fn test_gate_status_all_envelope() {
    let (temp, id) = setup_repo();
    // Register and require a manual gate; unattested it stays pending, so
    // status-all exits 4 while still emitting the envelope.
    let jit = |args: &[&str]| {
        Command::new(jit_binary())
            .current_dir(temp.path())
            .args(args)
            .output()
            .unwrap();
    };
    jit(&[
        "gate",
        "define",
        "envtest-gate",
        "--title",
        "Env Test",
        "--description",
        "gate for envelope test",
    ]);
    jit(&["gate", "add", &id, "envtest-gate"]);

    let value = run_json_any_exit(&temp, &["gate", "status-all", &id, "--json"]);
    assert_envelope(&value, "gates", &["gate", "status-all"]);
    assert!(
        value["count"].as_u64().unwrap() >= 1,
        "gate status-all count should be >= 1 after requiring a gate"
    );
}
