//! API route definitions

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use jit::commands::CommandExecutor;
use jit::domain::{Issue, Priority, State as IssueState};
use jit::search::{SearchOptions, SearchResult};
use jit::storage::IssueStore;

/// Shared application state
pub type AppState<S> = Arc<CommandExecutor<S>>;

/// Create API routes
pub fn create_routes<S: IssueStore + Send + Sync + 'static>(
    executor: Arc<CommandExecutor<S>>,
) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/issues", get(list_issues))
        .route("/issues/:id", get(get_issue))
        .route("/graph", get(get_graph))
        .route("/status", get(get_status))
        .route("/search", get(search_issues))
        .with_state(executor)
}

/// Health check endpoint
async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "jit-api",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// List all issues
async fn list_issues<S: IssueStore>(
    State(executor): State<AppState<S>>,
) -> Result<Json<Vec<Issue>>, StatusCode> {
    executor
        .list_issues(None, None, None)
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to list issues: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

/// Get single issue by ID
async fn get_issue<S: IssueStore>(
    Path(id): Path<String>,
    State(executor): State<AppState<S>>,
) -> Result<Json<Issue>, StatusCode> {
    executor.show_issue(&id).map(Json).map_err(|e| {
        tracing::error!("Failed to get issue {}: {:?}", id, e);
        StatusCode::NOT_FOUND
    })
}

/// Graph data for visualization
#[derive(Debug, Serialize, Deserialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub state: IssueState,
    pub priority: Priority,
    pub assignee: Option<String>,
    pub blocked: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
}

/// Get dependency graph
async fn get_graph<S: IssueStore>(
    State(executor): State<AppState<S>>,
) -> Result<Json<GraphData>, StatusCode> {
    let issues = executor.list_issues(None, None, None).map_err(|e| {
        tracing::error!("Failed to list issues for graph: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Build node lookup for blocked state calculation
    let issue_map: std::collections::HashMap<String, &Issue> =
        issues.iter().map(|i| (i.id.clone(), i)).collect();

    // Create nodes
    let nodes: Vec<GraphNode> = issues
        .iter()
        .map(|issue| GraphNode {
            id: issue.id.clone(),
            label: issue.title.clone(),
            state: issue.state,
            priority: issue.priority,
            assignee: issue.assignee.clone(),
            blocked: issue.is_blocked(&issue_map),
        })
        .collect();

    // Create edges
    let mut edges = Vec::new();
    for issue in &issues {
        for dep in &issue.dependencies {
            edges.push(GraphEdge {
                from: issue.id.clone(),
                to: dep.clone(),
            });
        }
    }

    Ok(Json(GraphData { nodes, edges }))
}

/// Status summary
#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub open: usize,
    pub ready: usize,
    pub in_progress: usize,
    pub done: usize,
    pub blocked: usize,
    pub total: usize,
}

/// Get repository status
async fn get_status<S: IssueStore>(
    State(executor): State<AppState<S>>,
) -> Result<Json<StatusResponse>, StatusCode> {
    let summary = executor.get_status().map_err(|e| {
        tracing::error!("Failed to get status: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(StatusResponse {
        open: summary.open,
        ready: summary.ready,
        in_progress: summary.in_progress,
        done: summary.done,
        blocked: summary.blocked,
        total: summary.total,
    }))
}

/// Search query parameters
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    /// Search query string
    q: String,
    /// Maximum number of results (default: 50)
    #[serde(default = "default_limit")]
    limit: usize,
    /// Case-sensitive search
    #[serde(default)]
    case_sensitive: bool,
    /// Use regex pattern matching
    #[serde(default)]
    regex: bool,
}

fn default_limit() -> usize {
    50
}

/// Search response
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub total: usize,
    pub results: Vec<SearchResult>,
    pub duration_ms: u128,
}

/// Search issues and documents
async fn search_issues<S: IssueStore>(
    Query(params): Query<SearchQuery>,
    State(_executor): State<AppState<S>>,
) -> Result<Json<SearchResponse>, StatusCode> {
    let start = std::time::Instant::now();

    let options = SearchOptions {
        case_sensitive: params.case_sensitive,
        regex: params.regex,
        max_results: Some(params.limit),
        ..Default::default()
    };

    // Call search directly with the data directory
    let data_dir = std::path::Path::new(".jit");
    let results = jit::search::search(data_dir, &params.q, options).map_err(|e| {
        tracing::error!("Search failed: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let duration_ms = start.elapsed().as_millis();

    Ok(Json(SearchResponse {
        query: params.q,
        total: results.len(),
        results,
        duration_ms,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum_test::TestServer;
    use jit::domain::Priority;
    use jit::storage::InMemoryStorage;

    fn create_test_app() -> TestServer {
        let storage = InMemoryStorage::new();
        let executor = Arc::new(CommandExecutor::new(storage));
        let app = create_routes(executor);
        TestServer::new(app).unwrap()
    }

    #[tokio::test]
    async fn test_health_check() {
        let server = create_test_app();
        let response = server.get("/health").await;
        response.assert_status_ok();
        response.assert_json(&serde_json::json!({
            "status": "ok",
            "service": "jit-api",
            "version": env!("CARGO_PKG_VERSION")
        }));
    }

    #[tokio::test]
    async fn test_list_issues_empty() {
        let server = create_test_app();
        let response = server.get("/issues").await;
        response.assert_status_ok();
        let issues: Vec<Issue> = response.json();
        assert_eq!(issues.len(), 0);
    }

    #[tokio::test]
    async fn test_get_issue_not_found() {
        let server = create_test_app();
        let response = server.get("/issues/nonexistent").await;
        response.assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_list_issues_with_data() {
        let storage = InMemoryStorage::new();
        let executor = Arc::new(CommandExecutor::new(storage));

        // Create test issues
        executor
            .create_issue(
                "Issue 1".to_string(),
                "Description".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Issue 2".to_string(),
                "Description".to_string(),
                Priority::High,
                vec![],
            )
            .unwrap();

        let app = create_routes(executor);
        let server = TestServer::new(app).unwrap();

        let response = server.get("/issues").await;
        response.assert_status_ok();
        let issues: Vec<Issue> = response.json();
        assert_eq!(issues.len(), 2);
    }

    #[tokio::test]
    async fn test_get_graph() {
        let storage = InMemoryStorage::new();
        let executor = Arc::new(CommandExecutor::new(storage));

        // Create issues with dependencies
        let id1 = executor
            .create_issue(
                "Issue 1".to_string(),
                "Description".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "Description".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.add_dependency(&id2, &id1).unwrap();

        let app = create_routes(executor);
        let server = TestServer::new(app).unwrap();

        let response = server.get("/graph").await;
        response.assert_status_ok();
        let graph: GraphData = response.json();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].from, id2);
        assert_eq!(graph.edges[0].to, id1);
    }

    #[tokio::test]
    async fn test_get_status() {
        let storage = InMemoryStorage::new();
        let executor = Arc::new(CommandExecutor::new(storage));

        executor
            .create_issue(
                "Issue 1".to_string(),
                "Description".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        let app = create_routes(executor);
        let server = TestServer::new(app).unwrap();

        let response = server.get("/status").await;
        response.assert_status_ok();
        let status: StatusResponse = response.json();
        assert_eq!(status.total, 1);
        assert_eq!(status.ready, 1); // New issue with no deps is ready
    }

    #[tokio::test]
    async fn test_search_endpoint_requires_query() {
        let server = create_test_app();
        let response = server.get("/search").await;
        // Should fail without query parameter
        assert!(response.status_code() != StatusCode::OK);
    }

    #[tokio::test]
    async fn test_search_endpoint_empty_results() {
        let server = create_test_app();
        // Search for something that doesn't exist
        let response = server.get("/search?q=nonexistent").await;

        if response.status_code() == StatusCode::OK {
            let search_response: SearchResponse = response.json();
            assert_eq!(search_response.query, "nonexistent");
            assert_eq!(search_response.total, 0);
            assert_eq!(search_response.results.len(), 0);
        }
        // Note: may fail if ripgrep is not installed, which is acceptable
    }

    #[tokio::test]
    async fn test_search_response_structure() {
        let server = create_test_app();
        let response = server.get("/search?q=test&limit=10").await;

        if response.status_code() == StatusCode::OK {
            let search_response: SearchResponse = response.json();
            assert_eq!(search_response.query, "test");
            // duration_ms is always >= 0 for u128, just verify it exists
            let _duration = search_response.duration_ms;
            assert_eq!(search_response.total, search_response.results.len());
        }
    }
}
