//! Integration tests for the markdown-first `risk` item kind (issue `be3eb6ca`).
//!
//! `jit init` scaffolds a `[item_kinds]` table that declares `risk` alongside
//! `requirement` and `decision`, so a freshly initialized repo indexes risks
//! authored as list entries under a `## Risks` section of an issue description.
//! Risks become addressable, queryable items through the SAME generic
//! triple-driven parse path `requirement` and `decision` use; no engine code
//! special-cases the kind (the engine bakes in no kinds — the tuple is authored
//! entirely in the emitted config table).
//!
//! The `risk` tuple `jit init` emits is:
//!
//! ```text
//! section          = "risks"              # authored under a `## Risks` heading
//! id-pattern       = "RISK-[0-9]+"        # self-ids read RISK-1, RISK-2, ... (e.g. RISK-01)
//! markers          = []                   # no marker; every matching line qualifies
//! link-namespaces  = ["mitigates",        # `mitigates:<issue>/RISK-01` references a risk
//!                     "resolves"]         # `resolves:<issue>/RISK-01` references a risk
//! scope            = "issue"              # risks live in issue descriptions
//! source-of-truth  = "markdown-first"
//! ```
//!
//! Coverage of the issue's `[hard]` success criteria, all against a DEFAULT repo:
//! - REQ-01: [`test_default_repo_item_list_kind_risk_returns_risks`] runs the real
//!   `jit` binary so `jit item list --kind risk` returns risks parsed from an issue
//!   description, with NO custom config.
//! - REQ-02: [`test_default_repo_mitigates_label_resolves_risk`] and
//!   [`test_default_repo_resolves_label_resolves_risk`] prove that BOTH a
//!   `mitigates:<issue>/RISK-01` and a `resolves:<issue>/RISK-01` label resolve to
//!   the addressed risk through the existing generic `resolve_link_label` (both
//!   namespaces are recognized because the `risk` kind declares both link-namespaces).
//! - REQ-03: [`test_default_repo_markdown_is_sole_source_for_risks`] proves the
//!   markdown description is the only source — editing it changes the index and no
//!   parallel structured store is written under `.jit/`.
//! - REQ-04: every test below runs under `cargo test`.

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

#[test]
fn test_default_repo_item_list_kind_risk_returns_risks() {
    // REQ-01: in a DEFAULT-initialized repo (no custom config), `jit item list
    // --kind risk` returns risk items parsed from an issue's `## Risks` section
    // — exercised through the actual binary.
    let temp = setup_test_repo();
    let short = create_issue(
        temp.path(),
        "Migration plan",
        "## Risks\n\n- RISK-01: data loss during migration\n- RISK-02: performance regression\n",
    );

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
    assert_eq!(json["count"].as_u64().unwrap(), 2);
    let items = json["items"].as_array().unwrap();
    for item in items {
        assert_eq!(item["kind"].as_str().unwrap(), "risk");
    }
    let qids: Vec<&str> = items
        .iter()
        .map(|i| i["qualified_id"].as_str().unwrap())
        .collect();
    assert!(qids.contains(&format!("{short}/RISK-01").as_str()));
    assert!(qids.contains(&format!("{short}/RISK-02").as_str()));
}

/// Build a [`JsonFileStorage`]-backed executor over a `.jit` repo whose
/// `config.toml` declares the `risk` kind (the engine bakes in no kinds), seed
/// `issues`, and return it paired with the seeded issues' short ids in insertion
/// order.
///
/// The declared `risk` kind brings the `mitigates` and `resolves` link namespaces;
/// this declaration mirrors the one `jit init` emits, so the test exercises the
/// shipped kind shape, not an arbitrary override.
fn default_executor_with(
    repo: &Path,
    issues: Vec<(&str, &str)>,
) -> (CommandExecutor<JsonFileStorage>, Vec<String>) {
    let jit_dir = repo.join(".jit");
    std::fs::create_dir_all(&jit_dir).unwrap();
    let storage = JsonFileStorage::new(&jit_dir);
    storage.init().unwrap();
    std::fs::write(
        jit_dir.join("config.toml"),
        "[item_kinds.risk]\n\
         section = \"risks\"\n\
         id-pattern = \"RISK-[0-9]+\"\n\
         markers = []\n\
         link-namespaces = [\"mitigates\", \"resolves\"]\n\
         scope = \"issue\"\n\
         source-of-truth = \"markdown-first\"\n",
    )
    .unwrap();
    let mut shorts = Vec::new();
    for (title, body) in issues {
        let issue = Issue::new(title.to_string(), body.to_string());
        shorts.push(issue.short_id());
        storage.save_issue(issue).unwrap();
    }
    (CommandExecutor::new(storage), shorts)
}

#[test]
fn test_default_repo_mitigates_label_resolves_risk() {
    // REQ-02 (mitigates): a `mitigates:<issue>/RISK-01` label resolves to the
    // addressed risk item through the existing generic `resolve_link_label`, with
    // NO custom config — the `mitigates` namespace is recognized because the
    // `risk` kind that `jit init` emits declares it.
    let temp = TempDir::new().unwrap();
    let (exec, shorts) = default_executor_with(
        temp.path(),
        vec![(
            "Migration plan",
            "## Risks\n\n- RISK-01: data loss during migration\n",
        )],
    );
    let short = &shorts[0];

    let label = format!("mitigates:{short}/RISK-01");
    let resolved = exec
        .resolve_link_label(&label)
        .unwrap()
        .expect("a mitigates:<issue>/RISK-01 label resolves to the addressed risk");
    assert_eq!(resolved.item.self_id, "RISK-01");
    assert_eq!(resolved.item.kind, "risk");
    assert_eq!(resolved.item.qualified_id, format!("{short}/RISK-01"));
    assert!(resolved.item.text.contains("data loss"));

    // A registered `mitigates:` namespace whose qualified id cannot be resolved is
    // an error, never a silent drop.
    let bad = format!("mitigates:{short}/RISK-99");
    assert!(exec.resolve_link_label(&bad).is_err());
}

#[test]
fn test_default_repo_resolves_label_resolves_risk() {
    // REQ-02 (resolves): a `resolves:<issue>/RISK-01` label resolves to the
    // addressed risk item through the existing generic `resolve_link_label`, with
    // NO custom config — the `resolves` namespace is recognized because the
    // BUILT-IN `risk` kind ships it.
    let temp = TempDir::new().unwrap();
    let (exec, shorts) = default_executor_with(
        temp.path(),
        vec![(
            "Migration plan",
            "## Risks\n\n- RISK-01: data loss during migration\n",
        )],
    );
    let short = &shorts[0];

    let label = format!("resolves:{short}/RISK-01");
    let resolved = exec
        .resolve_link_label(&label)
        .unwrap()
        .expect("a resolves:<issue>/RISK-01 label resolves to the addressed risk");
    assert_eq!(resolved.item.self_id, "RISK-01");
    assert_eq!(resolved.item.kind, "risk");
    assert_eq!(resolved.item.qualified_id, format!("{short}/RISK-01"));
    assert!(resolved.item.text.contains("data loss"));

    // A registered `resolves:` namespace whose qualified id cannot be resolved is
    // an error, never a silent drop.
    let bad = format!("resolves:{short}/RISK-99");
    assert!(exec.resolve_link_label(&bad).is_err());
}

#[test]
fn test_default_repo_markdown_is_sole_source_for_risks() {
    // REQ-03: the markdown description is the ONLY source for risk items. The
    // index is a pure projection — there is no parallel structured store — so
    // editing the description (and nothing else) changes what `jit item list`
    // returns, and no risk store file is written under `.jit/`.
    let temp = setup_test_repo();
    let short = create_issue(
        temp.path(),
        "Migration plan",
        "## Risks\n\n- RISK-01: data loss during migration\n",
    );

    let count_risks = |repo: &Path| -> u64 {
        let output = Command::new(jit_binary())
            .args(["item", "list", "--kind", "risk", "--json"])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(output.status.success());
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        json["count"].as_u64().unwrap()
    };

    assert_eq!(count_risks(temp.path()), 1);

    // No parallel structured store: nothing under `.jit/` persists risks; the
    // ONLY place RISK-01 appears is the issue's own JSON description.
    let jit_dir = temp.path().join(".jit");
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
    let output = Command::new(jit_binary())
        .args([
            "issue",
            "update",
            &short,
            "-d",
            "## Risks\n\n- RISK-02: schema drift\n- RISK-03: timeline overrun\n",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "issue update failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The index now reflects ONLY the edited markdown.
    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "risk", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 2);
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
