//! Regression coverage for this repository's issue-scoped documentation review policy.

use std::fs;
use std::path::Path;

fn repo_file(path: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join(path)).unwrap_or_else(|error| panic!("read {path}: {error}"))
}

#[test]
fn test_doc_review_prompt_defines_issue_scoped_impact_policy() {
    let prompt = repo_file("scripts/doc-review-prompt.md");

    for required in [
        "jit:<short-id>",
        "individual commit patches",
        "renames and deletions",
        "Uncommitted changes",
        "commit-based attribution was unavailable",
        "smallest documentation impact cone",
        "documentation that must change",
        "pre-existing",
        "advisory",
        "direct present-tense statements",
        "canonical reference",
        "implementation history",
        "if and only if",
        "unresolved issue-impact blocking",
    ] {
        assert!(
            prompt.contains(required),
            "missing policy phrase: {required}"
        );
    }
}

#[test]
fn test_doc_review_gate_description_matches_issue_scoped_policy() {
    let gates = repo_file(".jit/gates.toml");
    let parsed: toml::Value = toml::from_str(&gates).expect("valid gates registry");
    let doc_review = parsed["gates"]
        .as_array()
        .expect("gates array")
        .iter()
        .find(|gate| gate["key"].as_str() == Some("doc-review"))
        .expect("doc-review gate");
    let description = doc_review["description"].as_str().expect("description");

    for required in [
        "issue-scoped",
        "jit:<short-id>",
        "impact cone",
        "pre-existing drift",
        "advisory",
        "concise",
        "fail",
    ] {
        assert!(
            description.contains(required),
            "doc-review description missing policy phrase: {required}"
        );
    }
}

#[test]
fn test_ai_review_wrapper_allows_passing_advisory_findings() {
    let wrapper = repo_file("scripts/ai-review.sh");
    assert!(wrapper.contains("passing verdict may include advisory findings"));
    assert!(!wrapper.contains("Use \"pass\" with an empty findings array when there are none."));
}
