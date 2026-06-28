//! TDD tests for REQ-01 (positional title) and REQ-02 (--type flag).
//!
//! REQ-01: `jit issue create "Title"` works with the title taken positionally;
//!         `--title` / `-t` remain accepted.
//! REQ-02: `issue create` and `issue update` accept `--type <kind>`, write a
//!         `type:<kind>` label, and reject a kind not declared in config.

use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let out = Command::new(jit_binary())
        .arg("init")
        .current_dir(tmp.path())
        .output()
        .expect("failed to spawn jit init");
    assert!(out.status.success(), "jit init failed");
    tmp
}

/// Run `jit issue create` with the given args; return stdout + status.
fn create(repo: &TempDir, args: &[&str]) -> std::process::Output {
    Command::new(jit_binary())
        .arg("issue")
        .arg("create")
        .args(args)
        .current_dir(repo.path())
        .output()
        .expect("failed to spawn jit")
}

/// Parse the first issue ID out of `jit issue list` stdout.
fn first_issue_id(repo: &TempDir) -> String {
    let out = Command::new(jit_binary())
        .args(["issue", "list", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    v["issues"][0]["id"].as_str().unwrap().to_string()
}

/// Load a single issue as JSON via `jit issue show <id> --json`.
fn show(repo: &TempDir, id: &str) -> serde_json::Value {
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

/// `--type task` writes a `type:task` label (task is declared in the default
/// type_hierarchy).
#[test]
fn test_create_type_flag_writes_label() {
    let repo = setup_repo();
    let out = create(&repo, &["Type Label Test", "--type", "task"]);
    assert!(
        out.status.success(),
        "--type task should succeed; stderr: {}",
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
        labels.contains(&"type:task"),
        "labels should contain type:task, got: {:?}",
        labels
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

/// `issue update <id> --type story` writes a `type:story` label.
#[test]
fn test_update_type_flag_writes_label() {
    let repo = setup_repo();
    // Create without explicit type (gets default).
    let out = create(&repo, &["Update Type Test"]);
    assert!(out.status.success());

    let id = first_issue_id(&repo);

    // Update with --type story (declared in type_hierarchy).
    let out = Command::new(jit_binary())
        .args(["issue", "update", &id, "--type", "story"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "update --type story should succeed; stderr: {}",
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
        labels.contains(&"type:story"),
        "labels should contain type:story after update, got: {:?}",
        labels
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
