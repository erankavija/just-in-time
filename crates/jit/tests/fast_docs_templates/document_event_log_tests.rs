//! Event-log coverage for document reference mutations (@/inv/event-log)
//!
//! `jit doc add` and `jit doc remove` mutate an issue's `documents` field, so
//! each must append an `issue_updated` event with `fields: ["documents"]`,
//! mirroring `issue update` / `bulk_update` / dependency commands.

mod harness;
use harness::TestHarness;
use jit::domain::Event;
use jit::storage::IssueStore;

/// Assert the last event is `issue_updated` for `issue_id` with the expected
/// actor and `fields` containing `documents`.
fn assert_last_event_is_documents_update(h: &TestHarness, issue_id: &str, expected_actor: &str) {
    let events = h.storage.read_events().unwrap();
    let event = events.last().expect("event log should not be empty");

    assert_eq!(event.get_type(), "issue_updated");
    assert_eq!(event.get_issue_id(), issue_id);

    match event {
        Event::IssueUpdated {
            updated_by, fields, ..
        } => {
            assert_eq!(updated_by, expected_actor);
            assert!(
                fields.contains(&"documents".to_string()),
                "fields should contain 'documents', got {:?}",
                fields
            );
        }
        other => panic!("Expected IssueUpdated event, got {:?}", other),
    }
}

#[test]
fn test_add_document_reference_logs_issue_updated_event() {
    let h = TestHarness::new();
    let id = h.create_issue("Doc add event");

    h.executor
        .add_document_reference(&id, "docs/spec.md", None, None, None, true)
        .unwrap();

    assert_last_event_is_documents_update(&h, &id, "doc-add");
}

#[test]
fn test_remove_document_reference_logs_issue_updated_event() {
    let h = TestHarness::new();
    let id = h.create_issue("Doc remove event");

    h.executor
        .add_document_reference(&id, "docs/spec.md", None, None, None, true)
        .unwrap();

    h.executor
        .remove_document_reference(&id, "docs/spec.md")
        .unwrap();

    assert_last_event_is_documents_update(&h, &id, "doc-remove");
}
