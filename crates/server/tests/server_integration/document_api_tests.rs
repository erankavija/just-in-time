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

/// A Git repository committing `pinned` then `explicit` at `path`, leaving
/// uncommitted working-tree bytes, served with one issue linking `path` at the
/// first commit. Returns the repository guard, server, issue and both commits.
fn serve_two_document_versions(
    path: &str,
    pinned: &[u8],
    explicit: &[u8],
) -> (
    tempfile::TempDir,
    TestServer,
    jit::domain::identity::IssueId,
    String,
    String,
) {
    let (repo, executor) =
        jit::test_utils::setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n")
            .unwrap();
    let git = jit::test_harness::git::GitRepository::init(repo.path());
    let pin = git.write(path, pinned).commit("Pinned version");
    let at = git.write(path, explicit).commit("Explicit version");
    git.write(path, b"working\n");
    let paths =
        jit::storage::WorktreePaths::detect_for_data_root(&repo.path().join(".jit"), repo.path())
            .unwrap();
    let executor = executor.with_worktree_paths(paths);
    let id = executor
        .issues()
        .create_issue(
            "Selected versions".into(),
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
    executor
        .documents()
        .add_document_reference(&id, path, Some(&pin), None, None, true)
        .unwrap();
    (repo, serve(executor), id, pin, at)
}

/// The raw route serves the requested version's bytes unaltered.
#[tokio::test]
async fn test_get_document_raw_serves_requested_version_bytes() {
    let path = "artifact.dat";
    let (_repo, server, id, _pin, at) =
        serve_two_document_versions(path, b"pinned\0\xff\n", b"explicit\0\xfe\n");

    let raw = server
        .get(&format!("/issues/{id}/documents/{path}/raw"))
        .add_query_param("commit", &at)
        .await;

    raw.assert_status_ok();
    assert_eq!(raw.as_bytes().as_ref(), b"explicit\0\xfe\n");
}

/// The diff route compares the two requested versions.
#[tokio::test]
async fn test_get_document_diff_compares_requested_versions() {
    let path = "notes.txt";
    let (_repo, server, id, pin, at) =
        serve_two_document_versions(path, b"pinned\n", b"explicit\n");

    let diff = server
        .get(&format!("/issues/{id}/documents/{path}/diff"))
        .add_query_param("from", &pin)
        .add_query_param("to", &at)
        .await;

    diff.assert_status_ok();
    let diff: serde_json::Value = diff.json();
    let text = diff["diff"].as_str().unwrap();
    assert!(text.contains("-pinned"), "{text}");
    assert!(text.contains("+explicit"), "{text}");
}
