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
// updates that entry in place instead of appending a duplicate. The commit
// pin always reflects the invocation, as on a fresh add (supplied pins,
// omitted records unpinned, re-pointing a stale pin). `label` and `doc_type`
// follow the partial-update convention of `issue update` — an omitted flag
// (`None`) leaves the existing value alone rather than clearing it — while
// the scanned `format`/`assets` are always the freshly computed result,
// mirroring a fresh add.

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
fn test_add_document_reference_same_path_identical_is_noop() {
    let h = TestHarness::new();
    let id = h.create_issue("Doc re-add event tag");

    h.executor
        .add_document_reference(&id, "docs/spec.md", None, None, None, true)
        .unwrap();
    assert_last_event_is_documents_update(&h, &id, "doc-add");

    let issue_before = h.storage.load_issue(&id).unwrap();
    let events_before = h.storage.read_events().unwrap();
    h.executor
        .add_document_reference(&id, "docs/spec.md", None, None, None, true)
        .unwrap();
    let issue_after = h.storage.load_issue(&id).unwrap();
    assert_eq!(issue_after.updated_at, issue_before.updated_at);
    assert_eq!(issue_after.documents, issue_before.documents);
    assert_eq!(h.storage.read_events().unwrap(), events_before);
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
    assert_last_event_is_documents_update(&h, &id, "doc-update");

    let listed = h.executor.list_document_references(&id).unwrap();
    assert_eq!(listed.documents.len(), 1);
    assert_eq!(listed.documents[0].label.as_deref(), Some("Final"));
}

#[test]
fn test_add_document_reference_same_path_refreshes_pin_preserves_metadata() {
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

    // Re-add with no metadata flags: descriptive metadata (label/doc_type)
    // survives, while the commit pin behaves exactly as on a fresh add — an
    // omitted --commit records the reference unpinned, re-pointing a stale pin
    // at the current version instead of preserving it (the steward re-run
    // workflow: refresh the link after the document changed).
    let (result, _) = h
        .executor
        .add_document_reference(&id, "docs/spec.md", None, None, None, true)
        .unwrap();

    assert_eq!(
        result.document.commit, None,
        "an omitted --commit on re-add must shed the stale pin, not preserve it"
    );
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

#[test]
fn test_add_pinned_document_uses_pinned_document_and_asset_bytes() {
    use jit::commands::CommandExecutor;
    use jit::storage::{discover_repository_layout, JsonFileStorage};
    use sha2::{Digest, Sha256};

    let temp = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(temp.path()).unwrap();
    std::fs::create_dir_all(temp.path().join("docs")).unwrap();
    std::fs::write(temp.path().join("docs/guide.md"), "![pinned](./logo.png)\n").unwrap();
    std::fs::write(temp.path().join("docs/logo.png"), b"pinned asset").unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_path(std::path::Path::new("docs/guide.md"))
        .unwrap();
    index
        .add_path(std::path::Path::new("docs/logo.png"))
        .unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let signature = git2::Signature::now("JIT test", "jit@example.invalid").unwrap();
    let revision = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "pinned document",
            &tree,
            &[],
        )
        .unwrap()
        .to_string();

    let storage = JsonFileStorage::new(temp.path().join(".jit"));
    storage.init().unwrap();
    let issue = crate::fixture_issue("Pinned docs".into(), String::new());
    let id = issue.id.clone();
    storage.save_issue(issue).unwrap();
    let layout = discover_repository_layout(temp.path(), storage.root()).unwrap();
    let executor = CommandExecutor::new(storage.clone()).with_layout(layout);

    std::fs::write(
        temp.path().join("docs/guide.md"),
        "![working](./working.png)\n",
    )
    .unwrap();
    std::fs::write(temp.path().join("docs/logo.png"), b"working asset").unwrap();
    std::fs::write(temp.path().join("docs/working.png"), b"working only").unwrap();

    let (result, warnings) = executor
        .add_document_reference(&id, "docs/guide.md", Some(&revision), None, None, false)
        .unwrap();

    assert!(warnings.is_empty());
    assert_eq!(result.document.assets.len(), 1);
    let asset = &result.document.assets[0];
    let pinned_hash = format!("{:x}", Sha256::digest(b"pinned asset"));
    assert_eq!(asset.original_path, "./logo.png");
    assert_eq!(asset.asset_type, jit::document::AssetType::Local);
    assert_eq!(asset.content_hash.as_deref(), Some(pinned_hash.as_str()));
}

#[test]
fn test_add_document_rejects_asset_closure_over_fixed_budget_without_writes() {
    let h = TestHarness::new();
    let id = h.create_issue("Bounded document scan");
    let content = (0..300)
        .map(|index| format!("![asset](./asset-{index}.png)"))
        .collect::<Vec<_>>()
        .join("\n");
    h.storage.add_repo_file("docs/guide.md", &content);
    let issue_before = h.storage.load_issue(&id).unwrap();
    let events_before = h.storage.read_events().unwrap();

    let error = h
        .executor
        .add_document_reference(&id, "docs/guide.md", None, None, None, false)
        .unwrap_err();

    assert!(matches!(
        error.downcast_ref::<jit::repository_state::CaptureError>(),
        Some(jit::repository_state::CaptureError::PathBudgetExceeded {
            actual: 303,
            maximum: 256
        })
    ));
    assert_eq!(h.storage.load_issue(&id).unwrap(), issue_before);
    assert_eq!(h.storage.read_events().unwrap(), events_before);
}

#[test]
fn test_add_document_rejects_malformed_utf8_without_writes() {
    use jit::commands::CommandExecutor;
    use jit::storage::{discover_repository_layout, JsonFileStorage};

    let temp = tempfile::tempdir().unwrap();
    let storage = JsonFileStorage::new(temp.path().join(".jit"));
    storage.init().unwrap();
    let issue = crate::fixture_issue("Malformed document".into(), String::new());
    let id = issue.id.clone();
    storage.save_issue(issue).unwrap();
    std::fs::create_dir_all(temp.path().join("docs")).unwrap();
    std::fs::write(temp.path().join("docs/guide.md"), [0xff, 0xfe]).unwrap();
    let layout = discover_repository_layout(temp.path(), storage.root()).unwrap();
    let executor = CommandExecutor::new(storage.clone()).with_layout(layout);
    let issue_before = storage.load_issue(&id).unwrap();
    let events_before = storage.read_events().unwrap();

    let error = executor
        .add_document_reference(&id, "docs/guide.md", None, None, None, false)
        .unwrap_err();

    assert!(error.to_string().contains("is not UTF-8"));
    assert_eq!(storage.load_issue(&id).unwrap(), issue_before);
    assert_eq!(storage.read_events().unwrap(), events_before);
}
