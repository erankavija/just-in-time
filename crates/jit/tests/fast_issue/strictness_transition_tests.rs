//! End-to-end enforcement of `[validation].strictness` at the TRANSITION choke
//! point (issue afbbf6c3).
//!
//! Graph rules are enforced when an issue changes state. Strictness modulates
//! that block/allow decision with the SAME predicate as the write path, so these
//! tests prove the transition path is not silently inert at any level:
//!
//! - a `warn` graph rule's violation blocks a transition under `strict` but not
//!   under `loose`;
//! - an `enforce` error graph rule's violation blocks under `loose` but is
//!   downgraded to advisory under `permissive`.

use jit::commands::CommandExecutor;
use jit::domain::State;
use jit::storage::{InMemoryStorage, IssueStore};

/// A `warn` graph rule: an epic reaching `done` should depend on a `type:design`.
/// Never enforces, so under loose it only warns.
const DONE_WANTS_DESIGN_WARN: &str = r#"
[[rules]]
name = "epic-done-wants-design"
when = { type = "epic", state = "done" }
severity = "warn"
assert = { dependency-shape = { target = { type = "design" }, mode = "must" } }
"#;

/// An `enforce` error graph rule: an epic reaching `done` MUST depend on a
/// `type:design`.
const DONE_NEEDS_DESIGN_ENFORCE: &str = r#"
[[rules]]
name = "epic-done-needs-design"
when = { type = "epic", state = "done" }
severity = "error"
enforce = true
assert = { dependency-shape = { target = { type = "design" }, mode = "must" } }
"#;

/// Build an executor whose `config.toml` sets `[validation].strictness` and whose
/// `.jit/rules.toml` holds `rules_toml`. `JIT_TEST_MODE` keeps lease checks from
/// requiring a real worktree claim.
fn executor(strictness: &str, rules_toml: &str) -> CommandExecutor<InMemoryStorage> {
    std::env::set_var("JIT_TEST_MODE", "1");
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    std::fs::create_dir_all(storage.root()).unwrap();
    std::fs::write(
        storage.root().join("config.toml"),
        format!("[validation]\nstrictness = \"{strictness}\"\n"),
    )
    .unwrap();
    std::fs::write(storage.root().join("rules.toml"), rules_toml).unwrap();
    let layout = storage.repository_layout();
    CommandExecutor::new(storage).with_layout(layout)
}

/// Seed an in-progress epic with NO design dependency (so the rule under test is
/// violated at a `done` transition) and return its id.
fn seed_epic(executor: &CommandExecutor<InMemoryStorage>) -> String {
    let mut issue = crate::fixture_issue("Epic".to_string(), String::new());
    issue.labels = vec!["type:epic".to_string()];
    issue.state = State::InProgress;
    let id = issue.id.clone();
    executor.storage().save_issue(issue).unwrap();
    id
}

/// The persisted state of `id`.
fn state_of(executor: &CommandExecutor<InMemoryStorage>, id: &str) -> State {
    executor.storage().load_issue(id).unwrap().state
}

// --- warn graph rule: strict blocks the transition, loose does not --------

#[test]
fn test_warn_graph_rule_blocks_transition_under_strict() {
    let executor = executor("strict", DONE_WANTS_DESIGN_WARN);
    let epic = seed_epic(&executor);
    let result = executor.update_issue_state(&epic, State::Done);
    assert!(
        result.is_err(),
        "strict must block the done transition on a warning-only graph violation"
    );
    assert_eq!(
        state_of(&executor, &epic),
        State::InProgress,
        "a blocked transition persists nothing"
    );
}

#[test]
fn test_warn_graph_rule_does_not_block_transition_under_loose() {
    let executor = executor("loose", DONE_WANTS_DESIGN_WARN);
    let epic = seed_epic(&executor);
    let result = executor.update_issue_state(&epic, State::Done);
    assert!(
        result.is_ok(),
        "loose must allow the transition; a warning does not block: {result:?}"
    );
    assert_eq!(state_of(&executor, &epic), State::Done);
}

// --- enforce graph rule: loose blocks, permissive downgrades to advisory --

#[test]
fn test_enforced_graph_rule_blocks_transition_under_loose() {
    let executor = executor("loose", DONE_NEEDS_DESIGN_ENFORCE);
    let epic = seed_epic(&executor);
    let result = executor.update_issue_state(&epic, State::Done);
    assert!(
        result.is_err(),
        "loose must block the done transition on an enforced-error graph violation"
    );
    assert_eq!(state_of(&executor, &epic), State::InProgress);
}

#[test]
fn test_enforced_graph_rule_allowed_under_permissive() {
    let executor = executor("permissive", DONE_NEEDS_DESIGN_ENFORCE);
    let epic = seed_epic(&executor);
    let result = executor.update_issue_state(&epic, State::Done);
    assert!(
        result.is_ok(),
        "permissive must downgrade even an enforced-error graph violation: {result:?}"
    );
    assert_eq!(state_of(&executor, &epic), State::Done);
}
