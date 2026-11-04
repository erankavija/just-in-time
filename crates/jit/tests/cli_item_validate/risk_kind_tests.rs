//! Integration tests for the markdown-first `risk` item kind.
//!
//! These exercise the shipped config through item list and verify that issue
//! descriptions remain the sole source for risk items.
use jit::test_cli::JitCli;
use jit::test_utils::setup_test_repo_with_config;

#[test]
fn test_default_repo_item_list_kind_risk_returns_risks() {
    // REQ-01: in a fixture-initialized repo with the configured `risk` kind, `jit item list
    // --kind risk` returns risk items parsed from an issue's `## Risks` section
    // — exercised through the actual binary.
    let cli = JitCli::with_repository(
        setup_test_repo_with_config(&crate::config_with_shipped_kinds(&["risk"]))
            .unwrap()
            .0,
    );
    let short = cli
        .create_issue([
            "-t",
            "Migration plan",
            "-d",
            "## Risks\n\n- RISK-01: data loss during migration\n- RISK-02: performance regression\n",
        ])
        .unwrap()[..8]
        .to_owned();

    let json = cli.run_json(["item", "list", "--kind", "risk"]).unwrap();
    assert_eq!(json["items"].as_array().unwrap().len(), 2);
    let items = json["items"].as_array().unwrap();
    for item in items {
        assert_eq!(item["kind"].as_str().unwrap(), "risk");
    }
    let qids: Vec<&str> = items
        .iter()
        .map(|i| i["qualified_id"].as_str().unwrap())
        .collect();
    assert!(qids.contains(&format!("@/issue/{short}/risk/RISK-01").as_str()));
    assert!(qids.contains(&format!("@/issue/{short}/risk/RISK-02").as_str()));
}

#[test]
fn test_default_repo_markdown_is_sole_source_for_risks() {
    // REQ-03: the markdown description is the ONLY source for risk items. The
    // index is a pure projection — there is no parallel structured store — so
    // editing the description (and nothing else) changes what `jit item list`
    // returns, and no risk store file is written under `.jit/`.
    let cli = JitCli::with_repository(
        setup_test_repo_with_config(&crate::config_with_shipped_kinds(&["risk"]))
            .unwrap()
            .0,
    );
    let short = cli
        .create_issue([
            "-t",
            "Migration plan",
            "-d",
            "## Risks\n\n- RISK-01: data loss during migration\n",
        ])
        .unwrap()[..8]
        .to_owned();

    let listed = cli.run_json(["item", "list", "--kind", "risk"]).unwrap();
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);

    // No parallel structured store: nothing under `.jit/` persists risks; the
    // ONLY place RISK-01 appears is the issue's own JSON description.
    let jit_dir = cli.root().join(".jit");
    for entry in std::fs::read_dir(&jit_dir).unwrap() {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        // Any TOP-LEVEL `.jit` file (config/registry/event log) must not duplicate
        // the risk id; the issue's markdown (under `issues/`) is the sole source
        // and is checked separately below.
        if name.ends_with(".json") || name.ends_with(".jsonl") || name.ends_with(".toml") {
            let contents = std::fs::read_to_string(&path).unwrap();
            assert!(
                !contents.contains("RISK-01"),
                "no parallel structured store may duplicate risks; \
                 found 'RISK-01' in {name}"
            );
        }
    }
    // The risk DOES live in the owning issue's description (the sole source).
    let mut found_in_issue = false;
    for entry in std::fs::read_dir(jit_dir.join("issues")).unwrap() {
        let contents = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        if contents.contains(short.as_str()) && contents.contains("RISK-01") {
            found_in_issue = true;
            break;
        }
    }
    assert!(
        found_in_issue,
        "the risk must live in the issue description (the sole markdown source)"
    );

    // Editing the markdown description (the sole source) changes the index: drop
    // RISK-01, add RISK-02 and RISK-03.
    cli.run([
        "issue",
        "update",
        &short,
        "-d",
        "## Risks\n\n- RISK-02: schema drift\n- RISK-03: timeline overrun\n",
    ])
    .unwrap();

    // The index now reflects ONLY the edited markdown.
    let json = cli.run_json(["item", "list", "--kind", "risk"]).unwrap();
    assert_eq!(json["items"].as_array().unwrap().len(), 2);
    let self_ids: Vec<&str> = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["self_id"].as_str().unwrap())
        .collect();
    assert!(self_ids.contains(&"RISK-02"));
    assert!(self_ids.contains(&"RISK-03"));
    assert!(
        !self_ids.contains(&"RISK-01"),
        "the removed risk must vanish from the index (markdown is the sole source)"
    );
}
