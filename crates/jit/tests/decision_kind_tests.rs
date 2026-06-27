//! Integration tests for the markdown-first `decision` item kind (issue
//! `b629686b`).
//!
//! `jit init` scaffolds a `[item_kinds]` table that declares `decision` alongside
//! `requirement`, so a freshly initialized repo indexes decisions authored as list
//! entries under a `## Decisions` section of an issue description. Decisions become
//! addressable, queryable items through the SAME generic triple-driven parse path
//! `requirement` uses; no engine code special-cases the kind (the engine bakes in
//! no kinds — the tuple is authored entirely in the emitted config table).
//!
//! The `decision` tuple `jit init` emits is:
//!
//! ```text
//! section          = "decisions"          # authored under a `## Decisions` heading
//! id-pattern       = "D-[0-9]+"           # self-ids read D-1, D-2, ... (e.g. D-01)
//! markers          = []                   # no marker; every matching line qualifies
//! link-namespaces  = ["per"]             # a `per:<issue>/D-01` label references a decision
//! scope            = "issue"              # decisions live in issue descriptions
//! source-of-truth  = "markdown-first"
//! ```
//!
//! Coverage of the issue's `[hard]` success criteria, all against a DEFAULT repo:
//! - REQ-01: [`test_default_repo_item_list_kind_decision_returns_decisions`] runs
//!   the real `jit` binary so `jit item list --kind decision` returns decisions
//!   parsed from an issue description, with NO custom config.
//! - REQ-02: [`test_default_repo_per_label_resolves_decision`] proves a
//!   `per:<issue>/D-01` label resolves to the addressed decision through the
//!   existing generic `resolve_link_label` (the `per` namespace is recognized
//!   because the `decision` kind declares the `per` link-namespace).
//! - REQ-03: [`test_default_repo_markdown_is_sole_source_for_decisions`] proves
//!   the markdown description is the only source — editing it changes the index and
//!   no parallel structured store is written under `.jit/`.
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
fn test_default_repo_item_list_kind_decision_returns_decisions() {
    // REQ-01: in a DEFAULT-initialized repo (no custom config), `jit item list
    // --kind decision` returns decision items parsed from an issue's `## Decisions`
    // section — exercised through the actual binary.
    let temp = setup_test_repo();
    let short = create_issue(
        temp.path(),
        "Architecture",
        "## Decisions\n\n- D-01: use json storage\n- D-02: atomic temp-file writes\n",
    );

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
    assert_eq!(json["count"].as_u64().unwrap(), 2);
    let items = json["items"].as_array().unwrap();
    for item in items {
        assert_eq!(item["kind"].as_str().unwrap(), "decision");
    }
    let qids: Vec<&str> = items
        .iter()
        .map(|i| i["qualified_id"].as_str().unwrap())
        .collect();
    assert!(qids.contains(&format!("{short}/D-01").as_str()));
    assert!(qids.contains(&format!("{short}/D-02").as_str()));
}

#[test]
fn test_default_repo_item_show_resolves_decision_by_qualified_id() {
    // REQ-01: a decision resolves by its derived qualified id through the real
    // `jit item show` binary, in a default repo.
    let temp = setup_test_repo();
    let short = create_issue(
        temp.path(),
        "Architecture",
        "## Decisions\n\n- D-01: use json storage\n",
    );
    let qualified = format!("{short}/D-01");

    let output = Command::new(jit_binary())
        .args(["item", "show", &qualified, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "item show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["item"]["self_id"].as_str().unwrap(), "D-01");
    assert_eq!(json["item"]["kind"].as_str().unwrap(), "decision");
    assert_eq!(json["item"]["qualified_id"].as_str().unwrap(), qualified);
    assert!(json["item"]["text"]
        .as_str()
        .unwrap()
        .contains("json storage"));
}

/// Build a [`JsonFileStorage`]-backed executor over a `.jit` repo whose
/// `config.toml` declares the `decision` kind (the engine bakes in no kinds), seed
/// `issues`, and return it paired with the seeded issues' short ids in insertion
/// order.
///
/// The declared `decision` kind brings the `per` link namespace; this declaration
/// mirrors the one `jit init` emits, so the test exercises the shipped kind shape,
/// not an arbitrary override.
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
        "[item_kinds.decision]\n\
         section = \"decisions\"\n\
         id-pattern = \"D-[0-9]+\"\n\
         markers = []\n\
         link-namespaces = [\"per\"]\n\
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
fn test_default_repo_per_label_resolves_decision() {
    // REQ-02: a `per:<issue>/D-01` label resolves to the addressed decision item
    // through the existing generic `resolve_link_label`, with NO custom config —
    // the `per` namespace is recognized because the `decision` kind that `jit init`
    // emits declares it.
    let temp = TempDir::new().unwrap();
    let (exec, shorts) = default_executor_with(
        temp.path(),
        vec![("Architecture", "## Decisions\n\n- D-01: use json storage\n")],
    );
    let short = &shorts[0];

    let label = format!("per:{short}/D-01");
    let resolved = exec
        .resolve_link_label(&label)
        .unwrap()
        .expect("a per:<issue>/D-01 label resolves to the addressed decision");
    assert_eq!(resolved.item.self_id, "D-01");
    assert_eq!(resolved.item.kind, "decision");
    assert_eq!(resolved.item.qualified_id, format!("{short}/D-01"));
    assert!(resolved.item.text.contains("json storage"));

    // A registered `per:` namespace whose qualified id cannot be resolved is an
    // error, never a silent drop.
    let bad = format!("per:{short}/D-99");
    assert!(exec.resolve_link_label(&bad).is_err());
}

#[test]
fn test_default_repo_markdown_is_sole_source_for_decisions() {
    // REQ-03: the markdown description is the ONLY source for decision items. The
    // index is a pure projection — there is no parallel structured store — so
    // editing the description (and nothing else) changes what `jit item list`
    // returns, and no decision store file is written under `.jit/`.
    let temp = setup_test_repo();
    let short = create_issue(
        temp.path(),
        "Architecture",
        "## Decisions\n\n- D-01: use json storage\n",
    );

    let count_decisions = |repo: &Path| -> u64 {
        let output = Command::new(jit_binary())
            .args(["item", "list", "--kind", "decision", "--json"])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(output.status.success());
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        json["count"].as_u64().unwrap()
    };

    assert_eq!(count_decisions(temp.path()), 1);

    // No parallel structured store: nothing under `.jit/` persists decisions; the
    // ONLY place D-01 appears is the issue's own JSON description.
    let jit_dir = temp.path().join(".jit");
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
    let output = Command::new(jit_binary())
        .args([
            "issue",
            "update",
            &short,
            "-d",
            "## Decisions\n\n- D-02: switch to wal\n- D-03: add caching\n",
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
        .args(["item", "list", "--kind", "decision", "--json"])
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
    assert!(self_ids.contains(&"D-02"));
    assert!(self_ids.contains(&"D-03"));
    assert!(
        !self_ids.contains(&"D-01"),
        "the removed decision must vanish from the index (markdown is the sole source)"
    );
}
