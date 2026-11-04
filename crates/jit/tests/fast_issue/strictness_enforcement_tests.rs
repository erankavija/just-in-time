//! End-to-end enforcement of the `[validation].strictness` modulator (issue
//! afbbf6c3).
//!
//! These tests drive the real [`CommandExecutor`] against [`InMemoryStorage`]
//! with a `config.toml` selecting a strictness level and a `.jit/rules.toml`
//! carrying ONE rule, then exercise the write path (`create_issue`) end to end.
//! They prove each level produces a DISTINCT block/allow outcome on the SAME
//! violation set, so no accepted strictness value is silently inert (REQ-03):
//!
//! - a warning rule's violation blocks only under the strictest level;
//! - an enforced error rule's violation blocks under every level except the
//!   most permissive, which downgrades it to advisory;
//! - an unrecognized strictness value is rejected rather than silently ignored.

use jit::commands::test_helpers::ExecutorTestAccess;
use jit::commands::test_helpers::{memory_executor_declaring, MemoryDeclarations};
use jit::commands::CommandExecutor;
use jit::domain::Priority;
use jit::storage::{InMemoryStorage, IssueStore};

/// A `warn` rule (never enforces): epics should carry a `req:*` label.
const EPIC_REQ_WARN: &str = r#"
[[rules]]
name = "epic-req-warn"
when = { type = "epic" }
severity = "warn"
assert = { require-label = { label = "req:*", min = 1 } }
"#;

/// An `enforce` error rule: epics MUST carry a `req:*` label.
const EPIC_REQ_ENFORCE: &str = r#"
[[rules]]
name = "epic-req-enforce"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { require-label = { label = "req:*", min = 1 } }
"#;

/// Attempt to create an epic that violates the rule under test (no `req:*`
/// label). Returns the executor result so the caller asserts block/allow.
fn create_violating_epic(
    executor: &CommandExecutor<InMemoryStorage>,
) -> anyhow::Result<jit::commands::IssueCreation> {
    executor.issues().create_issue(
        "An epic".to_string(),
        String::new(),
        Priority::Normal,
        vec![],
        vec!["type:epic".to_string()],
        None,
        None,
        false,
    )
}

// --- warn-only violation: strict blocks, loose/permissive allow ----------

#[test]
fn test_warn_violation_blocks_under_strict() {
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(
                "[worktree]\nenforce_leases = \"off\"\n[validation]\nstrictness = \"strict\"\n",
            ),
            rules: Some(EPIC_REQ_WARN),
            ..MemoryDeclarations::default()
        },
    );
    let result = create_violating_epic(&executor);
    assert!(
        result.is_err(),
        "strict must block a warning-only violation"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("epic-req-warn"),
        "message names the rule: {msg}"
    );
    assert!(
        executor.storage().list_issues().unwrap().is_empty(),
        "nothing persisted on a blocked create"
    );
}

#[test]
fn test_warn_violation_allowed_under_loose() {
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(
                "[worktree]\nenforce_leases = \"off\"\n[validation]\nstrictness = \"loose\"\n",
            ),
            rules: Some(EPIC_REQ_WARN),
            ..MemoryDeclarations::default()
        },
    );
    let result = create_violating_epic(&executor);
    assert!(result.is_ok(), "loose must allow a warning-only violation");
    assert_eq!(executor.storage().list_issues().unwrap().len(), 1);
}

#[test]
fn test_warn_violation_allowed_under_permissive() {
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(
                "[worktree]\nenforce_leases = \"off\"\n[validation]\nstrictness = \"permissive\"\n",
            ),
            rules: Some(EPIC_REQ_WARN),
            ..MemoryDeclarations::default()
        },
    );
    let result = create_violating_epic(&executor);
    assert!(
        result.is_ok(),
        "permissive must allow a warning-only violation"
    );
    assert_eq!(executor.storage().list_issues().unwrap().len(), 1);
}

// --- enforced-error violation: strict/loose block, permissive allows ------

#[test]
fn test_enforced_error_blocks_under_loose() {
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(
                "[worktree]\nenforce_leases = \"off\"\n[validation]\nstrictness = \"loose\"\n",
            ),
            rules: Some(EPIC_REQ_ENFORCE),
            ..MemoryDeclarations::default()
        },
    );
    let result = create_violating_epic(&executor);
    assert!(result.is_err(), "loose must block an enforced error");
    assert!(executor.storage().list_issues().unwrap().is_empty());
}

#[test]
fn test_enforced_error_blocks_under_strict() {
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(
                "[worktree]\nenforce_leases = \"off\"\n[validation]\nstrictness = \"strict\"\n",
            ),
            rules: Some(EPIC_REQ_ENFORCE),
            ..MemoryDeclarations::default()
        },
    );
    let result = create_violating_epic(&executor);
    assert!(result.is_err(), "strict must block an enforced error");
    assert!(executor.storage().list_issues().unwrap().is_empty());
}

#[test]
fn test_enforced_error_allowed_under_permissive() {
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(
                "[worktree]\nenforce_leases = \"off\"\n[validation]\nstrictness = \"permissive\"\n",
            ),
            rules: Some(EPIC_REQ_ENFORCE),
            ..MemoryDeclarations::default()
        },
    );
    let result = create_violating_epic(&executor);
    assert!(
        result.is_ok(),
        "permissive must downgrade even an enforced error to advisory: {result:?}"
    );
    assert_eq!(
        executor.storage().list_issues().unwrap().len(),
        1,
        "the issue is created despite the enforced-error violation"
    );
}

// --- default and misconfiguration ----------------------------------------

#[test]
fn test_absent_strictness_defaults_to_loose() {
    // No `[validation]` section at all: an enforced error still blocks, a warning
    // still passes — the pre-strictness behavior, so no existing repo changes.
    let enforce_exec = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some("[worktree]\nenforce_leases = \"off\"\n"),
            rules: Some(EPIC_REQ_ENFORCE),
            ..MemoryDeclarations::default()
        },
    );
    assert!(
        create_violating_epic(&enforce_exec).is_err(),
        "default (loose) blocks an enforced error"
    );

    let warn_exec = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some("[worktree]\nenforce_leases = \"off\"\n"),
            rules: Some(EPIC_REQ_WARN),
            ..MemoryDeclarations::default()
        },
    );
    assert!(
        create_violating_epic(&warn_exec).is_ok(),
        "default (loose) allows a warning"
    );
}

#[test]
fn test_invalid_strictness_is_rejected_not_ignored() {
    // A selectable-but-unknown value must be an error, never silently ignored.
    // Rejection is eager: the invalid strictness makes the config fail to load,
    // so the operation errors at config load before any rule evaluation — the
    // opposite of a silently-inert value. (That the message names the bad value
    // is proven at the deserialize/`config set` boundaries in the config unit
    // tests, where the error is not re-wrapped by the config-load context.)
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(
                "[worktree]\nenforce_leases = \"off\"\n[validation]\nstrictness = \"banana\"\n",
            ),
            rules: Some(EPIC_REQ_ENFORCE),
            after_initialization: true,
            ..MemoryDeclarations::default()
        },
    );
    let result = create_violating_epic(&executor);
    assert!(result.is_err(), "an invalid strictness must fail the write");
    let chain = format!("{:#}", result.unwrap_err());
    assert!(
        chain.contains("config.toml"),
        "the write fails at config load, not silently: {chain}"
    );
}
