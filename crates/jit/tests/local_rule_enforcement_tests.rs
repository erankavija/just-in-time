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
        false,
    );
    let after = executor.storage().read_events().unwrap().len();
    assert_eq!(before, after, "an ordinary rejection must not log events");
}
