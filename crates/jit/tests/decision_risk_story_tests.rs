//! Acceptance tests for the decision + risk item kinds story (issue `205472c4`).
//!
//! This is a STORY-LEVEL test file whose distinct contribution is exercising
//! decision and risk kinds TOGETHER — both present in ONE issue description,
//! sharing ONE generic code path — in a DEFAULT-initialized repo (no custom
//! `[item_kinds]` config). The unit-kind files (`decision_kind_tests.rs`,
//! `risk_kind_tests.rs`) test each kind in isolation; this file proves they
//! coexist correctly and that the shared projection mechanism handles them both
//! without interference.
//!
//! ## Built-in kind tuples exercised
//!
//! ```text
//! decision:
//!   section         = "decisions"     # authored under `## Decisions`
//!   id-pattern      = "D-[0-9]+"      # self-ids: D-01, D-02, …
//!   link-namespaces = ["per"]         # `per:<issue>/D-01`
//!   scope           = "issue"
//!
//! risk:
//!   section         = "risks"         # authored under `## Risks`
//!   id-pattern      = "RISK-[0-9]+"   # self-ids: RISK-01, RISK-02, …
//!   link-namespaces = ["mitigates",   # `mitigates:<issue>/RISK-01`
//!                      "resolves"]    # `resolves:<issue>/RISK-01`
//!   scope           = "issue"
//! ```
//!
//! ## Coverage of the story's `[hard]` success criteria
//!
//! All tests run against a DEFAULT repo (no custom config).
//!
//! - REQ-01: [`test_story_item_list_decision_and_risk_coexist`] — `jit item list
//!   --kind decision` returns only decisions; `jit item list --kind risk` returns
//!   only risks, both parsed from the SAME issue carrying BOTH sections.
//! - REQ-02: same test — `jit item list --kind risk` returns risk items.
//! - REQ-03: [`test_story_per_label_resolves_decision`] — a `per:<issue>/D-01`
//!   label resolves to the decision item via the generic `resolve_link_label`.
//! - REQ-04: [`test_story_mitigates_and_resolves_labels_resolve_risk`] — BOTH a
//!   `mitigates:<issue>/RISK-01` AND a `resolves:<issue>/RISK-01` label resolve to
//!   the risk item via the same generic path.
//! - REQ-05: [`test_story_markdown_sole_source_both_kinds`] — editing the
//!   description (the only source) changes BOTH indexes, and no top-level `.jit/`
//!   file duplicates either `D-01` or `RISK-01`.

use jit::commands::CommandExecutor;
use jit::domain::Issue;
use jit::storage::{IssueStore, JsonFileStorage};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

/// `jit init` a fresh, DEFAULT repo in a tempdir (no custom `[item_kinds]`).
fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let output = Command::new(jit_binary())
        .arg("init")
        .current_dir(temp.path())
        .output()
        .expect("Failed to run jit init");
    assert!(output.status.success(), "jit init failed");
    temp
}

/// Create an issue with `body` through the real CLI and return its 8-char short id.
fn create_issue(repo: &Path, title: &str, body: &str) -> String {
    let output = Command::new(jit_binary())
        .args(["issue", "create", "-t", title, "-d", body, "--json"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "issue create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let full_id = json["id"].as_str().expect("created issue has an id");
    full_id.chars().take(8).collect()
}

/// Description body used throughout the story tests: one `## Decisions` section
/// with D-01 and one `## Risks` section with RISK-01, both in a single issue.
const STORY_BODY: &str = "\
## Decisions\n\
\n\
- D-01: adopt markdown-first item storage\n\
\n\
## Risks\n\
\n\
- RISK-01: parsing ambiguity between decision and risk ids\n\
";

/// Build a [`JsonFileStorage`]-backed executor over a DEFAULT `.jit` repo (no
/// custom `[item_kinds]` config), seed `issues`, and return it paired with the
/// seeded issues' short ids in insertion order.
///
/// With no config, the executor resolves the BUILT-IN default kind set which ships
/// `decision` (namespace `per`) and `risk` (namespaces `mitigates`, `resolves`) —
/// so this exercises shipped behavior, not a test-only override.
fn default_executor_with(
    repo: &Path,
    issues: Vec<(&str, &str)>,
) -> (CommandExecutor<JsonFileStorage>, Vec<String>) {
    let jit_dir = repo.join(".jit");
    std::fs::create_dir_all(&jit_dir).unwrap();
    let storage = JsonFileStorage::new(&jit_dir);
    storage.init().unwrap();
    let mut shorts = Vec::new();
    for (title, body) in issues {
        let issue = Issue::new(title.to_string(), body.to_string());
        shorts.push(issue.short_id());
        storage.save_issue(issue).unwrap();
    }
    (CommandExecutor::new(storage), shorts)
}

#[test]
fn test_story_item_list_decision_and_risk_coexist() {
    // REQ-01 + REQ-02: in a DEFAULT-initialized repo, an issue that carries BOTH a
    // `## Decisions` section and a `## Risks` section in its description is indexed
    // by BOTH kinds through the SAME generic code path. `jit item list --kind
    // decision` returns ONLY the decision items; `jit item list --kind risk`
    // returns ONLY the risk items — no kind bleeds into the other's index.
    let temp = setup_test_repo();
    let short = create_issue(temp.path(), "Architecture + risks", STORY_BODY);

    // REQ-01: decision kind returns the decision.
    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "decision", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "item list --kind decision failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["count"].as_u64().unwrap(),
        1,
        "exactly one decision item expected"
    );
    let dec_items = json["items"].as_array().unwrap();
    assert_eq!(dec_items[0]["kind"].as_str().unwrap(), "decision");
    assert_eq!(
        dec_items[0]["qualified_id"].as_str().unwrap(),
        format!("{short}/D-01")
    );
    // The decision index must NOT contain risk items.
    assert!(
        dec_items
            .iter()
            .all(|i| i["kind"].as_str().unwrap() == "decision"),
        "decision index must not contain non-decision items"
    );

    // REQ-02: risk kind returns the risk.
    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "risk", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "item list --kind risk failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["count"].as_u64().unwrap(),
        1,
        "exactly one risk item expected"
    );
    let risk_items = json["items"].as_array().unwrap();
    assert_eq!(risk_items[0]["kind"].as_str().unwrap(), "risk");
    assert_eq!(
        risk_items[0]["qualified_id"].as_str().unwrap(),
        format!("{short}/RISK-01")
    );
    // The risk index must NOT contain decision items.
    assert!(
        risk_items
            .iter()
            .all(|i| i["kind"].as_str().unwrap() == "risk"),
        "risk index must not contain non-risk items"
    );
}

#[test]
fn test_story_per_label_resolves_decision() {
    // REQ-03: a `per:<issue>/D-01` label resolves to the addressed decision item
    // through the generic `resolve_link_label`, even when the same issue also carries
    // a `## Risks` section. The coexistence of both kinds must not disrupt label
    // resolution for either kind.
    let temp = TempDir::new().unwrap();
    let (exec, shorts) =
        default_executor_with(temp.path(), vec![("Architecture + risks", STORY_BODY)]);
    let short = &shorts[0];

    let label = format!("per:{short}/D-01");
    let resolved = exec
        .resolve_link_label(&label)
        .unwrap()
        .expect("a per:<issue>/D-01 label resolves to the addressed decision");
    assert_eq!(resolved.item.self_id, "D-01");
    assert_eq!(resolved.item.kind, "decision");
    assert_eq!(resolved.item.qualified_id, format!("{short}/D-01"));
    assert!(
        resolved.item.text.contains("markdown-first"),
        "resolved decision text must match the authored entry"
    );

    // A registered `per:` namespace whose qualified id cannot be resolved is an
    // error, never a silent drop.
    let bad = format!("per:{short}/D-99");
    assert!(
        exec.resolve_link_label(&bad).is_err(),
        "unresolvable per: qualified id must be an error"
    );
}

#[test]
fn test_story_mitigates_and_resolves_labels_resolve_risk() {
    // REQ-04: BOTH a `mitigates:<issue>/RISK-01` AND a `resolves:<issue>/RISK-01`
    // label resolve to the addressed risk item through the generic
    // `resolve_link_label`, even when the same issue also carries a `## Decisions`
    // section. Both namespaces are recognized because the BUILT-IN `risk` kind ships
    // them; neither requires custom config.
    let temp = TempDir::new().unwrap();
    let (exec, shorts) =
        default_executor_with(temp.path(), vec![("Architecture + risks", STORY_BODY)]);
    let short = &shorts[0];

    // mitigates: namespace.
    let mit_label = format!("mitigates:{short}/RISK-01");
    let mit_resolved = exec
        .resolve_link_label(&mit_label)
        .unwrap()
        .expect("a mitigates:<issue>/RISK-01 label resolves to the addressed risk");
    assert_eq!(mit_resolved.item.self_id, "RISK-01");
    assert_eq!(mit_resolved.item.kind, "risk");
    assert_eq!(mit_resolved.item.qualified_id, format!("{short}/RISK-01"));
    assert!(
        mit_resolved.item.text.contains("parsing ambiguity"),
        "resolved risk text must match the authored entry"
    );

    // resolves: namespace.
    let res_label = format!("resolves:{short}/RISK-01");
    let res_resolved = exec
        .resolve_link_label(&res_label)
        .unwrap()
        .expect("a resolves:<issue>/RISK-01 label resolves to the addressed risk");
    assert_eq!(res_resolved.item.self_id, "RISK-01");
    assert_eq!(res_resolved.item.kind, "risk");
    assert_eq!(res_resolved.item.qualified_id, format!("{short}/RISK-01"));

    // Both namespaces ALSO return an error when the qualified id is unresolvable.
    let bad_mit = format!("mitigates:{short}/RISK-99");
    assert!(
        exec.resolve_link_label(&bad_mit).is_err(),
        "unresolvable mitigates: qualified id must be an error"
    );
    let bad_res = format!("resolves:{short}/RISK-99");
    assert!(
        exec.resolve_link_label(&bad_res).is_err(),
        "unresolvable resolves: qualified id must be an error"
    );
}

#[test]
fn test_story_markdown_sole_source_both_kinds() {
    // REQ-05: the markdown description is the ONLY source for BOTH decision and risk
    // items. The indexes are pure projections — no parallel structured store — so:
    //   (a) no top-level `.jit/` file (config, registry, event log) contains either
    //       `D-01` or `RISK-01`;
    //   (b) the owning issue file under `.jit/issues/` IS the sole occurrence of
    //       both ids;
    //   (c) editing the description and NOTHING else changes BOTH indexes.
    let temp = setup_test_repo();
    let short = create_issue(temp.path(), "Architecture + risks", STORY_BODY);

    // (a) No top-level `.jit/` file duplicates either id.
    let jit_dir = temp.path().join(".jit");
    for entry in std::fs::read_dir(&jit_dir).unwrap() {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.ends_with(".json") || name.ends_with(".jsonl") || name.ends_with(".toml") {
            let contents = std::fs::read_to_string(&path).unwrap();
            assert!(
                !contents.contains("D-01"),
                "no top-level .jit/ file may duplicate decision ids; found 'D-01' in {name}"
            );
            assert!(
                !contents.contains("RISK-01"),
                "no top-level .jit/ file may duplicate risk ids; found 'RISK-01' in {name}"
            );
        }
    }

    // (b) Both ids live in the owning issue description (the sole source).
    let mut found_decision = false;
    let mut found_risk = false;
    for entry in std::fs::read_dir(jit_dir.join("issues")).unwrap() {
        let contents = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        if contents.contains(short.as_str()) {
            if contents.contains("D-01") {
                found_decision = true;
            }
            if contents.contains("RISK-01") {
                found_risk = true;
            }
        }
    }
    assert!(
        found_decision,
        "D-01 must live in the issue description (the sole decision source)"
    );
    assert!(
        found_risk,
        "RISK-01 must live in the issue description (the sole risk source)"
    );

    // (c) Editing the description changes BOTH indexes. Replace D-01 with D-02 and
    // RISK-01 with RISK-02; the old ids must vanish, the new ones must appear.
    let updated_body = "\
## Decisions\n\
\n\
- D-02: switch to append-only log\n\
\n\
## Risks\n\
\n\
- RISK-02: log compaction complexity\n\
";
    let output = Command::new(jit_binary())
        .args(["issue", "update", &short, "-d", updated_body])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "issue update failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Decision index now shows D-02, not D-01.
    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "decision", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 1);
    let dec_self_ids: Vec<&str> = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["self_id"].as_str().unwrap())
        .collect();
    assert!(
        dec_self_ids.contains(&"D-02"),
        "edited decision D-02 must appear in the index"
    );
    assert!(
        !dec_self_ids.contains(&"D-01"),
        "removed decision D-01 must vanish from the index (markdown is the sole source)"
    );

    // Risk index now shows RISK-02, not RISK-01.
    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "risk", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 1);
    let risk_self_ids: Vec<&str> = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["self_id"].as_str().unwrap())
        .collect();
    assert!(
        risk_self_ids.contains(&"RISK-02"),
        "edited risk RISK-02 must appear in the index"
    );
    assert!(
        !risk_self_ids.contains(&"RISK-01"),
        "removed risk RISK-01 must vanish from the index (markdown is the sole source)"
    );
}
