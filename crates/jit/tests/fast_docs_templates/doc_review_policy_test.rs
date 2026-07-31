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
        "repository-configured hierarchy",
        "DAG-resolved descendants",
        "combined documentation contract",
    ] {
        assert!(
            description.contains(required),
            "doc-review description missing policy phrase: {required}"
        );
    }
}

#[test]
fn test_doc_review_prompt_distinguishes_leaf_footprint_from_container_footprint() {
    let prompt = repo_file("scripts/doc-review-prompt.md");
    let leaf = prompt
        .split("### Leaf review")
        .nth(1)
        .expect("leaf review section")
        .split("### Container review")
        .next()
        .expect("bounded leaf section");
    let container = prompt
        .split("### Container review")
        .nth(1)
        .expect("container review section")
        .split("## Derive the smallest documentation impact cone")
        .next()
        .expect("bounded container section");

    assert!(leaf.contains("only commits tagged for the context leaf"));
    assert!(leaf.contains("Do not include descendant"));
    assert!(container.contains("container plus every delivered descendant"));
    assert!(container.contains("DAG-authoritative resolved children"));
    assert!(container.contains("unrelated sequencing dependencies"));
}

#[test]
fn test_doc_review_prompt_defines_holistic_container_checks() {
    let prompt = repo_file("scripts/doc-review-prompt.md");

    for required in [
        "combined current documentation contract",
        "Do not replay leaf reviews",
        "container-level workflow coverage",
        "cross-child terminology and example consistency",
        "canonical placement versus duplication",
        "discoverability",
        "aggregate concision",
        "union of descendant impact cones",
    ] {
        assert!(
            prompt.contains(required),
            "missing container policy phrase: {required}"
        );
    }
}

#[test]
fn test_ai_review_wrapper_allows_passing_advisory_findings() {
    let wrapper = repo_file("contrib/gates/ai-review.sh");
    assert!(wrapper.contains("passing verdict may include advisory findings"));
    assert!(!wrapper.contains("Use \"pass\" with an empty findings array when there are none."));
}
