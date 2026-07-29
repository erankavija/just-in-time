//! Integration tests for document API endpoints

use axum_test::TestServer;
use jit::commands::CommandExecutor;
use jit::domain::Priority;
use jit::storage::InMemoryStorage;
use jit_server::routes::AppState;
use jit_server::watcher::ChangeTracker;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

fn test_memory_storage() -> InMemoryStorage {
    let storage = InMemoryStorage::new();
    storage.add_data_file("config.toml", "[worktree]\nenforce_leases = \"off\"\n");
    storage.add_data_file(
        "index.json",
        r#"{"schema_version":2,"all_ids":[],"deleted_ids":[]}"#,
    );
    storage
}

/// Helper to create a test server with empty storage.
async fn create_test_server() -> TestServer {
    let storage = test_memory_storage();
    let layout = storage.repository_layout();
    let executor = CommandExecutor::new(storage).with_layout(layout);

    let state = AppState {
        executor: Arc::new(executor),
        tracker: Arc::new(ChangeTracker::new(16)),
        project_name: "test-project".to_string(),
        shutdown: CancellationToken::new(),
    };
    let app = jit_server::routes::create_routes(state);
    TestServer::new(app).expect("Failed to create test server")
}

/// Helper to create test server with a test issue
async fn create_test_server_with_issue() -> (TestServer, String) {
    let storage = test_memory_storage();
    let layout = storage.repository_layout();
    let executor = CommandExecutor::new(storage).with_layout(layout);

    // Create a test issue
    let (issue_id, _) = executor
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
        .expect("Failed to create issue");

    let state = AppState {
        executor: Arc::new(executor),
        tracker: Arc::new(ChangeTracker::new(16)),
        project_name: "test-project".to_string(),
        shutdown: CancellationToken::new(),
    };
    let app = jit_server::routes::create_routes(state);
    let server = TestServer::new(app).expect("Failed to create test server");

    (server, issue_id)
}

#[tokio::test]
async fn test_get_document_content_missing_issue() {
    let server = create_test_server().await;

    let response = server
        .get("/issues/nonexistent/documents/README.md/content")
        .await;

    response.assert_status_not_found();
}

#[tokio::test]
async fn test_get_document_content_missing_document() {
    let (server, issue_id) = create_test_server_with_issue().await;

    let response = server
        .get(&format!("/issues/{}/documents/README.md/content", issue_id))
        .await;

    // Document not found in issue, expecting 404
    response.assert_status_not_found();
}

#[tokio::test]
async fn test_get_document_content_not_yet_implemented() {
    let storage = test_memory_storage();
    let layout = storage.repository_layout();
    let executor = CommandExecutor::new(storage).with_layout(layout);

    // Create issue with document reference
    let (issue_id, _) = executor
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
        .expect("Failed to create issue");

    // Add document reference (if method exists)
    // For now, this will fail because we need the document to exist

    let state = AppState {
        executor: Arc::new(executor),
        tracker: Arc::new(ChangeTracker::new(16)),
        project_name: "test-project".to_string(),
        shutdown: CancellationToken::new(),
    };
    let app = jit_server::routes::create_routes(state);
    let server = TestServer::new(app).expect("Failed to create test server");

    // Without a document, we get 404
    let response = server
        .get(&format!("/issues/{}/documents/README.md/content", issue_id))
        .await;

    response.assert_status_not_found();
}

#[tokio::test]
async fn test_get_document_history_missing_issue() {
    let server = create_test_server().await;

    let response = server
        .get("/issues/nonexistent/documents/README.md/history")
        .await;

    response.assert_status_not_found();
}

#[tokio::test]
async fn test_get_document_diff_missing_issue() {
    let server = create_test_server().await;

    let response = server
        .get("/issues/nonexistent/documents/README.md/diff?from=abc123")
        .await;

    response.assert_status_not_found();
}
