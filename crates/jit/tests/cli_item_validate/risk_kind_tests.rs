//! Integration tests for the markdown-first `risk` item kind (issue `be3eb6ca`).
//!
//! The `jit-default` package supplies a `[item_kinds]` table that declares `risk`
//! alongside `requirement` and `decision`, so a profiled repo indexes risks
//! authored as list entries under a `## Risks` section of an issue description.
//! Risks become addressable, queryable items through the SAME generic
//! triple-driven parse path `requirement` and `decision` use; no engine code
//! special-cases the kind. The declaration under test is the shipped one, derived
//! from the default package.
//!
//! Coverage of the issue's `[hard]` success criteria, all against a repository
//! initialized with the shared taxonomy fixture:
//! - REQ-01: [`test_default_repo_item_list_kind_risk_returns_risks`] runs the real
//!   `jit` binary so `jit item list --kind risk` returns risks parsed from an issue
//!   description with the configured `risk` item kind.
//! - REQ-02: [`test_default_repo_mitigates_label_resolves_risk`] and
//!   [`test_default_repo_resolves_label_resolves_risk`] prove that BOTH a
//!   `mitigates:<issue>/RISK-01` and a `resolves:<issue>/RISK-01` label resolve to
//!   the addressed risk through the existing generic `resolve_link_label` (both
//!   namespaces are recognized because the `risk` kind declares both link-namespaces).
//! - REQ-03: [`test_default_repo_markdown_is_sole_source_for_risks`] proves the
//!   markdown description is the only source — editing it changes the index and no
//!   parallel structured store is written under `.jit/`.
//! - REQ-04: every test below runs under `cargo test`.
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
fn test_default_repo_mitigates_label_resolves_risk() {
    // REQ-02 (mitigates): a `mitigates:<issue>/RISK-01` label resolves to the
    // addressed risk item through the existing generic `resolve_link_label`; the
    // configured `risk` kind declares the `mitigates` namespace.
    let (_temp, exec) =
        setup_test_repo_with_config(&crate::config_with_shipped_kinds(&["risk"])).unwrap();
    let issue = crate::create_issue(
        &exec,
        "Migration plan",
        "## Risks\n\n- RISK-01: data loss during migration\n",
    );
    let short = issue.short_id();

    let label = format!("mitigates:{short}/RISK-01");
    let resolved = exec
        .items()
        .resolve_link_label(&label)
        .unwrap()
        .expect("a mitigates:<issue>/RISK-01 label resolves to the addressed risk");
    assert_eq!(resolved.item.self_id, "RISK-01");
    assert_eq!(resolved.item.kind, "risk");
    assert_eq!(
        resolved.item.qualified_id,
        format!("@/issue/{short}/risk/RISK-01")
    );
    assert!(resolved.item.text.contains("data loss"));

    // A registered `mitigates:` namespace whose qualified id cannot be resolved is
    // an error, never a silent drop.
    let bad = format!("mitigates:{short}/RISK-99");
    assert!(exec.items().resolve_link_label(&bad).is_err());
}

#[test]
fn test_default_repo_resolves_label_resolves_risk() {
    // REQ-02 (resolves): a `resolves:<issue>/RISK-01` label resolves to the
    // addressed risk item through the existing generic `resolve_link_label`; the
    // configured `risk` kind declares the `resolves` namespace.
    let (_temp, exec) =
        setup_test_repo_with_config(&crate::config_with_shipped_kinds(&["risk"])).unwrap();
    let issue = crate::create_issue(
        &exec,
        "Migration plan",
        "## Risks\n\n- RISK-01: data loss during migration\n",
    );
    let short = issue.short_id();

    let label = format!("resolves:{short}/RISK-01");
    let resolved = exec
        .items()
        .resolve_link_label(&label)
        .unwrap()
        .expect("a resolves:<issue>/RISK-01 label resolves to the addressed risk");
    assert_eq!(resolved.item.self_id, "RISK-01");
    assert_eq!(resolved.item.kind, "risk");
    assert_eq!(
        resolved.item.qualified_id,
        format!("@/issue/{short}/risk/RISK-01")
    );
    assert!(resolved.item.text.contains("data loss"));

    // A registered `resolves:` namespace whose qualified id cannot be resolved is
    // an error, never a silent drop.
    let bad = format!("resolves:{short}/RISK-99");
    assert!(exec.items().resolve_link_label(&bad).is_err());
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
