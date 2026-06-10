//! Transition-time graph-rule enforcement (jit issue bc86f54c, CC-2).
//!
//! Graph rules now run at issue state transitions, not only in `jit validate`
//! and gate checkers. An enforcing graph rule (`enforce = true`, severity
//! `error`) whose finding is attributed to the transitioning issue blocks the
//! transition with exit 4 and records the attempt in the event log;
//! non-enforcing or non-attributed findings surface as warnings without
//! blocking; `--force` bypasses with a dedicated audit event; the evaluation is
//! scoped to the issue's dependency neighborhood, not the whole repository; and
//! a done transition diverted to `gated` by unpassed gates never reaches graph
//! enforcement.

use jit::commands::CommandExecutor;
use jit::domain::{Event, Issue, State};
use jit::errors::TransitionBlockedError;
use jit::storage::{InMemoryStorage, IssueStore};

/// Build an executor whose `.jit/rules.toml` holds exactly `rules_toml`.
///
/// Written to the storage root BEFORE the executor caches its effective rule
/// set, so the supplied rules are the operative set. A `config.toml` is created
/// so the content-format / namespace lookups have a real (empty) config.
fn executor_with_rules(rules_toml: &str) -> CommandExecutor<InMemoryStorage> {
    std::env::set_var("JIT_TEST_MODE", "1");
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    std::fs::create_dir_all(storage.root()).unwrap();
    std::fs::write(storage.root().join("config.toml"), "").unwrap();
    std::fs::write(storage.root().join("rules.toml"), rules_toml).unwrap();
    CommandExecutor::new(storage)
}

/// Save an issue directly into storage at a given state, bypassing validation.
fn seed_issue(
    executor: &CommandExecutor<InMemoryStorage>,
    title: &str,
    labels: &[&str],
    state: State,
) -> String {
    let mut issue = Issue::new(title.to_string(), String::new());
    issue.labels = labels.iter().map(|s| s.to_string()).collect();
    issue.state = state;
    let id = issue.id.clone();
    executor.storage().save_issue(issue).unwrap();
    id
}

/// A `dependency-shape` rule: every `type:epic` in state `done` must depend on a
/// `type:design`. Enforcing + error => blocks the done transition when violated.
const DONE_NEEDS_DESIGN: &str = r#"
[[rules]]
name = "epic-done-needs-design-dep"
when = { type = "epic", state = "done" }
severity = "error"
enforce = true
assert = { dependency-shape = { target = { type = "design" }, mode = "must" } }
"#;

#[test]
fn test_done_transition_blocked_by_enforcing_graph_rule_exit4_with_findings() {
    let executor = executor_with_rules(DONE_NEEDS_DESIGN);

    // An in-progress epic with NO design dependency. Transition to done must be
    // blocked by the enforcing graph rule.
    let epic = seed_issue(&executor, "Epic", &["type:epic"], State::InProgress);

    let result = executor.update_issue(
        &epic,
        None,
        None,
        None,
        Some(State::Done),
        vec![],
        vec![],
        None,
        false,
    );

    let err = result.expect_err("enforcing graph rule should block the done transition");
    let blocked = err
        .downcast_ref::<TransitionBlockedError>()
        .expect("error must be a TransitionBlockedError (main.rs maps this to exit 4)");

    // The finding text (rule name + message) is rendered in the error.
    let rendered = blocked.to_string();
    assert!(
        rendered.contains("epic-done-needs-design-dep"),
        "blocked-transition error must print the failing rule/finding: {rendered}"
    );

    // The issue is NOT persisted as done: nothing was saved.
    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(
        after.state,
        State::InProgress,
        "a blocked transition must persist nothing"
    );
}

#[test]
fn test_blocked_transition_records_event_in_log() {
    let executor = executor_with_rules(DONE_NEEDS_DESIGN);
    let epic = seed_issue(&executor, "Epic", &["type:epic"], State::InProgress);

    let _ = executor.update_issue(
        &epic,
        None,
        None,
        None,
        Some(State::Done),
        vec![],
        vec![],
        None,
        false,
    );

    let events = executor.storage().read_events().unwrap();
    let blocked: Vec<&Event> = events
        .iter()
        .filter(|e| e.get_type() == "transition_blocked")
        .collect();
    assert_eq!(
        blocked.len(),
        1,
        "exactly one TransitionBlocked event per blocking rule"
    );
    match blocked[0] {
        Event::TransitionBlocked {
            issue_id,
            target,
            rule,
            ..
        } => {
            assert_eq!(issue_id, &epic);
            assert_eq!(*target, State::Done);
            assert_eq!(rule, "epic-done-needs-design-dep");
        }
        other => panic!("unexpected event: {other:?}"),
    }
    // No state-change event was emitted (nothing persisted).
    assert!(
        !events.iter().any(|e| e.get_type() == "issue_state_changed"),
        "a blocked transition must not log a state change"
    );
}

#[test]
fn test_force_bypasses_blocking_and_logs_graph_rule_bypassed_event() {
    let executor = executor_with_rules(DONE_NEEDS_DESIGN);
    let epic = seed_issue(&executor, "Epic", &["type:epic"], State::InProgress);

    let result = executor.update_issue(
        &epic,
        None,
        None,
        None,
        Some(State::Done),
        vec![],
        vec![],
        None,
        true, // --force
    );
    assert!(result.is_ok(), "--force must bypass the graph-rule block");

    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(after.state, State::Done, "forced transition lands done");

    let events = executor.storage().read_events().unwrap();
    let bypassed: Vec<&Event> = events
        .iter()
        .filter(|e| e.get_type() == "graph_rule_bypassed")
        .collect();
    assert_eq!(
        bypassed.len(),
        1,
        "one GraphRuleBypassed per overridden rule"
    );
    match bypassed[0] {
        Event::GraphRuleBypassed {
            issue_id,
            target,
            rule,
            ..
        } => {
            assert_eq!(issue_id, &epic);
            assert_eq!(*target, State::Done);
            assert_eq!(rule, "epic-done-needs-design-dep");
        }
        other => panic!("unexpected event: {other:?}"),
    }
    // No TransitionBlocked event on the forced path.
    assert!(
        !events.iter().any(|e| e.get_type() == "transition_blocked"),
        "a forced override must not log a TransitionBlocked event"
    );
}

#[test]
fn test_non_enforcing_failure_is_reported_as_warning_not_blocking() {
    // Same shape but `enforce = false`: the finding must NOT block; it surfaces
    // as a warning on the (successful) transition.
    let rules = r#"
[[rules]]
name = "epic-done-should-have-design"
when = { type = "epic", state = "done" }
severity = "error"
enforce = false
assert = { dependency-shape = { target = { type = "design" }, mode = "must" } }
"#;
    let executor = executor_with_rules(rules);
    let epic = seed_issue(&executor, "Epic", &["type:epic"], State::InProgress);

    let warnings = executor
        .update_issue(
            &epic,
            None,
            None,
            None,
            Some(State::Done),
            vec![],
            vec![],
            None,
            false,
        )
        .expect("a non-enforcing finding must not block the transition");

    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(
        after.state,
        State::Done,
        "non-enforcing rule does not block"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("epic-done-should-have-design")),
        "the non-blocking finding must be surfaced as a warning: {warnings:?}"
    );
}

#[test]
fn test_neighborhood_scope_excludes_unrelated_issue_violation() {
    // A non-enforcing rule on ALL epics. The transitioning epic SATISFIES it
    // (it depends on a design). An UNRELATED epic violates it. With
    // neighborhood-scoped evaluation, the unrelated epic is not in the slice, so
    // its finding must not appear among the transition's warnings.
    let rules = r#"
[[rules]]
name = "epic-should-have-design"
when = { type = "epic" }
severity = "warn"
enforce = false
assert = { dependency-shape = { target = { type = "design" }, mode = "must" } }
"#;
    let executor = executor_with_rules(rules);

    // A design issue the transitioning epic depends on.
    let design = seed_issue(&executor, "Design", &["type:design"], State::Done);
    let epic = seed_issue(&executor, "Epic", &["type:epic"], State::InProgress);
    executor.add_dependency(&epic, &design).unwrap();

    // An unrelated epic with no design dependency (violates the rule). It is not
    // connected to `epic` in the dependency graph.
    let _unrelated = seed_issue(&executor, "Unrelated", &["type:epic"], State::InProgress);

    let warnings = executor
        .update_issue(
            &epic,
            None,
            None,
            None,
            Some(State::Done),
            vec![],
            vec![],
            None,
            false,
        )
        .expect("transition should succeed");

    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(after.state, State::Done);
    // The transitioning epic satisfies the rule, and the unrelated epic's
    // violation is OUTSIDE the neighborhood slice, so there are no warnings for
    // this rule at all.
    assert!(
        !warnings
            .iter()
            .any(|w| w.contains("epic-should-have-design")),
        "an unrelated issue's violation must not surface on this transition: {warnings:?}"
    );
}

#[test]
fn test_gated_diversion_runs_before_graph_enforcement() {
    // An epic with an UNPASSED gate AND a violated enforcing done-graph rule.
    // The unpassed gate diverts the done transition to `gated` and returns the
    // gate error; graph enforcement must NOT run, so no TransitionBlocked
    // (graph) event is logged and the issue lands in `gated`.
    let executor = executor_with_rules(DONE_NEEDS_DESIGN);

    let mut issue = Issue::new("Epic".to_string(), String::new());
    issue.labels = vec!["type:epic".to_string()];
    issue.state = State::InProgress;
    issue.gates_required = vec!["tests".to_string()];
    let epic = issue.id.clone();
    executor.storage().save_issue(issue).unwrap();

    // Register the gate so it is a valid reference, leaving it unpassed.
    executor.add_gate(&epic, "tests".to_string()).unwrap();

    let result = executor.update_issue(
        &epic,
        None,
        None,
        None,
        Some(State::Done),
        vec![],
        vec![],
        None,
        false,
    );
    let err = result.expect_err("unpassed gate should block done");
    let blocked = err.downcast_ref::<TransitionBlockedError>().unwrap();
    // The block is a GATE block (diverted to gated), not a graph-rule block.
    assert!(
        blocked.to_string().contains("gate") || blocked.to_string().contains("Gate"),
        "the block must be the gate diversion, not the graph rule: {blocked}"
    );

    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(after.state, State::Gated, "unpassed gates divert to gated");

    let events = executor.storage().read_events().unwrap();
    assert!(
        !events.iter().any(|e| e.get_type() == "transition_blocked"),
        "graph enforcement must not run when the done transition is gate-diverted"
    );
}

#[test]
fn test_update_issue_state_path_enforces_done_graph_rule() {
    // The state-only path (`update_issue_state`) must also enforce graph rules
    // at the Done arm.
    let executor = executor_with_rules(DONE_NEEDS_DESIGN);
    let epic = seed_issue(&executor, "Epic", &["type:epic"], State::InProgress);

    let result = executor.update_issue_state(&epic, State::Done);
    let err = result.expect_err("update_issue_state done must enforce graph rules");
    assert!(err.downcast_ref::<TransitionBlockedError>().is_some());

    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(after.state, State::InProgress, "blocked: nothing persisted");
}

#[test]
fn test_done_rule_does_not_fire_on_non_done_transition() {
    // A `state = "done"` rule must not fire when transitioning to ready: the
    // selector does not match the issue in the ready target state.
    let executor = executor_with_rules(DONE_NEEDS_DESIGN);
    // Backlog epic with no dependencies -> can go ready.
    let epic = seed_issue(&executor, "Epic", &["type:epic"], State::Backlog);

    let result = executor.update_issue(
        &epic,
        None,
        None,
        None,
        Some(State::Ready),
        vec![],
        vec![],
        None,
        false,
    );
    assert!(
        result.is_ok(),
        "a done-scoped rule must not block a ready transition"
    );
    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(after.state, State::Ready);
}
