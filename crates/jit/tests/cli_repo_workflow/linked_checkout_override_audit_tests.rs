//! Durability coverage for the linked-checkout override audit record (jit:0c8d38be) — the
//! record and the mutation it permits become durable through one materialization plan, or
//! neither does.

use jit::commands::CommandExecutor;
use jit::domain::{Event, LinkedCheckoutWriteStance, Priority};
use jit::storage::{
    IssueStore, JsonFileStorage, RepositoryStateStore, TransactionFailureInjector,
    TransactionFailurePoint,
};
use std::path::Path;
use std::sync::Arc;

fn override_annotation(repository: &Path) -> jit::repository_state::MutationContextAnnotation {
    jit::repository_state::MutationContextAnnotation::LinkedCheckoutWriteOverridden {
        checkout: repository.join(".worktrees/feature"),
        declared_stance: LinkedCheckoutWriteStance::Refuse,
        invocation_override: LinkedCheckoutWriteStance::Allow,
    }
}

/// Fails the transaction kernel at the commit decision, after the mutation's
/// coupled issue and event materialization has been staged but before it commits.
struct FailAtCommitDecision;

impl TransactionFailureInjector for FailAtCommitDecision {
    fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
        if point == &TransactionFailurePoint::RepositoryBeforeCommitDecision {
            return Err(std::io::Error::other(
                "injected override-audit persistence failure",
            ));
        }
        Ok(())
    }
}

#[test]
fn test_create_issue_under_dispatch_override_persists_the_audit_record_with_the_issue_events() {
    let (temp, storage, taxonomy) = jit::test_utils::setup_test_repo_with_taxonomy()
        .expect("initialize the fixture repository");
    let layout = jit::storage::discover_repository_layout(temp.path(), storage.root())
        .expect("discover the repository layout");
    let executor = CommandExecutor::new(storage.clone()).with_layout(layout);
    let events_before = storage
        .read_events()
        .expect("read the event log before the permitted mutation");
    let expected_checkout = temp.path().join(".worktrees/feature");
    let expected_declared_stance = LinkedCheckoutWriteStance::Refuse;
    let expected_invocation_override = LinkedCheckoutWriteStance::Allow;
    let annotation = override_annotation(temp.path());

    let (issue_id, _) = jit::commands::with_dispatch_mutation_annotation(annotation, || {
        executor.create_issue(
            "Permitted mutation".to_string(),
            "Issue created while an override permitted the invocation".to_string(),
            Priority::Normal,
            Vec::new(),
            vec![format!("type:{}", taxonomy.type_at_level(4))],
            None,
            None,
            false,
        )
    })
    .expect("the permitted mutation succeeds");

    let events_after = storage
        .read_events()
        .expect("read the event log after the permitted mutation");
    assert!(
        events_after.starts_with(&events_before),
        "the permitted mutation must preserve the prior event-log prefix"
    );
    let appended = &events_after[events_before.len()..];
    let override_records = appended
        .iter()
        .filter(|event| matches!(event, Event::LinkedCheckoutWriteOverridden { .. }))
        .collect::<Vec<_>>();
    assert_eq!(
        override_records.len(),
        1,
        "the permitted mutation must append exactly one linked-checkout override audit record"
    );
    let Event::LinkedCheckoutWriteOverridden {
        id,
        timestamp,
        checkout,
        declared_stance,
        invocation_override,
    } = override_records[0]
    else {
        unreachable!("the selected event is the linked-checkout override audit record");
    };
    assert_eq!(
        checkout, &expected_checkout,
        "the audit record must name the checkout selected by the dispatch site"
    );
    assert_eq!(
        declared_stance, &expected_declared_stance,
        "the audit record must preserve the declared linked-checkout write stance"
    );
    assert_eq!(
        invocation_override, &expected_invocation_override,
        "the audit record must preserve the invocation stance that permitted the mutation"
    );
    assert!(
        !id.is_empty(),
        "the finalizer must assign a non-empty identifier to the audit record"
    );

    let issue_created_timestamp = appended
        .iter()
        .find_map(|event| match event {
            Event::IssueCreated {
                issue_id: id,
                timestamp,
                ..
            } if id == &issue_id => Some(timestamp),
            _ => None,
        })
        .expect("the issue-created event must be in the same append as the audit record");
    assert_eq!(
        timestamp, issue_created_timestamp,
        "the audit record and issue-created event must share one mutation timestamp"
    );
    assert!(
        matches!(
            appended.last(),
            Some(Event::LinkedCheckoutWriteOverridden { .. })
        ),
        "canonical event ordering must place the override audit record last in the append"
    );
    storage
        .load_issue(&issue_id)
        .expect("the mutation explained by the audit record must be durable");
}

#[test]
fn test_create_issue_under_dispatch_override_persistence_failure_leaves_neither_issue_nor_audit_record(
) {
    let (temp, storage, taxonomy) = jit::test_utils::setup_test_repo_with_taxonomy()
        .expect("initialize the fixture repository");
    let layout = jit::storage::discover_repository_layout(temp.path(), storage.root())
        .expect("discover the repository layout");
    let events_before = storage
        .read_events()
        .expect("read the event log before the failed mutation");
    let issues_before = storage
        .read_issues()
        .expect("read the issue set before the failed mutation");
    let jit_dir = temp.path().join(".jit");
    let injected =
        JsonFileStorage::with_repository_state_failures(&jit_dir, Arc::new(FailAtCommitDecision));
    let executor = CommandExecutor::new(injected).with_layout(layout);
    let annotation = override_annotation(temp.path());

    let error = jit::commands::with_dispatch_mutation_annotation(annotation, || {
        executor.create_issue(
            "Permitted mutation".to_string(),
            "Issue created while an override permitted the invocation".to_string(),
            Priority::Normal,
            Vec::new(),
            vec![format!("type:{}", taxonomy.type_at_level(4))],
            None,
            None,
            false,
        )
    })
    .expect_err("an injected persistence failure must not be reported as a completed mutation");
    let error_text = format!("{error:#}");
    assert!(
        error_text.contains("injected override-audit persistence failure"),
        "the surfaced error must identify the injected persistence failure; error: {error_text}"
    );

    let recovered = JsonFileStorage::new(&jit_dir);
    let recovered_layout = jit::storage::discover_repository_layout(temp.path(), recovered.root())
        .expect("rediscover the repository layout for recovery");
    drop(
        RepositoryStateStore::open_mutation_session(&recovered, recovered_layout)
            .expect("recovery converges"),
    );

    let recovered_events = recovered
        .read_events()
        .expect("read the event log after persistence recovery");
    assert_eq!(
        recovered_events, events_before,
        "a failed materialization must leave neither the audit record nor issue events durable"
    );
    let recovered_issues = recovered
        .read_issues()
        .expect("read the issue set after persistence recovery");
    assert_eq!(
        recovered_issues.len(),
        issues_before.len(),
        "a failed materialization must leave the durable issue count unchanged"
    );
    assert!(
        !recovered_events
            .iter()
            .any(|event| matches!(event, Event::LinkedCheckoutWriteOverridden { .. })),
        "the audit record cannot outlive the mutation it explains"
    );
}
