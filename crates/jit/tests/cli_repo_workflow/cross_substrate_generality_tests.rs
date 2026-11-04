//! Acceptance test for cross-substrate generality across four representative item
//! kinds among those supplied by the `jit-default` package (JIT issue 72cdf315,
//! REQ-04). Two more package-supplied kinds, `rule` (jit:cdc33a0f, registry-first over `.jit/rules.toml`)
//! and `gate` (jit:bb7d57a2, registry-first over `.jit/gates.toml`), are each
//! covered by their own resolution test and are not this fixture's concern.
//!
//! ## What this file proves
//!
//! These four item kinds are declared by the `jit-default` package's
//! `[item_kinds]` table (the engine bakes in none) and route through ONE generic engine
//! (`list_items` / `search_items` / `show_item` in `commands/item.rs`):
//!
//! - **issue-scope, markdown-first** — `requirement` (`## Success Criteria`,
//!   `REQ-NN`), `decision` (`## Decisions`, `D-NN`), and `risk` (`## Risks`,
//!   `RISK-NN`) are parsed from an issue description and addressed as
//!   `@/issue/<short-id>/<kind>/<self-id>`.
//! - **project-scope, registry-first** — `invariant` is read from
//!   `.jit/invariants.toml` and addressed as `@/<kind>/<self-id>`.
//!
//! A single test (`test_all_four_kinds_through_one_generic_path`) exercises all
//! four through `jit item list`, `jit item list --kind <X>`, `jit item show`, and
//! `jit item search`, asserting that both substrates surface through the SAME
//! operations and that no separate command exists for either.
//!
//! ## Success-criteria traceability
//!
//! - REQ-01: all four kinds enumerated through `list`, `show`, `search` — see
//!   the single test function below.
//! - REQ-02: both substrates (issue-scope AND `@`-scope) surface through ONE
//!   unfiltered `jit item list` — comments and assertions within the test make
//!   this explicit.
//! - REQ-03: the test passes under `cargo test`.

use jit::domain::identity::IssueId;
use jit::test_cli::JitCli;
use jit::test_utils::profiled_repository_fixture;
use serde_json::Value;

/// Write `content` to `.jit/<filename>` inside the repository.
fn write_jit_file(cli: &JitCli, filename: &str, content: &str) {
    std::fs::write(cli.root().join(".jit").join(filename), content).unwrap();
}

/// Extract the `qualified_id` strings from a `jit item list/search` JSON result.
fn qualified_ids(json: &Value) -> Vec<&str> {
    json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["qualified_id"].as_str().unwrap())
        .collect()
}

// ---------------------------------------------------------------------------
// REQ-01 + REQ-02 + REQ-03: single cohesive acceptance test
// ---------------------------------------------------------------------------

/// Cross-substrate generality: all four canonical item kinds resolve through the
/// SAME `jit item list` / `jit item show` / `jit item search` operations with no
/// separate command or implementation for either substrate.
///
/// ## Fixture
///
/// - One issue whose description carries a `## Success Criteria` section
///   (a `REQ-01` requirement), a `## Decisions` section (a `D-01` decision),
///   and a `## Risks` section (a `RISK-01` risk) — three issue-scope,
///   markdown-first items.
/// - A `.jit/invariants.toml` with one invariant — the project-scope,
///   registry-first item.
///
/// ## Assertions (REQ-01)
///
/// 1. `jit item list` (unfiltered) returns items of ALL FOUR kinds.
/// 2. `jit item list --kind <each>` returns exactly the item(s) of that kind.
/// 3. `jit item show <qualified-id>` resolves one item of EACH kind:
///    - issue-scope: `<short>/<REQ-01>`, `<short>/D-01`, `<short>/RISK-01`
///    - project-scope: `@/invariant/<id>`
/// 4. `jit item search <term>` finds across kinds.
///
/// ## REQ-02 assertion
///
/// The unfiltered `jit item list` contains BOTH issue-scope items (scoped by the
/// issue short-id, sourced from markdown) AND the `@`-scoped invariant (sourced
/// from the registry). Both are resolved via the same `jit item show` command —
/// no separate command exists for either substrate.
#[test]
fn test_all_four_kinds_through_one_generic_path() {
    // A repository carrying the `jit-default` package's `[item_kinds]` table,
    // which declares this fixture's four kinds among the six it carries.
    let cli =
        JitCli::with_repository(profiled_repository_fixture("jit-default", "packages").unwrap());

    // --- Fixture: one issue with all three issue-scope kinds ---
    let issue_body = "\
## Success Criteria\n\n\
- [hard] REQ-01: writes must be atomic\n\n\
## Decisions\n\n\
- D-01: store issues as plain JSON files\n\n\
## Risks\n\n\
- RISK-01: concurrent writers may corrupt state\n";
    let issue: IssueId = cli
        .create_issue(["-t", "Cross-substrate fixture", "-d", issue_body])
        .unwrap()
        .parse()
        .expect("the binary prints a canonical issue id");
    let short = issue.short_id();

    // --- Fixture: one project-scope, registry-first invariant ---
    write_jit_file(
        &cli,
        "invariants.toml",
        "[[invariants]]\n\
         id = \"sample-invariant\"\n\
         statement = \"Every dependency edge stays acyclic.\"\n\
         kind = \"enforced\"\n\
         enforced-by = \"dag-no-cycles\"\n",
    );

    // -----------------------------------------------------------------------
    // REQ-01, part 1: `jit item list` (unfiltered) returns items of ALL FOUR
    // kinds through ONE operation. No separate command routes to either
    // substrate — the same `list_items` code path serves both.
    //
    // REQ-02: both issue-scope items (markdown-first, scoped by `<short>`) and
    // the project-scope invariant (registry-first, scoped `@`) appear in the
    // SAME unfiltered output. Neither substrate requires a separate command.
    // -----------------------------------------------------------------------
    let all = cli.run_json(["item", "list"]).unwrap();
    let all_qids = qualified_ids(&all);

    // Issue-scope, markdown-first items mint the uniform kind-segmented address
    // carrying the reserved `issue` segment, the short-id, the kind, and the self-id.
    let req_qid = format!("@/issue/{short}/requirement/REQ-01");
    let dec_qid = format!("@/issue/{short}/decision/D-01");
    let risk_qid = format!("@/issue/{short}/risk/RISK-01");

    assert!(
        all_qids.contains(&req_qid.as_str()),
        "unfiltered list must include the requirement {req_qid}: {all_qids:?}"
    );
    assert!(
        all_qids.contains(&dec_qid.as_str()),
        "unfiltered list must include the decision {dec_qid}: {all_qids:?}"
    );
    assert!(
        all_qids.contains(&risk_qid.as_str()),
        "unfiltered list must include the risk {risk_qid}: {all_qids:?}"
    );

    // Project-scope, registry-first item is scoped by `@` — NOT by any issue.
    // Both it and the issue-scope items surface through the SAME operation
    // (REQ-02): there is no separate `jit invariant list` or `jit item
    // list-registry`; the same `list_items` code path routes to the registry.
    assert!(
        all_qids.contains(&"@/invariant/sample-invariant"),
        "unfiltered list must include the invariant @/invariant/sample-invariant alongside issue-scope items: {all_qids:?}"
    );

    // Confirm the kinds are correctly tagged.
    let items = all["items"].as_array().unwrap();
    let kind_of = |qid: &str| {
        items
            .iter()
            .find(|i| i["qualified_id"].as_str() == Some(qid))
            .and_then(|i| i["kind"].as_str())
    };
    assert_eq!(kind_of(&req_qid), Some("requirement"));
    assert_eq!(kind_of(&dec_qid), Some("decision"));
    assert_eq!(kind_of(&risk_qid), Some("risk"));
    assert_eq!(kind_of("@/invariant/sample-invariant"), Some("invariant"));

    // Confirm substrate routing: issue-scope items are scoped by the issue
    // short-id; the invariant is `@`-scoped (registry, no owning issue).
    let inv = items
        .iter()
        .find(|i| i["qualified_id"] == "@/invariant/sample-invariant")
        .unwrap();
    assert_eq!(
        inv["scope"].as_str(),
        Some("@"),
        "invariant must carry the project scope '@'"
    );

    // -----------------------------------------------------------------------
    // REQ-01, part 2: `jit item list --kind <X>` returns ONLY the items of
    // that kind — the same generic path with a filter, for each of the four.
    // -----------------------------------------------------------------------

    let reqs = cli
        .run_json(["item", "list", "--kind", "requirement"])
        .unwrap();
    let req_qids = qualified_ids(&reqs);
    assert!(
        req_qids.contains(&req_qid.as_str()),
        "--kind requirement must return REQ-01: {req_qids:?}"
    );
    assert!(
        !req_qids.iter().any(|q| *q == "@/invariant/sample-invariant"
            || q.contains("/D-01")
            || q.contains("/RISK-01")),
        "--kind requirement must not return decisions, risks, or invariants: {req_qids:?}"
    );

    let decs = cli
        .run_json(["item", "list", "--kind", "decision"])
        .unwrap();
    let dec_qids = qualified_ids(&decs);
    assert!(
        dec_qids.contains(&dec_qid.as_str()),
        "--kind decision must return D-01: {dec_qids:?}"
    );
    assert!(
        !dec_qids.iter().any(|q| *q == "@/invariant/sample-invariant"
            || q.contains("/REQ-01")
            || q.contains("/RISK-01")),
        "--kind decision must not return requirements, risks, or invariants: {dec_qids:?}"
    );

    let risks = cli.run_json(["item", "list", "--kind", "risk"]).unwrap();
    let risk_qids = qualified_ids(&risks);
    assert!(
        risk_qids.contains(&risk_qid.as_str()),
        "--kind risk must return RISK-01: {risk_qids:?}"
    );
    assert!(
        !risk_qids
            .iter()
            .any(|q| *q == "@/invariant/sample-invariant"
                || q.contains("/REQ-01")
                || q.contains("/D-01")),
        "--kind risk must not return requirements, decisions, or invariants: {risk_qids:?}"
    );

    let invs = cli
        .run_json(["item", "list", "--kind", "invariant"])
        .unwrap();
    let inv_qids = qualified_ids(&invs);
    assert!(
        inv_qids.contains(&"@/invariant/sample-invariant"),
        "--kind invariant must return @/invariant/sample-invariant: {inv_qids:?}"
    );
    assert!(
        !inv_qids
            .iter()
            .any(|q| q.contains("/REQ-01") || q.contains("/D-01") || q.contains("/RISK-01")),
        "--kind invariant must not return issue-scope items: {inv_qids:?}"
    );

    // -----------------------------------------------------------------------
    // REQ-01, part 3: `jit item show <qualified-id>` resolves one item of
    // EACH kind — the same `show_item` code path, no substrate-specific
    // variant.
    //
    // Issue-scope items: `@/issue/<short>/{requirement,decision,risk}/<self-id>`
    // Project-scope item: `@/invariant/sample-invariant`
    //
    // REQ-02: all four resolve through the SAME `jit item show` command.
    // -----------------------------------------------------------------------

    // requirement (issue-scope, markdown-first)
    let shown_req = cli.run_json(["item", "show", &req_qid]).unwrap();
    assert_eq!(shown_req["item"]["self_id"].as_str(), Some("REQ-01"));
    assert_eq!(shown_req["item"]["kind"].as_str(), Some("requirement"));
    assert_eq!(
        shown_req["item"]["qualified_id"].as_str(),
        Some(req_qid.as_str())
    );
    assert!(
        shown_req["item"]["text"]
            .as_str()
            .unwrap()
            .contains("atomic"),
        "requirement text must contain 'atomic'"
    );
    // Issue-scope items carry an owning issue reference.
    assert!(
        shown_req["issue_full_id"].as_str().is_some(),
        "requirement must carry an owning issue_full_id"
    );

    // decision (issue-scope, markdown-first)
    let shown_dec = cli.run_json(["item", "show", &dec_qid]).unwrap();
    assert_eq!(shown_dec["item"]["self_id"].as_str(), Some("D-01"));
    assert_eq!(shown_dec["item"]["kind"].as_str(), Some("decision"));
    assert_eq!(
        shown_dec["item"]["qualified_id"].as_str(),
        Some(dec_qid.as_str())
    );
    assert!(
        shown_dec["item"]["text"].as_str().unwrap().contains("JSON"),
        "decision text must contain 'JSON'"
    );
    assert!(
        shown_dec["issue_full_id"].as_str().is_some(),
        "decision must carry an owning issue_full_id"
    );

    // risk (issue-scope, markdown-first)
    let shown_risk = cli.run_json(["item", "show", &risk_qid]).unwrap();
    assert_eq!(shown_risk["item"]["self_id"].as_str(), Some("RISK-01"));
    assert_eq!(shown_risk["item"]["kind"].as_str(), Some("risk"));
    assert_eq!(
        shown_risk["item"]["qualified_id"].as_str(),
        Some(risk_qid.as_str())
    );
    assert!(
        shown_risk["item"]["text"]
            .as_str()
            .unwrap()
            .contains("corrupt"),
        "risk text must contain 'corrupt'"
    );
    assert!(
        shown_risk["issue_full_id"].as_str().is_some(),
        "risk must carry an owning issue_full_id"
    );

    // invariant (project-scope, registry-first) — the same `jit item show`
    // command resolves it; no separate command exists for registry items.
    let shown_inv = cli
        .run_json(["item", "show", "@/invariant/sample-invariant"])
        .unwrap();
    assert_eq!(
        shown_inv["item"]["self_id"].as_str(),
        Some("sample-invariant")
    );
    assert_eq!(shown_inv["item"]["kind"].as_str(), Some("invariant"));
    assert_eq!(
        shown_inv["item"]["qualified_id"].as_str(),
        Some("@/invariant/sample-invariant")
    );
    assert_eq!(shown_inv["item"]["scope"].as_str(), Some("@"));
    assert!(
        shown_inv["item"]["text"]
            .as_str()
            .unwrap()
            .contains("acyclic"),
        "invariant text must contain 'acyclic'"
    );
    // Project-scope items carry NO owning issue — the registry is the source.
    assert_eq!(
        shown_inv["issue_full_id"],
        Value::Null,
        "invariant must have no owning issue (registry-first, project-scope)"
    );
    assert_eq!(
        shown_inv["issue_title"],
        Value::Null,
        "invariant must have no issue_title (registry-first, project-scope)"
    );

    // -----------------------------------------------------------------------
    // REQ-01, part 4: `jit item search <term>` finds across both substrates
    // through ONE operation — the same `search_items` code path.
    // -----------------------------------------------------------------------

    // Search for a term unique to the requirement.
    let hits_atomic = cli.run_json(["item", "search", "atomic"]).unwrap();
    let atomic_qids = qualified_ids(&hits_atomic);
    assert!(
        atomic_qids.contains(&req_qid.as_str()),
        "search 'atomic' must find the requirement: {atomic_qids:?}"
    );
    assert!(
        !atomic_qids.contains(&"@/invariant/sample-invariant"),
        "search 'atomic' must not return the invariant: {atomic_qids:?}"
    );

    // Search for a term unique to the decision.
    let hits_json = cli.run_json(["item", "search", "JSON"]).unwrap();
    let json_qids = qualified_ids(&hits_json);
    assert!(
        json_qids.contains(&dec_qid.as_str()),
        "search 'JSON' must find the decision: {json_qids:?}"
    );

    // Search for a term unique to the risk.
    let hits_corrupt = cli.run_json(["item", "search", "corrupt"]).unwrap();
    let corrupt_qids = qualified_ids(&hits_corrupt);
    assert!(
        corrupt_qids.contains(&risk_qid.as_str()),
        "search 'corrupt' must find the risk: {corrupt_qids:?}"
    );

    // Search for a term unique to the invariant (registry-first) — the same
    // `search_items` that found issue-scope items above also reaches the
    // invariant registry, confirming the one-code-path claim (REQ-02).
    let hits_acyclic = cli.run_json(["item", "search", "acyclic"]).unwrap();
    let acyclic_qids = qualified_ids(&hits_acyclic);
    assert!(
        acyclic_qids.contains(&"@/invariant/sample-invariant"),
        "search 'acyclic' must find the invariant through the same search path: {acyclic_qids:?}"
    );
    assert!(
        !acyclic_qids
            .iter()
            .any(|q| q.contains("/REQ-01") || q.contains("/D-01") || q.contains("/RISK-01")),
        "search 'acyclic' must not return issue-scope items: {acyclic_qids:?}"
    );

    // Search by self-id across substrates confirms both are reachable.
    let hits_inv01 = cli
        .run_json(["item", "search", "sample-invariant"])
        .unwrap();
    assert!(
        qualified_ids(&hits_inv01).contains(&"@/invariant/sample-invariant"),
        "searching by self-id 'sample-invariant' must find the invariant: {:?}",
        qualified_ids(&hits_inv01)
    );

    let hits_req01 = cli.run_json(["item", "search", "REQ-01"]).unwrap();
    assert!(
        qualified_ids(&hits_req01).contains(&req_qid.as_str()),
        "searching by self-id 'REQ-01' must find the requirement: {:?}",
        qualified_ids(&hits_req01)
    );
}
