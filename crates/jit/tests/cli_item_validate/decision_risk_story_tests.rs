//! Acceptance tests for decision and risk item kinds together.
//!
//! The suite checks that both kinds coexist in one issue description, stay
//! independently queryable, and continue to derive from that description.
use jit::test_cli::JitCli;
use jit::test_utils::setup_test_repo_with_config;

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

#[test]
fn test_story_item_list_decision_and_risk_coexist() {
    // REQ-01 + REQ-02: in a fixture-initialized repo, an issue that carries BOTH a
    // `## Decisions` section and a `## Risks` section in its description is indexed
    // by BOTH kinds through the SAME generic code path. `jit item list --kind
    // decision` returns ONLY the decision items; `jit item list --kind risk`
    // returns ONLY the risk items — no kind bleeds into the other's index.
    let cli = JitCli::with_repository(
        setup_test_repo_with_config(&crate::config_with_shipped_kinds(&["decision", "risk"]))
            .unwrap()
            .0,
    );
    let short = cli
        .create_issue(["-t", "Architecture + risks", "-d", STORY_BODY])
        .unwrap()[..8]
        .to_owned();

    // REQ-01: decision kind returns the decision.
    let json = cli
        .run_json(["item", "list", "--kind", "decision"])
        .unwrap();
    assert_eq!(
        json["items"].as_array().unwrap().len(),
        1,
        "exactly one decision item expected"
    );
    let dec_items = json["items"].as_array().unwrap();
    assert_eq!(dec_items[0]["kind"].as_str().unwrap(), "decision");
    assert_eq!(
        dec_items[0]["qualified_id"].as_str().unwrap(),
        format!("@/issue/{short}/decision/D-01")
    );
    // The decision index must NOT contain risk items.
    assert!(
        dec_items
            .iter()
            .all(|i| i["kind"].as_str().unwrap() == "decision"),
        "decision index must not contain non-decision items"
    );

    // REQ-02: risk kind returns the risk.
    let json = cli.run_json(["item", "list", "--kind", "risk"]).unwrap();
    assert_eq!(
        json["items"].as_array().unwrap().len(),
        1,
        "exactly one risk item expected"
    );
    let risk_items = json["items"].as_array().unwrap();
    assert_eq!(risk_items[0]["kind"].as_str().unwrap(), "risk");
    assert_eq!(
        risk_items[0]["qualified_id"].as_str().unwrap(),
        format!("@/issue/{short}/risk/RISK-01")
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
fn test_story_markdown_sole_source_both_kinds() {
    // REQ-05: the markdown description is the ONLY source for BOTH decision and risk
    // items. The indexes are pure projections — no parallel structured store — so:
    //   (a) no top-level `.jit/` file (config, registry, event log) contains either
    //       `D-01` or `RISK-01`;
    //   (b) the owning issue file under `.jit/issues/` IS the sole occurrence of
    //       both ids;
    //   (c) editing the description and NOTHING else changes BOTH indexes.
    let cli = JitCli::with_repository(
        setup_test_repo_with_config(&crate::config_with_shipped_kinds(&["decision", "risk"]))
            .unwrap()
            .0,
    );
    let short = cli
        .create_issue(["-t", "Architecture + risks", "-d", STORY_BODY])
        .unwrap()[..8]
        .to_owned();

    // (a) No top-level `.jit/` file duplicates either id.
    let jit_dir = cli.root().join(".jit");
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
    cli.run(["issue", "update", &short, "-d", updated_body])
        .unwrap();

    // Decision index now shows D-02, not D-01.
    let json = cli
        .run_json(["item", "list", "--kind", "decision"])
        .unwrap();
    assert_eq!(json["items"].as_array().unwrap().len(), 1);
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
    let json = cli.run_json(["item", "list", "--kind", "risk"]).unwrap();
    assert_eq!(json["items"].as_array().unwrap().len(), 1);
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
