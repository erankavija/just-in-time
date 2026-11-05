//! Integration tests for document API endpoints

use axum_test::TestServer;
use jit::commands::test_helpers::setup;
use jit::commands::CommandExecutor;
use jit::domain::Priority;
use jit::storage::InMemoryStorage;
use jit_server::routes::AppState;
use jit_server::watcher::ChangeTracker;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

fn serve(executor: CommandExecutor<InMemoryStorage>) -> TestServer {
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
