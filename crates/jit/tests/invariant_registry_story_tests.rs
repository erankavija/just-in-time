//! End-to-end acceptance story for the registry-first invariant substrate.
//!
//! Assembles the five hard criteria for JIT issue 69bd8cd5 into a single,
//! cohesive story that traces the full registry-first invariant path from
//! config load through CLI query:
//!
//! - REQ-01: an `.jit/invariants.toml` entry with `statement`, `kind`, and
//!   optional `enforced-by` loads at config time via `JitConfig::load`.
//! - REQ-02: invariants load on BOTH config-load return paths (with AND without
//!   a sibling `config.toml`).
//! - REQ-03: `jit item list --kind invariant` returns the invariant as `@/<self-id>`.
//! - REQ-04: an invalid invariant entry (missing field, bad `kind` token)
//!   makes config load fail with a typed, descriptive error.
//! - REQ-05: the registry is authoritative — an INV-looking line in an issue
//!   DESCRIPTION does NOT produce an invariant item; only `.jit/invariants.toml`
//!   entries do.

use jit::config::JitConfig;
use jit::validation::invariants::{InvariantConfigError, InvariantKind};
use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

/// Bootstrap a minimal `.jit/` repo (just enough for the invariant tests).
///
/// Writes the `invariants.toml` entry so all five criteria share the same
/// fixture shape.  Returns the path to the `.jit/` directory (= the
/// `jit_root` that `JitConfig::load` expects).
fn setup_jit_root_with_invariants_toml(with_config_toml: bool) -> (TempDir, std::path::PathBuf) {
    let temp = TempDir::new().unwrap();
    let jit_dir = temp.path().join(".jit");
    std::fs::create_dir_all(&jit_dir).unwrap();

    if with_config_toml {
        std::fs::write(
            jit_dir.join("config.toml"),
            "[type_hierarchy]\ntypes = { task = 1 }\n",
        )
        .unwrap();
    }

    std::fs::write(
        jit_dir.join("invariants.toml"),
        r#"
[[invariants]]
id = "INV-01"
statement = "Every dependency edge stays acyclic."
kind = "enforced"
enforced-by = "dag-no-cycles"

[[invariants]]
id = "INV-02"
statement = "All state changes must be logged."
kind = "advisory"
"#,
    )
    .unwrap();

    (temp, jit_dir)
}

/// Initialize a full `jit` repo (via the real binary) and write an
/// `invariants.toml` into it.  Used by CLI-path tests (REQ-03, REQ-05).
fn setup_cli_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let output = Command::new(jit_binary())
        .arg("init")
        .current_dir(temp.path())
        .output()
        .expect("jit init failed to spawn");
    assert!(
        output.status.success(),
        "jit init exited non-zero: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    std::fs::write(
        temp.path().join(".jit").join("invariants.toml"),
        r#"
[[invariants]]
id = "INV-01"
statement = "Every dependency edge stays acyclic."
kind = "enforced"
enforced-by = "dag-no-cycles"

[[invariants]]
id = "INV-02"
statement = "All state changes must be logged."
kind = "advisory"
"#,
    )
    .unwrap();

    temp
}

// ---------------------------------------------------------------------------
// REQ-01: entry with `statement`, `kind`, and optional `enforced-by` loads
// ---------------------------------------------------------------------------

#[test]
fn req01_invariant_entry_loads_at_config_time_config_present() {
    // REQ-01, config-present path: `JitConfig::load` populates `config.invariants`
    // with the parsed entries, including the full `enforced-by` binding.
    let (_temp, jit_root) = setup_jit_root_with_invariants_toml(true);

    let config = JitConfig::load(&jit_root).expect("config load must succeed");

    let invs = &config.invariants.invariants;
    assert_eq!(invs.len(), 2, "both invariants must be loaded");

    let inv01 = invs.iter().find(|i| i.id == "INV-01").unwrap();
    assert_eq!(
        inv01.statement, "Every dependency edge stays acyclic.",
        "statement must round-trip"
    );
    assert_eq!(inv01.kind, InvariantKind::Enforced, "kind must be Enforced");
    assert_eq!(
        inv01.enforced_by.as_deref(),
        Some("dag-no-cycles"),
        "enforced-by must round-trip"
    );

    let inv02 = invs.iter().find(|i| i.id == "INV-02").unwrap();
    assert_eq!(inv02.kind, InvariantKind::Advisory, "kind must be Advisory");
    assert!(
        inv02.enforced_by.is_none(),
        "advisory entry without enforced-by must have None"
    );
}

// ---------------------------------------------------------------------------
// REQ-02: invariants load on BOTH config-load return paths
// ---------------------------------------------------------------------------

#[test]
fn req02_invariants_load_without_config_toml() {
    // REQ-02, config-absent path: even when there is NO `config.toml`,
    // `JitConfig::load` still reads `invariants.toml` and populates the registry.
    let (_temp, jit_root) = setup_jit_root_with_invariants_toml(false);

    let config = JitConfig::load(&jit_root).expect("config load must succeed without config.toml");

    // Sanity: config-absent path must have returned an empty type-hierarchy.
    assert!(
        config.type_hierarchy.is_none(),
        "no config.toml means no type hierarchy"
    );

    // Invariants still populated.
    let invs = &config.invariants.invariants;
    assert_eq!(
        invs.len(),
        2,
        "invariants must load even when config.toml is absent"
    );
    assert!(
        invs.iter().any(|i| i.id == "INV-01"),
        "INV-01 must be present"
    );
    assert!(
        invs.iter().any(|i| i.id == "INV-02"),
        "INV-02 must be present"
    );
}

#[test]
fn req02_invariants_load_with_config_toml() {
    // REQ-02, config-present path: when `config.toml` exists, the chain-load of
    // `invariants.toml` still fires on the config-present code path.
    let (_temp, jit_root) = setup_jit_root_with_invariants_toml(true);

    let config = JitConfig::load(&jit_root).expect("config load must succeed with config.toml");

    // Sanity: config-present path must have a type hierarchy.
    assert!(
        config.type_hierarchy.is_some(),
        "config.toml supplies a type hierarchy"
    );

    // Invariants populated on this path too.
    let invs = &config.invariants.invariants;
    assert_eq!(
        invs.len(),
        2,
        "invariants must load on the config-present path"
    );
    assert!(
        invs.iter().any(|i| i.id == "INV-01"),
        "INV-01 must be present"
    );
}

#[test]
fn req02_absent_invariants_toml_is_empty_on_both_paths() {
    // REQ-02 boundary: when `.jit/invariants.toml` does not exist, both paths
    // yield an empty registry (not an error).
    for with_config in [false, true] {
        let temp = TempDir::new().unwrap();
        let jit_dir = temp.path().join(".jit");
        std::fs::create_dir_all(&jit_dir).unwrap();
        if with_config {
            std::fs::write(
                jit_dir.join("config.toml"),
                "[type_hierarchy]\ntypes = { task = 1 }\n",
            )
            .unwrap();
        }
        let config = JitConfig::load(&jit_dir)
            .unwrap_or_else(|e| panic!("load must succeed (with_config={with_config}): {e}"));
        assert!(
            config.invariants.invariants.is_empty(),
            "absent invariants.toml must yield an empty registry (with_config={with_config})"
        );
    }
}

// ---------------------------------------------------------------------------
// REQ-03: `jit item list --kind invariant` returns `@/<self-id>` addresses
// ---------------------------------------------------------------------------

#[test]
fn req03_item_list_kind_invariant_returns_at_qualified_ids() {
    // REQ-03: the real binary, given a `.jit/invariants.toml`, returns each
    // invariant addressed as `@/<self-id>` under `--kind invariant`.
    let temp = setup_cli_repo();

    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "invariant", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "item list --kind invariant must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value =
        serde_json::from_slice(&output.stdout).expect("--json output must be valid JSON");

    assert_eq!(
        json["count"].as_u64().unwrap(),
        2,
        "both invariants must be listed"
    );

    let items = json["items"].as_array().unwrap();
    let qids: Vec<&str> = items
        .iter()
        .map(|i| i["qualified_id"].as_str().unwrap())
        .collect();

    assert!(
        qids.contains(&"@/INV-01"),
        "INV-01 must be addressed as @/INV-01: {qids:?}"
    );
    assert!(
        qids.contains(&"@/INV-02"),
        "INV-02 must be addressed as @/INV-02: {qids:?}"
    );

    // Structural checks on one item.
    let inv01 = items.iter().find(|i| i["self_id"] == "INV-01").unwrap();
    assert_eq!(inv01["kind"].as_str().unwrap(), "invariant");
    assert_eq!(inv01["scope"].as_str().unwrap(), "@");
    assert_eq!(
        inv01["text"].as_str().unwrap(),
        "Every dependency edge stays acyclic.",
        "text must be the statement"
    );
}

#[test]
fn req03_item_show_at_qualified_id_resolves_invariant() {
    // REQ-03 (show path): `jit item show @/INV-01` also resolves the invariant
    // from the registry rather than the issue store.
    let temp = setup_cli_repo();

    let output = Command::new(jit_binary())
        .args(["item", "show", "@/INV-02", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "item show @/INV-02 must resolve: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["item"]["qualified_id"].as_str().unwrap(), "@/INV-02");
    assert_eq!(json["item"]["kind"].as_str().unwrap(), "invariant");
    assert_eq!(json["item"]["scope"].as_str().unwrap(), "@");
    assert!(
        json["item"]["text"]
            .as_str()
            .unwrap()
            .contains("state changes"),
        "text must carry the statement"
    );
}

// ---------------------------------------------------------------------------
// REQ-04: invalid entry makes config load fail with a typed, descriptive error
// ---------------------------------------------------------------------------

#[test]
fn req04_missing_statement_fails_with_typed_error() {
    // REQ-04: an invariant entry without `statement` makes `JitConfig::load` fail
    // with a typed error that names both the file and the missing field.
    for with_config in [false, true] {
        let temp = TempDir::new().unwrap();
        let jit_dir = temp.path().join(".jit");
        std::fs::create_dir_all(&jit_dir).unwrap();
        if with_config {
            std::fs::write(
                jit_dir.join("config.toml"),
                "[type_hierarchy]\ntypes = { task = 1 }\n",
            )
            .unwrap();
        }
        // Valid id and kind but no statement.
        std::fs::write(
            jit_dir.join("invariants.toml"),
            "[[invariants]]\nid = \"INV-01\"\nkind = \"advisory\"\n",
        )
        .unwrap();

        let err = JitConfig::load(&jit_dir).expect_err("missing statement must fail config load");

        // `JitConfig::load` wraps the underlying parse error with anyhow context
        // ("invalid .jit/invariants.toml"), so `err.to_string()` gives the context
        // layer.  The full chain (via `{err:#}`) carries the underlying TOML error
        // that names both the file (context layer) and the missing field (cause).
        let full = format!("{err:#}");
        assert!(
            full.contains("invariants.toml"),
            "error must name the file (with_config={with_config}): {full}"
        );
        assert!(
            full.contains("statement"),
            "error must name the missing field (with_config={with_config}): {full}"
        );
    }
}

#[test]
fn req04_invalid_kind_token_fails_with_descriptive_error() {
    // REQ-04: a `kind = "bogus"` token (not `enforced` or `advisory`) must fail
    // config load with an error that names both the file and the valid options.
    let temp = TempDir::new().unwrap();
    let jit_dir = temp.path().join(".jit");
    std::fs::create_dir_all(&jit_dir).unwrap();

    std::fs::write(
        jit_dir.join("invariants.toml"),
        "[[invariants]]\nid = \"INV-01\"\nstatement = \"x\"\nkind = \"bogus\"\n",
    )
    .unwrap();

    let err = JitConfig::load(&jit_dir).expect_err("invalid kind token must fail config load");

    // See req04_missing_statement_fails_with_typed_error for why we use `{err:#}`.
    let full = format!("{err:#}");
    assert!(
        full.contains("invariants.toml"),
        "error must name the file: {full}"
    );
    // The serde error for the enum lists the valid variants.
    assert!(
        full.contains("enforced") && full.contains("advisory"),
        "error must list valid kind values: {full}"
    );
}

#[test]
fn req04_error_type_is_invariant_config_error() {
    // REQ-04 (type check via invariants module directly): `InvariantRegistry::from_toml_str`
    // surfaces a typed `InvariantConfigError::Toml` for missing fields, and
    // `InvariantConfigError::DuplicateId` for duplicate ids.
    use jit::validation::invariants::InvariantRegistry;

    // Missing statement → InvariantConfigError::Toml naming the field.
    let err =
        InvariantRegistry::from_toml_str("[[invariants]]\nid = \"INV-01\"\nkind = \"advisory\"\n")
            .unwrap_err();
    assert!(
        matches!(err, InvariantConfigError::Toml(_)),
        "missing statement must produce InvariantConfigError::Toml, got: {err:?}"
    );
    assert!(
        err.to_string().contains("statement"),
        "Toml error must name the missing field: {err}"
    );

    // Duplicate id → InvariantConfigError::DuplicateId.
    let toml = r#"
[[invariants]]
id = "INV-01"
statement = "a"
kind = "advisory"

[[invariants]]
id = "INV-01"
statement = "b"
kind = "enforced"
"#;
    let err = InvariantRegistry::from_toml_str(toml).unwrap_err();
    assert!(
        matches!(err, InvariantConfigError::DuplicateId { ref id } if id == "INV-01"),
        "duplicate id must produce InvariantConfigError::DuplicateId, got: {err:?}"
    );
}

// ---------------------------------------------------------------------------
// REQ-05: registry is authoritative — no markdown index for invariants
// ---------------------------------------------------------------------------

#[test]
fn req05_inv_looking_line_in_issue_description_does_not_produce_invariant_item() {
    // REQ-05: the ONLY source of invariant items is `.jit/invariants.toml`.
    // An INV-tagged line inside an issue description (which would be indexed by
    // a markdown-first kind) must NOT appear as an invariant.
    let temp = setup_cli_repo();

    // Create an issue whose description contains an INV-looking criterion line.
    // If the engine were to parse markdown for invariants, "INV-99" would leak in.
    let output = Command::new(jit_binary())
        .args([
            "issue",
            "create",
            "-t",
            "Decoy issue",
            "-d",
            "## Success Criteria\n\n- [hard] INV-99: this must NOT appear as an invariant\n",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "issue create must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Query only `invariant`-kind items.
    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "invariant", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["items"].as_array().unwrap();
    let qids: Vec<&str> = items
        .iter()
        .map(|i| i["qualified_id"].as_str().unwrap())
        .collect();

    // Only the two registry entries may appear.
    assert_eq!(
        json["count"].as_u64().unwrap(),
        2,
        "only the registry's two entries must appear, not the markdown decoy: {qids:?}"
    );
    assert!(
        !qids.iter().any(|q| q.contains("INV-99")),
        "INV-99 from the issue description must NOT appear as an invariant: {qids:?}"
    );
    assert!(
        qids.contains(&"@/INV-01") && qids.contains(&"@/INV-02"),
        "the two registry entries must still be present: {qids:?}"
    );
}

#[test]
fn req05_item_list_without_kind_filter_does_not_mix_invariants_with_requirements() {
    // REQ-05 (cross-kind boundary): when listing ALL items, invariants appear only
    // from the registry and requirements appear only from issue markdown.  The
    // registry does not bleed across kind boundaries.
    let temp = setup_cli_repo();

    // Create an issue with a requirement criterion.
    let output = Command::new(jit_binary())
        .args([
            "issue",
            "create",
            "-t",
            "Feature",
            "-d",
            "## Success Criteria\n\n- [hard] REQ-01: something must work\n",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "issue create must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let issue_json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let issue_short: String = issue_json["id"].as_str().unwrap().chars().take(8).collect();

    // List all items (no kind filter).
    let output = Command::new(jit_binary())
        .args(["item", "list", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["items"].as_array().unwrap();
    let qids: Vec<&str> = items
        .iter()
        .map(|i| i["qualified_id"].as_str().unwrap())
        .collect();

    // Invariants from the registry must be present.
    assert!(
        qids.contains(&"@/INV-01"),
        "@/INV-01 must appear in the unfiltered list: {qids:?}"
    );
    assert!(
        qids.contains(&"@/INV-02"),
        "@/INV-02 must appear in the unfiltered list: {qids:?}"
    );

    // The requirement from the issue markdown must also be present.
    let req_qid = format!("{issue_short}/REQ-01");
    assert!(
        qids.contains(&req_qid.as_str()),
        "{req_qid} from issue markdown must appear: {qids:?}"
    );

    // Every invariant-kind item is from the registry (@-scope); none is issue-scoped.
    for item in items {
        if item["kind"].as_str() == Some("invariant") {
            assert_eq!(
                item["scope"].as_str(),
                Some("@"),
                "invariants must always be project-scoped (@): {item}"
            );
        }
    }
}
