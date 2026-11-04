//! Wrong-verb guess hints (jit:d0f88ee2).
//!
//! Session mining caught agents guessing spellings that don't exist in a
//! command group (`dep remove`, `issue rm`, `gate delete`, ...) and paying for
//! a failed command plus a retry. Policy: hints only, no new aliases — every
//! wrong guess still fails, but with a message naming the canonical spelling
//! instead of clap's generic "unrecognized subcommand" error. These tests
//! pin, for each wrong guess: failure, the hint naming the canonical command
//! and clap's diagnostic on stderr with empty stdout (parse failures stay
//! outside the `--json` envelope), and that the canonical spelling in the same
//! group is completely unaffected.

use jit::test_cli::JitCli;

/// Assert a wrong-verb guess is a parse failure: the hint and clap's
/// diagnostic on stderr, empty stdout, with and without `--json` and inside
/// or outside a repository.
fn assert_wrong_verb_hint(args: &[&str], expect_in_hint: &str) {
    let repo = JitCli::initialised().unwrap();
    let outside = JitCli::uninitialised().unwrap();
    let invocations = std::iter::once(args.to_vec()).chain([0, 1, args.len()].map(|position| {
        let mut argv = args.to_vec();
        argv.insert(position, "--json");
        argv
    }));

    for argv in invocations {
        for cli in [&repo, &outside] {
            let output = cli.command(&argv).output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(2), "{argv:?}: {stderr}");
            assert!(output.stdout.is_empty(), "{argv:?}: stdout must be empty");
            assert!(
                stderr.contains("Hint:") && stderr.contains(expect_in_hint),
                "{argv:?}: expected hint {expect_in_hint:?}, got: {stderr}"
            );
            assert!(
                stderr.contains("unrecognized subcommand"),
                "{argv:?}: expected clap diagnostic, got: {stderr}"
            );
        }
    }
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
    let cli = JitCli::uninitialised().unwrap();
    let output = cli.command(["label", "--help"]).output().unwrap();
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
    let cli = JitCli::initialised().unwrap();
    let a = cli.create_issue(["--title", "A"]).unwrap();
    let b = cli.create_issue(["--title", "B"]).unwrap();
    assert!(cli
        .command(["dep", "add", &a, &b])
        .status()
        .unwrap()
        .success());
    let output = cli.command(["dep", "rm", &a, &b]).output().unwrap();
    assert!(
        output.status.success(),
        "canonical `dep rm` should still work: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_issue_delete_canonical_still_works() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Delete me"]).unwrap();
    let output = cli
        .command_with_env(["issue", "delete", &id], [("JIT_ALLOW_DELETION", "1")])
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
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Update me"]).unwrap();
    let output = cli
        .command(["issue", "update", &id, "--title", "Updated"])
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
    let cli = JitCli::initialised().unwrap();
    assert!(cli
        .command([
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
    let output = cli
        .command(["gate", "remove", "some-gate"])
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
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Has doc"]).unwrap();
    std::fs::write(cli.root().join("notes.md"), "notes").unwrap();
    assert!(cli
        .command(["doc", "add", &id, "notes.md"])
        .status()
        .unwrap()
        .success());
    let output = cli
        .command(["doc", "remove", &id, "notes.md"])
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
    let cli = JitCli::initialised().unwrap();
    let output = cli.command(["label", "namespaces"]).output().unwrap();
    assert!(
        output.status.success(),
        "canonical `label namespaces` should still work: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
