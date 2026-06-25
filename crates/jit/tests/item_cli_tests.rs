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

    // An unknown kind yields an empty result (not an error).
    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "decision", "--json"])
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
    // Declare a domain-agnostic custom kind in config; the engine indexes it
    // purely from its four-tuple, never from its name (REQ-01).
    let config_path = temp.path().join(".jit").join("config.toml");
    let mut config = std::fs::read_to_string(&config_path).unwrap_or_default();
    config.push_str(
        "\n[item_kinds.decision]\nsection = \"decisions\"\nid-pattern = \"D-\\\\d+\"\nlink-namespaces = [\"per\"]\n",
    );
    std::fs::write(&config_path, config).unwrap();

    create_issue(
        temp.path(),
        "With decisions",
        "## Decisions\n\n- D-1: use json storage\n- D-2: atomic writes\n",
    );

    let output = Command::new(jit_binary())
        .args(["item", "list", "--kind", "decision", "--json"])
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
    assert_eq!(json["items"][0]["kind"].as_str().unwrap(), "decision");
    assert_eq!(json["items"][0]["self_id"].as_str().unwrap(), "D-1");
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

/// Append to `.jit/config.toml` a project-scope `invariant` kind sourced from
/// `project-items.md` (preserving any config `jit init` wrote), and optionally
/// write that source file.
fn configure_project_scope_kind(repo: &std::path::Path, source_md: Option<&str>) {
    let config_path = repo.join(".jit").join("config.toml");
    let mut config = std::fs::read_to_string(&config_path).unwrap_or_default();
    config.push_str(
        "\n[item_kinds.invariant]\n\
         scope = \"project\"\n\
         source = \"project-items.md\"\n\
         id-pattern = \"INV-[0-9]+\"\n",
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
        Some("## Success Criteria\n\n- INV-01: all writes are atomic\n"),
    );

    for verb in ["show", "resolve"] {
        let output = Command::new(jit_binary())
            .args(["item", verb, "@/INV-01", "--json"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "@/INV-01 must resolve via item {verb}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["item"]["self_id"].as_str().unwrap(), "INV-01");
        assert_eq!(json["item"]["qualified_id"].as_str().unwrap(), "@/INV-01");
        assert_eq!(json["item"]["scope"].as_str().unwrap(), "@");
        assert_eq!(json["item"]["kind"].as_str().unwrap(), "invariant");
        assert!(json["item"]["text"].as_str().unwrap().contains("atomic"));
    }

    // The same `@` id resolves through `jit issue show <qualified>` too.
    let output = Command::new(jit_binary())
        .args(["issue", "show", "@/INV-01", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "issue show @/INV-01 must resolve");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["item"]["qualified_id"].as_str().unwrap(), "@/INV-01");

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
        qids.contains(&"@/INV-01"),
        "list must include @/INV-01: {qids:?}"
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
        .args(["item", "show", "@/INV-01", "--json"])
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
