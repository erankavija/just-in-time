//! Write-path enforcement of local validation rules (issue 25ad2a02).
//!
//! These tests drive the real [`CommandExecutor`] against an
//! [`InMemoryStorage`] with a `.jit/rules.toml` written to the storage root,
//! exercising create / update / bulk paths end to end:
//!
//! - an `enforce` rule blocks a non-forced write;
//! - `--force` bypasses it AND logs the new `LocalRuleBypassed` event;
//! - a `warn` rule never blocks;
//! - a graph-scope rule is NOT evaluated on the write path;
//! - the batch path (`apply_bulk_update`) is also enforced.

use jit::commands::bulk_update::UpdateOperations;
use jit::commands::CommandExecutor;
use jit::domain::{Event, Issue, Priority, State};
use jit::query_engine::QueryFilter;
use jit::storage::{InMemoryStorage, IssueStore};

/// Build an executor whose `.jit/rules.toml` contains `rules_toml`.
fn executor_with_rules(rules_toml: &str) -> CommandExecutor<InMemoryStorage> {
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    std::fs::create_dir_all(storage.root()).unwrap();
    // Disable lease enforcement so updates do not require a claim.
    std::fs::write(
        storage.root().join("config.toml"),
        "[worktree]\nenforce_leases = \"off\"\n",
    )
    .unwrap();
    std::fs::write(storage.root().join("rules.toml"), rules_toml).unwrap();
    CommandExecutor::new(storage)
}

/// An enforce rule: epics must carry a `req:*` label.
const EPIC_NEEDS_REQ_ENFORCE: &str = r#"
[[rules]]
name = "epic-needs-req"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { require-label = { label = "req:*", min = 1 } }
"#;

fn bypass_events(executor: &CommandExecutor<InMemoryStorage>) -> Vec<Event> {
    executor
        .storage()
        .read_events()
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e, Event::LocalRuleBypassed { .. }))
        .collect()
}

#[test]
fn test_create_enforce_rule_blocks_without_force() {
    let executor = executor_with_rules(EPIC_NEEDS_REQ_ENFORCE);

    // An epic without a req: label violates the enforce rule -> blocked.
    let result = executor.create_issue(
        "An epic".to_string(),
        String::new(),
        Priority::Normal,
        vec![],
        vec!["type:epic".to_string()],
        None,
        false,
    );
    assert!(result.is_err(), "enforce rule must block the create");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("epic-needs-req"),
        "message names the rule: {msg}"
    );

    // Nothing should have been persisted or logged as a bypass.
    assert!(executor.storage().list_issues().unwrap().is_empty());
    assert!(bypass_events(&executor).is_empty());
}

#[test]
fn test_create_satisfying_issue_passes() {
    let executor = executor_with_rules(EPIC_NEEDS_REQ_ENFORCE);

    let result = executor.create_issue(
        "An epic".to_string(),
        String::new(),
        Priority::Normal,
        vec![],
        vec!["type:epic".to_string(), "req:REQ-01".to_string()],
        None,
        false,
    );
    assert!(result.is_ok(), "a satisfying epic must be created");
    assert_eq!(executor.storage().list_issues().unwrap().len(), 1);
    assert!(bypass_events(&executor).is_empty());
}

#[test]
fn test_create_force_bypasses_and_logs_event() {
    let executor = executor_with_rules(EPIC_NEEDS_REQ_ENFORCE);

    let (id, _warnings) = executor
        .create_issue(
            "An epic".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            None,
            true, // --force
        )
        .expect("force must allow the create");

    // The issue exists despite the violation.
    assert_eq!(executor.storage().list_issues().unwrap().len(), 1);

    // Exactly one bypass event was logged, naming the rule and issue.
    let events = bypass_events(&executor);
    assert_eq!(events.len(), 1, "one bypass event per bypassed rule");
    match &events[0] {
        Event::LocalRuleBypassed { issue_id, rule, .. } => {
            assert_eq!(issue_id, &id);
            assert_eq!(rule, "epic-needs-req");
        }
        other => panic!("expected LocalRuleBypassed, got {other:?}"),
    }
}

#[test]
fn test_warn_rule_does_not_block_create() {
    let executor = executor_with_rules(
        r#"
[[rules]]
name = "epic-warns-req"
when = { type = "epic" }
severity = "warn"
assert = { require-label = { label = "req:*", min = 1 } }
"#,
    );

    let (_, warnings) = executor
        .create_issue(
            "An epic".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            None,
            false,
        )
        .expect("a warn rule must not block");
    assert!(
        warnings.iter().any(|w| w.contains("epic-warns-req")),
        "the warning should be surfaced: {warnings:?}"
    );
    assert!(bypass_events(&executor).is_empty());
}

#[test]
fn test_raw_schema_requiring_sections_triggers_body_parse() {
    // A RAW json-schema rule that requires the body via `required: ["sections"]`
    // references the body through a STRING VALUE, not an object key. Body-need
    // detection must still trigger the lazy description parse, otherwise the
    // schema validates against a body-less projection and falsely rejects.
    let executor = executor_with_rules(
        r#"
[[rules]]
name = "must-have-body"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { json-schema = "schemas/needs-sections.json" }
"#,
    );
    let schemas = executor.storage().root().join("schemas");
    std::fs::create_dir_all(&schemas).unwrap();
    std::fs::write(
        schemas.join("needs-sections.json"),
        r#"{ "type": "object", "required": ["sections"] }"#,
    )
    .unwrap();

    // An epic with a body section: the description parses into a `sections`
    // projection, so `required: ["sections"]` is satisfied. Without the body
    // parse being triggered, `sections` is absent and this would wrongly fail.
    let result = executor.create_issue(
        "An epic".to_string(),
        "## Goals\n\n- ship it\n".to_string(),
        Priority::Normal,
        vec![],
        vec!["type:epic".to_string()],
        None,
        false,
    );
    assert!(
        result.is_ok(),
        "a raw schema requiring `sections` must trigger the body parse, got {result:?}"
    );
}

#[test]
fn test_graph_rule_not_evaluated_on_create() {
    // A graph-scope enforce rule that would "fail" must be skipped on write.
    let executor = executor_with_rules(
        r#"
[[rules]]
name = "coverage"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { label-coverage = { source = "req", child-state = "done" } }
"#,
    );
    let result = executor.create_issue(
        "An epic".to_string(),
        String::new(),
        Priority::Normal,
        vec![],
        vec!["type:epic".to_string()],
        None,
        false,
    );
    assert!(result.is_ok(), "graph rules must not block writes");
}

#[test]
fn test_update_enforce_rule_blocks_and_force_logs() {
    let executor = executor_with_rules(EPIC_NEEDS_REQ_ENFORCE);

    // Seed a satisfying epic directly (bypassing create enforcement).
    let mut issue = Issue::new("An epic".to_string(), String::new());
    issue.labels = vec!["type:epic".to_string(), "req:REQ-01".to_string()];
    issue.state = State::Ready;
    let id = issue.id.clone();
    executor.storage().save_issue(issue).unwrap();

    // Removing the only req: label would violate the enforce rule -> blocked.
    let blocked = executor.update_issue(
        &id,
        None,
        None,
        None,
        None,
        vec![],
        vec!["req:REQ-01".to_string()],
        None,
        false,
    );
    assert!(blocked.is_err(), "update must be blocked by enforce rule");
    // The label must NOT have been removed (write rejected).
    assert!(executor
        .storage()
        .load_issue(&id)
        .unwrap()
        .labels
        .contains(&"req:REQ-01".to_string()));

    // With --force the removal goes through and is logged.
    executor
        .update_issue(
            &id,
            None,
            None,
            None,
            None,
            vec![],
            vec!["req:REQ-01".to_string()],
            None,
            true,
        )
        .expect("force must allow the update");
    assert!(!executor
        .storage()
        .load_issue(&id)
        .unwrap()
        .labels
        .contains(&"req:REQ-01".to_string()));

    let events = bypass_events(&executor);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].get_issue_id(), id);
}

#[test]
fn test_update_force_noop_logs_bypass_for_each_violated_rule() {
    // Regression: a forced no-op update (force = true, no field edits, no state
    // arg) against an issue that ALREADY violates an enforce rule must still log
    // one LocalRuleBypassed event per bypassed rule. The user explicitly forced
    // the override; dropping the audit entry just because no other field changed
    // would lose that signal. Two enforce rules are violated, so exactly two
    // bypass events must be appended.
    let executor = executor_with_rules(
        r#"
[[rules]]
name = "epic-needs-req"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { require-label = { label = "req:*", min = 1 } }

[[rules]]
name = "epic-needs-owner"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { require-label = { label = "owner:*", min = 1 } }
"#,
    );

    // Seed an epic that violates BOTH enforce rules (no req:, no owner:),
    // bypassing create-time enforcement by writing through storage directly.
    let mut issue = Issue::new("An epic".to_string(), String::new());
    issue.labels = vec!["type:epic".to_string()];
    issue.state = State::Ready;
    let id = issue.id.clone();
    executor.storage().save_issue(issue).unwrap();

    let updated_at_before = executor.storage().load_issue(&id).unwrap().updated_at;
    let events_before = executor.storage().read_events().unwrap().len();

    // A pure no-op forced update: no title/description/priority/state change and
    // no label edits. force = true makes the still-violated enforce rules a
    // bypass rather than a rejection.
    executor
        .update_issue(&id, None, None, None, None, vec![], vec![], None, true)
        .expect("a forced no-op update must not be rejected by enforce rules");

    // Exactly one bypass event per violated enforce rule.
    let events = bypass_events(&executor);
    assert_eq!(
        events.len(),
        2,
        "one LocalRuleBypassed event per bypassed enforce rule"
    );
    let mut rules: Vec<String> = events
        .iter()
        .map(|e| match e {
            Event::LocalRuleBypassed { rule, issue_id, .. } => {
                assert_eq!(issue_id, &id);
                rule.clone()
            }
            other => panic!("expected LocalRuleBypassed, got {other:?}"),
        })
        .collect();
    rules.sort();
    assert_eq!(rules, vec!["epic-needs-owner", "epic-needs-req"]);

    // The bypass events are the ONLY new events: a no-op write must not bump
    // updated_at or emit a state-change/progress event.
    let events_after = executor.storage().read_events().unwrap().len();
    assert_eq!(
        events_after - events_before,
        2,
        "a no-op forced override appends only the bypass events"
    );
    assert_eq!(
        executor.storage().load_issue(&id).unwrap().updated_at,
        updated_at_before,
        "a no-op forced update must not bump updated_at"
    );
}

#[test]
fn test_bulk_update_enforce_rule_blocks() {
    // The batch path must enforce too: `--filter` cannot slip past enforce rules.
    let mut executor = executor_with_rules(
        r#"
[[rules]]
name = "no-bad-label"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { json-schema = "schemas/no-bad.json" }
"#,
    );
    // Schema: the `bad` namespace must NOT be present.
    let schemas = executor.storage().root().join("schemas");
    std::fs::create_dir_all(&schemas).unwrap();
    std::fs::write(
        schemas.join("no-bad.json"),
        r#"{ "type": "object",
             "properties": { "labels": { "type": "object", "not": { "required": ["bad"] } } } }"#,
    )
    .unwrap();

    let mut issue = Issue::new("An epic".to_string(), String::new());
    issue.labels = vec!["type:epic".to_string()];
    issue.state = State::Ready;
    let id = issue.id.clone();
    executor.storage().save_issue(issue).unwrap();

    // Adding a `bad:*` label via bulk update violates the schema -> error, no force.
    let filter = QueryFilter::parse("state:ready").unwrap();
    let ops = UpdateOperations {
        add_labels: vec!["bad:value".to_string()],
        ..Default::default()
    };
    let result = executor.apply_bulk_update(&filter, &ops, false).unwrap();
    assert_eq!(result.summary.total_errors, 1, "bulk write must be blocked");
    assert_eq!(result.summary.total_modified, 0);
    // The label must not have been added.
    assert!(!executor
        .storage()
        .load_issue(&id)
        .unwrap()
        .labels
        .contains(&"bad:value".to_string()));
    assert!(bypass_events(&executor).is_empty());

    // With --force the bulk write goes through and logs a bypass.
    let result = executor.apply_bulk_update(&filter, &ops, true).unwrap();
    assert_eq!(
        result.summary.total_modified, 1,
        "force must allow the write"
    );
    assert!(executor
        .storage()
        .load_issue(&id)
        .unwrap()
        .labels
        .contains(&"bad:value".to_string()));
    assert_eq!(bypass_events(&executor).len(), 1);
}

#[test]
fn test_update_gate_blocked_force_logs_bypass_and_persists_gated() {
    // Intersection of three behaviors on the update path: an issue with an
    // UNPASSED gate AND a violated `enforce` rule, updated with `--force
    // --state done`. The Done transition diverts to `Gated` (unpassed gate), the
    // forced enforce-rule bypass is still audited, yet the call returns Err
    // (gate-blocked). This is the audit-sensitive `handle_gate_blocking` path.
    let executor = executor_with_rules(
        r#"
[[rules]]
name = "task-needs-req"
when = { type = "task" }
severity = "error"
enforce = true
assert = { require-label = { label = "req:*", min = 1 } }
"#,
    );

    let mut issue = Issue::new("A task".to_string(), String::new());
    issue.labels = vec!["type:task".to_string()]; // no req: -> violates the rule
    issue.state = State::InProgress;
    issue.gates_required = vec!["manual-gate".to_string()]; // unpassed
    let id = issue.id.clone();
    executor.storage().save_issue(issue).unwrap();

    let result = executor.update_issue(
        &id,
        None,
        None,
        None,
        Some(State::Done),
        vec![],
        vec![],
        None,
        true, // --force
    );

    // (a) The unpassed gate blocks the Done transition.
    assert!(
        result.is_err(),
        "an unpassed gate must block the Done transition"
    );
    // (b) The issue is persisted in Gated (gate-diversion).
    assert_eq!(
        executor.storage().load_issue(&id).unwrap().state,
        State::Gated
    );
    // (c) The forced enforce-rule override is audited exactly once, even though
    // the overall call returned Err (gate-blocked).
    let events = bypass_events(&executor);
    assert_eq!(
        events.len(),
        1,
        "a forced enforce-rule bypass must be audited on the gate-blocked path"
    );
    match &events[0] {
        Event::LocalRuleBypassed { rule, .. } => assert_eq!(rule, "task-needs-req"),
        other => panic!("expected LocalRuleBypassed, got {other:?}"),
    }
}

#[test]
fn test_bulk_update_force_noop_logs_bypass() {
    // A forced bulk update that makes NO effective field change to an issue which
    // ALREADY violates an enforce rule must still audit the `--force` override
    // (the bulk path must not gate bypass logging behind persistence).
    let mut executor = executor_with_rules(
        r#"
[[rules]]
name = "no-bad-label"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { json-schema = "schemas/no-bad.json" }
"#,
    );
    let schemas = executor.storage().root().join("schemas");
    std::fs::create_dir_all(&schemas).unwrap();
    std::fs::write(
        schemas.join("no-bad.json"),
        r#"{ "type": "object",
             "properties": { "labels": { "type": "object", "not": { "required": ["bad"] } } } }"#,
    )
    .unwrap();

    // Seed an issue that already violates the rule (it carries a `bad:` label).
    let mut issue = Issue::new("An epic".to_string(), String::new());
    issue.labels = vec!["type:epic".to_string(), "bad:value".to_string()];
    issue.state = State::Ready;
    executor.storage().save_issue(issue).unwrap();

    // Adding a label the issue already has is a no-op (no field change), but the
    // issue still violates the enforce rule, so a forced write must log a bypass.
    let filter = QueryFilter::parse("state:ready").unwrap();
    let ops = UpdateOperations {
        add_labels: vec!["bad:value".to_string()],
        ..Default::default()
    };
    let result = executor.apply_bulk_update(&filter, &ops, true).unwrap();
    assert_eq!(result.summary.total_errors, 0, "force must allow the write");
    assert_eq!(
        result.summary.total_modified, 0,
        "no field actually changed (no-op write)"
    );
    assert_eq!(
        bypass_events(&executor).len(),
        1,
        "a forced override on a no-op bulk write must still log exactly one bypass event"
    );
}

/// An enforce rule keyed on the FINAL state: a `ready` issue must carry a
/// `req:*` label. This only fires if rules are evaluated against the
/// post-transition shape (create auto-promotes to Ready; update transitions
/// into Ready), proving the single-issue paths validate the final shape.
const READY_NEEDS_REQ_ENFORCE: &str = r#"
[[rules]]
name = "ready-needs-req"
when = { state = "ready" }
severity = "error"
enforce = true
assert = { require-label = { label = "req:*", min = 1 } }
"#;

#[test]
fn test_create_evaluates_rules_against_post_autopromote_shape() {
    // A dependency-free issue auto-promotes Backlog -> Ready on create. A rule
    // keyed on `state = "ready"` must therefore fire even though the issue was
    // constructed in Backlog. This fails unless the final (Ready) shape is
    // validated.
    let executor = executor_with_rules(READY_NEEDS_REQ_ENFORCE);

    let blocked = executor.create_issue(
        "A task".to_string(),
        String::new(),
        Priority::Normal,
        vec![],
        vec!["type:task".to_string()], // no req: label
        None,
        false,
    );
    assert!(
        blocked.is_err(),
        "a ready-state enforce rule must fire on create auto-promotion"
    );
    assert!(blocked.unwrap_err().to_string().contains("ready-needs-req"));
    assert!(executor.storage().list_issues().unwrap().is_empty());

    // A satisfying issue (carrying req:) is created and promoted to Ready.
    let (id, _) = executor
        .create_issue(
            "A task".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:task".to_string(), "req:REQ-1".to_string()],
            None,
            false,
        )
        .expect("satisfying issue must be created");
    assert_eq!(
        executor.storage().load_issue(&id).unwrap().state,
        State::Ready
    );
}

#[test]
fn test_update_evaluates_rules_against_post_transition_shape() {
    // Seed a Backlog issue with NO req: label. While Backlog, the
    // `state = "ready"` rule does not match. Transitioning it to Ready must
    // make the rule fire (final-shape evaluation), blocking the transition.
    let executor = executor_with_rules(READY_NEEDS_REQ_ENFORCE);

    let mut issue = Issue::new("A task".to_string(), String::new());
    issue.labels = vec!["type:task".to_string()];
    issue.state = State::Backlog;
    let id = issue.id.clone();
    executor.storage().save_issue(issue).unwrap();

    let blocked = executor.update_issue(
        &id,
        None,
        None,
        None,
        Some(State::Ready), // transition into the selected state
        vec![],
        vec![],
        None,
        false,
    );
    assert!(
        blocked.is_err(),
        "transitioning to ready must trigger the ready-state enforce rule"
    );
    assert!(blocked.unwrap_err().to_string().contains("ready-needs-req"));
    // The transition must NOT have been persisted.
    assert_eq!(
        executor.storage().load_issue(&id).unwrap().state,
        State::Backlog
    );
    assert!(bypass_events(&executor).is_empty());

    // Adding the req: label in the same update unblocks the transition.
    executor
        .update_issue(
            &id,
            None,
            None,
            None,
            Some(State::Ready),
            vec!["req:REQ-1".to_string()],
            vec![],
            None,
            false,
        )
        .expect("satisfying the rule unblocks the transition");
    assert_eq!(
        executor.storage().load_issue(&id).unwrap().state,
        State::Ready
    );
}

#[test]
fn test_force_bypass_event_emitted_after_successful_create_save() {
    // The bypass event must be deferred until after the write commits. On a
    // forced create, IssueCreated is appended only after save_issue succeeds, so
    // the LocalRuleBypassed event (now logged after the save) must come AFTER
    // the IssueCreated event in the log — proving it is not emitted pre-write.
    let executor = executor_with_rules(EPIC_NEEDS_REQ_ENFORCE);

    executor
        .create_issue(
            "An epic".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            None,
            true, // --force
        )
        .expect("force must allow the create");

    let events = executor.storage().read_events().unwrap();
    let created_idx = events
        .iter()
        .position(|e| matches!(e, Event::IssueCreated { .. }))
        .expect("an IssueCreated event must be logged");
    let bypass_idx = events
        .iter()
        .position(|e| matches!(e, Event::LocalRuleBypassed { .. }))
        .expect("a LocalRuleBypassed event must be logged");
    assert!(
        bypass_idx > created_idx,
        "bypass event must be emitted only after the successful save (and its \
         IssueCreated event), got bypass at {bypass_idx}, created at {created_idx}"
    );
}

#[test]
fn test_no_bypass_event_when_save_fails() {
    // A storage whose save_issue always fails. A forced create that bypasses an
    // enforce rule must NOT leave a LocalRuleBypassed entry when the write fails,
    // because the bypass event is now emitted only after a successful save.
    let inner = InMemoryStorage::new();
    inner.init().unwrap();
    std::fs::create_dir_all(inner.root()).unwrap();
    std::fs::write(
        inner.root().join("config.toml"),
        "[worktree]\nenforce_leases = \"off\"\n",
    )
    .unwrap();
    std::fs::write(inner.root().join("rules.toml"), EPIC_NEEDS_REQ_ENFORCE).unwrap();

    let storage = FailingSaveStorage::new(inner);
    let executor = CommandExecutor::new(storage);

    let result = executor.create_issue(
        "An epic".to_string(),
        String::new(),
        Priority::Normal,
        vec![],
        vec!["type:epic".to_string()],
        None,
        true, // --force: would bypass, but the save will fail
    );
    assert!(result.is_err(), "the failing save must surface an error");

    // No bypass event must have been written, because the save never committed.
    let bypasses: Vec<_> = executor
        .storage()
        .read_events()
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e, Event::LocalRuleBypassed { .. }))
        .collect();
    assert!(
        bypasses.is_empty(),
        "no bypass event may be logged when the issue write fails"
    );
}

#[test]
fn test_ordinary_rejection_is_not_logged() {
    // A blocked (non-forced) write must NOT append any event at all.
    let executor = executor_with_rules(EPIC_NEEDS_REQ_ENFORCE);
    let before = executor.storage().read_events().unwrap().len();
    let _ = executor.create_issue(
        "An epic".to_string(),
        String::new(),
        Priority::Normal,
        vec![],
        vec!["type:epic".to_string()],
        None,
        false,
    );
    let after = executor.storage().read_events().unwrap().len();
    assert_eq!(before, after, "an ordinary rejection must not log events");
}

/// A storage backend that delegates every operation to an inner
/// [`InMemoryStorage`] EXCEPT `save_issue`, which always fails. Used to prove
/// that a `--force` bypass event is never written when the issue write fails.
#[derive(Clone)]
struct FailingSaveStorage {
    inner: InMemoryStorage,
}

impl FailingSaveStorage {
    fn new(inner: InMemoryStorage) -> Self {
        Self { inner }
    }
}

impl IssueStore for FailingSaveStorage {
    fn init(&self) -> anyhow::Result<()> {
        self.inner.init()
    }

    fn save_issue(&self, _issue: Issue) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("simulated save failure"))
    }

    fn load_issue(&self, id: &str) -> anyhow::Result<Issue> {
        self.inner.load_issue(id)
    }

    fn resolve_issue_id(&self, partial_id: &str) -> anyhow::Result<String> {
        self.inner.resolve_issue_id(partial_id)
    }

    fn delete_issue(&self, id: &str) -> anyhow::Result<()> {
        self.inner.delete_issue(id)
    }

    fn list_issues(&self) -> anyhow::Result<Vec<Issue>> {
        self.inner.list_issues()
    }

    fn load_gate_registry(&self) -> anyhow::Result<jit::storage::GateRegistry> {
        self.inner.load_gate_registry()
    }

    fn save_gate_registry(&self, registry: &jit::storage::GateRegistry) -> anyhow::Result<()> {
        self.inner.save_gate_registry(registry)
    }

    fn append_event(&self, event: &Event) -> anyhow::Result<()> {
        self.inner.append_event(event)
    }

    fn read_events(&self) -> anyhow::Result<Vec<Event>> {
        self.inner.read_events()
    }

    fn save_gate_run_result(&self, result: &jit::domain::GateRunResult) -> anyhow::Result<()> {
        self.inner.save_gate_run_result(result)
    }

    fn load_gate_run_result(&self, run_id: &str) -> anyhow::Result<jit::domain::GateRunResult> {
        self.inner.load_gate_run_result(run_id)
    }

    fn list_gate_runs_for_issue(
        &self,
        issue_id: &str,
    ) -> anyhow::Result<Vec<jit::domain::GateRunResult>> {
        self.inner.list_gate_runs_for_issue(issue_id)
    }

    fn root(&self) -> &std::path::Path {
        self.inner.root()
    }

    fn read_repo_file(
        &self,
        rel_path: &str,
    ) -> Result<Option<String>, jit::storage::PathReadError> {
        self.inner.read_repo_file(rel_path)
    }

    fn list_gate_presets(&self) -> anyhow::Result<Vec<jit::gate_presets::PresetInfo>> {
        self.inner.list_gate_presets()
    }

    fn get_gate_preset(
        &self,
        name: &str,
    ) -> anyhow::Result<jit::gate_presets::GatePresetDefinition> {
        self.inner.get_gate_preset(name)
    }

    fn save_gate_preset(
        &self,
        preset: &jit::gate_presets::GatePresetDefinition,
    ) -> anyhow::Result<std::path::PathBuf> {
        self.inner.save_gate_preset(preset)
    }

    fn read_path_bytes(
        &self,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<(Vec<u8>, String), jit::storage::PathReadError> {
        self.inner.read_path_bytes(path, at_commit)
    }
}
