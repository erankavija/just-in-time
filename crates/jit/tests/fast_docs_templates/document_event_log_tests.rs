//! Event-log coverage for document reference mutations (@/inv/event-log)
//!
//! `jit doc add` and `jit doc remove` mutate an issue's `documents` field, so
//! each must append an `issue_updated` event with `fields: ["documents"]`,
//! mirroring `issue update` / `bulk_update` / dependency commands.
use crate::harness::TestHarness;
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

// ========== Idempotent re-add (jit:8917c558) ==========
//
// Identity is (issue, path): re-adding a path already linked to the issue
// updates that entry in place instead of appending a duplicate. `commit`,
// `label`, and `doc_type` follow the same partial-update convention as
// `issue update` — an omitted flag (`None`) leaves the existing value alone
// rather than clearing it — while the scanned `format`/`assets` are always
// the freshly computed result, mirroring a fresh add.

#[test]
fn test_add_document_reference_same_path_updates_in_place() {
    let h = TestHarness::new();
    let id = h.create_issue("Doc re-add dedup");

    let (first, _) = h
        .executor
        .add_document_reference(&id, "docs/spec.md", None, None, None, true)
        .unwrap();
    assert!(
        !first.updated,
        "first add of a new path must not be flagged updated"
    );

    let (second, _) = h
        .executor
        .add_document_reference(&id, "docs/spec.md", None, None, None, true)
        .unwrap();
    assert!(
        second.updated,
        "re-add of an already-linked path must be flagged updated"
    );

    let listed = h.executor.list_document_references(&id).unwrap();
    assert_eq!(
        listed.documents.len(),
        1,
        "re-adding the same path must not duplicate the reference"
    );
    assert_eq!(listed.documents[0].path, "docs/spec.md");
}

#[test]
fn test_add_document_reference_same_path_logs_update_event_not_add() {
    let h = TestHarness::new();
    let id = h.create_issue("Doc re-add event tag");

    h.executor
        .add_document_reference(&id, "docs/spec.md", None, None, None, true)
        .unwrap();
    assert_last_event_is_documents_update(&h, &id, "doc-add");

    h.executor
        .add_document_reference(&id, "docs/spec.md", None, None, None, true)
        .unwrap();
    assert_last_event_is_documents_update(&h, &id, "doc-update");

    // Exactly one `doc-add` and one `doc-update` tag among the `documents`
    // mutations - not two `doc-add`s.
    let events = h.storage.read_events().unwrap();
    let doc_tags: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::IssueUpdated {
                updated_by, fields, ..
            } if fields.contains(&"documents".to_string()) => Some(updated_by.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(doc_tags, vec!["doc-add", "doc-update"]);
}

#[test]
fn test_add_document_reference_same_path_refreshes_given_metadata() {
    let h = TestHarness::new();
    let id = h.create_issue("Doc re-add metadata refresh");

    h.executor
        .add_document_reference(
            &id,
            "docs/spec.md",
            None,
            Some("Draft"),
            Some("design"),
            true,
        )
        .unwrap();

    let (result, _) = h
        .executor
        .add_document_reference(
            &id,
            "docs/spec.md",
            Some("abc1234"),
            Some("Final"),
            Some("spec"),
            true,
        )
        .unwrap();

    assert_eq!(result.document.commit.as_deref(), Some("abc1234"));
    assert_eq!(result.document.label.as_deref(), Some("Final"));
    assert_eq!(result.document.doc_type.as_deref(), Some("spec"));

    let listed = h.executor.list_document_references(&id).unwrap();
    assert_eq!(listed.documents.len(), 1);
    assert_eq!(listed.documents[0].label.as_deref(), Some("Final"));
}

#[test]
fn test_add_document_reference_same_path_preserves_omitted_metadata() {
    let h = TestHarness::new();
    let id = h.create_issue("Doc re-add metadata preserved");

    h.executor
        .add_document_reference(
            &id,
            "docs/spec.md",
            Some("abc1234"),
            Some("Draft"),
            Some("design"),
            true,
        )
        .unwrap();

    // Re-add with no metadata flags: the prior label/doc_type/commit must
    // survive rather than being blanked out.
    let (result, _) = h
        .executor
        .add_document_reference(&id, "docs/spec.md", None, None, None, true)
        .unwrap();

    assert_eq!(result.document.commit.as_deref(), Some("abc1234"));
    assert_eq!(result.document.label.as_deref(), Some("Draft"));
    assert_eq!(result.document.doc_type.as_deref(), Some("design"));
}

#[test]
fn test_add_document_reference_new_path_appends() {
    let h = TestHarness::new();
    let id = h.create_issue("Doc add distinct paths");

    h.executor
        .add_document_reference(&id, "docs/spec.md", None, None, None, true)
        .unwrap();
    h.executor
        .add_document_reference(&id, "docs/other.md", None, None, None, true)
        .unwrap();

    let listed = h.executor.list_document_references(&id).unwrap();
    assert_eq!(
        listed.documents.len(),
        2,
        "distinct paths must both be kept"
    );

    let paths: Vec<&str> = listed.documents.iter().map(|d| d.path.as_str()).collect();
    assert!(paths.contains(&"docs/spec.md"));
    assert!(paths.contains(&"docs/other.md"));

    // Both are genuine adds, not updates.
    let events = h.storage.read_events().unwrap();
    let add_tags: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::IssueUpdated { updated_by, .. } => Some(updated_by.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(add_tags, vec!["doc-add", "doc-add"]);
}
