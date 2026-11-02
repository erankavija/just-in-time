//! Integration tests for the markdown-first `decision` item kind (issue
//! `b629686b`).
//!
//! The `jit-default` package supplies a `[item_kinds]` table that declares
//! `decision` alongside `requirement`, so a profiled repo indexes decisions authored as list
//! entries under a `## Decisions` section of an issue description. Decisions become
//! addressable, queryable items through the SAME generic triple-driven parse path
//! `requirement` uses; no engine code special-cases the kind. The declaration under
//! test is the shipped one, derived from the default package.
//!
//! Coverage of the issue's `[hard]` success criteria, all against a repository
//! initialized with the shared taxonomy fixture:
//! - REQ-01: [`test_default_repo_item_list_kind_decision_returns_decisions`] runs
//!   the real `jit` binary so `jit item list --kind decision` returns decisions
//!   parsed from an issue description with the configured `decision` item kind.
//! - REQ-02: [`test_default_repo_per_label_resolves_decision`] proves a
//!   `per:<issue>/D-01` label resolves to the addressed decision through the
//!   existing generic `resolve_link_label` (the `per` namespace is recognized
//!   because the `decision` kind declares the `per` link-namespace).
//! - REQ-03: [`test_default_repo_markdown_is_sole_source_for_decisions`] proves
//!   the markdown description is the only source — editing it changes the index and
//!   no parallel structured store is written under `.jit/`.
//! - REQ-04: every test below runs under `cargo test`.
use jit::test_cli::JitCli;
use jit::test_utils::setup_test_repo_with_config;

#[test]
fn test_default_repo_item_list_kind_decision_returns_decisions() {
    // REQ-01: in a fixture-initialized repo with the configured `decision` kind, `jit item list
    // --kind decision` returns decision items parsed from an issue's `## Decisions`
    // section — exercised through the actual binary.
    let cli = JitCli::with_repository(
        setup_test_repo_with_config(&crate::config_with_shipped_kinds(&["decision"]))
            .unwrap()
            .0,
    );
    let short = cli
        .create_issue([
            "-t",
            "Architecture",
            "-d",
            "## Decisions\n\n- D-01: use json storage\n- D-02: atomic temp-file writes\n",
        ])
        .unwrap()[..8]
        .to_owned();

    let json = cli
        .run_json(["item", "list", "--kind", "decision"])
        .unwrap();
    assert_eq!(json["items"].as_array().unwrap().len(), 2);
    let items = json["items"].as_array().unwrap();
    for item in items {
        assert_eq!(item["kind"].as_str().unwrap(), "decision");
    }
    let qids: Vec<&str> = items
        .iter()
        .map(|i| i["qualified_id"].as_str().unwrap())
        .collect();
    assert!(qids.contains(&format!("@/issue/{short}/decision/D-01").as_str()));
    assert!(qids.contains(&format!("@/issue/{short}/decision/D-02").as_str()));
}

#[test]
fn test_default_repo_item_show_resolves_decision_by_qualified_id() {
    // REQ-01: a decision resolves through the real `jit item show` binary by its
    // `<short-id>/<self-id>` sugar INPUT, and reports the canonical uniform
    // kind-segmented qualified id.
    let cli = JitCli::with_repository(
        setup_test_repo_with_config(&crate::config_with_shipped_kinds(&["decision"]))
            .unwrap()
            .0,
    );
    let short = cli
        .create_issue([
            "-t",
            "Architecture",
            "-d",
            "## Decisions\n\n- D-01: use json storage\n",
        ])
        .unwrap()[..8]
        .to_owned();
    let sugar = format!("{short}/D-01");

    let json = cli.run_json(["item", "show", &sugar]).unwrap();
    assert_eq!(json["item"]["self_id"].as_str().unwrap(), "D-01");
    assert_eq!(json["item"]["kind"].as_str().unwrap(), "decision");
    assert_eq!(
        json["item"]["qualified_id"].as_str().unwrap(),
        format!("@/issue/{short}/decision/D-01")
    );
    assert!(json["item"]["text"]
        .as_str()
        .unwrap()
        .contains("json storage"));
}

#[test]
fn test_default_repo_per_label_resolves_decision() {
    // REQ-02: a `per:<issue>/D-01` label resolves to the addressed decision item
    // through the existing generic `resolve_link_label`; the configured `decision`
    // kind declares the `per` namespace.
    let (_temp, exec) =
        setup_test_repo_with_config(&crate::config_with_shipped_kinds(&["decision"])).unwrap();
    let issue = crate::create_issue(
        &exec,
        "Architecture",
        "## Decisions\n\n- D-01: use json storage\n",
    );
    let short = issue.short_id();

    let label = format!("per:{short}/D-01");
    let resolved = exec
        .items()
        .resolve_link_label(&label)
        .unwrap()
        .expect("a per:<issue>/D-01 label resolves to the addressed decision");
    assert_eq!(resolved.item.self_id, "D-01");
    assert_eq!(resolved.item.kind, "decision");
    assert_eq!(
        resolved.item.qualified_id,
        format!("@/issue/{short}/decision/D-01")
    );
    assert!(resolved.item.text.contains("json storage"));

    // A registered `per:` namespace whose qualified id cannot be resolved is an
    // error, never a silent drop.
    let bad = format!("per:{short}/D-99");
    assert!(exec.items().resolve_link_label(&bad).is_err());
}

#[test]
fn test_default_repo_markdown_is_sole_source_for_decisions() {
    // REQ-03: the markdown description is the ONLY source for decision items. The
    // index is a pure projection — there is no parallel structured store — so
    // editing the description (and nothing else) changes what `jit item list`
    // returns, and no decision store file is written under `.jit/`.
    let cli = JitCli::with_repository(
        setup_test_repo_with_config(&crate::config_with_shipped_kinds(&["decision"]))
            .unwrap()
            .0,
    );
    let short = cli
        .create_issue([
            "-t",
            "Architecture",
            "-d",
            "## Decisions\n\n- D-01: use json storage\n",
        ])
        .unwrap()[..8]
        .to_owned();

    let listed = cli
        .run_json(["item", "list", "--kind", "decision"])
        .unwrap();
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);

    // No parallel structured store: nothing under `.jit/` persists decisions; the
    // ONLY place D-01 appears is the issue's own JSON description.
    let jit_dir = cli.root().join(".jit");
    for entry in std::fs::read_dir(&jit_dir).unwrap() {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        // Any TOP-LEVEL `.jit` file (config/registry/event log) must not duplicate
        // the decision id; the issue's markdown (under `issues/`) is the sole
        // source and is checked separately below.
        if name.ends_with(".json") || name.ends_with(".jsonl") || name.ends_with(".toml") {
            let contents = std::fs::read_to_string(&path).unwrap();
            assert!(
                !contents.contains("D-01"),
                "no parallel structured store may duplicate decisions; \
                 found 'D-01' in {name}"
            );
        }
    }
    // The decision DOES live in the owning issue's description (the sole source).
    let mut found_in_issue = false;
    for entry in std::fs::read_dir(jit_dir.join("issues")).unwrap() {
        let contents = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        if contents.contains(short.as_str()) && contents.contains("D-01") {
            found_in_issue = true;
            break;
        }
    }
    assert!(
        found_in_issue,
        "the decision must live in the issue description (the sole markdown source)"
    );

    // Editing the markdown description (the sole source) changes the index: drop
    // D-01, add D-02 and D-03.
    cli.run([
        "issue",
        "update",
        &short,
        "-d",
        "## Decisions\n\n- D-02: switch to wal\n- D-03: add caching\n",
    ])
    .unwrap();

    // The index now reflects ONLY the edited markdown.
    let json = cli
        .run_json(["item", "list", "--kind", "decision"])
        .unwrap();
    assert_eq!(json["items"].as_array().unwrap().len(), 2);
    let self_ids: Vec<&str> = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["self_id"].as_str().unwrap())
        .collect();
    assert!(self_ids.contains(&"D-02"));
    assert!(self_ids.contains(&"D-03"));
    assert!(
        !self_ids.contains(&"D-01"),
        "the removed decision must vanish from the index (markdown is the sole source)"
    );
}
