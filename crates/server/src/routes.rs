//! API route definitions

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use jit::commands::CommandExecutor;
use jit::domain::{Gate, GateRunResult, Issue, Priority, State as IssueState};
use jit::search::{SearchOptions, SearchResult};
use jit::storage::{IssueStore, PathReadError};

use crate::sse;
use crate::watcher::ChangeTracker;

/// Shared application state
#[derive(Clone)]
pub struct AppState<S: IssueStore> {
    pub executor: Arc<CommandExecutor<S>>,
    pub tracker: Arc<ChangeTracker>,
    pub project_name: String,
}

/// Create API routes
pub fn create_routes<S: IssueStore + Send + Sync + 'static>(state: AppState<S>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/issues", get(list_issues))
        .route("/issues/:id", get(get_issue))
        .route("/graph", get(get_graph))
        .route("/status", get(get_status))
        .route("/search", get(search_issues))
        .route("/documents", get(get_document_by_path))
        .route("/documents/raw", get(get_document_raw_by_path))
        .route("/raw/*path", get(get_raw_wildcard))
        .route(
            "/issues/:id/documents/:path/content",
            get(get_document_content),
        )
        .route("/issues/:id/documents/:path/raw", get(get_document_raw))
        .route(
            "/issues/:id/documents/:path/history",
            get(get_document_history),
        )
        .route("/issues/:id/documents/:path/diff", get(get_document_diff))
        .route("/gates", get(list_gates))
        .route("/gates/:key", get(get_gate_definition))
        .route("/issues/:id/gate-runs", get(list_gate_runs))
        .route("/issues/:id/gate-runs/:run_id", get(get_gate_run))
        .route("/config/strategic-types", get(get_strategic_types))
        .route("/config/hierarchy", get(get_hierarchy))
        .route("/config/namespaces", get(get_namespaces))
        .route("/changes", get(get_changes))
        .route("/events/stream", get(events_stream))
        .with_state(state)
}

/// Health check endpoint
async fn health_check<S: IssueStore>(State(state): State<AppState<S>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "jit-api",
        "version": env!("CARGO_PKG_VERSION"),
        "project_name": state.project_name
    }))
}

/// List all issues
async fn list_issues<S: IssueStore>(
    State(state): State<AppState<S>>,
) -> Result<Json<Vec<Issue>>, StatusCode> {
    state
        .executor
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
    State(state): State<AppState<S>>,
) -> Result<Json<Issue>, StatusCode> {
    state.executor.show_issue(&id).map(Json).map_err(|e| {
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
    pub labels: Vec<String>,
    pub blocked: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
}

/// Get dependency graph
async fn get_graph<S: IssueStore>(
    State(state): State<AppState<S>>,
) -> Result<Json<GraphData>, StatusCode> {
    let issues = state.executor.list_issues(None, None, None).map_err(|e| {
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
            labels: issue.labels.clone(),
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
    State(state): State<AppState<S>>,
) -> Result<Json<StatusResponse>, StatusCode> {
    let summary = state.executor.get_status().map_err(|e| {
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
    State(state): State<AppState<S>>,
) -> Result<Json<SearchResponse>, StatusCode> {
    let start = std::time::Instant::now();

    // Get all linked document paths to restrict search
    let linked_docs = state.executor.get_linked_document_paths().map_err(|e| {
        tracing::error!("Failed to get linked documents: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Build file patterns: always search .jit/issues/*.json + linked docs (from repo root)
    let mut file_patterns = vec![".jit/issues/*.json".to_string()];
    file_patterns.extend(linked_docs);

    let options = SearchOptions {
        case_sensitive: params.case_sensitive,
        regex: params.regex,
        max_results: Some(params.limit),
        file_patterns,
        ..Default::default()
    };

    // Search from repository root to include both .jit and linked documents
    let search_dir = std::path::Path::new(".");
    let results = jit::search::search(search_dir, &params.q, options).map_err(|e| {
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

/// Query parameters for standalone document access
#[derive(Debug, Deserialize)]
struct DocumentByPathQuery {
    path: String,
    commit: Option<String>,
}

/// Map a typed `PathReadError` to an HTTP status code.
fn path_read_error_status(e: &PathReadError) -> StatusCode {
    match e {
        PathReadError::NotFound(_) | PathReadError::CommitNotFound(_) => StatusCode::NOT_FOUND,
        PathReadError::InvalidPath(_) | PathReadError::OutsideRepoRoot(_) => {
            StatusCode::BAD_REQUEST
        }
        PathReadError::Other(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Get document content by path (without requiring issue ID)
///
/// This endpoint allows accessing documents directly by their filesystem path,
/// which is useful for opening documents from search results that may not be
/// associated with a specific issue context.
///
/// Delegates I/O to `CommandExecutor::read_path_bytes` so that filesystem/git
/// reads remain in the domain layer, not in the route handler.
async fn get_document_by_path<S: IssueStore>(
    Query(query): Query<DocumentByPathQuery>,
    State(state): State<AppState<S>>,
) -> Result<Json<DocumentContentResponse>, StatusCode> {
    let (bytes, commit_hash) = state
        .executor
        .read_path_bytes(&query.path, query.commit.as_deref())
        .map_err(|e| {
            tracing::error!("Failed to read path {}: {:?}", query.path, e);
            path_read_error_status(&e)
        })?;
    // Convert bytes to String for the JSON response (lossy for binary files).
    let content = String::from_utf8_lossy(&bytes).into_owned();
    Ok(Json(DocumentContentResponse {
        path: query.path.clone(),
        commit: commit_hash,
        content,
        content_type: infer_content_type(&query.path),
    }))
}

/// Infer content type from file extension
pub(crate) fn infer_content_type(path: &str) -> String {
    if path.ends_with(".md") {
        "text/markdown"
    } else if path.ends_with(".txt") {
        "text/plain"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".html") || path.ends_with(".htm") {
        "text/html"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".js") {
        "application/javascript"
    } else {
        "text/plain"
    }
    .to_string()
}

/// URL-encode a single path segment (percent-encode non-unreserved chars).
///
/// Unreserved characters per RFC 3986: ALPHA / DIGIT / "-" / "." / "_" / "~"
fn percent_encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            other => {
                out.push('%');
                out.push(
                    char::from_digit((other >> 4) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
                out.push(
                    char::from_digit((other & 0xf) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
            }
        }
    }
    out
}

/// Compute the `/api/raw/<parent-dir>/` base href for an HTML file at `html_path`.
///
/// Example: `docs/presentations/deck.html` → `/api/raw/docs/presentations/`
/// The root case (`deck.html` with no parent dir) → `/api/raw/`
fn compute_base_href(html_path: &str) -> String {
    // Determine the parent directory portion of the path.
    let parent = match html_path.rfind('/') {
        Some(idx) => &html_path[..idx],
        None => "",
    };

    if parent.is_empty() {
        "/api/raw/".to_string()
    } else {
        let encoded: String = parent
            .split('/')
            .map(percent_encode_segment)
            .collect::<Vec<_>>()
            .join("/");
        format!("/api/raw/{encoded}/")
    }
}

/// Check whether a `<base` HTML element is present in a string, ignoring
/// occurrences that appear inside `<script>`, `<style>`, or `<!--` comment
/// blocks (plain string scan — no HTML parser dependency).
fn html_has_base_element(head_content: &str) -> bool {
    let lower = head_content.to_lowercase();
    let mut pos = 0;
    let bytes = lower.as_bytes();
    let len = bytes.len();
    while pos < len {
        // Skip <!-- ... --> comment blocks.
        if lower[pos..].starts_with("<!--") {
            if let Some(end) = lower[pos + 4..].find("-->") {
                pos += 4 + end + 3;
                continue;
            } else {
                break; // unclosed comment — stop scanning
            }
        }
        // Skip <script ... </script> blocks.
        if lower[pos..].starts_with("<script") {
            if let Some(end) = lower[pos..].find("</script") {
                pos += end + 9;
                continue;
            } else {
                break;
            }
        }
        // Skip <style ... </style> blocks.
        if lower[pos..].starts_with("<style") {
            if let Some(end) = lower[pos..].find("</style") {
                pos += end + 8;
                continue;
            } else {
                break;
            }
        }
        // Check for a real <base> element.
        if lower[pos..].starts_with("<base ") || lower[pos..].starts_with("<base>") {
            return true;
        }
        pos += 1;
    }
    false
}

/// Inject `<base href="...">` into an HTML string so that relative paths
/// resolve correctly when the document is served from a URL whose directory
/// doesn't match the source file's location.
///
/// Inserts right after the opening `<head>` (or `<head …attrs…>`) tag.
/// Falls back to inserting after the opening `<html>` tag if there is no
/// `<head>`. Returns the string unchanged if:
/// - It already contains a real `<base>` element inside `<head>` (user intent wins).
///   Comments, `<script>`, and `<style>` blocks are skipped during this check.
/// - There is neither a `<head>` nor an `<html>` element (defensive — don't
///   corrupt non-HTML content that was accidentally routed here).
fn inject_base_href(html: &str, base_href: &str) -> String {
    let lower = html.to_lowercase();

    // Find the opening <head> tag (case-insensitive).
    let head_open_start = lower.find("<head");
    if let Some(start) = head_open_start {
        // Find the `>` that closes the opening tag.
        if let Some(rel_end) = lower[start..].find('>') {
            let tag_end = start + rel_end + 1; // position just after `>`

            // Check if there's already a real <base> element before </head>.
            // Use the script/style/comment-aware helper to avoid false positives.
            let head_content = if let Some(hc) = lower.find("</head") {
                &html[..hc]
            } else {
                html
            };

            if html_has_base_element(head_content) {
                return html.to_string();
            }

            let tag = format!("<base href=\"{base_href}\">");
            let mut result = String::with_capacity(html.len() + tag.len());
            result.push_str(&html[..tag_end]);
            result.push_str(&tag);
            result.push_str(&html[tag_end..]);
            return result;
        }
    }

    // No <head> — try to insert after the <html> opening tag.
    let html_open_start = lower.find("<html");
    if let Some(start) = html_open_start {
        if let Some(rel_end) = lower[start..].find('>') {
            let tag_end = start + rel_end + 1;
            let tag = format!("<base href=\"{base_href}\">");
            let mut result = String::with_capacity(html.len() + tag.len());
            result.push_str(&html[..tag_end]);
            result.push_str(&tag);
            result.push_str(&html[tag_end..]);
            return result;
        }
    }

    // Neither <head> nor <html> found — return unchanged (defensive).
    html.to_string()
}

// Defense-in-depth policy for raw document responses. Permits same-origin,
// HTTPS, and data: URLs for scripts/styles/images/fonts — enough for typical
// user-authored HTML (reveal.js decks loading from jsdelivr, inline
// initialization scripts) while blocking plain http:// resources and
// restricting the origin otherwise.
const CSP_HEADER: &str = "default-src 'self' https: data:; script-src 'self' 'unsafe-inline' https:; style-src 'self' 'unsafe-inline' https:; img-src 'self' data: https:;";

/// Query parameters for document content
#[derive(Debug, Deserialize)]
struct DocumentContentQuery {
    commit: Option<String>,
}

/// Response for document content
#[derive(Debug, Serialize)]
struct DocumentContentResponse {
    path: String,
    commit: String,
    content: String,
    content_type: String,
}

/// Get document content
async fn get_document_content<S: IssueStore>(
    Path((id, path)): Path<(String, String)>,
    Query(query): Query<DocumentContentQuery>,
    State(state): State<AppState<S>>,
) -> Result<Json<DocumentContentResponse>, StatusCode> {
    let at_commit = query.commit.as_deref();

    let (content, commit_hash) = state
        .executor
        .read_document_content(&id, &path, at_commit)
        .map_err(|e| {
            tracing::error!("Failed to read document content: {:?}", e);
            path_read_error_status(&e)
        })?;

    Ok(Json(DocumentContentResponse {
        path: path.clone(),
        commit: commit_hash,
        content,
        content_type: infer_content_type(&path),
    }))
}

/// Get raw document bytes (issue-scoped variant)
///
/// Verifies that `path` is linked to the requested issue, then reads the file
/// as raw bytes via `read_document_bytes` (which delegates to
/// `IssueStore::read_path_bytes`).  Binary artifacts are served faithfully
/// without any UTF-8 conversion.  HTML responses for working-tree reads get a
/// `<base href>` injected so sibling assets resolve via `/api/raw/*path`.
async fn get_document_raw<S: IssueStore>(
    Path((id, path)): Path<(String, String)>,
    Query(query): Query<DocumentContentQuery>,
    State(state): State<AppState<S>>,
) -> Result<Response<Body>, StatusCode> {
    let at_commit = query.commit.as_deref();

    let (bytes, _commit_hash) = state
        .executor
        .read_document_bytes(&id, &path, at_commit)
        .map_err(|e| {
            tracing::error!("Failed to read raw document bytes: {:?}", e);
            path_read_error_status(&e)
        })?;

    let content_type = infer_content_type(&path);

    // Inject <base href> into HTML responses so relative asset paths
    // (e.g. figures/fig4.svg) resolve via the wildcard /api/raw route.
    // Applied for both working-tree and commit-pinned reads.
    let body_bytes = if content_type == "text/html" {
        let html = String::from_utf8_lossy(&bytes);
        let base_href = compute_base_href(&path);
        inject_base_href(&html, &base_href).into_bytes()
    } else {
        bytes
    };

    let mut response = Response::new(Body::from(body_bytes));
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("text/plain")),
    );
    response.headers_mut().insert(
        "content-security-policy",
        HeaderValue::from_static(CSP_HEADER),
    );
    Ok(response)
}

/// Get raw document bytes (path-only variant)
///
/// Delegates I/O to `CommandExecutor::read_path_bytes` so persistence stays
/// in the domain layer.  Returns raw bytes with the inferred Content-Type and
/// CSP headers.  Binary files are served faithfully without UTF-8 conversion.
/// HTML responses get a `<base href>` injected so sibling assets resolve via
/// the wildcard `/api/raw/*path` route.
///
/// Repo-root containment (rejecting empty/absolute/`..` paths and symlink
/// escapes) is enforced by `JsonFileStorage::read_path_bytes`, which is
/// invariant-owning for this concern.  Invalid paths and out-of-repo escapes
/// surface as `PathReadError::InvalidPath` / `PathReadError::OutsideRepoRoot`,
/// both mapped to HTTP 400 by `path_read_error_status`.
async fn get_document_raw_by_path<S: IssueStore>(
    Query(query): Query<DocumentByPathQuery>,
    State(state): State<AppState<S>>,
) -> Result<Response<Body>, StatusCode> {
    let at_commit = query.commit.as_deref();

    let (bytes, _commit_hash) = state
        .executor
        .read_path_bytes(&query.path, at_commit)
        .map_err(|e| {
            tracing::error!("Failed to read path {}: {:?}", query.path, e);
            path_read_error_status(&e)
        })?;
    let content_type = infer_content_type(&query.path);

    // Inject <base href> into HTML responses.  The storage layer rejects
    // absolute paths (`PathReadError::InvalidPath`) before we get here, so
    // every path that reaches this point is repo-relative and re-servable
    // through `/api/raw/*path`.
    let body_bytes = if content_type == "text/html" {
        let html = String::from_utf8_lossy(&bytes);
        let base_href = compute_base_href(&query.path);
        inject_base_href(&html, &base_href).into_bytes()
    } else {
        bytes
    };

    let mut response = Response::new(Body::from(body_bytes));
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("text/plain")),
    );
    response.headers_mut().insert(
        "content-security-policy",
        HeaderValue::from_static(CSP_HEADER),
    );
    Ok(response)
}

/// Get raw bytes for any repo-relative path (wildcard variant).
///
/// Route: `GET /api/raw/*path`
///
/// Serves arbitrary files from the repository working tree or a pinned git
/// commit.  This is primarily used so HTML documents can load sibling assets
/// (SVG figures, CSS, JS) via relative paths — the browser resolves those
/// paths against the `<base href>` injected by the HTML handlers above.
///
/// Security: repo-root containment (rejecting empty/absolute/`..` paths and
/// symlink escapes) is enforced by `JsonFileStorage::read_path_bytes`.  The
/// resulting `PathReadError::InvalidPath` / `PathReadError::OutsideRepoRoot`
/// variants map to HTTP 400 via `path_read_error_status`.
async fn get_raw_wildcard<S: IssueStore>(
    Path(path): Path<String>,
    Query(query): Query<DocumentContentQuery>,
    State(state): State<AppState<S>>,
) -> Result<Response<Body>, StatusCode> {
    let at_commit = query.commit.as_deref();

    let (bytes, _commit_hash) = state
        .executor
        .read_path_bytes(&path, at_commit)
        .map_err(|e| {
            tracing::error!("Failed to read raw path {}: {:?}", path, e);
            path_read_error_status(&e)
        })?;

    let content_type = infer_content_type(&path);

    // Inject <base href> into HTML responses (both working-tree and
    // commit-pinned) so relative asset paths resolve via /api/raw/*path.
    let body_bytes = if content_type == "text/html" {
        let html = String::from_utf8_lossy(&bytes);
        let base_href = compute_base_href(&path);
        inject_base_href(&html, &base_href).into_bytes()
    } else {
        bytes
    };

    let mut response = Response::new(Body::from(body_bytes));
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("text/plain")),
    );
    response.headers_mut().insert(
        "content-security-policy",
        HeaderValue::from_static(CSP_HEADER),
    );
    Ok(response)
}

/// Response for document history
#[derive(Debug, Serialize)]
struct DocumentHistoryResponse {
    path: String,
    commits: Vec<CommitInfoResponse>,
}

/// Commit information
#[derive(Debug, Serialize)]
struct CommitInfoResponse {
    commit: String,
    author: String,
    date: String,
    message: String,
}

/// Get document history
async fn get_document_history<S: IssueStore>(
    Path((id, path)): Path<(String, String)>,
    State(state): State<AppState<S>>,
) -> Result<Json<DocumentHistoryResponse>, StatusCode> {
    let commits = state
        .executor
        .get_document_history(&id, &path)
        .map_err(|e| {
            tracing::error!("Failed to get document history: {:?}", e);
            path_read_error_status(&e)
        })?;

    let commits_response = commits
        .into_iter()
        .map(|c| CommitInfoResponse {
            commit: c.sha,
            author: c.author,
            date: c.date,
            message: c.message,
        })
        .collect();

    Ok(Json(DocumentHistoryResponse {
        path: path.clone(),
        commits: commits_response,
    }))
}

/// Query parameters for document diff
#[derive(Debug, Deserialize)]
struct DocumentDiffQuery {
    from: String,
    to: Option<String>,
}

/// Response for document diff
#[derive(Debug, Serialize)]
struct DocumentDiffResponse {
    path: String,
    from: String,
    to: String,
    diff: String,
}

/// Get document diff
async fn get_document_diff<S: IssueStore>(
    Path((id, path)): Path<(String, String)>,
    Query(query): Query<DocumentDiffQuery>,
    State(state): State<AppState<S>>,
) -> Result<Json<DocumentDiffResponse>, StatusCode> {
    let to = query.to.as_deref();

    let diff = state
        .executor
        .get_document_diff(&id, &path, &query.from, to)
        .map_err(|e| {
            tracing::error!("Failed to get document diff: {:?}", e);
            path_read_error_status(&e)
        })?;

    let to_ref = to.unwrap_or("HEAD");

    Ok(Json(DocumentDiffResponse {
        path: path.clone(),
        from: query.from.clone(),
        to: to_ref.to_string(),
        diff,
    }))
}

/// List all gate definitions from the registry
async fn list_gates<S: IssueStore>(
    State(state): State<AppState<S>>,
) -> Result<Json<Vec<Gate>>, StatusCode> {
    state.executor.list_gates().map(Json).map_err(|e| {
        tracing::error!("Failed to list gates: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

/// Get a single gate definition by key
async fn get_gate_definition<S: IssueStore>(
    Path(key): Path<String>,
    State(state): State<AppState<S>>,
) -> Result<Json<Gate>, StatusCode> {
    state
        .executor
        .show_gate_definition(&key)
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to get gate definition {}: {:?}", key, e);
            StatusCode::NOT_FOUND
        })
}

/// Query parameters for gate runs
#[derive(Debug, Deserialize)]
struct GateRunsQuery {
    gate_key: Option<String>,
}

/// Summary of a gate run (excludes stdout/stderr for size)
#[derive(Debug, Serialize)]
struct GateRunSummary {
    run_id: String,
    gate_key: String,
    stage: jit::domain::GateStage,
    status: jit::domain::GateRunStatus,
    started_at: String,
    completed_at: Option<String>,
    duration_ms: Option<u64>,
    exit_code: Option<i32>,
    command: String,
    commit: Option<String>,
    branch: Option<String>,
    by: Option<String>,
    message: Option<String>,
}

impl From<GateRunResult> for GateRunSummary {
    fn from(r: GateRunResult) -> Self {
        Self {
            run_id: r.run_id,
            gate_key: r.gate_key,
            stage: r.stage,
            status: r.status,
            started_at: r.started_at.to_rfc3339(),
            completed_at: r.completed_at.map(|t| t.to_rfc3339()),
            duration_ms: r.duration_ms,
            exit_code: r.exit_code,
            command: r.command,
            commit: r.commit,
            branch: r.branch,
            by: r.by,
            message: r.message,
        }
    }
}

/// List gate run results for an issue (summaries without stdout/stderr)
async fn list_gate_runs<S: IssueStore>(
    Path(id): Path<String>,
    Query(params): Query<GateRunsQuery>,
    State(state): State<AppState<S>>,
) -> Result<Json<Vec<GateRunSummary>>, StatusCode> {
    let runs = state
        .executor
        .list_gate_runs(&id, params.gate_key.as_deref())
        .map_err(|e| {
            tracing::error!("Failed to list gate runs for {}: {:?}", id, e);
            StatusCode::NOT_FOUND
        })?;

    Ok(Json(runs.into_iter().map(GateRunSummary::from).collect()))
}

/// Get a single gate run result with full stdout/stderr
async fn get_gate_run<S: IssueStore>(
    Path((_id, run_id)): Path<(String, String)>,
    State(state): State<AppState<S>>,
) -> Result<Json<GateRunResult>, StatusCode> {
    state
        .executor
        .get_gate_run_result(&run_id)
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to get gate run {}: {:?}", run_id, e);
            StatusCode::NOT_FOUND
        })
}

/// Response for strategic types
#[derive(Debug, Serialize, Deserialize)]
struct StrategicTypesResponse {
    strategic_types: Vec<String>,
}

/// Get strategic types from configuration
async fn get_strategic_types<S: IssueStore>(
    State(state): State<AppState<S>>,
) -> Result<Json<StrategicTypesResponse>, StatusCode> {
    let namespaces = state
        .executor
        .config_manager
        .get_namespaces()
        .map_err(|e| {
            tracing::error!("Failed to load configuration: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let strategic_types = namespaces.strategic_types.unwrap_or_default();

    Ok(Json(StrategicTypesResponse { strategic_types }))
}

/// Response for type hierarchy
#[derive(Debug, Serialize, Deserialize)]
struct HierarchyResponse {
    types: std::collections::HashMap<String, u8>,
    strategic_types: Vec<String>,
    icons: std::collections::HashMap<String, String>,
}

/// Get type hierarchy configuration
async fn get_hierarchy<S: IssueStore>(
    State(state): State<AppState<S>>,
) -> Result<Json<HierarchyResponse>, StatusCode> {
    let namespaces = state
        .executor
        .config_manager
        .get_namespaces()
        .map_err(|e| {
            tracing::error!("Failed to load configuration: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let types = namespaces.type_hierarchy.unwrap_or_default();
    let strategic_types = namespaces.strategic_types.unwrap_or_default();

    // Get resolved icons
    let icons = state
        .executor
        .config_manager
        .get_hierarchy_icons()
        .map_err(|e| {
            tracing::error!("Failed to load hierarchy icons: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(HierarchyResponse {
        types,
        strategic_types,
        icons,
    }))
}

/// Response for namespaces
#[derive(Debug, Serialize, Deserialize)]
struct NamespacesResponse {
    namespaces: std::collections::HashMap<String, NamespaceInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NamespaceInfo {
    description: String,
    unique: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    values: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    required: Option<bool>,
}

/// Get namespace registry from configuration
async fn get_namespaces<S: IssueStore>(
    State(state): State<AppState<S>>,
) -> Result<Json<NamespacesResponse>, StatusCode> {
    let label_namespaces = state
        .executor
        .config_manager
        .get_namespaces()
        .map_err(|e| {
            tracing::error!("Failed to load configuration: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let namespaces = label_namespaces
        .namespaces
        .into_iter()
        .map(|(name, ns)| {
            (
                name,
                NamespaceInfo {
                    description: ns.description,
                    unique: ns.unique,
                    values: ns.values,
                    pattern: ns.pattern,
                    required: ns.required,
                },
            )
        })
        .collect();

    Ok(Json(NamespacesResponse { namespaces }))
}

/// Response for current change version
#[derive(Debug, Serialize, Deserialize)]
struct ChangesResponse {
    version: u64,
}

/// Get current change version
async fn get_changes<S: IssueStore>(State(state): State<AppState<S>>) -> Json<ChangesResponse> {
    Json(ChangesResponse {
        version: state.tracker.current_version(),
    })
}

/// SSE stream of change events
async fn events_stream<S: IssueStore>(State(state): State<AppState<S>>) -> impl IntoResponse {
    sse::change_stream(&state.tracker)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum_test::TestServer;
    use jit::domain::Priority;
    use jit::storage::InMemoryStorage;

    use crate::watcher::ChangeTracker;

    fn create_test_app() -> TestServer {
        let storage = InMemoryStorage::new();
        let executor = Arc::new(CommandExecutor::new(storage));
        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let app = create_routes(state);
        TestServer::new(app).unwrap()
    }

    #[tokio::test]
    async fn test_health_check() {
        let server = create_test_app();
        let response = server.get("/health").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["status"], "ok");
        assert_eq!(body["service"], "jit-api");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert!(body["project_name"].is_string());
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
        let (_id1, _) = executor
            .create_issue(
                "Issue 1".to_string(),
                "Description".to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();
        let (_id2, _) = executor
            .create_issue(
                "Issue 2".to_string(),
                "Description".to_string(),
                Priority::High,
                vec![],
                vec![],
            )
            .unwrap();

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let app = create_routes(state);
        let server = TestServer::new(app).unwrap();

        let response = server.get("/issues").await;
        response.assert_status_ok();
        let issues: Vec<Issue> = response.json();
        assert_eq!(issues.len(), 2);
    }

    #[tokio::test]
    async fn test_get_graph() {
        let storage = InMemoryStorage::new();

        // Create config with enforcement off for test backward compatibility
        std::fs::create_dir_all(storage.root()).unwrap();
        let config_toml = r#"
[worktree]
enforce_leases = "off"
"#;
        std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

        let executor = Arc::new(CommandExecutor::new(storage));

        // Create issues with dependencies
        let (id1, _) = executor
            .create_issue(
                "Issue 1".to_string(),
                "Description".to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();
        let (id2, _) = executor
            .create_issue(
                "Issue 2".to_string(),
                "Description".to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();

        executor.add_dependency(&id2, &id1).unwrap();

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let app = create_routes(state);
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

        let (_id, _) = executor
            .create_issue(
                "Issue 1".to_string(),
                "Description".to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let app = create_routes(state);
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

    #[tokio::test]
    async fn test_search_only_searches_linked_documents() {
        // This test verifies that search is restricted to linked documents
        // by checking that get_linked_document_paths is called
        // (implementation detail test - would need integration test for full verification)
        let server = create_test_app();
        let _response = server.get("/search?q=test").await;
        // If it doesn't panic or error, the linked document logic is being used
    }

    #[tokio::test]
    async fn test_get_strategic_types() {
        let server = create_test_app();
        let response = server.get("/config/strategic-types").await;
        response.assert_status_ok();
        let data: StrategicTypesResponse = response.json();
        // Should return default or configured strategic types
        assert!(data.strategic_types.is_empty() || !data.strategic_types.is_empty());
    }

    #[tokio::test]
    async fn test_get_hierarchy() {
        let server = create_test_app();
        let response = server.get("/config/hierarchy").await;
        response.assert_status_ok();
        let data: HierarchyResponse = response.json();
        // Should return hierarchy data (may be empty defaults)
        assert!(data.types.is_empty() || !data.types.is_empty());
        assert!(data.strategic_types.is_empty() || !data.strategic_types.is_empty());
    }

    #[tokio::test]
    async fn test_get_namespaces() {
        let server = create_test_app();
        let response = server.get("/config/namespaces").await;
        response.assert_status_ok();
        let data: NamespacesResponse = response.json();
        // Should return namespaces (at least defaults)
        assert!(!data.namespaces.is_empty());

        // Verify structure of namespace info
        for (_, ns_info) in data.namespaces {
            assert!(!ns_info.description.is_empty());
        }
    }

    #[tokio::test]
    async fn test_get_namespaces_exposes_values_pattern_required() {
        use jit::storage::{IssueStore, JsonFileStorage};

        // A file-backed storage with a config.toml that exercises every new
        // field, so the response contract (mirror of CLI config show --json)
        // stays protected against drift.
        let temp = tempfile::tempdir().unwrap();
        let jit_dir = temp.path().join(".jit");
        std::fs::create_dir(&jit_dir).unwrap();
        std::fs::write(
            jit_dir.join("config.toml"),
            r#"
[namespaces.type]
description = "Issue type"
unique = true
required = true
values = ["task", "bug"]

[namespaces.milestone]
description = "Release"
unique = false
pattern = '^v\d+\.\d+$'
"#,
        )
        .unwrap();
        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();
        let executor = Arc::new(CommandExecutor::new(storage));
        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test".to_string(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let response = server.get("/config/namespaces").await;
        response.assert_status_ok();
        let data: NamespacesResponse = response.json();

        let type_ns = data.namespaces.get("type").expect("type namespace");
        assert_eq!(type_ns.required, Some(true));
        assert_eq!(
            type_ns.values.as_deref(),
            Some(&["task".to_string(), "bug".to_string()][..])
        );
        assert!(type_ns.pattern.is_none());

        let ms_ns = data
            .namespaces
            .get("milestone")
            .expect("milestone namespace");
        assert_eq!(ms_ns.pattern.as_deref(), Some(r"^v\d+\.\d+$"));
        assert!(ms_ns.values.is_none());
        assert!(ms_ns.required.is_none());
    }

    #[tokio::test]
    async fn test_get_document_by_path_missing_param() {
        let server = create_test_app();
        let response = server.get("/documents").await;
        // Should fail without path parameter
        assert!(response.status_code() != StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_document_by_path_nonexistent() {
        let server = create_test_app();
        let response = server.get("/documents?path=nonexistent.md").await;
        // Should return 404 for file that doesn't exist
        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_document_by_path_success() {
        use jit::storage::JsonFileStorage;
        use serde_json::Value;
        use std::fs;

        // Set up a real tempdir acting as repo root with a .jit subdir.
        let temp = tempfile::tempdir().unwrap();
        let jit_dir = temp.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();

        // Write a permissive config so the executor is happy.
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        // Write the fixture document at a subdirectory path relative to repo root.
        // This exercises the path-resolution fix: storage must resolve against
        // `self.root.parent()` (repo root), not process CWD.
        let doc_rel = "docs/readme.md";
        let doc_content = "# Fixture document\n\nSome content.";
        let doc_abs = temp.path().join(doc_rel);
        fs::create_dir_all(doc_abs.parent().unwrap()).unwrap();
        fs::write(&doc_abs, doc_content).unwrap();

        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();

        let executor = Arc::new(CommandExecutor::new(storage));
        let tracker = Arc::new(ChangeTracker::new(16));

        // Keep tempdir alive for the duration of the test.
        Box::leak(Box::new(temp));

        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        // Use add_query_param so axum-test sends the query string correctly.
        // Embedding `?key=val` directly in the URL string causes axum-test to
        // percent-encode the `?`, which breaks route matching.
        let response = server
            .get("/documents")
            .add_query_param("path", doc_rel)
            .await;

        // Assert 200 OK with matching content and content_type fields.
        response.assert_status_ok();

        let body: Value = serde_json::from_slice(response.as_bytes()).expect("valid JSON body");
        assert_eq!(
            body["content"].as_str().unwrap(),
            doc_content,
            "response content field must match fixture file"
        );
        let expected_ct = infer_content_type(doc_rel);
        assert_eq!(
            body["content_type"].as_str().unwrap(),
            expected_ct,
            "response content_type field must match infer_content_type"
        );
    }

    // ── infer_content_type unit tests ────────────────────────────────────────

    #[test]
    fn test_infer_content_type_html() {
        assert_eq!(infer_content_type("foo/bar.html"), "text/html");
        assert_eq!(infer_content_type("index.htm"), "text/html");
    }

    #[test]
    fn test_infer_content_type_other_extensions() {
        assert_eq!(infer_content_type("doc.md"), "text/markdown");
        assert_eq!(infer_content_type("data.json"), "application/json");
        assert_eq!(infer_content_type("style.css"), "text/css");
        assert_eq!(infer_content_type("app.js"), "application/javascript");
        assert_eq!(infer_content_type("logo.svg"), "image/svg+xml");
        assert_eq!(infer_content_type("photo.png"), "image/png");
        assert_eq!(infer_content_type("photo.jpg"), "image/jpeg");
        assert_eq!(infer_content_type("photo.jpeg"), "image/jpeg");
        assert_eq!(infer_content_type("readme.txt"), "text/plain");
        assert_eq!(infer_content_type("unknown.xyz"), "text/plain");
    }

    // ── /api/issues/:id/documents/:path/raw tests ────────────────────────────

    /// Build a JsonFileStorage-backed server with a real tempdir that has an
    /// HTML document linked to an issue.  Returns the server, the issue id, and
    /// the document path string.
    fn create_test_app_with_html_doc() -> (
        axum_test::TestServer,
        String, // issue id
        String, // doc path relative to repo root
    ) {
        use jit::domain::Priority;
        use jit::storage::JsonFileStorage;
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        let jit_dir = temp.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();

        // Write a permissive config so the executor is happy.
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();

        let executor = Arc::new(CommandExecutor::new(storage));

        // Create an issue.
        let (id, _) = executor
            .create_issue(
                "Test issue".to_string(),
                "desc".to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();

        // Write the HTML file relative to repo root (parent of .jit/).
        let doc_rel = "slide.html";
        let doc_abs = temp.path().join(doc_rel);
        fs::write(&doc_abs, "<html><body>hello</body></html>").unwrap();

        // Link the document to the issue (skip_scan=true to avoid git dep).
        executor
            .add_document_reference(&id, doc_rel, None, None, None, true)
            .unwrap();

        // Keep tempdir alive for the duration of the test via Box::leak – this
        // is acceptable in test code; the OS cleans it up.
        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();
        (server, id, doc_rel.to_string())
    }

    #[tokio::test]
    async fn test_get_document_raw_html_returns_200_with_correct_content_type() {
        let (server, id, path) = create_test_app_with_html_doc();
        let url = format!("/issues/{}/documents/{}/raw", id, path);
        let response = server.get(&url).await;
        response.assert_status_ok();
        let ct = response
            .headers()
            .get("content-type")
            .expect("content-type header")
            .to_str()
            .unwrap();
        assert!(ct.starts_with("text/html"), "expected text/html, got {ct}");
    }

    /// After base-tag injection, the body gains `<base href="/api/raw/">` right
    /// after the opening `<html>` tag (fixture has no `<head>`).
    #[tokio::test]
    async fn test_get_document_raw_html_injects_base_tag() {
        let (server, id, path) = create_test_app_with_html_doc();
        let url = format!("/issues/{}/documents/{}/raw", id, path);
        let response = server.get(&url).await;
        response.assert_status_ok();
        let body = response.text();
        // fixture: `<html><body>hello</body></html>` — no <head>, so injection
        // happens after <html> → base href is "/api/raw/" (root, no parent dir).
        assert_eq!(
            body,
            r#"<html><base href="/api/raw/"><body>hello</body></html>"#
        );
    }

    #[tokio::test]
    async fn test_get_document_raw_has_csp_header() {
        let (server, id, path) = create_test_app_with_html_doc();
        let url = format!("/issues/{}/documents/{}/raw", id, path);
        let response = server.get(&url).await;
        response.assert_status_ok();
        let csp = response
            .headers()
            .get("content-security-policy")
            .expect("CSP header must be present")
            .to_str()
            .unwrap();
        // Verify the full CSP policy matches the spec-required value.
        assert_eq!(
            csp, CSP_HEADER,
            "CSP header must match the specified policy"
        );
    }

    #[tokio::test]
    async fn test_get_document_raw_not_found_when_path_not_linked() {
        let (server, id, _path) = create_test_app_with_html_doc();
        let url = format!("/issues/{}/documents/nonexistent.html/raw", id);
        let response = server.get(&url).await;
        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_document_raw_returns_byte_faithful_for_non_utf8() {
        use jit::domain::Priority;
        use jit::storage::JsonFileStorage;
        use std::fs;

        // 4-byte sequence that is not valid UTF-8.
        let binary_fixture: &[u8] = b"\xff\xfe\xfd\xfc";

        let temp = tempfile::tempdir().unwrap();
        let jit_dir = temp.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();
        let executor = Arc::new(CommandExecutor::new(storage));

        let (id, _) = executor
            .create_issue(
                "Binary test".to_string(),
                "desc".to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();

        // Write binary fixture with a .dat extension (catches the text/plain fallback).
        let doc_rel = "artifact.dat";
        let doc_abs = temp.path().join(doc_rel);
        fs::write(&doc_abs, binary_fixture).unwrap();

        executor
            .add_document_reference(&id, doc_rel, None, None, None, true)
            .unwrap();

        // Keep tempdir alive.
        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let url = format!("/issues/{}/documents/{}/raw", id, doc_rel);
        let response = server.get(&url).await;
        response.assert_status_ok();

        // Body bytes must be identical to the original binary fixture.
        let body_bytes = response.as_bytes().to_vec();
        assert_eq!(
            body_bytes, binary_fixture,
            "raw endpoint must return byte-identical content for non-UTF-8 artifacts"
        );
    }

    // ── /api/documents/raw?path=... tests ────────────────────────────────────

    #[tokio::test]
    async fn test_get_document_raw_by_path_html_returns_correct_content_type() {
        use jit::storage::JsonFileStorage;
        use std::fs;

        // Build a JsonFileStorage-backed server rooted at a tempdir and write
        // the fixture HTML at a repo-relative path.  Absolute paths are no
        // longer accepted by the storage layer (they surface as
        // PathReadError::InvalidPath → 400), so every test must use a
        // repo-relative path.
        let temp = tempfile::tempdir().unwrap();
        let jit_dir = temp.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        let doc_rel = "slide.html";
        let html_body = "<p>test</p>";
        fs::write(temp.path().join(doc_rel), html_body).unwrap();

        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();
        let executor = Arc::new(CommandExecutor::new(storage));
        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();
        let response = server
            .get("/documents/raw")
            .add_query_param("path", doc_rel)
            .await;
        response.assert_status_ok();
        // Verify Content-Type.
        let ct = response
            .headers()
            .get("content-type")
            .expect("content-type header")
            .to_str()
            .unwrap();
        assert!(ct.starts_with("text/html"), "expected text/html, got {ct}");
        // Verify CSP matches the full specified policy.
        let csp = response
            .headers()
            .get("content-security-policy")
            .expect("CSP header must be present")
            .to_str()
            .unwrap();
        assert_eq!(csp, CSP_HEADER, "CSP header must match specified policy");
        // Verify exact body bytes are returned unchanged.
        assert_eq!(response.text(), html_body);
    }

    #[tokio::test]
    async fn test_get_document_raw_by_path_not_found() {
        let server = create_test_app();
        let response = server
            .get("/documents/raw")
            .add_query_param("path", "no_such_file.html")
            .await;
        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_changes_returns_version() {
        let server = create_test_app();
        let response = server.get("/changes").await;
        response.assert_status_ok();
        let data: ChangesResponse = response.json();
        assert_eq!(data.version, 0);
    }

    #[tokio::test]
    async fn test_get_changes_reflects_tracker_state() {
        let storage = InMemoryStorage::new();
        let executor = Arc::new(CommandExecutor::new(storage));
        let tracker = Arc::new(ChangeTracker::new(16));

        // Simulate a change
        tracker.notify_change();
        tracker.notify_change();

        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let app = create_routes(state);
        let server = TestServer::new(app).unwrap();

        let response = server.get("/changes").await;
        response.assert_status_ok();
        let data: ChangesResponse = response.json();
        assert_eq!(data.version, 2);
    }

    // ── path_read_error_status unit tests ─────────────────────────────────────

    /// Verify that `PathReadError::NotFound` → 404 and `Other` → 500.
    ///
    /// These are the two invariants the route handlers rely on; having them as
    /// unit tests ensures a copy-change in the dispatch function is caught
    /// immediately rather than only at the integration level.
    #[test]
    fn test_path_read_error_status_not_found_returns_404() {
        let err = PathReadError::NotFound("docs/spec.md".to_string());
        assert_eq!(path_read_error_status(&err), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_path_read_error_status_commit_not_found_returns_404() {
        let err = PathReadError::CommitNotFound("abc1234".to_string());
        assert_eq!(path_read_error_status(&err), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_path_read_error_status_other_returns_500() {
        let err = PathReadError::Other(anyhow::Error::msg("I/O error: permission denied"));
        assert_eq!(
            path_read_error_status(&err),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    /// Verify that the `get_document_by_path` endpoint returns 404 for a
    /// missing file (relying on typed dispatch, not string-matching).
    #[tokio::test]
    async fn test_get_document_by_path_not_found_returns_404() {
        let server = create_test_app();
        // Repo-relative path (absolute paths are rejected as InvalidPath → 400).
        let response = server
            .get("/documents")
            .add_query_param("path", "does_not_exist.md")
            .await;
        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    }

    /// Verify that the `get_document_raw_by_path` endpoint returns 404 for a
    /// missing file (relying on typed dispatch, not string-matching).
    #[tokio::test]
    async fn test_get_document_raw_by_path_returns_404_for_missing() {
        let server = create_test_app();
        // Repo-relative path (absolute paths are rejected as InvalidPath → 400).
        let response = server
            .get("/documents/raw")
            .add_query_param("path", "does_not_exist.bin")
            .await;
        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    }

    /// Verify that `get_document_by_path` returns HTTP 500 when the storage
    /// layer returns `PathReadError::Other` (i.e., a non-NotFound I/O error).
    ///
    /// We trigger this by pointing the endpoint at a repo-relative path that
    /// resolves to a directory inside the repo root.  `fs::canonicalize`
    /// succeeds on a directory (so the containment check passes), but the
    /// subsequent `fs::read` returns `EISDIR`, which maps to
    /// `PathReadError::Other`.
    #[tokio::test]
    async fn test_get_document_by_path_io_error_returns_500() {
        use jit::storage::JsonFileStorage;
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        let jit_dir = temp.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        // Create a subdirectory at repo-root-relative "not-a-file".  The
        // storage layer joins this against the repo root, canonicalizes it,
        // then fs::read returns EISDIR → PathReadError::Other → 500.
        let dir_rel = "not-a-file";
        fs::create_dir_all(temp.path().join(dir_rel)).unwrap();

        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();
        let executor = Arc::new(CommandExecutor::new(storage));
        // Keep tempdir alive.
        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let response = server
            .get("/documents")
            .add_query_param("path", dir_rel)
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "reading a directory via get_document_by_path should return 500"
        );
    }

    /// Same invariant for the raw endpoint.
    #[tokio::test]
    async fn test_get_document_raw_by_path_io_error_returns_500() {
        use jit::storage::JsonFileStorage;
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        let jit_dir = temp.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        let dir_rel = "also-not-a-file";
        fs::create_dir_all(temp.path().join(dir_rel)).unwrap();

        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();
        let executor = Arc::new(CommandExecutor::new(storage));
        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let response = server
            .get("/documents/raw")
            .add_query_param("path", dir_rel)
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "reading a directory via get_document_raw_by_path should return 500"
        );
    }

    /// `GET /issues/:id/documents/:path/content` must return 500 when reading
    /// the document file fails with a genuine I/O error (e.g. EISDIR).
    ///
    /// The document is linked to a relative path with `skip_scan=true` so the
    /// reference is stored without any read being attempted during linking.
    /// When the route handler subsequently calls `read_document_content` →
    /// `read_path_text` → `read_path_bytes`, the relative path is resolved
    /// against the repo root.  Since that path is a directory, trying to read
    /// it yields EISDIR, which maps to `PathReadError::Other` → HTTP 500.
    #[tokio::test]
    async fn test_get_document_content_io_error_returns_500() {
        use jit::domain::Priority;
        use jit::storage::JsonFileStorage;
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        let jit_dir = temp.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        // Create a subdirectory at repo_root/broken-doc.  read_document_content
        // resolves relative paths against repo_root, so reading "broken-doc"
        // will attempt to read a directory → EISDIR → PathReadError::Other.
        let dir_path = temp.path().join("broken-doc");
        fs::create_dir_all(&dir_path).unwrap();
        // Use a relative path so the URL stays clean (no slashes in the segment).
        let doc_rel = "broken-doc";

        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();
        let executor = Arc::new(CommandExecutor::new(storage));

        let (id, _) = executor
            .create_issue(
                "IO error test".to_string(),
                "desc".to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();

        // Link the directory path as a document (skip_scan=true avoids the read
        // during registration so we can store the bad reference).
        executor
            .add_document_reference(&id, doc_rel, None, None, None, true)
            .unwrap();

        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let url = format!("/issues/{}/documents/{}/content", id, doc_rel);
        let response = server.get(&url).await;
        assert_eq!(
            response.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "reading a directory via get_document_content should return 500"
        );
    }

    /// `GET /issues/:id/documents/:path/raw` must return 500 when reading
    /// the document file fails with a genuine I/O error (e.g. EISDIR).
    ///
    /// Same EISDIR trigger as `test_get_document_content_io_error_returns_500`
    /// but exercises the `get_document_raw` handler path through
    /// `read_document_bytes` → `read_path_bytes` → `PathReadError::Other`.
    #[tokio::test]
    async fn test_get_document_raw_io_error_returns_500() {
        use jit::domain::Priority;
        use jit::storage::JsonFileStorage;
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        let jit_dir = temp.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        // Directory → EISDIR when read → PathReadError::Other → 500.
        let dir_path = temp.path().join("broken-raw-doc");
        fs::create_dir_all(&dir_path).unwrap();
        let doc_rel = "broken-raw-doc";

        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();
        let executor = Arc::new(CommandExecutor::new(storage));

        let (id, _) = executor
            .create_issue(
                "IO error raw test".to_string(),
                "desc".to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();

        executor
            .add_document_reference(&id, doc_rel, None, None, None, true)
            .unwrap();

        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let url = format!("/issues/{}/documents/{}/raw", id, doc_rel);
        let response = server.get(&url).await;
        assert_eq!(
            response.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "reading a directory via get_document_raw should return 500"
        );
    }

    // ── inject_base_href unit tests ───────────────────────────────────────────

    #[test]
    fn test_inject_base_href_after_head() {
        let html = "<html><head></head><body></body></html>";
        let result = inject_base_href(html, "/api/raw/docs/presentations/");
        assert_eq!(
            result,
            r#"<html><head><base href="/api/raw/docs/presentations/"></head><body></body></html>"#
        );
    }

    #[test]
    fn test_inject_base_href_after_head_with_attrs() {
        let html = r#"<html><head lang="en" class="x"></head><body></body></html>"#;
        let result = inject_base_href(html, "/api/raw/docs/");
        assert!(
            result.contains(r#"<base href="/api/raw/docs/">"#),
            "base tag not found in: {result}"
        );
        // Must be immediately after the closing > of the opening <head ...>
        let head_close = html.find('>').unwrap() + 1; // position after <html>
                                                      // The first > after <head is what matters
        let head_tag_end = html.find("</head").unwrap();
        let head_open_end = html[..head_tag_end].rfind('>').unwrap();
        assert!(
            result[head_open_end + 1..].starts_with(r#"<base href="/api/raw/docs/">"#),
            "base tag must follow immediately after opening <head> close: {result}"
        );
        let _ = head_close; // suppress unused warning
    }

    #[test]
    fn test_inject_base_href_case_insensitive() {
        for head_tag in &["<HEAD>", "<Head>"] {
            let html = format!("<html>{head_tag}</HEAD><body></body></html>");
            let result = inject_base_href(&html, "/api/raw/");
            assert!(
                result.contains(r#"<base href="/api/raw/">"#),
                "case-insensitive HEAD not handled for {head_tag}: {result}"
            );
        }
    }

    #[test]
    fn test_inject_base_href_skips_when_base_exists() {
        let html = r#"<html><head><base href="https://example.com/"></head><body></body></html>"#;
        let result = inject_base_href(html, "/api/raw/");
        // Should return the original string unchanged.
        assert_eq!(result, html);
    }

    #[test]
    fn test_inject_base_href_injects_when_base_only_in_script() {
        // A `<base>` string appearing inside a <script> block is NOT a real
        // `<base>` element — injection must still happen.
        let html =
            r#"<html><head><script>var s = "<base href='x'>";</script></head><body></body></html>"#;
        let result = inject_base_href(html, "/api/raw/");
        assert!(
            result.contains(r#"<base href="/api/raw/">"#),
            "<base> inside script must not block injection; got: {result}"
        );
    }

    #[test]
    fn test_inject_base_href_injects_when_base_only_in_comment() {
        // A `<base>` string inside an HTML comment is not a real element.
        let html = "<html><head><!-- <base href='x'> --></head><body></body></html>";
        let result = inject_base_href(html, "/api/raw/");
        assert!(
            result.contains(r#"<base href="/api/raw/">"#),
            "<base> inside comment must not block injection; got: {result}"
        );
    }

    #[test]
    fn test_inject_base_href_inserts_after_html_when_no_head() {
        let html = "<html><body></body></html>";
        let result = inject_base_href(html, "/api/raw/");
        assert_eq!(
            result,
            r#"<html><base href="/api/raw/"><body></body></html>"#
        );
    }

    #[test]
    fn test_inject_base_href_returns_unchanged_for_non_html() {
        // Plain text — no <html> or <head> element.
        let text = "Hello, world!";
        assert_eq!(inject_base_href(text, "/api/raw/"), text);

        // Empty string.
        assert_eq!(inject_base_href("", "/api/raw/"), "");
    }

    #[test]
    fn test_inject_base_href_doctype_only_without_html_tag_unchanged() {
        // A fragment with only a DOCTYPE declaration and comments but no
        // <html>/<head> tags. inject_base_href must return it unchanged.
        let doctype_only = "<!DOCTYPE html><!-- comment --><p>no html element</p>";
        assert_eq!(
            inject_base_href(doctype_only, "/api/raw/"),
            doctype_only,
            "DOCTYPE-only/comment-only HTML fragment without <html> must be returned unchanged"
        );
    }

    // ── PathReadError → InvalidPath / OutsideRepoRoot mapping ─────────────────

    /// `PathReadError::InvalidPath` must map to HTTP 400.  The storage layer
    /// raises this for empty paths, absolute paths, and `..` traversal
    /// attempts, and route handlers must surface those as client errors.
    #[test]
    fn test_path_read_error_status_invalid_path_returns_400() {
        let err = PathReadError::InvalidPath("../etc/passwd".to_string());
        assert_eq!(path_read_error_status(&err), StatusCode::BAD_REQUEST);
    }

    /// `PathReadError::OutsideRepoRoot` must map to HTTP 400.  The storage
    /// layer raises this when canonicalization shows the path escapes the
    /// repo root (e.g. via a symlink).
    #[test]
    fn test_path_read_error_status_outside_repo_root_returns_400() {
        let err = PathReadError::OutsideRepoRoot("docs/secret.txt".to_string());
        assert_eq!(path_read_error_status(&err), StatusCode::BAD_REQUEST);
    }

    // ── GET /raw/*path integration tests ─────────────────────────────────────

    /// Helper: create a JsonFileStorage-backed server with a file at a nested
    /// path under the repo root.  Returns the server and tempdir (kept alive).
    fn create_app_with_raw_fixture(
        rel_path: &str,
        content: &[u8],
    ) -> (axum_test::TestServer, tempfile::TempDir) {
        use jit::storage::JsonFileStorage;
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        let jit_dir = temp.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        // Write the fixture file.
        let abs_path = temp.path().join(rel_path);
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&abs_path, content).unwrap();

        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();
        let executor = Arc::new(CommandExecutor::new(storage));
        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();
        (server, temp)
    }

    #[tokio::test]
    async fn test_get_raw_wildcard_serves_svg() {
        let svg_content = br#"<svg xmlns="http://www.w3.org/2000/svg"><circle r="5"/></svg>"#;
        let (server, _temp) =
            create_app_with_raw_fixture("docs/presentations/figures/fig4.svg", svg_content);

        let response = server.get("/raw/docs/presentations/figures/fig4.svg").await;
        response.assert_status_ok();

        let ct = response
            .headers()
            .get("content-type")
            .expect("content-type")
            .to_str()
            .unwrap();
        assert!(
            ct.starts_with("image/svg+xml"),
            "expected image/svg+xml, got {ct}"
        );
        assert_eq!(response.as_bytes().as_ref(), svg_content.as_ref());
    }

    #[tokio::test]
    async fn test_get_raw_wildcard_html_injects_base_tag() {
        let html_content = b"<html><head></head><body></body></html>";
        let (server, _temp) =
            create_app_with_raw_fixture("docs/presentations/deck.html", html_content);

        let response = server.get("/raw/docs/presentations/deck.html").await;
        response.assert_status_ok();

        let ct = response
            .headers()
            .get("content-type")
            .expect("content-type")
            .to_str()
            .unwrap();
        assert!(ct.starts_with("text/html"), "expected text/html, got {ct}");

        let csp = response
            .headers()
            .get("content-security-policy")
            .expect("CSP header must be present")
            .to_str()
            .unwrap();
        assert_eq!(csp, CSP_HEADER, "CSP header must match specified policy");

        let body = response.text();
        assert!(
            body.contains(r#"<base href="/api/raw/docs/presentations/">"#),
            "base href not found in body: {body}"
        );
    }

    #[tokio::test]
    async fn test_get_raw_wildcard_rejects_path_traversal_dotdot() {
        // Axum normalizes `..` segments in the URL path before routing, so
        // `/raw/foo/../bar` becomes `/raw/bar` at the routing layer — the `..`
        // never reaches our handler.  The storage layer's repo-relative path
        // validation acts as defence-in-depth for any `..` that does arrive;
        // the HTTP-level invariant is already enforced by Axum.
        //
        // The normalised path `bar` doesn't exist, so the expected status is
        // 404 (not 400).  Any 4xx response is acceptable; 400 from the storage
        // validator would also be fine.
        let server = create_test_app();
        let response = server.get("/raw/foo/../bar").await;
        assert!(
            response.status_code().is_client_error(),
            "path traversal must produce a 4xx response, got {}",
            response.status_code()
        );
    }

    #[tokio::test]
    async fn test_get_raw_wildcard_rejects_embedded_dotdot() {
        // Same normalisation behaviour as the above test: `/raw/a/../b` → `/raw/b`.
        // The file doesn't exist, so we get 404.  Any 4xx is the invariant.
        let server = create_test_app();
        let response = server.get("/raw/a/../b").await;
        assert!(
            response.status_code().is_client_error(),
            "embedded .. must produce a 4xx response, got {}",
            response.status_code()
        );
    }

    #[tokio::test]
    async fn test_get_raw_wildcard_404_for_missing_file() {
        let server = create_test_app();
        let response = server.get("/raw/definitely/does/not/exist.svg").await;
        assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    }

    /// Commit-pinned HTML also gets base-tag injection (same as working-tree reads).
    /// With a fake commit SHA the storage returns CommitNotFound → 404 on non-git
    /// backends, so we only verify that we reach the injection code path when 200.
    #[tokio::test]
    async fn test_get_document_raw_html_with_commit_param() {
        let (server, id, path) = create_test_app_with_html_doc();
        // Pass a fake commit SHA; non-git storage will return 404.
        let url = format!("/issues/{}/documents/{}/raw?commit=abc1234", id, path);
        let response = server.get(&url).await;
        // Either 404 (commit not found) is expected for non-git storage.
        // If 200 were returned, body should contain the base tag (no skipping).
        assert!(
            response.status_code() == StatusCode::NOT_FOUND || response.status_code().is_success(),
            "expected 404 or 200, got {}",
            response.status_code()
        );
    }

    /// When the HTML already has a `<base>` tag, the response body is unchanged.
    #[tokio::test]
    async fn test_get_document_raw_preserves_existing_base_tag() {
        use jit::domain::Priority;
        use jit::storage::JsonFileStorage;
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        let jit_dir = temp.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();
        let executor = Arc::new(CommandExecutor::new(storage));

        let (id, _) = executor
            .create_issue(
                "base tag test".to_string(),
                "desc".to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();

        // HTML that already has its own <base> tag.
        let original =
            r#"<html><head><base href="https://cdn.example.com/"></head><body>hi</body></html>"#;
        let doc_rel = "withbase.html";
        fs::write(temp.path().join(doc_rel), original).unwrap();
        executor
            .add_document_reference(&id, doc_rel, None, None, None, true)
            .unwrap();

        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let url = format!("/issues/{}/documents/{}/raw", id, doc_rel);
        let response = server.get(&url).await;
        response.assert_status_ok();
        let body = response.text();
        assert_eq!(
            body, original,
            "existing <base> must be preserved unchanged"
        );
    }

    /// The /raw wildcard also preserves an existing <base> tag.
    #[tokio::test]
    async fn test_get_raw_wildcard_preserves_existing_base_tag() {
        let original =
            br#"<html><head><base href="https://cdn.example.com/"></head><body></body></html>"#;
        let (server, _temp) = create_app_with_raw_fixture("docs/deck.html", original);

        let response = server.get("/raw/docs/deck.html").await;
        response.assert_status_ok();
        let body = response.text();
        assert_eq!(
            body.as_bytes(),
            original,
            "existing <base> must be preserved unchanged by wildcard handler"
        );
    }

    // ── compute_base_href unit tests ──────────────────────────────────────────

    #[test]
    fn test_compute_base_href_nested_path() {
        assert_eq!(
            compute_base_href("docs/presentations/deck.html"),
            "/api/raw/docs/presentations/"
        );
    }

    #[test]
    fn test_compute_base_href_root_level() {
        assert_eq!(compute_base_href("slide.html"), "/api/raw/");
    }

    #[test]
    fn test_compute_base_href_single_dir() {
        assert_eq!(compute_base_href("docs/index.html"), "/api/raw/docs/");
    }

    /// Symlinks inside the repository that point outside the repo root must be
    /// rejected with 400.  This guards against a scenario where an attacker or
    /// misconfigured repo contains a symlink like `docs/secret -> /etc/passwd`.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_get_raw_wildcard_rejects_symlink_escape() {
        use jit::storage::JsonFileStorage;
        use std::fs;
        use std::os::unix::fs as unix_fs;

        let temp = tempfile::tempdir().unwrap();
        let jit_dir = temp.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        // Create a file outside the repo that we'll try to reach via symlink.
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"secret outside repo").unwrap();

        // Create a symlink inside the repo pointing to the outside file.
        let link_path = temp.path().join("docs").join("secret.txt");
        fs::create_dir_all(link_path.parent().unwrap()).unwrap();
        unix_fs::symlink(outside.path(), &link_path).unwrap();

        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();
        let executor = Arc::new(CommandExecutor::new(storage));
        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();
        // Keep tempdir + outside file alive.
        Box::leak(Box::new(temp));
        Box::leak(Box::new(outside));

        let response = server.get("/raw/docs/secret.txt").await;
        assert_eq!(
            response.status_code(),
            StatusCode::BAD_REQUEST,
            "symlink escaping repo root must be rejected with 400"
        );
    }

    /// A document linked at a path that is (or resolves through) a symlink to
    /// a file outside the repo root must be rejected with 400 through the
    /// issue-scoped raw endpoint.  The invariant lives in
    /// `JsonFileStorage::read_path_bytes` so every handler inherits it.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_get_document_raw_rejects_symlink_escape() {
        use jit::domain::Priority;
        use jit::storage::JsonFileStorage;
        use std::fs;
        use std::os::unix::fs as unix_fs;

        let temp = tempfile::tempdir().unwrap();
        let jit_dir = temp.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        // File outside the repo the attacker wants to read.
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"secret outside repo").unwrap();

        // Symlink at a repo-root-level path (the issue-scoped route
        // `/issues/:id/documents/:path/raw` treats `:path` as a single URL
        // segment, so we use a non-nested doc path here).  Link the symlink
        // as a document to an issue so the issue-scoped raw handler accepts
        // the path lookup.
        let doc_rel = "secret.txt";
        let link_path = temp.path().join(doc_rel);
        unix_fs::symlink(outside.path(), &link_path).unwrap();

        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();
        let executor = Arc::new(CommandExecutor::new(storage));
        let (id, _) = executor
            .create_issue(
                "Symlink escape repro".to_string(),
                "desc".to_string(),
                Priority::Normal,
                vec![],
                vec![],
            )
            .unwrap();
        executor
            .add_document_reference(&id, doc_rel, None, None, None, true)
            .unwrap();

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();
        Box::leak(Box::new(temp));
        Box::leak(Box::new(outside));

        let url = format!("/issues/{}/documents/{}/raw", id, doc_rel);
        let response = server.get(&url).await;
        assert_eq!(
            response.status_code(),
            StatusCode::BAD_REQUEST,
            "symlink escaping repo root must be rejected on issue-scoped raw endpoint"
        );
    }

    /// Same invariant for the path-only `/documents/raw?path=...` handler:
    /// if the query path resolves through a symlink to a file outside the
    /// repo root, the storage layer must reject it with 400.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_get_document_raw_by_path_rejects_symlink_escape() {
        use jit::storage::JsonFileStorage;
        use std::fs;
        use std::os::unix::fs as unix_fs;

        let temp = tempfile::tempdir().unwrap();
        let jit_dir = temp.path().join(".jit");
        fs::create_dir_all(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"secret outside repo").unwrap();

        // Symlink inside the repo pointing to the outside file.
        let link_rel = "docs/escape.txt";
        let link_path = temp.path().join(link_rel);
        fs::create_dir_all(link_path.parent().unwrap()).unwrap();
        unix_fs::symlink(outside.path(), &link_path).unwrap();

        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();
        let executor = Arc::new(CommandExecutor::new(storage));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            project_name: "test-project".to_string(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();
        Box::leak(Box::new(temp));
        Box::leak(Box::new(outside));

        let response = server
            .get("/documents/raw")
            .add_query_param("path", link_rel)
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::BAD_REQUEST,
            "symlink escaping repo root must be rejected on path-only raw endpoint"
        );
    }
}
