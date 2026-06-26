//! Contract test: a config-declared, markdown-first project-kind indexes its
//! items with no Rust change and without touching this repo's real `.jit/`.
//!
//! ## What this file proves (REQ-01, issue 241a0c03 / story 90a2dbfd)
//!
//! The addressable-items engine already reads project-scoped items from any
//! `.md` file named in `[item_kinds.<name>] source = "..."`.  This test pins
//! that contract so a future change cannot silently re-narrow the markdown
//! source path.
//!
//! The fixture:
//! - A fresh, isolated temp repo (`jit init`).
//! - A custom `[item_kinds.policy]` table written into that repo's
//!   `.jit/config.toml`, naming an arbitrary source file (`policies.md`).
//! - A `policies.md` created at the repo root with two items (`POL-01`,
//!   `POL-02`) under a `## Policies` section.
//!
//! Assertions:
//! - Both items surface in `jit item list --json` with `@/POL-NN` qualified
//!   ids and kind `"policy"`.
//! - `jit item list --kind policy` returns exactly the two items.
//! - `jit item show @/POL-01 --json` resolves the item by its qualified id.
//!
//! The test uses ONLY the isolated temp repo — this repo's real
//! `.jit/config.toml` is never read or written.

use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

/// Bootstrap an isolated, default-initialized repo in a temp directory.
fn setup_isolated_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let output = Command::new(jit_binary())
        .arg("init")
        .current_dir(temp.path())
        .output()
        .expect("failed to run jit init");
    assert!(output.status.success(), "jit init failed");
    temp
}

/// Append an arbitrary `[item_kinds.policy]` table to the repo's
/// `.jit/config.toml`, naming `policies.md` as the source.
///
/// Uses a non-reserved kind name (`policy`) and a non-default source filename
/// to exercise the fully-generic markdown path rather than any built-in shortcut.
fn configure_policy_kind(repo: &Path) {
    let config_path = repo.join(".jit").join("config.toml");
    let mut config = std::fs::read_to_string(&config_path).unwrap_or_default();
    config.push_str(
        "\n[item_kinds.policy]\n\
         section = \"policies\"\n\
         id-pattern = \"POL-[0-9]+\"\n\
         markers = []\n\
         link-namespaces = []\n\
         scope = \"project\"\n\
         source = \"policies.md\"\n\
         source-of-truth = \"markdown-first\"\n",
    );
    std::fs::write(&config_path, config).unwrap();
}

/// Write a `policies.md` with two `POL-NN` items under a `## Policies`
/// section at the repo root.
fn write_policy_source(repo: &Path) {
    std::fs::write(
        repo.join("policies.md"),
        "# Project Policies\n\n\
         ## Policies\n\n\
         - POL-01: all file writes must be atomic (temp-then-rename)\n\
         - POL-02: no production Rust code may use unsafe\n",
    )
    .unwrap();
}

/// Run `jit item list [--kind <kind>] --json` from `repo` and return parsed JSON.
fn item_list(repo: &Path, kind: Option<&str>) -> Value {
    let mut cmd = Command::new(jit_binary());
    cmd.arg("item").arg("list");
    if let Some(k) = kind {
        cmd.arg("--kind").arg(k);
    }
    cmd.arg("--json");
    let output = cmd.current_dir(repo).output().unwrap();
    assert!(
        output.status.success(),
        "jit item list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Run `jit item show <qualified_id> --json` from `repo` and return parsed JSON.
fn item_show(repo: &Path, qualified_id: &str) -> Value {
    let output = Command::new(jit_binary())
        .args(["item", "show", qualified_id, "--json"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "jit item show {qualified_id} failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Extract `qualified_id` strings from a `jit item list/search` JSON result.
fn qualified_ids(json: &Value) -> Vec<&str> {
    json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["qualified_id"].as_str().unwrap())
        .collect()
}

// ---------------------------------------------------------------------------
// REQ-01 contract test (issue 241a0c03)
// ---------------------------------------------------------------------------

/// A config-declared markdown kind over an arbitrary `.md` source indexes its
/// items with no Rust change, using only an isolated temp repo.
///
/// Verifies (REQ-01, story 90a2dbfd):
/// - `[item_kinds.policy]` with `source = "policies.md"` is sufficient to make
///   `jit item list` return `@/POL-01` and `@/POL-02`.
/// - `--kind policy` filter returns exactly those items.
/// - `jit item show @/POL-01` resolves the item by its qualified id.
///
/// This repo's real `.jit/config.toml` is never touched.
#[test]
fn test_config_declared_markdown_kind_indexes_project_items() {
    let temp = setup_isolated_repo();
    configure_policy_kind(temp.path());
    write_policy_source(temp.path());

    // -----------------------------------------------------------------------
    // Part 1: unfiltered `jit item list` contains both policy items.
    // -----------------------------------------------------------------------
    let all = item_list(temp.path(), None);
    let all_qids = qualified_ids(&all);
    assert!(
        all_qids.contains(&"@/POL-01"),
        "unfiltered list must include @/POL-01; got: {all_qids:?}"
    );
    assert!(
        all_qids.contains(&"@/POL-02"),
        "unfiltered list must include @/POL-02; got: {all_qids:?}"
    );

    // Both items carry the declared kind and the project scope.
    let items = all["items"].as_array().unwrap();
    for qid in ["@/POL-01", "@/POL-02"] {
        let item = items
            .iter()
            .find(|i| i["qualified_id"].as_str() == Some(qid))
            .unwrap_or_else(|| panic!("item {qid} missing from list result"));
        assert_eq!(
            item["kind"].as_str(),
            Some("policy"),
            "{qid} must carry kind 'policy'"
        );
        assert_eq!(
            item["scope"].as_str(),
            Some("@"),
            "{qid} must carry the project scope '@'"
        );
    }

    // -----------------------------------------------------------------------
    // Part 2: `--kind policy` returns exactly the two policy items and nothing
    // else (no issue-scope items from built-in kinds bleed into the result).
    // -----------------------------------------------------------------------
    let by_kind = item_list(temp.path(), Some("policy"));
    assert_eq!(
        by_kind["count"].as_u64().unwrap(),
        2,
        "--kind policy must return exactly 2 items; got: {:?}",
        qualified_ids(&by_kind)
    );
    let kind_qids = qualified_ids(&by_kind);
    assert!(
        kind_qids.contains(&"@/POL-01"),
        "--kind policy must return @/POL-01: {kind_qids:?}"
    );
    assert!(
        kind_qids.contains(&"@/POL-02"),
        "--kind policy must return @/POL-02: {kind_qids:?}"
    );

    // -----------------------------------------------------------------------
    // Part 3: `jit item show @/POL-01 --json` resolves through the same
    // generic path — no substrate-specific command or code branch required.
    // -----------------------------------------------------------------------
    let shown = item_show(temp.path(), "@/POL-01");
    assert_eq!(
        shown["item"]["self_id"].as_str(),
        Some("POL-01"),
        "show must return self_id = 'POL-01'"
    );
    assert_eq!(
        shown["item"]["qualified_id"].as_str(),
        Some("@/POL-01"),
        "show must return qualified_id = '@/POL-01'"
    );
    assert_eq!(
        shown["item"]["kind"].as_str(),
        Some("policy"),
        "show must return kind = 'policy'"
    );
    assert_eq!(
        shown["item"]["scope"].as_str(),
        Some("@"),
        "show must return scope = '@'"
    );
    assert!(
        shown["item"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("atomic"),
        "show must return the item text containing 'atomic'"
    );
    // Project-scope items have no owning issue.
    assert_eq!(
        shown["issue_full_id"],
        Value::Null,
        "project-scope item must carry no issue_full_id"
    );
}
