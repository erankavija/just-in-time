//! Cross-surface contract for how an issue's gate list is named in
//! machine-readable output (jit:f40f1b0a).
//!
//! Two rules govern which shape the gate list takes:
//!
//! - Projected ISSUE VIEWS expose it as a `gates` array of `{key, status, ...}`
//!   objects, never the raw storage names. Covered here: `issue show`,
//!   `issue show --summary`, `issue status`, and `issue create` (which returns
//!   the same enriched projection as `issue show`).
//! - Raw RECORD DUMPS echo the on-disk issue record verbatim, so the gate list
//!   stays under `gates_required` / `gates_status`. Covered here: the `--full`
//!   list dumps (`graph export --full`, `query all --full`, `issue list --full`,
//!   top-level `list --full`, `issue search --full`) and the single-issue
//!   lifecycle mutation confirmations (`issue assign`, `unassign`, `reject`,
//!   `release`, `claim`, `claim-next`). The `--full` list dumps additionally
//!   omit the gate list from their default summary shape.
//!
//! A help-text assert guards the generated `issue show --help` against
//! reintroducing the storage names.
//!
//! These asserts pin the emitted side; `crate::schema` unit tests pin the
//! matching `jit --schema` declaration, so a rename on either side fails the
//! build (REQ-04).

use jit::test_cli::JitCli;
use serde_json::Value;

/// Create an issue carrying one required (manual) gate so every surface has a
/// non-empty gate list to project.
fn seed_gated_issue(temp: &JitCli) -> String {
    let id = temp
        .create_issue(["--title", "Gated", "--description", "Body"])
        .unwrap();

    temp.run([
        "gate",
        "define",
        "manual-gate",
        "--title",
        "Manual",
        "--description",
        "Manual gate",
        "--mode",
        "manual",
    ])
    .unwrap();
    temp.run(["issue", "update", &id, "--add-gate", "manual-gate"])
        .unwrap();
    id
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
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let id = seed_gated_issue(&temp);
    let view = temp.run_json(["issue", "show", &id]).unwrap();
    assert_gates_array(&view, "issue show --json");
}

#[test]
fn test_issue_show_summary_exposes_gates_array() {
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let id = seed_gated_issue(&temp);
    let view = temp.run_json(["issue", "show", &id, "--summary"]).unwrap();
    assert_gates_array(&view, "issue show --summary --json");
}

#[test]
fn test_issue_status_exposes_gates_array() {
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let id = seed_gated_issue(&temp);
    let view = temp.run_json(["issue", "status", &id]).unwrap();
    assert_gates_array(&view, "issue status --json");
}

#[test]
fn test_graph_export_full_keeps_storage_gate_fields() {
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let id = seed_gated_issue(&temp);
    let doc = temp
        .run_json(["graph", "export", "--format", "json", "--full"])
        .unwrap();
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
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let id = seed_gated_issue(&temp);

    let full = temp.run_json(["query", "all", "--full"]).unwrap();
    assert_storage_record(
        find_issue(&full, &id, "query all --full"),
        "query all --full",
    );

    let summary = temp.run_json(["query", "all"]).unwrap();
    assert_summary_omits_gates(find_issue(&summary, &id, "query all"), "query all");
}

#[test]
fn test_query_variant_full_dumps_keep_storage_gate_fields() {
    // The gated fixture issue is unassigned and unblocked, so it appears in
    // `query available`; rejecting it afterwards makes it a `query closed`
    // member. `query strategic` needs a strategic-typed issue, so one is
    // created with the same gate. Together these pin every documented
    // raw-record query variant to the storage shape.
    let taxonomy_repo = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let temp = JitCli::with_repository(taxonomy_repo.temp);
    let id = seed_gated_issue(&temp);

    let available = temp.run_json(["query", "available", "--full"]).unwrap();
    assert_storage_record(
        find_issue(&available, &id, "query available --full"),
        "query available --full",
    );
    assert_summary_omits_gates(
        find_issue(
            &temp.run_json(["query", "available"]).unwrap(),
            &id,
            "query available",
        ),
        "query available",
    );

    let strategic_type = taxonomy_repo
        .taxonomy
        .strategic_types
        .first()
        .expect("the declared vocabulary names a strategic type")
        .clone();
    let strategic_id = temp
        .create_issue([
            "--title",
            "Strategic gated",
            "--type",
            &strategic_type,
            "--description",
            "Body",
        ])
        .unwrap();
    temp.run([
        "issue",
        "update",
        &strategic_id,
        "--add-gate",
        "manual-gate",
    ])
    .unwrap();
    let strategic = temp.run_json(["query", "strategic", "--full"]).unwrap();
    assert_storage_record(
        find_issue(&strategic, &strategic_id, "query strategic --full"),
        "query strategic --full",
    );

    temp.run(["issue", "reject", &id]).unwrap();
    let closed = temp.run_json(["query", "closed", "--full"]).unwrap();
    assert_storage_record(
        find_issue(&closed, &id, "query closed --full"),
        "query closed --full",
    );
}

#[test]
fn test_issue_list_full_keeps_storage_gate_fields_and_summary_omits_them() {
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let id = seed_gated_issue(&temp);

    let full = temp.run_json(["issue", "list", "--full"]).unwrap();
    assert_storage_record(
        find_issue(&full, &id, "issue list --full"),
        "issue list --full",
    );

    let summary = temp.run_json(["issue", "list"]).unwrap();
    assert_summary_omits_gates(find_issue(&summary, &id, "issue list"), "issue list");
}

#[test]
fn test_top_level_list_full_keeps_storage_gate_fields() {
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let id = seed_gated_issue(&temp);

    // Top-level `list` normalizes to `issue list`, so `--full` must expose the
    // same raw record.
    let full = temp.run_json(["list", "--full"]).unwrap();
    assert_storage_record(find_issue(&full, &id, "list --full"), "list --full");
}

#[test]
fn test_issue_search_full_keeps_storage_gate_fields_and_summary_omits_them() {
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let id = seed_gated_issue(&temp);

    let full = temp
        .run_json(["issue", "search", "Gated", "--full"])
        .unwrap();
    assert_storage_record(
        find_issue(&full, &id, "issue search --full"),
        "issue search --full",
    );

    let summary = temp.run_json(["issue", "search", "Gated"]).unwrap();
    assert_summary_omits_gates(find_issue(&summary, &id, "issue search"), "issue search");
}

// ---------------------------------------------------------------------------
// Single-issue mutation confirmations echo the raw stored record, so the gate
// list stays under the storage names. `issue create` is the exception: it
// returns the same enriched projection as `issue show`, so it exposes `gates`.
// ---------------------------------------------------------------------------

#[test]
fn test_issue_create_exposes_gates_array() {
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    temp.run([
        "gate",
        "define",
        "manual-gate",
        "--title",
        "Manual",
        "--description",
        "Manual gate",
        "--mode",
        "manual",
    ])
    .unwrap();
    // `issue create --json` returns the `issue show` projection, so it exposes
    // the required gate under `gates`, never the storage split.
    let view = temp
        .run_json([
            "issue",
            "create",
            "--title",
            "Fresh",
            "--description",
            "Body",
            "--gate",
            "manual-gate",
        ])
        .unwrap();
    assert_gates_array(&view, "issue create --json");
}

#[test]
fn test_issue_assign_keeps_storage_gate_fields() {
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let id = seed_gated_issue(&temp);
    let record = temp
        .run_json(["issue", "assign", &id, "agent:worker-1"])
        .unwrap();
    assert_storage_record(&record, "issue assign");
}

#[test]
fn test_issue_unassign_keeps_storage_gate_fields() {
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let id = seed_gated_issue(&temp);
    temp.run(["issue", "assign", &id, "agent:worker-1"])
        .unwrap();
    let record = temp.run_json(["issue", "unassign", &id]).unwrap();
    assert_storage_record(&record, "issue unassign");
}

#[test]
fn test_issue_reject_keeps_storage_gate_fields() {
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let id = seed_gated_issue(&temp);
    let record = temp.run_json(["issue", "reject", &id]).unwrap();
    assert_storage_record(&record, "issue reject");
}

#[test]
fn test_issue_claim_keeps_storage_gate_fields() {
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let id = seed_gated_issue(&temp);
    let record = temp
        .run_json(["issue", "claim", &id, "agent:worker-1"])
        .unwrap();
    assert_storage_record(&record, "issue claim");
}

#[test]
fn test_issue_claim_next_keeps_storage_gate_fields() {
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let _id = seed_gated_issue(&temp);
    // The freshly created dependency-free issue auto-promotes to ready, so it is
    // the next claimable issue.
    let record = temp
        .run_json(["issue", "claim-next", "agent:worker-1"])
        .unwrap();
    assert_storage_record(&record, "issue claim-next");
}

#[test]
fn test_issue_release_keeps_storage_gate_fields() {
    let temp = JitCli::with_repository(jit::test_taxonomy::setup_test_repo_with_taxonomy().temp);
    let id = seed_gated_issue(&temp);
    temp.run(["issue", "claim", &id, "agent:worker-1"]).unwrap();
    let record = temp.run_json(["issue", "release", &id, "timeout"]).unwrap();
    assert_storage_record(&record, "issue release");
}

#[test]
fn test_issue_show_summary_help_names_gates_not_storage_fields() {
    let temp = JitCli::uninitialised().unwrap();
    // `--help` exits 0 and prints the generated command help, including the
    // `--summary` description that must promise `gates`, not the storage split.
    let out = temp.command(["issue", "show", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("gates"),
        "issue show --help should name the `gates` field; got: {text}"
    );
    assert!(
        !text.contains("gates_required") && !text.contains("gates_status"),
        "issue show --help must not promise the storage gate fields; got: {text}"
    );
}
