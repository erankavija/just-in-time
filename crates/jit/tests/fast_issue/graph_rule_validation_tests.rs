//! End-to-end tests proving that `RuleScope::Graph` rules defined in
//! `.jit/rules.toml` actually RUN inside the `jit validate` execution path
//! (`ValidationOps::validate_silent`) and affect its pass/fail result.
//!
//! These are wiring tests: the pure `evaluate_graph` engine is unit-tested in
//! `validation::graph`. Here we verify the production validate path loads the
//! ruleset, selects graph-scope rules, runs them over the store's issues, and
//! folds error-severity findings into validation failure.

use jit::commands::test_helpers::{memory_executor_declaring, MemoryDeclarations};
use jit::domain::State;
use jit::storage::InMemoryStorage;

fn with_default_rules(custom: &str) -> String {
    let namespaces = jit::config_manager::namespaces_from_config(&toml::from_str("").unwrap());
    let defaults = jit::repository_state::serialize_ruleset(
        &jit::repository_state::default_ruleset(&namespaces),
    )
    .unwrap();
    defaults.rules_toml + custom
}

/// A `dependency-shape` graph rule: every `type:task` must depend on a
/// `type:story`. Error severity.
const SHAPE_RULES: &str = r#"
[[rules]]
name = "task-needs-story-dep"
when = { type = "task" }
severity = "error"
assert = { dependency-shape = { target = { type = "story" }, mode = "must" } }
"#;

#[test]
fn test_validate_fails_on_error_severity_graph_rule_violation() {
    // A story exists, and a task that does NOT depend on it -> violation.
    let mut story = crate::fixture_issue("a story".to_string(), String::new());
    story.labels = vec!["type:story".to_string()];
    let mut task = crate::fixture_issue("a task".to_string(), String::new());
    task.labels = vec!["type:task".to_string()];
    task.state = State::Ready;
    // The two issues still need to be connected for the (unrelated) isolated-node
    // check, so make the story depend on the task. This does NOT satisfy the
    // shape rule (which requires task -> story), so the graph rule still fires.
    story.dependencies = vec![task.id.clone()];
    let storage = InMemoryStorage::new();
    jit::test_harness::seed_memory_issue(&storage, &story);
    jit::test_harness::seed_memory_issue(&storage, &task);

    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            config: Some(""),
            rules: Some(&with_default_rules(SHAPE_RULES)),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );
    let result = executor.validation().validate_silent();

    assert!(
        result.is_err(),
        "error-severity graph-rule violation must fail validation"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("must depend on an issue matching the target selector"),
        "message should explain the dependency-shape violation: {msg}"
    );
    assert!(
        msg.contains("task-needs-story-dep"),
        "message should name the violated rule: {msg}"
    );
}

#[test]
fn test_validate_passes_when_graph_rule_is_satisfied() {
    // The task DOES depend on the story -> rule satisfied.
    let mut story = crate::fixture_issue("a story".to_string(), String::new());
    story.labels = vec!["type:story".to_string()];
    story.state = State::Ready;
    let mut task = crate::fixture_issue("a task".to_string(), String::new());
    task.labels = vec!["type:task".to_string()];
    task.dependencies = vec![story.id.clone()];
    let storage = InMemoryStorage::new();
    jit::test_harness::seed_memory_issue(&storage, &story);
    jit::test_harness::seed_memory_issue(&storage, &task);

    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            config: Some(""),
            rules: Some(&with_default_rules(SHAPE_RULES)),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );
    let result = executor.validation().validate_silent();

    assert!(
        result.is_ok(),
        "satisfied graph rule must not fail validation: {result:?}"
    );
}

#[test]
fn test_validate_does_not_fail_on_warn_severity_graph_rule() {
    // Same violating shape, but severity = warn -> validation must still pass.
    let warn_rules = r#"
[[rules]]
name = "task-should-have-story"
when = { type = "task" }
severity = "warn"
assert = { dependency-shape = { target = { type = "story" }, mode = "should" } }
"#;
    let mut story = crate::fixture_issue("a story".to_string(), String::new());
    story.labels = vec!["type:story".to_string()];
    let mut task = crate::fixture_issue("a task".to_string(), String::new());
    task.labels = vec!["type:task".to_string()];
    task.state = State::Ready;
    story.dependencies = vec![task.id.clone()]; // connect, but don't satisfy rule
    let storage = InMemoryStorage::new();
    jit::test_harness::seed_memory_issue(&storage, &story);
    jit::test_harness::seed_memory_issue(&storage, &task);

    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            config: Some(""),
            rules: Some(&with_default_rules(warn_rules)),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );
    let result = executor.validation().validate_silent();

    assert!(
        result.is_ok(),
        "warn-severity graph-rule violation must NOT fail validation: {result:?}"
    );
}

#[test]
fn test_validate_fails_on_label_coverage_violation() {
    // An epic declares a success criterion that no child satisfies.
    let coverage_rules = r#"
[[rules]]
name = "epic-criteria-covered"
when = { type = "epic" }
severity = "error"
assert = { label-coverage = { } }
"#;
    let mut epic = crate::fixture_issue(
        "epic".to_string(),
        "## Success Criteria\n\n- [hard] REQ-01: do the thing\n".to_string(),
    );
    epic.labels = vec!["type:epic".to_string()];
    epic.state = State::Ready;
    // A dependent child that carries no satisfies label.
    let mut child = crate::fixture_issue("child".to_string(), String::new());
    child.labels = vec!["type:task".to_string()];
    child.dependencies = vec![epic.id.clone()];
    let storage = InMemoryStorage::new();
    jit::test_harness::seed_memory_issue(&storage, &epic);
    jit::test_harness::seed_memory_issue(&storage, &child);

    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            config: Some(""),
            rules: Some(&with_default_rules(coverage_rules)),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );
    let result = executor.validation().validate_silent();

    assert!(
        result.is_err(),
        "uncovered success criterion must fail validation"
    );
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("REQ-01"), "message names the criterion: {msg}");
    assert!(
        msg.contains("epic-criteria-covered"),
        "message names the rule: {msg}"
    );
}

#[test]
fn test_validate_surfaces_malformed_graph_rule_as_failure() {
    // Validation propagates a rule load error naming the malformed rule.
    let bad_rules = r#"
[[rules]]
name = "bad-coverage"
when = { type = "epic" }
severity = "error"
assert = { label-coverage = { child-link = "bogus" } }
"#;
    let mut epic = crate::fixture_issue(
        "epic".to_string(),
        "## Success Criteria\n\n- [hard] REQ-01: x\n".to_string(),
    );
    epic.labels = vec!["type:epic".to_string()];
    epic.state = State::Ready;
    let storage = InMemoryStorage::new();
    jit::test_harness::seed_memory_issue(&storage, &epic);

    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            config: Some(""),
            rules: Some(bad_rules),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );
    let result = executor.validation().validate_silent();

    assert!(result.is_err(), "rule load error must fail validation");
    assert!(
        result.unwrap_err().to_string().contains("bad-coverage"),
        "message should name the broken rule"
    );
}
