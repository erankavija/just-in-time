//! In-process tests for the `jit validate` rule runner: per-issue + whole-repo
//! local/graph rule evaluation, `--explain` outcomes, and error-severity
//! exit-code threading (issue b8ba1b10).
//!
//! The pure local/graph engines are unit-tested in their own modules; these are
//! wiring tests over `CommandExecutor::run_rules` / `explain_rules` proving the
//! production orchestration loads the ruleset, scopes correctly, and reports
//! findings with the right severity.
//!
//! NOTE: post-a0f0f342 the effective rule set ALSO includes the built-in default
//! rules derived from the scaffolded `config.toml` (label format, namespace
//! registry, etc.). These tests therefore filter to the USER rule under test
//! rather than asserting on the total finding/outcome count, which now legitimately
//! includes default-rule activity.

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
    // `epic:auth` keeps the epic strategically consistent so the built-in
    // `default:strategic-consistency` graph warning does not add findings beyond
    // the user rule under test.
    e.labels = if req {
        vec![
            "type:epic".to_string(),
            "epic:auth".to_string(),
            "req:REQ-01".to_string(),
        ]
    } else {
        vec!["type:epic".to_string(), "epic:auth".to_string()]
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

    // The user rule is satisfied: no `epic-needs-req` finding. (Default rules may
    // independently flag the scaffolded-config namespaces; out of scope here.)
    assert!(
        !report.findings.iter().any(|f| f.rule == "epic-needs-req"),
        "satisfied user rule must not appear: {report:?}"
    );
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
    // The effective set also contains always-on default rules (e.g. label-format);
    // assert on the USER rule's outcome by name.
    let outcome = report
        .outcomes
        .iter()
        .find(|o| o.rule == "epic-needs-req")
        .expect("epic-needs-req outcome present");
    assert_eq!(outcome.scope, jit::validation::rules::Scope::Local);
    assert_eq!(outcome.severity, jit::validation::rules::Severity::Error);
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

    // The USER rule passes for a compliant epic; assert on it by name (default
    // rules derived from the scaffolded config are evaluated alongside it).
    let outcome = report
        .outcomes
        .iter()
        .find(|o| o.rule == "epic-needs-req")
        .expect("epic-needs-req outcome present");
    assert!(outcome.passed, "satisfied user rule passes");
    assert!(outcome.messages.is_empty());
}

#[test]
fn test_explain_non_matching_rule_is_reported_as_skipped() {
    let storage = store_with_rules(EPIC_NEEDS_REQ);
    // A task does not match the epic selector.
    let mut task = Issue::new("a task".to_string(), String::new());
    task.labels = vec!["type:task".to_string()];
    let id = task.id.clone();
    storage.save_issue(task).unwrap();

    let executor = CommandExecutor::new(storage);
    let report = executor.explain_rules(&id).unwrap();

    // The epic-selected USER rule does not match a task, but is now STILL listed
    // as a skipped outcome carrying the reason its selector excluded the issue.
    let outcome = report
        .outcomes
        .iter()
        .find(|o| o.rule == "epic-needs-req")
        .expect("non-matching epic rule must still appear as an outcome");
    assert!(!outcome.matched, "task does not match the epic selector");
    assert!(outcome.passed, "a skipped rule produces no failures");
    assert!(outcome.messages.is_empty());
    let reason = outcome
        .skip_reason
        .as_deref()
        .expect("skipped rule carries a reason");
    assert!(
        reason.contains("type predicate did not match"),
        "reason names the failing dimension: {reason}"
    );
    // A skipped rule never counts as a failure.
    assert!(!report.has_failures());
}

#[test]
fn test_explain_matched_rule_with_state_predicate_shows_state_in_selector() {
    // A rule scoped to a state the issue IS in: it matches, its selector renders
    // the state, and it yields a PASS/FAIL outcome (here FAIL: missing section).
    let rules = r#"
[[rules]]
name = "in-progress-needs-plan"
when = { state = "in_progress" }
severity = "error"
assert = { require-section = { heading = "Plan" } }
"#;
    let storage = store_with_rules(rules);
    let mut issue = Issue::new("a task".to_string(), String::new());
    issue.state = jit::domain::State::InProgress;
    let id = issue.id.clone();
    storage.save_issue(issue).unwrap();

    let executor = CommandExecutor::new(storage);
    let report = executor.explain_rules(&id).unwrap();

    let outcome = report
        .outcomes
        .iter()
        .find(|o| o.rule == "in-progress-needs-plan")
        .expect("state-scoped rule outcome present");
    assert!(outcome.matched, "issue is in_progress so the rule matches");
    assert!(outcome.skip_reason.is_none());
    assert!(
        outcome.selector.contains("state=in_progress"),
        "selector shows the state predicate: {}",
        outcome.selector
    );
    assert!(!outcome.passed, "missing 'Plan' section fails the rule");
    assert!(!outcome.messages.is_empty());
}

#[test]
fn test_explain_state_predicate_mismatch_names_state_and_tokens() {
    // The success criterion: `--explain` shows whether the state predicate
    // matched. A rule wanting `done` against an `in_progress` issue is skipped
    // with a reason naming BOTH the issue state and the predicate token.
    let rules = r#"
[[rules]]
name = "done-needs-summary"
when = { state = "done" }
severity = "error"
assert = { require-section = { heading = "Summary" } }
"#;
    let storage = store_with_rules(rules);
    let mut issue = Issue::new("a task".to_string(), String::new());
    issue.state = jit::domain::State::InProgress;
    let id = issue.id.clone();
    storage.save_issue(issue).unwrap();

    let executor = CommandExecutor::new(storage);
    let report = executor.explain_rules(&id).unwrap();

    let outcome = report
        .outcomes
        .iter()
        .find(|o| o.rule == "done-needs-summary")
        .expect("state-mismatched rule still appears as an outcome");
    assert!(!outcome.matched);
    let reason = outcome
        .skip_reason
        .as_deref()
        .expect("skipped rule carries a reason");
    assert!(
        reason.contains("state predicate did not match"),
        "reason calls out the state dimension: {reason}"
    );
    assert!(
        reason.contains("in_progress"),
        "reason names the issue's current state: {reason}"
    );
    assert!(
        reason.contains("done"),
        "reason names the predicate token: {reason}"
    );
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
