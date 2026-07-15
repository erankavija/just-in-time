//! Cross-surface contract for how an issue's gate list is named in
//! machine-readable output (jit:f40f1b0a).
//!
//! Two rules govern which shape the gate list takes:
//!
//! - Projected ISSUE VIEWS expose it as a `gates` array of `{key, status, ...}`
//!   objects, never the raw storage names. Covered here: `issue show`,
//!   `issue show --summary`, `issue status`.
//! - Raw RECORD DUMPS (any `--full` surface) emit the on-disk issue record
//!   verbatim, so the gate list stays under `gates_required` / `gates_status`,
//!   and their default summary shape omits it. Covered here: `graph export
//!   --full`, `query all --full`, `issue list --full`, top-level `list --full`,
//!   `issue search --full`.
//!
//! These asserts pin the emitted side; `crate::schema` unit tests pin the
//! matching `jit --schema` declaration, so a rename on either side fails the
//! build (REQ-04).

use assert_cmd::prelude::*;
use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn jit(temp: &TempDir, args: &[&str]) -> Vec<u8> {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone()
}

/// Create an issue carrying one required (manual) gate so every surface has a
/// non-empty gate list to project.
fn setup_repo_with_gated_issue() -> (TempDir, String) {
    let temp = TempDir::new().unwrap();
    jit(&temp, &["init"]);

    let stdout = jit(
        &temp,
        &[
            "issue",
            "create",
            "--title",
            "Gated",
            "--description",
            "Body",
        ],
    );
    let stdout = String::from_utf8_lossy(&stdout);
    let id = stdout
        .lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    jit(
        &temp,
        &[
            "gate",
            "define",
            "manual-gate",
            "--title",
            "Manual",
            "--description",
            "Manual gate",
            "--mode",
            "manual",
        ],
    );
    jit(
        &temp,
        &["issue", "update", &id, "--add-gate", "manual-gate"],
    );
    (temp, id)
}

fn json(temp: &TempDir, args: &[&str]) -> Value {
    serde_json::from_slice(&jit(temp, args)).unwrap()
}

/// Assert the value carries the projected `gates` array and neither raw
/// storage field.
fn assert_gates_array(view: &Value, ctx: &str) {
    let gates = view["gates"]
        .as_array()
        .unwrap_or_else(|| panic!("{ctx}: `gates` must be an array; got: {view}"));
    assert_eq!(
        gates.len(),
        1,
        "{ctx}: one entry per required gate; got: {view}"
    );
    assert_eq!(
        gates[0]["key"].as_str(),
        Some("manual-gate"),
        "{ctx}: gate entry carries its key; got: {view}"
    );
    assert!(
        gates[0]["status"].is_string(),
        "{ctx}: gate entry carries a status; got: {view}"
    );
    assert!(
        view.get("gates_required").is_none(),
        "{ctx}: projected view must not carry the storage field gates_required; got: {view}"
    );
    assert!(
        view.get("gates_status").is_none(),
        "{ctx}: projected view must not carry the storage field gates_status; got: {view}"
    );
}

#[test]
fn test_issue_show_full_exposes_gates_array() {
    let (temp, id) = setup_repo_with_gated_issue();
    let view = json(&temp, &["issue", "show", &id, "--json"]);
    assert_gates_array(&view, "issue show --json");
}

#[test]
fn test_issue_show_summary_exposes_gates_array() {
    let (temp, id) = setup_repo_with_gated_issue();
    let view = json(&temp, &["issue", "show", &id, "--summary", "--json"]);
    assert_gates_array(&view, "issue show --summary --json");
}

#[test]
fn test_issue_status_exposes_gates_array() {
    let (temp, id) = setup_repo_with_gated_issue();
    let view = json(&temp, &["issue", "status", &id, "--json"]);
    assert_gates_array(&view, "issue status --json");
}

#[test]
fn test_graph_export_full_keeps_storage_gate_fields() {
    let (temp, id) = setup_repo_with_gated_issue();
    let doc = json(&temp, &["graph", "export", "--format", "json", "--full"]);
    let node = doc["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"].as_str() == Some(id.as_str()))
        .expect("full export node for the gated issue");

    assert!(
        node["gates_required"]
            .as_array()
            .is_some_and(|g| g.iter().any(|k| k == "manual-gate")),
        "graph export --full node carries the storage gates_required list; got: {node}"
    );
    assert!(
        node["gates_status"].is_object(),
        "graph export --full node carries the storage gates_status map; got: {node}"
    );
    assert!(
        node.get("gates").is_none(),
        "raw record dump must not carry the projected `gates` array; got: {node}"
    );
}

/// Pick the entry for `id` out of an `{issues: [...]}` list-envelope response.
fn find_issue<'a>(envelope: &'a Value, id: &str, ctx: &str) -> &'a Value {
    envelope["issues"]
        .as_array()
        .unwrap_or_else(|| panic!("{ctx}: `issues` must be an array; got: {envelope}"))
        .iter()
        .find(|n| n["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("{ctx}: record for the gated issue; got: {envelope}"))
}

/// Assert a raw-record dump entry carries the on-disk storage gate fields and
/// not the projected `gates` array.
fn assert_storage_record(record: &Value, ctx: &str) {
    assert!(
        record["gates_required"]
            .as_array()
            .is_some_and(|g| g.iter().any(|k| k == "manual-gate")),
        "{ctx}: --full record carries the storage gates_required list; got: {record}"
    );
    assert!(
        record["gates_status"].is_object(),
        "{ctx}: --full record carries the storage gates_status map; got: {record}"
    );
    assert!(
        record.get("gates").is_none(),
        "{ctx}: raw record dump must not carry the projected `gates` array; got: {record}"
    );
}

/// Assert a lean summary entry omits the gate list under every name.
fn assert_summary_omits_gates(record: &Value, ctx: &str) {
    for absent in ["gates", "gates_required", "gates_status"] {
        assert!(
            record.get(absent).is_none(),
            "{ctx}: summary omits the gate list ({absent}); got: {record}"
        );
    }
}

#[test]
fn test_query_full_keeps_storage_gate_fields_and_summary_omits_them() {
    let (temp, id) = setup_repo_with_gated_issue();

    let full = json(&temp, &["query", "all", "--full", "--json"]);
    assert_storage_record(
        find_issue(&full, &id, "query all --full"),
        "query all --full",
    );

    let summary = json(&temp, &["query", "all", "--json"]);
    assert_summary_omits_gates(find_issue(&summary, &id, "query all"), "query all");
}

#[test]
fn test_issue_list_full_keeps_storage_gate_fields_and_summary_omits_them() {
    let (temp, id) = setup_repo_with_gated_issue();

    let full = json(&temp, &["issue", "list", "--full", "--json"]);
    assert_storage_record(
        find_issue(&full, &id, "issue list --full"),
        "issue list --full",
    );

    let summary = json(&temp, &["issue", "list", "--json"]);
    assert_summary_omits_gates(find_issue(&summary, &id, "issue list"), "issue list");
}

#[test]
fn test_top_level_list_full_keeps_storage_gate_fields() {
    let (temp, id) = setup_repo_with_gated_issue();

    // Top-level `list` normalizes to `issue list`, so `--full` must expose the
    // same raw record.
    let full = json(&temp, &["list", "--full", "--json"]);
    assert_storage_record(find_issue(&full, &id, "list --full"), "list --full");
}

#[test]
fn test_issue_search_full_keeps_storage_gate_fields_and_summary_omits_them() {
    let (temp, id) = setup_repo_with_gated_issue();

    let full = json(&temp, &["issue", "search", "Gated", "--full", "--json"]);
    assert_storage_record(
        find_issue(&full, &id, "issue search --full"),
        "issue search --full",
    );

    let summary = json(&temp, &["issue", "search", "Gated", "--json"]);
    assert_summary_omits_gates(find_issue(&summary, &id, "issue search"), "issue search");
}
