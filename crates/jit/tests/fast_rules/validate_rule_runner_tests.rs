//! Harness tests for the `jit validate <id>` and `--explain` views: the
//! selection and explanation of the report one validation capture computes.
//!
//! The effective rule set also holds the built-in default rules derived from the
//! scaffolded `config.toml`, so these tests filter to the user rule under test.

use jit::commands::test_helpers::{memory_executor_declaring, MemoryDeclarations};
use jit::commands::CommandExecutor;
use jit::domain::identity::{IssueId, IssueIdPrefix};
use jit::domain::Issue;
use jit::storage::InMemoryStorage;
use jit::validation::report::{BuiltInPass, Selection};
use jit::validation::{ExplainReport, RuleReport};

/// The report `jit validate <id>` renders.
fn issue_view(executor: &CommandExecutor<InMemoryStorage>, id: &IssueId) -> RuleReport {
    executor
        .validation()
        .selection_report()
        .unwrap()
        .select(&Selection::Issue(IssueIdPrefix::from(id)))
        .unwrap()
}

/// The report `jit validate <id> --explain` renders.
fn explain_view(executor: &CommandExecutor<InMemoryStorage>, id: &IssueId) -> ExplainReport {
    executor
        .validation()
        .selection_report()
        .unwrap()
        .explain(&IssueIdPrefix::from(id))
        .unwrap()
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
    let mut e = crate::fixture_issue("an epic".to_string(), String::new());
    // `epic:auth` keeps the epic strategically consistent if the workflow
    // profile's `strategic-consistency` graph rule is installed, so it does not
    // add findings beyond the user rule under test.
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
    let storage = InMemoryStorage::new();
    let bad = epic(false);
    let id = bad.id.clone();
    jit::test_harness::seed_memory_issue(&storage, &bad);

    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            rules: Some(EPIC_NEEDS_REQ),
            ..MemoryDeclarations::default()
        },
    );
    let report = issue_view(&executor, &id);

    assert!(report.has_errors(), "missing req:* label is an error");
    assert_eq!(report.error_count(), 1);
    assert!(report.findings.iter().any(|f| f.rule == "epic-needs-req"));
    assert!(report
        .findings
        .iter()
        .all(|f| f.issue_id.as_ref() == Some(&id)));
}

#[test]
fn test_per_issue_local_rule_passes() {
    let storage = InMemoryStorage::new();
    let good = epic(true);
    let id = good.id.clone();
    jit::test_harness::seed_memory_issue(&storage, &good);

    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            rules: Some(EPIC_NEEDS_REQ),
            ..MemoryDeclarations::default()
        },
    );
    let report = issue_view(&executor, &id);

    // The user rule is satisfied: no `epic-needs-req` finding. (Default rules may
    // independently flag the scaffolded-config namespaces; out of scope here.)
    assert!(
        !report.findings.iter().any(|f| f.rule == "epic-needs-req"),
        "satisfied user rule must not appear: {report:?}"
    );
}

#[test]
fn test_explain_passing_rule_marks_pass() {
    let storage = InMemoryStorage::new();
    let good = epic(true);
    let id = good.id.clone();
    jit::test_harness::seed_memory_issue(&storage, &good);

    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            rules: Some(EPIC_NEEDS_REQ),
            ..MemoryDeclarations::default()
        },
    );
    let report = explain_view(&executor, &id);

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
    let storage = InMemoryStorage::new();
    // A task does not match the epic selector.
    let mut task = crate::fixture_issue("a task".to_string(), String::new());
    task.labels = vec!["type:task".to_string()];
    let id = task.id.clone();
    jit::test_harness::seed_memory_issue(&storage, &task);

    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            rules: Some(EPIC_NEEDS_REQ),
            ..MemoryDeclarations::default()
        },
    );
    let report = explain_view(&executor, &id);

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
    let storage = InMemoryStorage::new();
    let mut issue = crate::fixture_issue("a task".to_string(), String::new());
    issue.state = jit::domain::State::InProgress;
    let id = issue.id.clone();
    jit::test_harness::seed_memory_issue(&storage, &issue);

    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            rules: Some(rules),
            ..MemoryDeclarations::default()
        },
    );
    let report = explain_view(&executor, &id);

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
    // matched. A rule wanting one state against an issue in another is skipped
    // with a reason naming BOTH the issue state and the predicate token.
    let rules = r#"
[[rules]]
name = "done-needs-summary"
when = { state = "done" }
severity = "error"
assert = { require-section = { heading = "Summary" } }
"#;
    let storage = InMemoryStorage::new();
    let mut issue = crate::fixture_issue("a task".to_string(), String::new());
    issue.state = jit::domain::State::InProgress;
    let id = issue.id.clone();
    jit::test_harness::seed_memory_issue(&storage, &issue);

    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            rules: Some(rules),
            ..MemoryDeclarations::default()
        },
    );
    let report = explain_view(&executor, &id);

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
    let storage = InMemoryStorage::new();
    let bad = epic(false);
    let id = bad.id.clone();
    jit::test_harness::seed_memory_issue(&storage, &bad);

    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            rules: Some(rules),
            ..MemoryDeclarations::default()
        },
    );
    let report = issue_view(&executor, &id);

    assert!(!report.has_errors(), "warn never fails validate");
    assert_eq!(report.error_count(), 0);
    assert_eq!(report.findings.len(), 1, "but is still reported");
}

/// A coverage rule whose firing epic carries no `brackets:` pointer fails with
/// a config error naming that epic.
const POINTER_RULE: &str = r#"
[[rules]]
name = "epic-pointer"
when = { type = "epic" }
severity = "error"
assert = { label-coverage = { container-from-label = "brackets" } }
"#;

#[test]
fn test_validate_issue_omits_a_pointer_error_fired_by_another_issue() {
    let storage = InMemoryStorage::new();
    let (other, selected) = (epic(true), epic(true));
    jit::test_harness::seed_memory_issue(&storage, &other);
    jit::test_harness::seed_memory_issue(&storage, &selected);
    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            rules: Some(POINTER_RULE),
            ..MemoryDeclarations::default()
        },
    );

    let report = issue_view(&executor, &selected.id);

    let pointer: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.rule == "epic-pointer")
        .collect();
    assert!(
        pointer
            .iter()
            .any(|f| f.message.contains(&selected.short_id())),
        "{report:?}"
    );
    assert!(
        pointer
            .iter()
            .all(|f| !f.message.contains(&other.short_id())),
        "{report:?}"
    );
}

#[test]
fn test_validate_issue_with_unloadable_rules_reports_rules_file_without_drift() {
    let storage = InMemoryStorage::new();
    let issue = epic(true);
    jit::test_harness::seed_memory_issue(&storage, &issue);
    let config = jit::test_utils::shipped_item_kind_tables(&["invariant"]);
    let executor = memory_executor_declaring(
        storage,
        MemoryDeclarations {
            config: Some(&config),
            rules: Some("not toml = ["),
            invariants: Some(
                "[[invariants]]\nid = \"bound\"\nstatement = \"s\"\n\
                 kind = \"enforced\"\nenforced-by = \"@/rule/bad-rule\"\n",
            ),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );

    let report = issue_view(&executor, &issue.id);

    let rules: Vec<_> = report.findings.iter().map(|f| f.rule.as_str()).collect();
    assert!(
        rules.contains(&BuiltInPass::RulesFile.as_str()),
        "{rules:?}"
    );
    assert!(
        !rules.contains(&BuiltInPass::EnforcementDrift.as_str()),
        "{rules:?}"
    );
}
