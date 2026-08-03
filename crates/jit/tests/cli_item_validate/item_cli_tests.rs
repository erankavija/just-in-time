//! Integration tests for the `jit item` subcommands (list/show/search/resolve).
//!
//! Exercises the addressable-item model end-to-end through the real CLI: an
//! issue's success-criteria requirements are indexed, addressed by qualified id,
//! filtered by kind, searched, and resolved. Also covers graceful degradation
//! (a prose line is not indexed) and a config-declared custom kind (REQ-01).

use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_test_repo() -> TempDir {
    crate::setup_repo_with_default_vocabulary()
}

/// Create an issue with the given body and return its short id.
fn create_issue(dir: &std::path::Path, title: &str, body: &str) -> String {
    let output = Command::new(jit_binary())
        .args(["issue", "create", "-t", title, "-d", body, "--json"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "issue create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let full_id = json["id"].as_str().expect("created issue has an id");
    // The qualified-id scope uses the git-style 8-char short id.
    full_id.chars().take(8).collect()
}

#[test]
fn test_item_list_indexes_requirements() {
    let temp = setup_test_repo();
    let body = "## Success Criteria\n\n- [hard] REQ-01: first\n- [hard] REQ-02: second\n";
    let short = create_issue(temp.path(), "Foundational", body);

    // Scoped to `requirement`: an unfiltered list also carries the project-scope
    // `rule` items the applied package contributes via `.jit/rules.toml`
    // (jit:cdc33a0f).
    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "requirement", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 2);
    let items = json["items"].as_array().unwrap();
    let qids: Vec<&str> = items
        .iter()
        .map(|i| i["qualified_id"].as_str().unwrap())
        .collect();
    assert!(qids.contains(&format!("@/issue/{short}/requirement/REQ-01").as_str()));
    assert!(qids.contains(&format!("@/issue/{short}/requirement/REQ-02").as_str()));
    assert_eq!(items[0]["kind"].as_str().unwrap(), "requirement");
}

#[test]
fn test_item_list_kind_filter() {
    let temp = setup_test_repo();
    create_issue(
        temp.path(),
        "Foundational",
        "## Success Criteria\n\n- [hard] REQ-01: a\n",
    );

    // The package-supplied kind name matches.
    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "requirement", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 1);

    // `decision` is supplied by the applied package, so it is recognized; this issue
    // has no `## Decisions` section, so the recognized kind yields 0 items (not an
    // error). (Decision indexing is covered in decision_kind_tests.rs.)
    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "decision", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 0);

    // A genuinely unknown kind name also yields an empty result (not an error).
    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "nonexistent", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 0);
}

#[test]
fn test_item_graceful_degradation() {
    let temp = setup_test_repo();
    // A prose criterion line with no self-id is not indexed (REQ-06).
    let body = "## Success Criteria\n\n- [hard] REQ-01: real\n- [hard] just prose, no id here\n";
    create_issue(temp.path(), "Mixed", body);

    // Scoped to `requirement`: an unfiltered list also carries the project-scope
    // `rule` items the applied package contributes via `.jit/rules.toml`
    // (jit:cdc33a0f).
    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "requirement", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 1);
}

#[test]
fn test_item_show_and_resolve_by_qualified_id() {
    let temp = setup_test_repo();
    let short = create_issue(
        temp.path(),
        "Foundational",
        "## Success Criteria\n\n- [hard] REQ-01: atomic writes\n",
    );
    // The `<short-id>/<self-id>` sugar is the INPUT; the minted qualified id is the
    // canonical uniform kind-segmented form.
    let sugar = format!("{short}/REQ-01");
    let expected_qid = format!("@/issue/{short}/requirement/REQ-01");

    for verb in ["show", "resolve"] {
        let output = Command::new(jit_binary())
            .args(["item", verb, &sugar, "--json"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "item {verb} failed");
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["item"]["self_id"].as_str().unwrap(), "REQ-01");
        assert_eq!(json["item"]["qualified_id"].as_str().unwrap(), expected_qid);
        assert!(json["item"]["text"]
            .as_str()
            .unwrap()
            .contains("atomic writes"));
    }
}

#[test]
fn test_item_show_unknown_self_id_fails() {
    let temp = setup_test_repo();
    let short = create_issue(
        temp.path(),
        "Foundational",
        "## Success Criteria\n\n- [hard] REQ-01: a\n",
    );
    let output = Command::new(jit_binary())
        .args(["item", "show", &format!("{short}/REQ-99")])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_item_search_by_text() {
    let temp = setup_test_repo();
    create_issue(
        temp.path(),
        "Foundational",
        "## Success Criteria\n\n- [hard] REQ-01: atomic writes\n- [hard] REQ-02: cycle detect\n",
    );
    let output = Command::new(jit_binary())
        .args(["item", "search", "atomic", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 1);
    assert_eq!(json["items"][0]["self_id"].as_str().unwrap(), "REQ-01");
}

#[test]
fn test_item_custom_kind_from_config() {
    let temp = setup_test_repo();
    // Declare a domain-agnostic custom kind in config (a name JIT does NOT ship as
    // a package-supplied kind); the engine indexes it purely from its tuple, never from its
    // name (REQ-01). An explicit declaration sets all six required fields.
    let config_path = temp.path().join(".jit").join("config.toml");
    let mut config = std::fs::read_to_string(&config_path).unwrap_or_default();
    config.push_str(
        "\n[item_kinds.adr]\n\
         section = \"records\"\n\
         id-pattern = \"ADR-[0-9]+\"\n\
         markers = []\n\
         link-namespaces = [\"records\"]\n\
         scope = \"issue\"\n\
         source-of-truth = \"markdown-first\"\n",
    );
    std::fs::write(&config_path, config).unwrap();

    create_issue(
        temp.path(),
        "With records",
        "## Records\n\n- ADR-1: use json storage\n- ADR-2: atomic writes\n",
    );

    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "adr", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "item list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 2);
    assert_eq!(json["items"][0]["kind"].as_str().unwrap(), "adr");
    assert_eq!(json["items"][0]["self_id"].as_str().unwrap(), "ADR-1");
}

#[test]
fn test_issue_show_resolves_qualified_item_id() {
    // Finding 3: `jit issue show <issue>/<self-id>` resolves the addressed item
    // through the existing show dispatch.
    let temp = setup_test_repo();
    let short = create_issue(
        temp.path(),
        "Foundational",
        "## Success Criteria\n\n- [hard] REQ-01: atomic writes\n",
    );
    let sugar = format!("{short}/REQ-01");

    let output = Command::new(jit_binary())
        .args(["issue", "show", &sugar, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "issue show <qualified> failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["item"]["self_id"].as_str().unwrap(), "REQ-01");
    assert_eq!(
        json["item"]["qualified_id"].as_str().unwrap(),
        format!("@/issue/{short}/requirement/REQ-01")
    );

    // Human (non-JSON) path also renders the addressed item.
    let output = Command::new(jit_binary())
        .args(["issue", "show", &sugar])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Self id:") && stdout.contains("REQ-01"));
}

#[test]
fn test_item_command_failure_emits_json() {
    // Finding 4: an item command FAILURE with --json must emit a JSON object on
    // stdout, not a plain `Error: ...` line.
    let temp = setup_test_repo();
    let short = create_issue(
        temp.path(),
        "Foundational",
        "## Success Criteria\n\n- [hard] REQ-01: a\n",
    );

    let output = Command::new(jit_binary())
        .args(["item", "show", &format!("{short}/REQ-99"), "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(!output.status.success(), "unknown self-id should fail");
    // stdout must be valid JSON carrying an error object.
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("--json failure must emit valid JSON on stdout");
    assert!(
        json.get("error").is_some(),
        "JSON error object expected, got: {json}"
    );

    // The qualified-id path through `jit issue show` also emits JSON on failure.
    let output = Command::new(jit_binary())
        .args(["issue", "show", &format!("{short}/REQ-99"), "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("issue show <qualified> --json failure must emit valid JSON");
    assert!(json.get("error").is_some());
}

/// Append to `.jit/config.toml` a markdown-first project-scope `glossary` kind
/// sourced from `project-items.md` (preserving the existing config), and
/// optionally write that source file.
///
/// Uses the name `glossary` to exercise generic markdown-first project-scope
/// sourcing through a kind distinct from the registry-first `invariant` default.
fn configure_project_scope_kind(repo: &std::path::Path, source_md: Option<&str>) {
    let config_path = repo.join(".jit").join("config.toml");
    let mut config = std::fs::read_to_string(&config_path).unwrap_or_default();
    config.push_str(
        "\n[item_kinds.glossary]\n\
         section = \"success_criteria\"\n\
         id-pattern = \"GLOSS-[0-9]+\"\n\
         markers = []\n\
         link-namespaces = [\"defines\"]\n\
         scope = \"project\"\n\
         source = \"project-items.md\"\n\
         source-of-truth = \"markdown-first\"\n",
    );
    std::fs::write(&config_path, config).unwrap();
    if let Some(md) = source_md {
        std::fs::write(repo.join("project-items.md"), md).unwrap();
    }
}

#[test]
fn test_item_show_project_scope_resolves_through_real_cli() {
    // REQ-01: `@/<kind>/<self-id>` RESOLVES through the actual `jit item show` binary,
    // sourced from a config-declared repository-local file (no test seam).
    let temp = setup_test_repo();
    configure_project_scope_kind(
        temp.path(),
        Some("## Success Criteria\n\n- GLOSS-01: all writes are atomic\n"),
    );

    for verb in ["show", "resolve"] {
        let output = Command::new(jit_binary())
            .args(["item", verb, "@/glossary/GLOSS-01", "--json"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "@/glossary/GLOSS-01 must resolve via item {verb}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["item"]["self_id"].as_str().unwrap(), "GLOSS-01");
        assert_eq!(
            json["item"]["qualified_id"].as_str().unwrap(),
            "@/glossary/GLOSS-01"
        );
        assert_eq!(json["item"]["scope"].as_str().unwrap(), "@");
        assert_eq!(json["item"]["kind"].as_str().unwrap(), "glossary");
        assert!(json["item"]["text"].as_str().unwrap().contains("atomic"));
    }

    // The same `@` id resolves through `jit issue show <qualified>` too.
    let output = Command::new(jit_binary())
        .args(["issue", "show", "@/glossary/GLOSS-01", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "issue show @/glossary/GLOSS-01 must resolve"
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["item"]["qualified_id"].as_str().unwrap(),
        "@/glossary/GLOSS-01"
    );

    // And it appears in `jit item list`.
    let output = Command::new(jit_binary())
        .args(["item", "list", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let qids: Vec<&str> = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["qualified_id"].as_str().unwrap())
        .collect();
    assert!(
        qids.contains(&"@/glossary/GLOSS-01"),
        "list must include @/glossary/GLOSS-01: {qids:?}"
    );
}

#[test]
fn test_item_show_project_scope_absent_source_is_graceful() {
    // REQ-01 (degradation): a project-scope kind whose source file is absent
    // resolves to a descriptive not-found error (not a panic, not the issue
    // resolver), with a JSON error object on stdout under --json.
    let temp = setup_test_repo();
    configure_project_scope_kind(temp.path(), None);

    let output = Command::new(jit_binary())
        .args(["item", "show", "@/glossary/GLOSS-01", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("--json failure must emit valid JSON on stdout");
    let msg = json["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("project scope") && msg.contains("no addressable item"),
        "error must describe the missing project-scope item, got: {json}"
    );
    assert!(
        !msg.contains("resolve issue scope"),
        "@ must route to the project scope, not the issue resolver: {json}"
    );
}

#[test]
fn test_item_list_kind_invariant_registry_first_through_real_cli() {
    // REQ-01 + Finding 1/2 (rework): the SHIPPED CLI returns each invariant from
    // `.jit/invariants.toml` as `@/<kind>/<self-id>`, with the package-supplied
    // registry-first invariant kind and NO markdown source involved.
    let temp = setup_test_repo();
    std::fs::write(
        temp.path().join(".jit").join("invariants.toml"),
        "[[invariants]]\n\
         id = \"sample-invariant\"\n\
         statement = \"Every dependency edge stays acyclic.\"\n\
         kind = \"enforced\"\n\
         enforced-by = \"dag-no-cycles\"\n\n\
         [[invariants]]\n\
         id = \"second-invariant\"\n\
         statement = \"All state changes are logged.\"\n\
         kind = \"advisory\"\n",
    )
    .unwrap();
    // An issue whose description carries an invariant-looking line must NOT leak in as an
    // invariant (registry is authoritative; no markdown index for invariants).
    create_issue(
        temp.path(),
        "Decoy",
        "## Success Criteria\n\n- [hard] missing-invariant: looks like an invariant\n",
    );

    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "invariant", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "item list --kind invariant failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 2);
    let items = json["items"].as_array().unwrap();
    let qids: Vec<&str> = items
        .iter()
        .map(|i| i["qualified_id"].as_str().unwrap())
        .collect();
    assert!(
        qids.contains(&"@/invariant/sample-invariant"),
        "list must include @/invariant/sample-invariant: {qids:?}"
    );
    assert!(
        qids.contains(&"@/invariant/second-invariant"),
        "list must include @/invariant/second-invariant: {qids:?}"
    );
    // The decoy missing-invariant from an issue description is NOT an invariant (REQ-02).
    assert!(
        !qids.iter().any(|q| q.contains("missing-invariant")),
        "no markdown index for invariants: {qids:?}"
    );
    let inv01 = items
        .iter()
        .find(|i| i["self_id"] == "sample-invariant")
        .unwrap();
    assert_eq!(inv01["kind"].as_str().unwrap(), "invariant");
    assert_eq!(inv01["scope"].as_str().unwrap(), "@");
    assert_eq!(
        inv01["text"].as_str().unwrap(),
        "Every dependency edge stays acyclic."
    );

    // `jit item show @/invariant/second-invariant` resolves through the shipped binary too.
    let output = Command::new(jit_binary())
        .args(["item", "show", "@/invariant/second-invariant", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "item show @/invariant/second-invariant must resolve"
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["item"]["qualified_id"].as_str().unwrap(),
        "@/invariant/second-invariant"
    );
    assert_eq!(json["item"]["kind"].as_str().unwrap(), "invariant");
}

#[test]
fn test_item_kind_alias_resolves_through_real_cli() {
    // REQ-01/REQ-03: the shipped package declares `aliases = ["inv"]`
    // on the invariant kind. The alias is accepted anywhere the registry name is
    // (the `--kind` filter and the kind segment of a project-scope address), and
    // canonical output still uses the registry name `invariant`.
    let temp = setup_test_repo();
    std::fs::write(
        temp.path().join(".jit").join("invariants.toml"),
        "[[invariants]]\n\
         id = \"sample-invariant\"\n\
         statement = \"Every dependency edge stays acyclic.\"\n\
         kind = \"enforced\"\n",
    )
    .unwrap();

    // `--kind inv` filters the same items as `--kind invariant`; qualified_id and
    // kind fields are canonical (`invariant`, not the alias).
    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "inv", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "item list --kind inv failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 1);
    let item = &json["items"][0];
    assert_eq!(
        item["qualified_id"].as_str().unwrap(),
        "@/invariant/sample-invariant"
    );
    assert_eq!(item["kind"].as_str().unwrap(), "invariant");

    // `jit item show @/inv/sample-invariant` resolves the SAME item as `@/invariant/sample-invariant`,
    // and the canonical qualified_id uses the registry name.
    let output = Command::new(jit_binary())
        .args(["item", "show", "@/inv/sample-invariant", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "item show @/inv/sample-invariant must resolve: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["item"]["qualified_id"].as_str().unwrap(),
        "@/invariant/sample-invariant"
    );
    assert_eq!(json["item"]["kind"].as_str().unwrap(), "invariant");
}

#[test]
fn test_invariant_name_is_no_longer_reserved_through_real_cli() {
    // REQ-03: the SHIPPED CLI no longer reserves the `invariant` name. A
    // config-declared markdown-first `[item_kinds.invariant]` (once rejected) now
    // resolves as ORDINARY config, indexing project items from its declared markdown
    // source like any other markdown-first project kind.
    let temp = setup_test_repo();
    let config_path = temp.path().join(".jit").join("config.toml");
    // REPLACE the package-supplied (registry-first) `invariant` kind with a
    // markdown-first one: the `invariant` name carries no reserved routing, so a
    // markdown-first project `invariant` kind (once rejected) now resolves as
    // ordinary config and indexes from its declared markdown source.
    std::fs::write(
        &config_path,
        "[version]\nschema = 2\n\n\
         [item_kinds.invariant]\n\
         section = \"success_criteria\"\n\
         id-pattern = \"[a-z][a-z0-9-]*\"\n\
         markers = []\n\
         link-namespaces = [\"enforces\"]\n\
         scope = \"project\"\n\
         source = \"project-items.md\"\n\
         source-of-truth = \"markdown-first\"\n",
    )
    .unwrap();
    std::fs::write(
        temp.path().join("project-items.md"),
        "## Success Criteria\n\n- sample-invariant: a markdown-sourced invariant item\n",
    )
    .unwrap();

    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "invariant", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "markdown-first invariant is now ordinary config: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 1);
    let items = json["items"].as_array().unwrap();
    assert_eq!(
        items[0]["qualified_id"].as_str().unwrap(),
        "@/invariant/sample-invariant"
    );
    assert_eq!(items[0]["kind"].as_str().unwrap(), "invariant");
}

#[test]
fn test_item_list_and_show_kind_gate_registry_first_through_real_cli() {
    // REQ-02, REQ-03 (jit:42898915): the SHIPPED CLI addresses gates from
    // `.jit/gates.toml` as `@/gate/<key>`, a project-scoped registry-first kind
    // mirroring `invariant`/`rule`. `[item_kinds.gate]` is supplied by the applied
    // package (jit:bb7d57a2), so a profiled repo already declares it;
    // this test only needs to seed a gate entry in `.jit/gates.toml`.
    let temp = setup_test_repo();

    std::fs::write(
        temp.path().join(".jit").join("gates.toml"),
        "[[gates]]\n\
         key = \"cargo-ci\"\n\
         title = \"Cargo CI\"\n\
         description = \"Full Rust CI pipeline must pass.\"\n\
         stage = \"postcheck\"\n",
    )
    .unwrap();

    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "gate", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "item list --kind gate failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 1);
    let item = &json["items"][0];
    assert_eq!(item["kind"].as_str().unwrap(), "gate");
    assert_eq!(item["self_id"].as_str().unwrap(), "cargo-ci");
    assert_eq!(item["scope"].as_str().unwrap(), "@");
    assert_eq!(
        item["text"].as_str().unwrap(),
        "Full Rust CI pipeline must pass."
    );

    // `jit item show @/gate/cargo-ci` resolves through the shipped binary too.
    let output = Command::new(jit_binary())
        .args(["item", "show", "@/gate/cargo-ci", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "item show @/gate/cargo-ci failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["item"]["kind"].as_str().unwrap(), "gate");
    assert_eq!(
        json["item"]["text"].as_str().unwrap(),
        "Full Rust CI pipeline must pass."
    );
}

/// Append a `config.toml` namespace registration so the `satisfies` link-namespace
/// label passes the default namespace-registry check, leaving the
/// dangling-item-link finding as the only validation error under test.
/// `enforces` is NOT appended here: the applied package already declares
/// `[namespaces.enforces]` (jit:d30695e4), so appending it again would define the
/// same TOML table twice and fail to parse.
fn register_link_namespaces(dir: &std::path::Path) {
    let config_path = dir.join(".jit").join("config.toml");
    let mut config = std::fs::read_to_string(&config_path).unwrap();
    config
        .push_str("\n[namespaces.satisfies]\ndescription = \"Satisfied item.\"\nunique = false\n");
    std::fs::write(config_path, config).unwrap();
}

#[test]
fn test_validate_reports_dangling_item_link() {
    // REQ-03 end-to-end via the real `jit validate` binary: a node carrying a
    // qualified-but-unresolvable link (`satisfies:<scope>/BOGUS`) surfaces a
    // `dangling-item-link` error finding, NOT silently dropped, and fails
    // validation (non-zero exit).
    let temp = setup_test_repo();
    register_link_namespaces(temp.path());

    let target = create_issue(
        temp.path(),
        "target",
        "## Success Criteria\n\n- [hard] REQ-01: real\n",
    );
    // A node that links to a non-existent self-id in the target's scope.
    let node = Command::new(jit_binary())
        .args([
            "issue",
            "create",
            "-t",
            "node",
            "-d",
            "node body",
            "-l",
            &format!("satisfies:{target}/BOGUS"),
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let node_json: Value = serde_json::from_slice(&node.stdout).unwrap();
    let node_short: String = node_json["id"].as_str().unwrap().chars().take(8).collect();
    // Connect the two so neither is an isolated node (an integrity error that
    // would abort before the rule report is built).
    Command::new(jit_binary())
        .args(["dep", "add", &node_short, &target])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(jit_binary())
        .args(["validate", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    // A dangling link is an error-severity finding -> non-zero exit.
    assert!(
        !output.status.success(),
        "validate must fail on a dangling item link"
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let findings = json["error"]["details"]["rule_findings"]
        .as_array()
        .expect("rule_findings array");
    let dangling: Vec<&Value> = findings
        .iter()
        .filter(|f| f["rule"].as_str() == Some("dangling-item-link"))
        .collect();
    assert_eq!(
        dangling.len(),
        1,
        "exactly one dangling-item-link finding expected, got: {json}"
    );
    let msg = dangling[0]["message"].as_str().unwrap();
    assert!(
        msg.contains("BOGUS"),
        "finding names the dangling id: {msg}"
    );
    assert_eq!(dangling[0]["severity"].as_str(), Some("error"));
}

#[test]
fn test_validate_no_finding_for_resolvable_item_link() {
    // A resolvable qualified link produces NO dangling-item-link finding.
    let temp = setup_test_repo();
    register_link_namespaces(temp.path());

    let target = create_issue(
        temp.path(),
        "target",
        "## Success Criteria\n\n- [hard] REQ-01: real\n",
    );
    let node = Command::new(jit_binary())
        .args([
            "issue",
            "create",
            "-t",
            "node",
            "-d",
            "node body",
            "-l",
            &format!("satisfies:{target}/REQ-01"),
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let node_json: Value = serde_json::from_slice(&node.stdout).unwrap();
    let node_short: String = node_json["id"].as_str().unwrap().chars().take(8).collect();
    Command::new(jit_binary())
        .args(["dep", "add", &node_short, &target])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(jit_binary())
        .args(["validate", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let findings = json["rule_findings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !findings
            .iter()
            .any(|f| f["rule"].as_str() == Some("dangling-item-link")),
        "a resolvable link must yield no dangling-item-link finding: {json}"
    );
}

#[test]
fn test_validate_passes_with_enforces_rule_and_gate_links_through_real_cli() {
    // REQ-01/REQ-02 (jit:d30695e4): `rule` and `gate` declare
    // `link-namespaces = ["enforces"]`, so an authored `enforces:@/rule/<name>`
    // and `enforces:@/gate/<key>` label both resolve through the shipped `jit
    // item show`, AND `jit validate` reports neither a dangling-item-link finding
    // (the labels resolve) nor a namespace-registry finding (the applied package
    // declares `[namespaces.enforces]`) — no `register_link_namespaces` helper
    // needed here, unlike the `satisfies` tests above.
    let temp = setup_test_repo();

    // Bare init writes `.jit/rules.toml` with the default `label-format` rule
    // and leaves `.jit/gates.toml` empty; define a gate so `@/gate/cargo-ci`
    // resolves.
    let define_output = Command::new(jit_binary())
        .args([
            "gate",
            "define",
            "cargo-ci",
            "--title",
            "Cargo CI",
            "--description",
            "Full Rust CI pipeline must pass.",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        define_output.status.success(),
        "gate define failed: {}",
        String::from_utf8_lossy(&define_output.stderr)
    );

    let target = create_issue(
        temp.path(),
        "target",
        "## Success Criteria\n\n- [hard] REQ-01: real\n",
    );
    let node = Command::new(jit_binary())
        .args([
            "issue",
            "create",
            "-t",
            "node",
            "-d",
            "node body",
            "-l",
            "enforces:@/rule/label-format",
            "-l",
            "enforces:@/gate/cargo-ci",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        node.status.success(),
        "issue create failed: {}",
        String::from_utf8_lossy(&node.stderr)
    );
    let node_json: Value = serde_json::from_slice(&node.stdout).unwrap();
    let node_short: String = node_json["id"].as_str().unwrap().chars().take(8).collect();
    // Connect the two so neither is an isolated node (an integrity error that
    // would abort validation before the rule report is built).
    Command::new(jit_binary())
        .args(["dep", "add", &node_short, &target])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Both kind-segmented addresses resolve through the shipped CLI.
    let show_rule = Command::new(jit_binary())
        .args(["item", "show", "@/rule/label-format", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        show_rule.status.success(),
        "item show @/rule/label-format failed: {}",
        String::from_utf8_lossy(&show_rule.stderr)
    );
    let rule_json: Value = serde_json::from_slice(&show_rule.stdout).unwrap();
    assert_eq!(rule_json["item"]["kind"].as_str().unwrap(), "rule");

    let show_gate = Command::new(jit_binary())
        .args(["item", "show", "@/gate/cargo-ci", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        show_gate.status.success(),
        "item show @/gate/cargo-ci failed: {}",
        String::from_utf8_lossy(&show_gate.stderr)
    );
    let gate_json: Value = serde_json::from_slice(&show_gate.stdout).unwrap();
    assert_eq!(gate_json["item"]["kind"].as_str().unwrap(), "gate");

    let output = Command::new(jit_binary())
        .args(["validate", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let findings = json["rule_findings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !findings
            .iter()
            .any(|f| f["rule"].as_str() == Some("dangling-item-link")),
        "an enforces: rule/gate link must resolve, not dangle: {json}"
    );
    assert!(
        !findings
            .iter()
            .any(|f| f["rule"].as_str() == Some("namespace-registry")),
        "the applied package must register the enforces namespace: {json}"
    );
}

#[test]
fn test_item_list_qualified_ids_round_trip_through_show() {
    // REQ-01/REQ-04: every qualified id `jit item list --json` prints is itself a
    // canonical kind-segmented address that resolves through `jit item show`,
    // yielding the SAME id back. Exercises both substrates the applied package
    // populates: issue-scope items (from the created issue's markdown) and
    // project-scope registry-first items (the seeded `.jit/rules.toml` rules).
    let temp = setup_test_repo();
    create_issue(
        temp.path(),
        "Foundational",
        "## Success Criteria\n\n- [hard] REQ-01: atomic writes\n- [hard] REQ-02: cycle detect\n",
    );

    let output = Command::new(jit_binary())
        .args(["item", "list", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let list: Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = list["items"].as_array().unwrap();
    assert!(
        !items.is_empty(),
        "the applied package must list at least the created issue's requirements"
    );

    let mut saw_issue_scope = false;
    let mut saw_project_scope = false;
    for item in items {
        let qid = item["qualified_id"].as_str().unwrap();
        // Every minted id carries the kind segment; none is the retired kindless
        // `@/<self-id>` project form.
        assert!(
            qid.starts_with("@/issue/") || qid.starts_with('@'),
            "every minted id is kind-segmented under the `@` scheme: {qid}"
        );
        if qid.starts_with("@/issue/") {
            saw_issue_scope = true;
        } else {
            saw_project_scope = true;
        }

        let shown = Command::new(jit_binary())
            .args(["item", "show", qid, "--json"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(
            shown.status.success(),
            "qualified id '{qid}' from `item list` must resolve via `item show`: {}",
            String::from_utf8_lossy(&shown.stdout)
        );
        let shown_json: Value = serde_json::from_slice(&shown.stdout).unwrap();
        assert_eq!(
            shown_json["item"]["qualified_id"].as_str().unwrap(),
            qid,
            "round-trip must return the same qualified id"
        );
        assert_eq!(
            shown_json["item"]["self_id"].as_str(),
            item["self_id"].as_str(),
            "round-trip must return the same self-id for {qid}"
        );
    }
    assert!(
        saw_issue_scope,
        "the created issue's requirements must appear as issue-scope items"
    );
    assert!(
        saw_project_scope,
        "the package-contributed rules registry must appear as project-scope items"
    );
}

#[test]
fn test_item_show_rule_renders_description_and_name_fallback() {
    // REQ-03: through the shipped binary, `jit item show @/rule/<name>` displays
    // the rule's DESCRIPTION as its text (the item kind's `text-field` is
    // `description`), and a description-less rule falls back to its NAME.
    let temp = setup_test_repo();

    // The applied package contributes described default rules to `.jit/rules.toml`;
    // append a
    // hand-authored rule that deliberately omits `description` to exercise the
    // name fallback on the same registry.
    let rules_path = temp.path().join(".jit").join("rules.toml");
    let mut rules = std::fs::read_to_string(&rules_path).unwrap();
    rules.push_str(
        "\n[[rules]]\nname = \"no-description-rule\"\n\
         assert = { require-section = { heading = \"Goals\" } }\n",
    );
    std::fs::write(&rules_path, rules).unwrap();

    // A seeded rule shows its description verbatim.
    let described = Command::new(jit_binary())
        .args(["item", "show", "@/rule/label-format", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        described.status.success(),
        "item show @/rule/label-format failed: {}",
        String::from_utf8_lossy(&described.stderr)
    );
    let described_json: Value = serde_json::from_slice(&described.stdout).unwrap();
    let text = described_json["item"]["text"].as_str().unwrap();
    assert!(
        text.contains("canonical `namespace:value` format"),
        "rule item text must render the seeded description, got: {text}"
    );
    // The name is NOT the display text once a description exists.
    assert_ne!(text, "label-format");

    // The description-less rule falls back to its name as display text.
    let bare = Command::new(jit_binary())
        .args(["item", "show", "@/rule/no-description-rule", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        bare.status.success(),
        "item show @/rule/no-description-rule failed: {}",
        String::from_utf8_lossy(&bare.stderr)
    );
    let bare_json: Value = serde_json::from_slice(&bare.stdout).unwrap();
    assert_eq!(
        bare_json["item"]["text"].as_str().unwrap(),
        "no-description-rule",
        "a description-less rule must fall back to its name as display text"
    );
}

#[test]
fn test_namespace_unique_rule_addressable_after_config_driven_write() {
    // REQ-02 (jit:d74a9ed1): a namespace hand-declared unique in config.toml —
    // with no intervening jit write to regenerate rules.toml — becomes
    // resolvable at `@/rule/<name>` (here, its `namespace-unique-<ns>` row)
    // once the NEXT jit-driven write (here, `jit config set`) runs. The
    // registry-first `rule` item kind reads `.jit/rules.toml` straight off
    // disk, not the in-memory-reconciled ruleset, so this only holds because
    // the write-through actually persists
    // the row.
    let temp = setup_test_repo();
    let config_path = temp.path().join(".jit").join("config.toml");
    let mut config = std::fs::read_to_string(&config_path).unwrap();
    config.push_str("\n[namespaces.squad]\ndescription = \"Owning squad\"\nunique = true\n");
    std::fs::write(&config_path, config).unwrap();

    // Precondition: the freshly hand-edited registry has NOT yet reached
    // rules.toml.
    let rules_path = temp.path().join(".jit").join("rules.toml");
    assert!(
        !std::fs::read_to_string(&rules_path)
            .unwrap()
            .contains("namespace-unique-squad"),
        "precondition: rules.toml has not been synced yet"
    );

    // Any jit-driven config write triggers the sync, regardless of the key set.
    let set = Command::new(jit_binary())
        .args(["config", "set", "project.name", "demo-project"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        set.status.success(),
        "config set failed: {}",
        String::from_utf8_lossy(&set.stderr)
    );

    let shown = Command::new(jit_binary())
        .args(["item", "show", "@/rule/namespace-unique-squad", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        shown.status.success(),
        "@/rule/namespace-unique-squad must resolve after the write-through: {}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let shown_json: Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(
        shown_json["item"]["qualified_id"].as_str().unwrap(),
        "@/rule/namespace-unique-squad"
    );
}
