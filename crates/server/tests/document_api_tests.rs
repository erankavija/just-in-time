//! Integration tests for document API endpoints

use axum_test::TestServer;
use jit::commands::CommandExecutor;
use jit::domain::Priority;
use jit::storage::InMemoryStorage;
use std::sync::Arc;

/// Helper to create test server with initialized storage
async fn create_test_server() -> TestServer {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().expect("Failed to init");

    let app = jit_server::routes::create_routes(Arc::new(executor));
    TestServer::new(app).expect("Failed to create test server")
}

/// Helper to create test server with a test issue
async fn create_test_server_with_issue() -> (TestServer, String) {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().expect("Failed to init");

    // Create a test issue
    let issue_id = executor
        .create_issue(
            "Test Issue".to_string(),
            "Test description".to_string(),
            Priority::High,
            vec![],
        )
        .expect("Failed to create issue");

    let app = jit_server::routes::create_routes(Arc::new(executor));
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
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().expect("Failed to init");

    // Create issue with document reference
    let issue_id = executor
        .create_issue(
            "Test Issue".to_string(),
            "Test".to_string(),
            Priority::High,
            vec![],
        )
        .expect("Failed to create issue");

    // Add document reference (if method exists)
    // For now, this will fail because we need the document to exist

    let app = jit_server::routes::create_routes(Arc::new(executor));
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
