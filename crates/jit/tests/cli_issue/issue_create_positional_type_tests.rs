//! TDD tests for REQ-01 (positional title) and REQ-02 (--type flag).
//!
//! REQ-01: `jit issue create "Title"` works with the title taken positionally;
//!         `--title` / `-t` remain accepted.
//! REQ-02: `issue create` and `issue update` accept `--type <kind>`, write a
//!         `type:<kind>` label, and reject a kind not declared in config.

use crate::{setup_test_repo_with_taxonomy, TaxonomyRepo};
use std::process::Command;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

/// A repository whose configuration declares the type vocabulary these cases
/// name, so a rejected type is one the repository never declared rather than
/// one no repository could have.
fn setup_repo() -> TaxonomyRepo {
    setup_test_repo_with_taxonomy()
}

/// Run `jit issue create` with the given args; return stdout + status.
fn create(repo: &TaxonomyRepo, args: &[&str]) -> std::process::Output {
    Command::new(jit_binary())
        .arg("issue")
        .arg("create")
        .args(args)
        .current_dir(repo.path())
        .output()
        .expect("failed to spawn jit")
}

/// Parse the first issue ID out of `jit issue list` stdout.
fn first_issue_id(repo: &TaxonomyRepo) -> String {
    let out = Command::new(jit_binary())
        .args(["issue", "list", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    v["issues"][0]["id"].as_str().unwrap().to_string()
}

/// Load a single issue as JSON via `jit issue show <id> --json`.
fn show(repo: &TaxonomyRepo, id: &str) -> serde_json::Value {
    let out = Command::new(jit_binary())
        .args(["issue", "show", id, "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

// ---------------------------------------------------------------------------
// REQ-01: positional title
// ---------------------------------------------------------------------------

/// The canonical new form: `jit issue create "Title"` (no --title flag).
#[test]
fn test_create_positional_title_succeeds() {
    let repo = setup_repo();
    let out = create(&repo, &["Positional Title Here"]);
    assert!(
        out.status.success(),
        "positional title should succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The created issue title should match.
    let id = first_issue_id(&repo);
    let issue = show(&repo, &id);
    assert_eq!(issue["title"].as_str().unwrap(), "Positional Title Here");
}

/// Legacy `--title` flag must still work.
#[test]
fn test_create_flag_title_still_works() {
    let repo = setup_repo();
    let out = create(&repo, &["--title", "Flag Title"]);
    assert!(
        out.status.success(),
        "--title flag should still work; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = first_issue_id(&repo);
    let issue = show(&repo, &id);
    assert_eq!(issue["title"].as_str().unwrap(), "Flag Title");
}

/// Short `-t` alias must still work.
#[test]
fn test_create_short_t_title_still_works() {
    let repo = setup_repo();
    let out = create(&repo, &["-t", "Short Flag Title"]);
    assert!(
        out.status.success(),
        "-t flag should still work; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = first_issue_id(&repo);
    let issue = show(&repo, &id);
    assert_eq!(issue["title"].as_str().unwrap(), "Short Flag Title");
}

// ---------------------------------------------------------------------------
// REQ-02: --type on create
// ---------------------------------------------------------------------------

/// `--type <declared>` writes the matching `type:<declared>` label.
#[test]
fn test_create_type_flag_writes_label() {
    let repo = setup_repo();
    let leaf = repo.taxonomy.type_at_level(4).to_string();
    let out = create(&repo, &["Type Label Test", "--type", &leaf]);
    assert!(
        out.status.success(),
        "--type {leaf} should succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = first_issue_id(&repo);
    let issue = show(&repo, &id);
    let labels: Vec<&str> = issue["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        labels.contains(&format!("type:{leaf}").as_str()),
        "labels should contain type:{leaf}, got: {labels:?}"
    );
}

/// An undeclared type kind is rejected through the existing validation layer
/// (exits non-zero).
#[test]
fn test_create_unknown_type_is_rejected() {
    let repo = setup_repo();
    let out = create(&repo, &["Type Test Issue", "--type", "xyzzy-unknown-type"]);
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
    let repo = setup_repo();
    // Create without explicit type (gets default).
    let out = create(&repo, &["Update Type Test"]);
    assert!(out.status.success());

    let id = first_issue_id(&repo);
    let container = repo.taxonomy.type_at_level(3).to_string();

    let out = Command::new(jit_binary())
        .args(["issue", "update", &id, "--type", &container])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "update --type {container} should succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let issue = show(&repo, &id);
    let labels: Vec<&str> = issue["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        labels.contains(&format!("type:{container}").as_str()),
        "labels should contain type:{container} after update, got: {labels:?}"
    );
}

/// `--type` REPLACES any prior `type:*` label rather than accumulating a second
/// one: the command layer derives a single canonical `type:<kind>` label. A
/// create gets the configured default type, and one update must leave exactly
/// one `type:` label.
#[test]
fn test_update_type_flag_replaces_existing_type_label() {
    let repo = setup_repo();
    // Create without explicit type (gets the configured default type label).
    let out = create(&repo, &["Replace Type Test"]);
    assert!(out.status.success());
    let id = first_issue_id(&repo);
    let container = repo.taxonomy.type_at_level(3).to_string();

    let out = Command::new(jit_binary())
        .args(["issue", "update", &id, "--type", &container])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "update --type {container} should succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let issue = show(&repo, &id);
    let type_labels: Vec<&str> = issue["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .filter(|l| l.starts_with("type:"))
        .collect();
    assert_eq!(
        type_labels,
        vec![format!("type:{container}")],
        "update --type must leave exactly one type label, got: {type_labels:?}"
    );
}

/// `--type` on create overrides the configured default type rather than adding a
/// second `type:*` label.
#[test]
fn test_create_type_flag_overrides_default_type() {
    let repo = setup_repo();
    let strategic = repo.taxonomy.type_at_level(2).to_string();
    assert_ne!(strategic, repo.taxonomy.default_type);
    let out = create(&repo, &["Override Default Type", "--type", &strategic]);
    assert!(
        out.status.success(),
        "--type {strategic} should succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = first_issue_id(&repo);
    let issue = show(&repo, &id);
    let type_labels: Vec<&str> = issue["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .filter(|l| l.starts_with("type:"))
        .collect();
    assert_eq!(
        type_labels,
        vec![format!("type:{strategic}")],
        "create --type must produce exactly one type label, got: {type_labels:?}"
    );
}

/// `issue update <id> --type <undeclared>` is rejected.
#[test]
fn test_update_unknown_type_is_rejected() {
    let repo = setup_repo();
    let out = create(&repo, &["Update Unknown Type Test"]);
    assert!(out.status.success());

    let id = first_issue_id(&repo);

    let out = Command::new(jit_binary())
        .args(["issue", "update", &id, "--type", "xyzzy-unknown-type"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "update with unknown type should be rejected; stdout: {}, stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
