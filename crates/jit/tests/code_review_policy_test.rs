//! Regression coverage for this repository's issue-scoped code-review policy.

use std::fs;
use std::path::Path;

fn repo_file(path: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join(path)).unwrap_or_else(|error| panic!("read {path}: {error}"))
}

#[test]
fn test_code_review_prompt_defines_attribution_and_fallback_policy() {
    let prompt = repo_file("scripts/code-review-prompt.md");

    for required in [
        "jit:<short-id>",
        "individual patch",
        "renames and deletions",
        "commit attribution was unavailable",
        "Do not attribute uncommitted changes",
        "issue intent and linked documents",
        "current tree",
    ] {
        assert!(
            prompt.contains(required),
            "missing policy phrase: {required}"
        );
    }
}

#[test]
fn test_code_review_prompt_defines_current_evidence_and_debt_policy() {
    let prompt = repo_file("scripts/code-review-prompt.md");

    for required in [
        "latest recorded status and exit code",
        "newer successful run supersedes older failures",
        "in-flight verdict supersedes its prior projection",
        "do not block solely because",
        "issue-impact",
        "blocking",
        "pre-existing",
        "advisory",
        "explicit evidence",
        "if and only if",
    ] {
        assert!(
            prompt.contains(required),
            "missing policy phrase: {required}"
        );
    }
}

#[test]
fn test_code_review_prompt_requires_read_only_bounded_inspection() {
    let prompt = repo_file("scripts/code-review-prompt.md");

    for required in [
        "read-only",
        "Do not invoke issue-lifecycle skills",
        "diff statistics",
        "changed-file lists",
        "bounded calls",
        "truncation marker",
        "Do not issue a verdict",
        "disposition",
        "origin",
    ] {
        assert!(
            prompt.contains(required),
            "missing policy phrase: {required}"
        );
    }
}

#[test]
fn test_code_review_prompt_does_not_duplicate_common_findings_contract() {
    let prompt = repo_file("scripts/code-review-prompt.md");

    assert!(!prompt.contains("<<<JIT-FINDINGS-JSON"));
    assert!(!prompt.contains("Before the verdict line"));
}

#[test]
fn test_code_review_gate_uses_explicit_read_only_sandbox() {
    let gates = repo_file(".jit/gates.toml");
    let parsed: toml::Value = toml::from_str(&gates).expect("valid gates registry");
    let gate = parsed["gates"]
        .as_array()
        .expect("gates array")
        .iter()
        .find(|gate| gate["key"].as_str() == Some("code-review"))
        .expect("code-review gate");
    let reviewer = gate["checker"]["env"]["REVIEWER_AGENT"]
        .as_str()
        .expect("reviewer command");

    assert!(reviewer.contains("--sandbox read-only"));
}
