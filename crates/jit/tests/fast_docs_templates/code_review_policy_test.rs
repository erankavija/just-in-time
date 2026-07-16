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
fn test_code_review_prompt_rejects_blanket_public_api_examples() {
    let prompt = repo_file("scripts/code-review-prompt.md");
    let agents = repo_file("AGENTS.md");

    for required in [
        "Do not require a `# Examples` section merely because an API is public",
        "non-obvious-only standard",
        "repetitive examples for straightforward",
        "avoidable documentation and CI burden",
    ] {
        assert!(
            prompt.contains(required),
            "missing selective-example review policy: {required}"
        );
    }

    for required in [
        "Do not require an example for every public API",
        "tautological examples",
        "one type- or module-level walkthrough",
    ] {
        assert!(
            agents.contains(required),
            "missing selective-example repository policy: {required}"
        );
    }

    assert!(
        !prompt
            .contains("All public APIs must have doc comments with description and `# Examples`"),
        "blanket example requirement returned to the review prompt"
    );
}

#[test]
fn test_code_review_prompt_does_not_treat_pending_peer_review_as_a_defect() {
    let prompt = repo_file("scripts/code-review-prompt.md");

    for required in [
        "Include every required gate's latest status in the evidence header",
        "peer review or judgment gate",
        "is not by itself an implementation defect",
        "must not create a blocking finding or fail the verdict",
        "Executable CI or validation gate evidence may support a blocking finding",
        "demonstrates an attributable failure",
        "leaves a hard criterion materially unverified",
    ] {
        assert!(
            prompt.contains(required),
            "missing peer-review gate evidence policy: {required}"
        );
    }

    assert!(
        !prompt.contains("A currently pending, failed, or errored required"),
        "prompt retains the blanket rule that made unfinished peer review block code review"
    );
}

#[test]
fn test_code_review_prompt_discovers_applicable_policy_in_precedence_order() {
    let prompt = repo_file("scripts/code-review-prompt.md");

    for required in [
        "complete prose baseline",
        "repository root",
        "affected path",
        "closer `AGENTS.md`",
        "specializes",
        "unresolved contradiction",
        "blocking",
    ] {
        assert!(
            prompt.contains(required),
            "missing policy phrase: {required}"
        );
    }
}

#[test]
fn test_code_review_prompt_bounds_and_resolves_addressable_policy() {
    let prompt = repo_file("scripts/code-review-prompt.md");

    for required in [
        "applicable `AGENTS.md`",
        "issue content",
        "relationship labels",
        "linked documents",
        "attributable patches",
        "directly implicated behavior",
        "Do not enumerate every project item",
        "configured source of truth",
        "markdown-first",
        "registry-first",
        "rendered projection",
    ] {
        assert!(
            prompt.contains(required),
            "missing policy phrase: {required}"
        );
    }
}

#[test]
fn test_code_review_prompt_checks_relationship_evidence_claims() {
    let prompt = repo_file("scripts/code-review-prompt.md");

    for required in [
        "`satisfies:`",
        "`enforces:`",
        "`per:`",
        "evidence claims",
        "dangling",
        "contradictory",
        "unsupported",
        "unrelated pre-existing",
        "advisory",
    ] {
        assert!(
            prompt.contains(required),
            "missing policy phrase: {required}"
        );
    }
}

#[test]
fn test_code_review_prompt_requires_complete_evidence_header_and_empty_values() {
    let prompt = repo_file("scripts/code-review-prompt.md");

    for field in [
        "Attribution:",
        "Policy sources:",
        "Resolved items:",
        "Gate evidence:",
        "Truncation recovery:",
    ] {
        assert_eq!(
            prompt.matches(field).count(),
            1,
            "evidence field must occur exactly once: {field}"
        );
    }
    assert!(prompt.contains("use `none` for every empty value"));
}

#[test]
fn test_code_review_prompt_uses_recorded_gates_and_structured_references() {
    let prompt = repo_file("scripts/code-review-prompt.md");

    for required in [
        "latest recorded",
        "Do not rerun a gate that is currently recorded as passing",
        "@/gate/jit-validate",
        "valid resolved qualified IDs",
        "`references`",
        "empty array",
        "repository's configured registries",
        "do not invent, hardcode, or emit unresolved references",
    ] {
        assert!(
            prompt.contains(required),
            "missing policy phrase: {required}"
        );
    }
    assert!(
        !prompt.contains("@/inv/pid-safety"),
        "portable review policy must not require a repository-local invariant"
    );
}

#[test]
fn test_code_review_prompt_is_procedure_not_a_registry_or_engineering_rubric() {
    let prompt = repo_file("scripts/code-review-prompt.md");

    assert!(!prompt.contains("## Review rubric"));
    for copied_registry_statement in [
        "All file writes use the temp-file + atomic-rename pattern.",
        "Every state change appends an event to events.jsonl.",
        "Every label is namespace:value",
        "Process-signaling code rejects sentinel or lossy PID conversions",
    ] {
        assert!(
            !prompt.contains(copied_registry_statement),
            "prompt copied registry prose: {copied_registry_statement}"
        );
    }
}

#[test]
fn test_code_review_documentation_explains_policy_and_transport_ownership() {
    let custom_gates = repo_file("docs/how-to/custom-gates.md");
    let contributor = repo_file("dev/index.md");
    let custom_gates_normalized = custom_gates
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let contributor_normalized = contributor.split_whitespace().collect::<Vec<_>>().join(" ");

    for required in [
        "applicable `AGENTS.md`",
        "qualified IDs",
        "configured source of truth",
        "`references`",
        "repository-specific prompt",
        "tool-agnostic wrapper",
    ] {
        assert!(
            custom_gates_normalized.contains(required),
            "documentation is missing: {required}"
        );
    }

    for required in [
        "The reviewer emits human-readable findings, a structured findings block, and a terminal verdict.",
        "The wrapper transports the prompt and context and determines its checker exit code from the terminal verdict.",
        "jit parses and persists the structured findings block.",
    ] {
        assert!(
            custom_gates_normalized.contains(required),
            "documentation is missing exact findings ownership: {required}"
        );
    }

    for inaccurate in [
        "parses the shared findings and verdict contract",
        "parses the common verdict contract",
    ] {
        assert!(
            !custom_gates_normalized.contains(inaccurate),
            "documentation assigns JIT's structured-findings parsing to the wrapper: {inaccurate}"
        );
    }

    for required in [
        "applicable `AGENTS.md`",
        "canonical",
        "engineering prose",
        "Repository-specific review",
        "tool-agnostic",
        "`references`",
    ] {
        assert!(
            contributor.contains(required),
            "contributor guidance is missing: {required}"
        );
    }

    for required in [
        "The reviewer emits human-readable findings, a structured findings block, and a terminal verdict.",
        "The shared AI-review wrapper stays tool-agnostic: it transports the prompt and context and determines its checker exit code from the terminal verdict.",
        "jit parses and persists the structured findings block.",
    ] {
        assert!(
            contributor_normalized.contains(required),
            "contributor guidance is missing exact findings ownership: {required}"
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
