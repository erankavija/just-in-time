//! Golden test for the `jit init`-authored `[item_kinds]` table (issue
//! `fbe0a401`, REQ-04).
//!
//! The engine bakes in NO item kinds: kinds are authored entirely in the
//! `[item_kinds]` config table that `jit init` scaffolds. This test pins, through
//! the SHIPPED binary, that:
//!
//! 1. `jit init` emits an editable `[item_kinds]` table carrying the complete
//!    default set (`requirement`/`decision`/`risk`/`invariant`) — golden block.
//! 2. A repo whose ONLY config is that emitted one indexes all four kinds (the
//!    table, not a baked default, is what makes them index).
//! 3. A repo with NO `[item_kinds]` table indexes NOTHING — proving there are no
//!    baked built-ins.

use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

/// `jit init` a fresh repo in a tempdir.
fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let output = Command::new(jit_binary())
        .arg("init")
        .current_dir(temp.path())
        .output()
        .expect("failed to run jit init");
    assert!(output.status.success(), "jit init failed");
    temp
}

/// Run `jit item list [--kind <kind>] --json` and return the parsed JSON.
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

/// The exact `[item_kinds]` block `jit init` must emit. A drift here silently
/// changes indexing, so it is pinned byte-for-byte.
const GOLDEN_ITEM_KINDS_BLOCK: &str = "\
[item_kinds.requirement]
section = \"success_criteria\"
id-pattern = \"[A-Z][A-Z0-9]*-[0-9]+\"
markers = [\"[hard]\"]
link-namespaces = [\"satisfies\"]
scope = \"issue\"
source-of-truth = \"markdown-first\"

[item_kinds.decision]
section = \"decisions\"
id-pattern = \"D-[0-9]+\"
markers = []
link-namespaces = [\"per\"]
scope = \"issue\"
source-of-truth = \"markdown-first\"

[item_kinds.risk]
section = \"risks\"
id-pattern = \"RISK-[0-9]+\"
markers = []
link-namespaces = [\"mitigates\", \"resolves\"]
scope = \"issue\"
source-of-truth = \"markdown-first\"

[item_kinds.invariant]
section = \"success_criteria\"
id-pattern = \"[A-Z][A-Z0-9]*-[0-9]+\"
markers = []
link-namespaces = [\"enforces\"]
scope = \"project\"
source = { toml = \".jit/invariants.toml\", table = \"invariants\", id-field = \"id\", text-field = \"statement\" }
source-of-truth = \"registry-first\"
";

#[test]
fn test_init_emits_golden_item_kinds_table() {
    // REQ-04, clause 1: `jit init` emits the complete, editable `[item_kinds]`
    // table (golden block) into the on-disk config.
    let temp = setup_test_repo();
    let config = std::fs::read_to_string(temp.path().join(".jit").join("config.toml")).unwrap();
    assert!(
        config.contains(GOLDEN_ITEM_KINDS_BLOCK),
        "jit init must emit the golden [item_kinds] block; got:\n{config}"
    );
}

#[test]
fn test_init_authored_table_indexes_all_kinds() {
    // REQ-04, clause 2: a repo whose ONLY config is the emitted one indexes all
    // four kinds — the table, not a baked default, makes them index.
    let temp = setup_test_repo();

    let issue_body = "\
## Success Criteria\n\n- [hard] REQ-01: writes are atomic\n\n\
## Decisions\n\n- D-01: store issues as JSON\n\n\
## Risks\n\n- RISK-01: concurrent writers\n";
    let output = Command::new(jit_binary())
        .args([
            "issue", "create", "-t", "fixture", "-d", issue_body, "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "issue create failed");
    let created: Value = serde_json::from_slice(&output.stdout).unwrap();
    let short: String = created["id"].as_str().unwrap().chars().take(8).collect();

    std::fs::write(
        temp.path().join(".jit").join("invariants.toml"),
        "[[invariants]]\nid = \"INV-01\"\nstatement = \"acyclic\"\nkind = \"enforced\"\n",
    )
    .unwrap();

    let all = item_list(temp.path(), None);
    let kinds: Vec<&str> = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    for expected in ["requirement", "decision", "risk", "invariant"] {
        assert!(
            kinds.contains(&expected),
            "the init-authored table must index a {expected} item: {kinds:?}"
        );
    }
    // Spot-check addressing across both substrates.
    let qids: Vec<&str> = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["qualified_id"].as_str().unwrap())
        .collect();
    assert!(qids.contains(&format!("{short}/REQ-01").as_str()));
    assert!(qids.contains(&"@/INV-01"));
}

#[test]
fn test_no_item_kinds_table_indexes_nothing() {
    // REQ-04, clause 3 (no built-ins): with the `[item_kinds]` table removed, a
    // repo carrying issue criteria AND an invariants registry indexes NOTHING —
    // there are no baked default kinds.
    let temp = setup_test_repo();

    let issue_body = "\
## Success Criteria\n\n- [hard] REQ-01: writes are atomic\n\n\
## Decisions\n\n- D-01: store issues as JSON\n\n\
## Risks\n\n- RISK-01: concurrent writers\n";
    let output = Command::new(jit_binary())
        .args([
            "issue", "create", "-t", "fixture", "-d", issue_body, "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "issue create failed");

    std::fs::write(
        temp.path().join(".jit").join("invariants.toml"),
        "[[invariants]]\nid = \"INV-01\"\nstatement = \"acyclic\"\nkind = \"enforced\"\n",
    )
    .unwrap();

    // Strip the [item_kinds] table: write a config with none.
    std::fs::write(
        temp.path().join(".jit").join("config.toml"),
        "[version]\nschema = 2\n",
    )
    .unwrap();

    let all = item_list(temp.path(), None);
    assert_eq!(
        all["count"].as_u64().unwrap(),
        0,
        "with no [item_kinds] table there are no kinds (no baked built-ins): {all}"
    );
}
