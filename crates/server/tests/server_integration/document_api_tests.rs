//! Integration tests for document API endpoints

use axum_test::TestServer;
use jit::commands::test_helpers::setup;
use jit::commands::CommandExecutor;
use jit::domain::Priority;
use jit::storage::{IssueStore, RepositoryStateStore};
use jit_server::routes::AppState;
use jit_server::watcher::ChangeTracker;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

fn serve<S: IssueStore + RepositoryStateStore + Send + Sync + 'static>(
    executor: CommandExecutor<S>,
) -> TestServer {
    let state = AppState {
        executor: Arc::new(executor),
        tracker: Arc::new(ChangeTracker::new(16)),
        capture_cache: Default::default(),
        project_name: "test-project".to_string(),
        shutdown: CancellationToken::new(),
    };
    TestServer::new(jit_server::routes::create_routes(state)).expect("Failed to create test server")
}

#[tokio::test]
async fn test_get_document_content_missing_issue() {
    let server = serve(setup());

    let response = server
        .get("/issues/nonexistent/documents/README.md/content")
        .await;

    response.assert_status_not_found();
}

#[tokio::test]
async fn test_get_document_content_missing_document() {
    let executor = setup();
    let issue_id = executor
        .issues()
        .create_issue(
            "Test Issue".to_string(),
            "Test description".to_string(),
            Priority::High,
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .expect("Failed to create issue")
        .issue
        .id;
    let server = serve(executor);

    let response = server
        .get(&format!("/issues/{}/documents/README.md/content", issue_id))
        .await;

    // Document not found in issue, expecting 404
    response.assert_status_not_found();
}

#[tokio::test]
async fn test_get_document_content_not_yet_implemented() {
    let executor = setup();

    // Create issue with document reference
    let issue_id = executor
        .issues()
        .create_issue(
            "Test Issue".to_string(),
            "Test".to_string(),
            Priority::High,
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .expect("Failed to create issue")
        .issue
        .id;

    // Add document reference (if method exists)
    // For now, this will fail because we need the document to exist

    let server = serve(executor);

    // Without a document, we get 404
    let response = server
        .get(&format!("/issues/{}/documents/README.md/content", issue_id))
        .await;

    response.assert_status_not_found();
}

#[tokio::test]
async fn test_get_document_history_missing_issue() {
    let server = serve(setup());

    let response = server
        .get("/issues/nonexistent/documents/README.md/history")
        .await;

    response.assert_status_not_found();
}

#[tokio::test]
async fn test_get_document_diff_missing_issue() {
    let server = serve(setup());

    let response = server
        .get("/issues/nonexistent/documents/README.md/diff?from=abc123")
        .await;

    response.assert_status_not_found();
}

/// The raw route serves the requested version's bytes unaltered, and the diff
/// route compares the two requested versions.
#[tokio::test]
async fn test_document_routes_serve_selected_version_bytes_and_diff() {
    let (repo, executor) =
        jit::test_utils::setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n")
            .unwrap();
    let git = jit::test_harness::git::GitRepository::init(repo.path());
    let (path, notes) = ("artifact.dat", "notes.txt");
    let pin = git
        .write(path, b"pinned\0\xff\n")
        .write(notes, "pinned\n")
        .commit("Pinned version");
    let at = git
        .write(path, b"explicit\0\xfe\n")
        .write(notes, "explicit\n")
        .commit("Explicit version");
    git.write(path, b"working\n");
    let paths =
        jit::storage::WorktreePaths::detect_for_data_root(&repo.path().join(".jit"), repo.path())
            .unwrap();
    let executor = executor.with_worktree_paths(paths);
    let id = executor
        .issues()
        .create_issue(
            "Selected bytes".into(),
            String::new(),
            Priority::Normal,
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .unwrap()
        .issue
        .id;
    for document in [path, notes] {
        executor
            .documents()
            .add_document_reference(&id, document, Some(&pin), None, None, true)
            .unwrap();
    }
    let server = serve(executor);
    let base = format!("/issues/{id}/documents");

    let raw = server
        .get(&format!("{base}/{path}/raw"))
        .add_query_param("commit", &at)
        .await;
    raw.assert_status_ok();
    assert_eq!(raw.as_bytes().as_ref(), b"explicit\0\xfe\n");

    let diff = server
        .get(&format!("{base}/{notes}/diff"))
        .add_query_param("from", &pin)
        .add_query_param("to", &at)
        .await;
    diff.assert_status_ok();
    let diff: serde_json::Value = diff.json();
    let text = diff["diff"].as_str().unwrap();
    assert!(text.contains("-pinned"), "{text}");
    assert!(text.contains("+explicit"), "{text}");
}
