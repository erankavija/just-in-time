//! TDD tests for REQ-01 (positional title) and REQ-02 (--type flag).
//!
//! REQ-01: `jit issue create "Title"` works with the title taken positionally;
//!         `--title` / `-t` remain accepted.
//! REQ-02: `issue create` and `issue update` accept `--type <kind>`, write a
//!         `type:<kind>` label, and reject a kind not declared in config.

use jit::test_cli::JitCli;
use jit::test_taxonomy::type_label;

fn first_issue_id(repo: &JitCli) -> String {
    repo.run_json(["issue", "list"]).unwrap()["issues"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

// ---------------------------------------------------------------------------
// REQ-01: positional title
// ---------------------------------------------------------------------------

/// The canonical new form: `jit issue create "Title"` (no --title flag).
#[test]
fn test_create_positional_title_succeeds() {
    let taxonomy_repo = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let repo = JitCli::with_repository(taxonomy_repo.temp);
    let out = repo
        .command(["issue", "create", "Positional Title Here"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "positional title should succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The created issue title should match.
    let id = first_issue_id(&repo);
    let issue = repo.run_json(["issue", "show", &id]).unwrap();
    assert_eq!(issue["title"].as_str().unwrap(), "Positional Title Here");
}

/// Legacy `--title` flag must still work.
#[test]
fn test_create_flag_title_still_works() {
    let taxonomy_repo = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let repo = JitCli::with_repository(taxonomy_repo.temp);
    let out = repo
        .command(["issue", "create", "--title", "Flag Title"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "--title flag should still work; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = first_issue_id(&repo);
    let issue = repo.run_json(["issue", "show", &id]).unwrap();
    assert_eq!(issue["title"].as_str().unwrap(), "Flag Title");
}

/// Short `-t` alias must still work.
#[test]
fn test_create_short_t_title_still_works() {
    let taxonomy_repo = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let repo = JitCli::with_repository(taxonomy_repo.temp);
    let out = repo
        .command(["issue", "create", "-t", "Short Flag Title"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "-t flag should still work; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = first_issue_id(&repo);
    let issue = repo.run_json(["issue", "show", &id]).unwrap();
    assert_eq!(issue["title"].as_str().unwrap(), "Short Flag Title");
}

// ---------------------------------------------------------------------------
// REQ-02: --type on create
// ---------------------------------------------------------------------------

/// `--type <declared>` writes the matching `type:<declared>` label.
#[test]
fn test_create_type_flag_writes_label() {
    let taxonomy_repo = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let repo = JitCli::with_repository(taxonomy_repo.temp);
    let leaf = taxonomy_repo.taxonomy.type_at_level(4).to_string();
    let out = repo
        .command(["issue", "create", "Type Label Test", "--type", &leaf])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "--type {leaf} should succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = first_issue_id(&repo);
    let issue = repo.run_json(["issue", "show", &id]).unwrap();
    let labels: Vec<&str> = issue["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        labels.contains(&type_label(&taxonomy_repo.taxonomy, 4).as_str()),
        "labels should contain type:{leaf}, got: {labels:?}"
    );
}

/// An undeclared type kind is rejected through the existing validation layer
/// (exits non-zero).
#[test]
fn test_create_unknown_type_is_rejected() {
    let taxonomy_repo = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let repo = JitCli::with_repository(taxonomy_repo.temp);
    let out = repo
        .command([
            "issue",
            "create",
            "Type Test Issue",
            "--type",
            "xyzzy-unknown-type",
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "unknown type should be rejected; stdout: {}, stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

// ---------------------------------------------------------------------------
// REQ-02: --type on update
// ---------------------------------------------------------------------------

/// `issue update <id> --type <declared>` writes the matching label.
#[test]
fn test_update_type_flag_writes_label() {
    let taxonomy_repo = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let repo = JitCli::with_repository(taxonomy_repo.temp);
    // Create without explicit type (gets default).
    let out = repo
        .command(["issue", "create", "Update Type Test"])
        .output()
        .unwrap();
    assert!(out.status.success());

    let id = first_issue_id(&repo);
    let container = taxonomy_repo.taxonomy.type_at_level(3).to_string();

    let out = repo
        .command(["issue", "update", &id, "--type", &container])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "update --type {container} should succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let issue = repo.run_json(["issue", "show", &id]).unwrap();
    let labels: Vec<&str> = issue["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        labels.contains(&type_label(&taxonomy_repo.taxonomy, 3).as_str()),
        "labels should contain type:{container} after update, got: {labels:?}"
    );
}

/// `--type` REPLACES any prior `type:*` label rather than accumulating a second
/// one: the command layer derives a single canonical `type:<kind>` label. A
/// create gets the configured default type, and one update must leave exactly
/// one `type:` label.
#[test]
fn test_update_type_flag_replaces_existing_type_label() {
    let taxonomy_repo = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let repo = JitCli::with_repository(taxonomy_repo.temp);
    // Create without explicit type (gets the configured default type label).
    let out = repo
        .command(["issue", "create", "Replace Type Test"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let id = first_issue_id(&repo);
    let container = taxonomy_repo.taxonomy.type_at_level(3).to_string();

    let out = repo
        .command(["issue", "update", &id, "--type", &container])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "update --type {container} should succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let issue = repo.run_json(["issue", "show", &id]).unwrap();
    let type_labels: Vec<&str> = issue["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .filter(|l| l.starts_with("type:"))
        .collect();
    assert_eq!(
        type_labels,
        vec![type_label(&taxonomy_repo.taxonomy, 3)],
        "update --type must leave exactly one type label, got: {type_labels:?}"
    );
}

/// `--type` on create overrides the configured default type rather than adding a
/// second `type:*` label.
#[test]
fn test_create_type_flag_overrides_default_type() {
    let taxonomy_repo = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let repo = JitCli::with_repository(taxonomy_repo.temp);
    let strategic = taxonomy_repo.taxonomy.type_at_level(2).to_string();
    assert_ne!(strategic, taxonomy_repo.taxonomy.default_type);
    let out = repo
        .command([
            "issue",
            "create",
            "Override Default Type",
            "--type",
            &strategic,
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "--type {strategic} should succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = first_issue_id(&repo);
    let issue = repo.run_json(["issue", "show", &id]).unwrap();
    let type_labels: Vec<&str> = issue["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .filter(|l| l.starts_with("type:"))
        .collect();
    assert_eq!(
        type_labels,
        vec![type_label(&taxonomy_repo.taxonomy, 2)],
        "create --type must produce exactly one type label, got: {type_labels:?}"
    );
}

/// `issue update <id> --type <undeclared>` is rejected.
#[test]
fn test_update_unknown_type_is_rejected() {
    let taxonomy_repo = jit::test_taxonomy::setup_test_repo_with_taxonomy();
    let repo = JitCli::with_repository(taxonomy_repo.temp);
    let out = repo
        .command(["issue", "create", "Update Unknown Type Test"])
        .output()
        .unwrap();
    assert!(out.status.success());

    let id = first_issue_id(&repo);

    let out = repo
        .command(["issue", "update", &id, "--type", "xyzzy-unknown-type"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "update with unknown type should be rejected; stdout: {}, stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
