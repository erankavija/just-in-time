//! In-process tests for the `jit validate` rule runner: per-issue + whole-repo
//! local/graph rule evaluation, `--explain` outcomes, and error-severity
//! exit-code threading (issue b8ba1b10).
//!
//! The pure local/graph engines are unit-tested in their own modules; these are
//! wiring tests over `CommandExecutor::run_rules` / `explain_rules` proving the
//! production orchestration loads the ruleset, scopes correctly, and reports
//! findings with the right severity.

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

/// A local rule: every `type:epic` must carry a `req:*` label. Error severity.
const EPIC_NEEDS_REQ: &str = r#"
[[rules]]
name = "epic-needs-req"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { require-label = { label = "req:*", min = 1 } }
"#;

fn epic(req: bool) -> Issue {
    let mut e = Issue::new("an epic".to_string(), String::new());
    e.labels = if req {
        vec!["type:epic".to_string(), "req:REQ-01".to_string()]
    } else {
        vec!["type:epic".to_string()]
    };
    e
}

#[test]
fn test_per_issue_local_rule_fails() {
    let storage = store_with_rules(EPIC_NEEDS_REQ);
    let bad = epic(false);
    let id = bad.id.clone();
    storage.save_issue(bad).unwrap();

    let executor = CommandExecutor::new(storage);
    let report = executor.run_rules(Some(&id)).unwrap();

    assert!(report.has_errors(), "missing req:* label is an error");
    assert_eq!(report.error_count(), 1);
    assert!(report.findings.iter().any(|f| f.rule == "epic-needs-req"));
    assert!(report
        .findings
        .iter()
        .all(|f| f.issue_id.as_deref() == Some(id.as_str())));
}

#[test]
fn test_per_issue_local_rule_passes() {
    let storage = store_with_rules(EPIC_NEEDS_REQ);
    let good = epic(true);
    let id = good.id.clone();
    storage.save_issue(good).unwrap();

    let executor = CommandExecutor::new(storage);
    let report = executor.run_rules(Some(&id)).unwrap();

    assert!(!report.has_errors(), "satisfied rule: {report:?}");
    assert!(report.findings.is_empty());
}

#[test]
fn test_whole_repo_collects_local_findings_for_all_issues() {
    let storage = store_with_rules(EPIC_NEEDS_REQ);
    // Two bad epics; both must surface.
    storage.save_issue(epic(false)).unwrap();
    storage.save_issue(epic(false)).unwrap();

    let executor = CommandExecutor::new(storage);
    let report = executor.run_rules(None).unwrap();

    assert_eq!(report.error_count(), 2, "both epics fail: {report:?}");
    assert!(report.has_errors());
}

#[test]
fn test_per_issue_surfaces_relevant_graph_finding() {
    // A graph rule: every type:task must depend on a type:story.
    let rules = r#"
[[rules]]
name = "task-needs-story-dep"
when = { type = "task" }
severity = "error"
assert = { dependency-shape = { target = { type = "story" }, mode = "must" } }
"#;
    let storage = store_with_rules(rules);
    let mut story = Issue::new("a story".to_string(), String::new());
    story.labels = vec!["type:story".to_string()];
    let mut task = Issue::new("a task".to_string(), String::new());
    task.labels = vec!["type:task".to_string()]; // no story dependency -> violation
    let task_id = task.id.clone();
    storage.save_issue(story).unwrap();
    storage.save_issue(task).unwrap();

    let executor = CommandExecutor::new(storage);
    let report = executor.run_rules(Some(&task_id)).unwrap();

    assert!(report.has_errors(), "graph rule fires for the task");
    assert!(report
        .findings
        .iter()
        .any(|f| f.rule == "task-needs-story-dep"));
}

#[test]
fn test_explain_lists_matched_rules_and_outcomes() {
    let storage = store_with_rules(EPIC_NEEDS_REQ);
    let bad = epic(false);
    let id = bad.id.clone();
    storage.save_issue(bad).unwrap();

    let executor = CommandExecutor::new(storage);
    let report = executor.explain_rules(&id).unwrap();

    assert_eq!(report.issue_id, id);
    assert_eq!(report.outcomes.len(), 1, "one matching rule");
    let outcome = &report.outcomes[0];
    assert_eq!(outcome.rule, "epic-needs-req");
    assert_eq!(outcome.scope, "local");
    assert_eq!(outcome.severity, "error");
    assert_eq!(outcome.selector, "type=epic");
    assert!(!outcome.passed, "rule failed for this issue");
    assert!(!outcome.messages.is_empty());
    assert!(report.has_failures());
    assert!(report.has_errors());
}

#[test]
fn test_explain_passing_rule_marks_pass() {
    let storage = store_with_rules(EPIC_NEEDS_REQ);
    let good = epic(true);
    let id = good.id.clone();
    storage.save_issue(good).unwrap();

    let executor = CommandExecutor::new(storage);
    let report = executor.explain_rules(&id).unwrap();

    assert_eq!(report.outcomes.len(), 1);
    assert!(report.outcomes[0].passed, "satisfied rule passes");
    assert!(report.outcomes[0].messages.is_empty());
    assert!(!report.has_failures());
}

#[test]
fn test_explain_non_matching_issue_has_no_outcomes() {
    let storage = store_with_rules(EPIC_NEEDS_REQ);
    // A task does not match the epic selector.
    let mut task = Issue::new("a task".to_string(), String::new());
    task.labels = vec!["type:task".to_string()];
    let id = task.id.clone();
    storage.save_issue(task).unwrap();

    let executor = CommandExecutor::new(storage);
    let report = executor.explain_rules(&id).unwrap();

    assert!(report.outcomes.is_empty(), "no rules match a task");
    assert!(!report.has_failures());
}

#[test]
fn test_warn_severity_does_not_count_as_error() {
    let rules = r#"
[[rules]]
name = "epic-warns-req"
when = { type = "epic" }
severity = "warn"
assert = { require-label = { label = "req:*", min = 1 } }
"#;
    let storage = store_with_rules(rules);
    let bad = epic(false);
    let id = bad.id.clone();
    storage.save_issue(bad).unwrap();

    let executor = CommandExecutor::new(storage);
    let report = executor.run_rules(Some(&id)).unwrap();

    assert!(!report.has_errors(), "warn never fails validate");
    assert_eq!(report.error_count(), 0);
    assert_eq!(report.findings.len(), 1, "but is still reported");
}
