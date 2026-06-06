//! End-to-end tests proving that `Scope::Graph` rules defined in
//! `.jit/rules.toml` actually RUN inside the `jit validate` execution path
//! (`CommandExecutor::validate_silent`) and affect its pass/fail result.
//!
//! These are wiring tests: the pure `evaluate_graph` engine is unit-tested in
//! `validation::graph`. Here we verify the production validate path loads the
//! ruleset, selects graph-scope rules, runs them over the store's issues, and
//! folds error-severity findings into validation failure.

use jit::commands::CommandExecutor;
use jit::domain::Issue;
use jit::storage::{InMemoryStorage, IssueStore};

/// Build a fresh in-memory store whose on-disk root (used only for `rules.toml`)
/// holds the supplied rules file contents.
fn store_with_rules(rules_toml: &str) -> InMemoryStorage {
    std::env::set_var("JIT_TEST_MODE", "1");
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    std::fs::create_dir_all(storage.root()).unwrap();
    std::fs::write(storage.root().join("rules.toml"), rules_toml).unwrap();
    storage
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
    let storage = store_with_rules(SHAPE_RULES);

    // A story exists, and a task that does NOT depend on it -> violation.
    let mut story = Issue::new("a story".to_string(), String::new());
    story.labels = vec!["type:story".to_string()];
    let mut task = Issue::new("a task".to_string(), String::new());
    task.labels = vec!["type:task".to_string()];
    // The two issues still need to be connected for the (unrelated) isolated-node
    // check, so make the story depend on the task. This does NOT satisfy the
    // shape rule (which requires task -> story), so the graph rule still fires.
    story.dependencies = vec![task.id.clone()];
    storage.save_issue(story).unwrap();
    storage.save_issue(task).unwrap();

    let executor = CommandExecutor::new(storage);
    let result = executor.validate_silent();

    assert!(
        result.is_err(),
        "error-severity graph-rule violation must fail validation"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("Graph rule validation failed"),
        "message should explain a graph rule failed: {msg}"
    );
    assert!(
        msg.contains("task-needs-story-dep"),
        "message should name the violated rule: {msg}"
    );
}

#[test]
fn test_validate_passes_when_graph_rule_is_satisfied() {
    let storage = store_with_rules(SHAPE_RULES);

    // The task DOES depend on the story -> rule satisfied.
    let mut story = Issue::new("a story".to_string(), String::new());
    story.labels = vec!["type:story".to_string()];
    let mut task = Issue::new("a task".to_string(), String::new());
    task.labels = vec!["type:task".to_string()];
    task.dependencies = vec![story.id.clone()];
    storage.save_issue(story).unwrap();
    storage.save_issue(task).unwrap();

    let executor = CommandExecutor::new(storage);
    let result = executor.validate_silent();

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
    let storage = store_with_rules(warn_rules);

    let mut story = Issue::new("a story".to_string(), String::new());
    story.labels = vec!["type:story".to_string()];
    let mut task = Issue::new("a task".to_string(), String::new());
    task.labels = vec!["type:task".to_string()];
    story.dependencies = vec![task.id.clone()]; // connect, but don't satisfy rule
    storage.save_issue(story).unwrap();
    storage.save_issue(task).unwrap();

    let executor = CommandExecutor::new(storage);
    let result = executor.validate_silent();

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
    let storage = store_with_rules(coverage_rules);

    let mut epic = Issue::new(
        "epic".to_string(),
        "## Success Criteria\n\n- [hard] REQ-01: do the thing\n".to_string(),
    );
    epic.labels = vec!["type:epic".to_string()];
    // A dependent child that does NOT carry satisfies:REQ-01.
    let mut child = Issue::new("child".to_string(), String::new());
    child.labels = vec!["type:task".to_string()];
    child.dependencies = vec![epic.id.clone()];
    storage.save_issue(epic).unwrap();
    storage.save_issue(child).unwrap();

    let executor = CommandExecutor::new(storage);
    let result = executor.validate_silent();

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
    // A graph rule with a bad `child-link` value yields a config-error finding
    // (error severity) and must therefore fail validation rather than be ignored.
    let bad_rules = r#"
[[rules]]
name = "bad-coverage"
when = { type = "epic" }
severity = "error"
assert = { label-coverage = { child-link = "bogus" } }
"#;
    let storage = store_with_rules(bad_rules);

    let mut epic = Issue::new(
        "epic".to_string(),
        "## Success Criteria\n\n- [hard] REQ-01: x\n".to_string(),
    );
    epic.labels = vec!["type:epic".to_string()];
    storage.save_issue(epic).unwrap();

    let executor = CommandExecutor::new(storage);
    let result = executor.validate_silent();

    assert!(result.is_err(), "config-error finding must fail validation");
    assert!(
        result.unwrap_err().to_string().contains("config error"),
        "message should mention the config error"
    );
}
