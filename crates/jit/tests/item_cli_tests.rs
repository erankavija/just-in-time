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
    let temp = TempDir::new().unwrap();
    let output = Command::new(jit_binary())
        .arg("init")
        .current_dir(temp.path())
        .output()
        .expect("Failed to run jit init");
    assert!(output.status.success(), "jit init failed");
    temp
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

    let output = Command::new(jit_binary())
        .args(["item", "list", "--json"])
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
    assert!(qids.contains(&format!("{short}/REQ-01").as_str()));
    assert!(qids.contains(&format!("{short}/REQ-02").as_str()));
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

    // The built-in kind name matches.
    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "requirement", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 1);

    // `decision` is now a shipped built-in kind, so it is recognized; this issue
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

    let output = Command::new(jit_binary())
        .args(["item", "list", "--json"])
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
    let qualified = format!("{short}/REQ-01");

    for verb in ["show", "resolve"] {
        let output = Command::new(jit_binary())
            .args(["item", verb, &qualified, "--json"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "item {verb} failed");
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["item"]["self_id"].as_str().unwrap(), "REQ-01");
        assert_eq!(json["item"]["qualified_id"].as_str().unwrap(), qualified);
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
    // a built-in); the engine indexes it purely from its tuple, never from its
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
    let qualified = format!("{short}/REQ-01");

    let output = Command::new(jit_binary())
        .args(["issue", "show", &qualified, "--json"])
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
    assert_eq!(json["item"]["qualified_id"].as_str().unwrap(), qualified);

    // Human (non-JSON) path also renders the addressed item.
    let output = Command::new(jit_binary())
        .args(["issue", "show", &qualified])
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
/// sourced from `project-items.md` (preserving any config `jit init` wrote), and
/// optionally write that source file.
///
/// Uses the non-reserved name `glossary` (not `invariant`, which is reserved as a
/// registry-first kind) to exercise generic markdown-first project-scope sourcing.
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
    // REQ-01: `@/<self-id>` RESOLVES through the actual `jit item show` binary,
    // sourced from a config-declared repository-local file (no test seam).
    let temp = setup_test_repo();
    configure_project_scope_kind(
        temp.path(),
        Some("## Success Criteria\n\n- GLOSS-01: all writes are atomic\n"),
    );

    for verb in ["show", "resolve"] {
        let output = Command::new(jit_binary())
            .args(["item", verb, "@/GLOSS-01", "--json"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "@/GLOSS-01 must resolve via item {verb}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["item"]["self_id"].as_str().unwrap(), "GLOSS-01");
        assert_eq!(json["item"]["qualified_id"].as_str().unwrap(), "@/GLOSS-01");
        assert_eq!(json["item"]["scope"].as_str().unwrap(), "@");
        assert_eq!(json["item"]["kind"].as_str().unwrap(), "glossary");
        assert!(json["item"]["text"].as_str().unwrap().contains("atomic"));
    }

    // The same `@` id resolves through `jit issue show <qualified>` too.
    let output = Command::new(jit_binary())
        .args(["issue", "show", "@/GLOSS-01", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "issue show @/GLOSS-01 must resolve"
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["item"]["qualified_id"].as_str().unwrap(), "@/GLOSS-01");

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
        qids.contains(&"@/GLOSS-01"),
        "list must include @/GLOSS-01: {qids:?}"
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
        .args(["item", "show", "@/GLOSS-01", "--json"])
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
    // `.jit/invariants.toml` as `@/<self-id>`, with NO `[item_kinds]` config (the
    // built-in registry-first invariant kind), and NO markdown source involved.
    let temp = setup_test_repo();
    std::fs::write(
        temp.path().join(".jit").join("invariants.toml"),
        "[[invariants]]\n\
         id = \"INV-01\"\n\
         statement = \"Every dependency edge stays acyclic.\"\n\
         kind = \"enforced\"\n\
         enforced-by = \"dag-no-cycles\"\n\n\
         [[invariants]]\n\
         id = \"INV-02\"\n\
         statement = \"All state changes are logged.\"\n\
         kind = \"advisory\"\n",
    )
    .unwrap();
    // An issue whose description carries an INV-looking line must NOT leak in as an
    // invariant (registry is authoritative; no markdown index for invariants).
    create_issue(
        temp.path(),
        "Decoy",
        "## Success Criteria\n\n- [hard] INV-99: looks like an invariant\n",
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
        qids.contains(&"@/INV-01"),
        "list must include @/INV-01: {qids:?}"
    );
    assert!(
        qids.contains(&"@/INV-02"),
        "list must include @/INV-02: {qids:?}"
    );
    // The decoy INV-99 from an issue description is NOT an invariant (REQ-02).
    assert!(
        !qids.iter().any(|q| q.contains("INV-99")),
        "no markdown index for invariants: {qids:?}"
    );
    let inv01 = items.iter().find(|i| i["self_id"] == "INV-01").unwrap();
    assert_eq!(inv01["kind"].as_str().unwrap(), "invariant");
    assert_eq!(inv01["scope"].as_str().unwrap(), "@");
    assert_eq!(
        inv01["text"].as_str().unwrap(),
        "Every dependency edge stays acyclic."
    );

    // `jit item show @/INV-02` resolves through the shipped binary too.
    let output = Command::new(jit_binary())
        .args(["item", "show", "@/INV-02", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "item show @/INV-02 must resolve");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["item"]["qualified_id"].as_str().unwrap(), "@/INV-02");
    assert_eq!(json["item"]["kind"].as_str().unwrap(), "invariant");
}

#[test]
fn test_markdown_first_invariant_config_rejected_through_real_cli() {
    // Finding 1 (rework): the SHIPPED CLI rejects a markdown-first
    // `[item_kinds.invariant]` declaration — invariant is reserved as registry-first.
    let temp = setup_test_repo();
    let config_path = temp.path().join(".jit").join("config.toml");
    let mut config = std::fs::read_to_string(&config_path).unwrap_or_default();
    config.push_str(
        "\n[item_kinds.invariant]\n\
         section = \"success_criteria\"\n\
         id-pattern = \"INV-[0-9]+\"\n\
         markers = []\n\
         link-namespaces = [\"enforces\"]\n\
         scope = \"project\"\n\
         source = \"project-items.md\"\n\
         source-of-truth = \"markdown-first\"\n",
    );
    std::fs::write(&config_path, config).unwrap();

    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "invariant", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "markdown-first invariant must be rejected"
    );
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("--json failure must emit valid JSON on stdout");
    let msg = json["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("project-scoped") && msg.contains("registry-first"),
        "error must name both reserved requirements, got: {json}"
    );
}

#[test]
fn test_registry_first_issue_scoped_invariant_rejected_through_real_cli() {
    // REQ-02 (final hole): the SHIPPED CLI rejects an `[item_kinds.invariant]`
    // declared registry-first BUT issue-scoped — invariant is reserved as project +
    // registry-first, so it can never be parsed from issue descriptions.
    let temp = setup_test_repo();
    let config_path = temp.path().join(".jit").join("config.toml");
    let mut config = std::fs::read_to_string(&config_path).unwrap_or_default();
    config.push_str(
        "\n[item_kinds.invariant]\n\
         section = \"success_criteria\"\n\
         id-pattern = \"INV-[0-9]+\"\n\
         markers = []\n\
         link-namespaces = [\"enforces\"]\n\
         scope = \"issue\"\n\
         source-of-truth = \"registry-first\"\n",
    );
    std::fs::write(&config_path, config).unwrap();

    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "invariant", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "registry-first issue-scoped invariant must be rejected"
    );
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("--json failure must emit valid JSON on stdout");
    let msg = json["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("project-scoped") && msg.contains("registry-first"),
        "error must name both reserved requirements, got: {json}"
    );
}
