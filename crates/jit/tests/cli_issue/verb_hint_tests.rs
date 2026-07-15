//! Wrong-verb guess hints (jit:d0f88ee2).
//!
//! Session mining caught agents guessing spellings that don't exist in a
//! command group (`dep remove`, `issue rm`, `gate delete`, ...) and paying for
//! a failed command plus a retry. Policy: hints only, no new aliases — every
//! wrong guess still fails, but with a message naming the canonical spelling
//! instead of clap's generic "unrecognized subcommand" error. These tests
//! pin, for each wrong guess: exit code 2, a human-readable hint naming the
//! canonical command, the same hint in the `--json` error envelope, and that
//! the canonical spelling in the same group is completely unaffected.

use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_test_env() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .arg("init")
        .status()
        .unwrap();
    assert!(status.success());
    temp_dir
}

fn json_issue_id(output: &std::process::Output) -> String {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json["id"].as_str().expect("id should exist").to_string()
}

/// Assert a wrong-verb guess fails with exit 2, names `expect_in_hint` on
/// stderr, and (under `--json`) carries the same substring in the JSON error
/// envelope's message or suggestions.
fn assert_wrong_verb_hint(args: &[&str], expect_in_hint: &str) {
    let temp_dir = setup_test_env();

    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "args {:?} should exit 2, stderr: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expect_in_hint),
        "args {:?}: expected stderr to contain {:?}, got: {}",
        args,
        expect_in_hint,
        stderr
    );

    let mut json_args: Vec<&str> = args.to_vec();
    json_args.push("--json");
    let json_output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(&json_args)
        .output()
        .unwrap();
    assert_eq!(
        json_output.status.code(),
        Some(2),
        "args {:?} --json should exit 2",
        args
    );
    let json: serde_json::Value = serde_json::from_slice(&json_output.stdout)
        .unwrap_or_else(|e| panic!("args {:?} --json: stdout not JSON: {}", args, e));
    let message = json["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("args {:?} --json: no error.message", args));
    let suggestions: Vec<&str> = json["error"]["suggestions"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(
        message.contains(expect_in_hint) || suggestions.iter().any(|s| s.contains(expect_in_hint)),
        "args {:?} --json: expected hint {:?} in message or suggestions, got: {}",
        args,
        expect_in_hint,
        json_output_pretty(&json)
    );
}

fn json_output_pretty(json: &serde_json::Value) -> String {
    serde_json::to_string_pretty(json).unwrap_or_default()
}

// ============================================================================
// REQ-01: wrong removal-verb spellings per group
// ============================================================================

#[test]
fn test_dep_remove_hints_dep_rm() {
    assert_wrong_verb_hint(&["dep", "remove", "a", "b"], "jit dep rm");
}

#[test]
fn test_dep_delete_hints_dep_rm() {
    assert_wrong_verb_hint(&["dep", "delete", "a", "b"], "jit dep rm");
}

#[test]
fn test_issue_rm_hints_issue_delete() {
    assert_wrong_verb_hint(&["issue", "rm", "abc123"], "jit issue delete");
}

#[test]
fn test_issue_remove_hints_issue_delete() {
    assert_wrong_verb_hint(&["issue", "remove", "abc123"], "jit issue delete");
}

#[test]
fn test_gate_rm_hints_gate_remove() {
    assert_wrong_verb_hint(&["gate", "rm", "some-gate"], "jit gate remove");
}

#[test]
fn test_gate_delete_hints_gate_remove() {
    assert_wrong_verb_hint(&["gate", "delete", "some-gate"], "jit gate remove");
}

#[test]
fn test_doc_rm_hints_doc_remove() {
    assert_wrong_verb_hint(&["doc", "rm", "abc123", "path.md"], "jit doc remove");
}

#[test]
fn test_doc_delete_hints_doc_remove() {
    assert_wrong_verb_hint(&["doc", "delete", "abc123", "path.md"], "jit doc remove");
}

// ============================================================================
// REQ-02: `issue complete` / `issue edit`
// ============================================================================

#[test]
fn test_issue_complete_hints_update_with_state_flag() {
    assert_wrong_verb_hint(&["issue", "complete", "abc123"], "jit issue update");
    assert_wrong_verb_hint(&["issue", "complete", "abc123"], "--state done");
}

#[test]
fn test_issue_edit_hints_update() {
    assert_wrong_verb_hint(&["issue", "edit", "abc123"], "jit issue update");
}

// ============================================================================
// REQ-03: adding/removing a label to an issue via `jit label ...`
// ============================================================================

#[test]
fn test_label_add_hints_issue_update_label_flag() {
    assert_wrong_verb_hint(&["label", "add", "abc123", "area:foo"], "jit issue update");
    assert_wrong_verb_hint(&["label", "add", "abc123", "area:foo"], "--label");
}

#[test]
fn test_label_rm_hints_issue_update_remove_label_flag() {
    assert_wrong_verb_hint(&["label", "rm", "abc123", "area:foo"], "--remove-label");
}

#[test]
fn test_label_remove_hints_issue_update_remove_label_flag() {
    assert_wrong_verb_hint(&["label", "remove", "abc123", "area:foo"], "--remove-label");
}

#[test]
fn test_label_help_disambiguates_namespace_management_from_issue_labeling() {
    let output = Command::new(jit_binary())
        .args(["label", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(
        help.contains("jit issue update") && help.contains("--label"),
        "expected `jit label --help` to point at `jit issue update --label`, got: {}",
        help
    );
}

// ============================================================================
// Canonical spellings still work (no behavior change)
// ============================================================================

#[test]
fn test_dep_rm_canonical_still_works() {
    let temp_dir = setup_test_env();
    let a = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "A", "--json"])
            .output()
            .unwrap(),
    );
    let b = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "B", "--json"])
            .output()
            .unwrap(),
    );
    assert!(Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", &a, &b])
        .status()
        .unwrap()
        .success());
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "rm", &a, &b])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "canonical `dep rm` should still work: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_issue_delete_canonical_still_works() {
    let temp_dir = setup_test_env();
    let id = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Delete me", "--json"])
            .output()
            .unwrap(),
    );
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .env("JIT_ALLOW_DELETION", "1")
        .args(["issue", "delete", &id])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "canonical `issue delete` should still work: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_issue_update_canonical_still_works() {
    let temp_dir = setup_test_env();
    let id = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Update me", "--json"])
            .output()
            .unwrap(),
    );
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "update", &id, "--title", "Updated"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "canonical `issue update` should still work: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_gate_remove_canonical_still_works() {
    let temp_dir = setup_test_env();
    assert!(Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args([
            "gate",
            "define",
            "some-gate",
            "--title",
            "Some gate",
            "--description",
            "desc",
        ])
        .status()
        .unwrap()
        .success());
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["gate", "remove", "some-gate"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "canonical `gate remove` should still work: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_doc_remove_canonical_still_works() {
    let temp_dir = setup_test_env();
    let id = json_issue_id(
        &Command::new(jit_binary())
            .current_dir(&temp_dir)
            .args(["issue", "create", "--title", "Has doc", "--json"])
            .output()
            .unwrap(),
    );
    std::fs::write(temp_dir.path().join("notes.md"), "notes").unwrap();
    assert!(Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["doc", "add", &id, "notes.md"])
        .status()
        .unwrap()
        .success());
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["doc", "remove", &id, "notes.md"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "canonical `doc remove` should still work: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_label_namespaces_and_values_canonical_still_work() {
    let temp_dir = setup_test_env();
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["label", "namespaces"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "canonical `label namespaces` should still work: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
