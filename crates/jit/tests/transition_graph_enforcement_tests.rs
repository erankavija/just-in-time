//! Transition-time graph-rule enforcement (jit issue bc86f54c, CC-2).
//!
//! Graph rules now run at issue state transitions, not only in `jit validate`
//! and gate checkers. An enforcing graph rule (`enforce = true`, severity
//! `error`) whose finding is attributed to the transitioning issue blocks the
//! transition with exit 4 and records the attempt in the event log;
//! non-enforcing or non-attributed findings surface as warnings without
//! blocking; `--force` bypasses with a dedicated audit event; the evaluation is
//! scoped to the issue's dependency neighborhood, not the whole repository; and
//! a done transition diverted to `gated` by unpassed gates enforces against
//! the GATED target state (done-keyed rules do not fire; gated-keyed rules
//! block the diversion).

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
    // The unpassed gate diverts the done transition to `gated`, so graph rules
    // are enforced against the GATED target state — the done-keyed rule does
    // not match, no TransitionBlocked (graph) event is logged, and the issue
    // lands in `gated` with the gate error. (A gated-keyed rule WOULD block
    // the diversion; see
    // `test_gated_diversion_enforces_gated_keyed_graph_rule`.)
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
        "a done-keyed rule must not fire on a diversion whose target is gated"
    );
}

#[test]
fn test_gated_diversion_enforces_gated_keyed_graph_rule() {
    // The gate-diversion path lands in `gated`, so a gated-keyed enforcing
    // rule must block the diversion itself — the same enforcement an explicit
    // `--state gated` transition gets. Nothing persists on the block.
    let rules = r#"
[[rules]]
name = "gated-needs-design-dep"
when = { type = "epic", state = "gated" }
severity = "error"
enforce = true
assert = { dependency-shape = { target = { type = "design" }, mode = "must" } }
"#;
    let executor = executor_with_rules(rules);

    let mut issue = Issue::new("Epic".to_string(), String::new());
    issue.labels = vec!["type:epic".to_string()];
    issue.state = State::InProgress;
    issue.gates_required = vec!["tests".to_string()];
    let epic = issue.id.clone();
    executor.storage().save_issue(issue).unwrap();
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
    let err = result.expect_err("the gated-keyed rule must block the diversion");
    let blocked = err.downcast_ref::<TransitionBlockedError>().unwrap();
    assert!(
        blocked.to_string().contains("gated-needs-design-dep"),
        "the block must be the graph rule, not the gate error: {blocked}"
    );

    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(
        after.state,
        State::InProgress,
        "a blocked diversion must persist nothing"
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

/// A `dependency-shape` rule missing its `target` key: a CONFIG ERROR. The rule
/// applies to `type:epic` issues at the done transition and `enforce = true`, so
/// a broken (misconfigured) guard must BLOCK rather than silently degrade to a
/// warning. Neighborhood-local (dependency-shape is not repo-wide), so it runs
/// at transition time.
const DONE_RULE_MISCONFIGURED: &str = r#"
[[rules]]
name = "epic-done-broken-shape"
when = { type = "epic", state = "done" }
severity = "error"
enforce = true
assert = { dependency-shape = { mode = "must" } }
"#;

#[test]
fn test_enforce_rule_config_error_blocks_transition() {
    // A typo in an `enforce = true` rule (here: a dependency-shape rule with no
    // `target`) must NOT silently disable the guard. The config error blocks the
    // done transition with exit 4, and the rendered error makes clear the rule
    // itself is misconfigured.
    let executor = executor_with_rules(DONE_RULE_MISCONFIGURED);
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

    let err = result.expect_err("a misconfigured enforce rule must block the transition");
    let blocked = err
        .downcast_ref::<TransitionBlockedError>()
        .expect("config error from an enforce rule maps to exit 4");
    let rendered = blocked.to_string();
    assert!(
        rendered.contains("epic-done-broken-shape"),
        "blocked error must name the broken rule: {rendered}"
    );
    assert!(
        rendered.contains("misconfigured"),
        "blocked error must flag the rule as misconfigured: {rendered}"
    );

    // Nothing persisted: the issue is still in progress.
    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(
        after.state,
        State::InProgress,
        "a config-error block must persist nothing"
    );
}

#[test]
fn test_warn_rule_config_error_does_not_block_transition() {
    // The same misconfigured rule, but `enforce = false`: a config error from a
    // non-enforcing rule stays a warning and does not block the transition.
    let rules = r#"
[[rules]]
name = "epic-done-broken-shape-warn"
when = { type = "epic", state = "done" }
severity = "error"
enforce = false
assert = { dependency-shape = { mode = "must" } }
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
        .expect("a non-enforcing rule's config error must not block");

    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(
        after.state,
        State::Done,
        "non-enforcing config error does not block"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("epic-done-broken-shape-warn") && w.contains("config error")),
        "the config error must surface as a warning: {warnings:?}"
    );
}

#[test]
fn test_enforce_at_done_rule_blocks_gated_auto_done_path() {
    // An enforce-at-done coverage/shape rule must also guard the Gated -> Done
    // auto-transition path (gates pass via postcheck, then the issue would
    // auto-complete). Without enforcement on this path an auto gate-pass could
    // complete an issue past an enforce-at-done rule. Here: an epic with a
    // PASSED gate (so it would auto-done) but NO design dependency (so the
    // enforce-at-done rule fails) must stay Gated with the block surfaced.
    use jit::domain::{GateState, GateStatus};

    let executor = executor_with_rules(DONE_NEEDS_DESIGN);

    let mut issue = Issue::new("Epic".to_string(), String::new());
    issue.labels = vec!["type:epic".to_string()];
    issue.state = State::Gated;
    issue.gates_required = vec!["tests".to_string()];
    issue.gates_status.insert(
        "tests".to_string(),
        GateState {
            status: GateStatus::Passed,
            updated_by: Some("ci:test".to_string()),
            updated_at: chrono::Utc::now(),
        },
    );
    let epic = issue.id.clone();
    executor.storage().save_issue(issue).unwrap();

    // Drive the Gated arm: it runs postchecks, which try to auto-transition to
    // done. The enforce-at-done graph rule must block that auto-done.
    let result = executor.update_issue_state(&epic, State::Gated);
    let err = result.expect_err("enforce-at-done rule must block the gated auto-done path");
    let blocked = err
        .downcast_ref::<TransitionBlockedError>()
        .expect("graph-rule block maps to exit 4");
    assert!(
        blocked.to_string().contains("epic-done-needs-design-dep"),
        "the block must name the failing done rule: {blocked}"
    );

    // The issue stays gated: the auto-done never persisted.
    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(
        after.state,
        State::Gated,
        "a blocked auto-done leaves the issue gated"
    );
    // No issue_completed event was emitted.
    let events = executor.storage().read_events().unwrap();
    assert!(
        !events.iter().any(|e| e.get_type() == "issue_completed"),
        "a blocked auto-done must not log completion"
    );
}

#[test]
fn test_update_issue_state_enforces_non_terminal_arm_transition() {
    // M3: the `_` arm (e.g. Ready -> InProgress) previously skipped graph
    // enforcement. An enforcing rule scoped to the in-progress state must now
    // block this state-only transition too.
    let rules = r#"
[[rules]]
name = "inprogress-needs-design-dep"
when = { type = "epic", state = "in_progress" }
severity = "error"
enforce = true
assert = { dependency-shape = { target = { type = "design" }, mode = "must" } }
"#;
    let executor = executor_with_rules(rules);
    let epic = seed_issue(&executor, "Epic", &["type:epic"], State::Ready);

    let result = executor.update_issue_state(&epic, State::InProgress);
    let err = result.expect_err("the `_` arm must enforce graph rules");
    assert!(err.downcast_ref::<TransitionBlockedError>().is_some());

    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(after.state, State::Ready, "blocked: nothing persisted");
}

#[test]
fn test_update_issue_state_enforces_explicit_gated_transition() {
    // The explicit `--state gated` arm previously saved and returned before
    // graph enforcement, so a `state = "gated"` enforce rule was bypassable.
    // It must now block before anything persists.
    let rules = r#"
[[rules]]
name = "gated-needs-design-dep"
when = { type = "epic", state = "gated" }
severity = "error"
enforce = true
assert = { dependency-shape = { target = { type = "design" }, mode = "must" } }
"#;
    let executor = executor_with_rules(rules);
    let epic = seed_issue(&executor, "Epic", &["type:epic"], State::InProgress);

    let result = executor.update_issue_state(&epic, State::Gated);
    let err = result.expect_err("the gated arm must enforce graph rules");
    assert!(err.downcast_ref::<TransitionBlockedError>().is_some());

    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(after.state, State::InProgress, "blocked: nothing persisted");
}

#[test]
fn test_bulk_update_blocked_by_enforcing_done_graph_rule() {
    // The reviewer's exact scenario (jit bc86f54c): a bulk
    // `jit issue update --filter ... --state done` must NOT bypass an enforcing
    // graph rule that blocks the same single-issue transition. The state change
    // now routes through the chokepoint, so the bulk update for the violating
    // issue fails (recorded in `errors`) and the issue stays unchanged.
    use jit::commands::UpdateOperations;
    use jit::query_engine::QueryFilter;

    let mut executor = executor_with_rules(DONE_NEEDS_DESIGN);
    let epic = seed_issue(&executor, "Epic", &["type:epic"], State::InProgress);

    let filter = QueryFilter::parse("label:type:epic").unwrap();
    let ops = UpdateOperations {
        state: Some(State::Done),
        ..Default::default()
    };
    let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

    assert_eq!(result.summary.total_matched, 1);
    assert_eq!(
        result.summary.total_modified, 0,
        "the enforcing done-rule must block the bulk transition"
    );
    assert_eq!(result.summary.total_errors, 1);
    assert_eq!(result.errors[0].0, epic);
    assert!(
        result.errors[0].1.contains("epic-done-needs-design-dep"),
        "the per-issue error must name the failing rule: {}",
        result.errors[0].1
    );

    // The issue is unchanged: still InProgress, not Done.
    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(
        after.state,
        State::InProgress,
        "a blocked bulk transition must persist nothing for that issue"
    );
}

#[test]
fn test_bulk_update_warn_rule_passes_with_warning() {
    // A non-enforcing (warn) graph rule must NOT block the bulk transition: the
    // issue completes the state change, AND the warning is surfaced in
    // BulkUpdateResult::warnings with issue-id attribution.
    use jit::commands::UpdateOperations;
    use jit::query_engine::QueryFilter;

    let rules = r#"
[[rules]]
name = "epic-done-should-have-design"
when = { type = "epic", state = "done" }
severity = "error"
enforce = false
assert = { dependency-shape = { target = { type = "design" }, mode = "must" } }
"#;
    let mut executor = executor_with_rules(rules);
    let epic = seed_issue(&executor, "Epic", &["type:epic"], State::InProgress);

    let filter = QueryFilter::parse("label:type:epic").unwrap();
    let ops = UpdateOperations {
        state: Some(State::Done),
        ..Default::default()
    };
    let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();

    assert_eq!(result.summary.total_matched, 1);
    assert_eq!(
        result.summary.total_modified, 1,
        "a non-enforcing rule must not block the bulk transition"
    );
    assert_eq!(result.summary.total_errors, 0);

    // The warning must be surfaced with the issue id and rule name.
    assert_eq!(
        result.warnings.len(),
        1,
        "non-enforcing rule must surface a per-issue warning in BulkUpdateResult::warnings"
    );
    assert_eq!(
        result.warnings[0].0, epic,
        "warning must be attributed to the correct issue id"
    );
    assert!(
        result.warnings[0]
            .1
            .contains("epic-done-should-have-design"),
        "warning message must name the failing rule: {}",
        result.warnings[0].1
    );

    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(
        after.state,
        State::Done,
        "non-enforcing rule does not block"
    );
}

#[test]
fn test_bulk_update_force_bypasses_enforcing_done_graph_rule() {
    // `--force` on the bulk path must bypass the enforcing graph rule through the
    // chokepoint, completing the transition and recording the override.
    use jit::commands::UpdateOperations;
    use jit::query_engine::QueryFilter;

    let mut executor = executor_with_rules(DONE_NEEDS_DESIGN);
    let epic = seed_issue(&executor, "Epic", &["type:epic"], State::InProgress);

    let filter = QueryFilter::parse("label:type:epic").unwrap();
    let ops = UpdateOperations {
        state: Some(State::Done),
        ..Default::default()
    };
    let result = executor.apply_bulk_update(&filter, &ops, true).unwrap();

    assert_eq!(result.summary.total_modified, 1);
    assert_eq!(result.summary.total_errors, 0);

    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(
        after.state,
        State::Done,
        "forced bulk transition lands done"
    );

    let events = executor.storage().read_events().unwrap();
    assert!(
        events.iter().any(|e| e.get_type() == "graph_rule_bypassed"),
        "a forced bulk override must log a GraphRuleBypassed event"
    );
}

#[test]
fn test_rejected_transition_bypasses_enforcement_through_chokepoint() {
    // Rejection deliberately bypasses graph-rule enforcement (the policy is
    // encoded inside the chokepoint). An enforcing rule that WOULD block a done
    // transition must NOT block a transition to Rejected, regardless of which
    // entry point is used.
    let executor = executor_with_rules(DONE_NEEDS_DESIGN);
    let epic = seed_issue(&executor, "Epic", &["type:epic"], State::InProgress);

    // The state-only path (`jit issue reject`) routes through the chokepoint.
    let result = executor.update_issue_state(&epic, State::Rejected);
    assert!(
        result.is_ok(),
        "rejection must bypass enforcement even with a violated enforce rule"
    );
    let after = executor.storage().load_issue(&epic).unwrap();
    assert_eq!(after.state, State::Rejected);

    // No TransitionBlocked event was logged: enforcement never ran on rejection.
    let events = executor.storage().read_events().unwrap();
    assert!(
        !events.iter().any(|e| e.get_type() == "transition_blocked"),
        "rejection must not run graph enforcement"
    );
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
