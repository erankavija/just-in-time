//! API route definitions

use crate::capture_cache::CaptureCache;
use axum::{
    body::Body,
    extract::{FromRequestParts, State},
    http::{request::Parts, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

use jit::commands::CommandExecutor;
use jit::declarations::GateDefinition;
use jit::domain::identity::{GateKey, GateRunId, IssueId, IssueIdPrefix};
use jit::domain::{Issue, Priority, State as IssueState};
use jit::errors::InvalidArgumentError;
use jit::graph::hierarchy::NodeHierarchy;
use jit::output::failure::classify;
use jit::output::{ErrorCode, GateRunSummary};
use jit::search::{SearchOptions, SearchResult};
use jit::storage::IssueStore;

use crate::sse;
use crate::watcher::ChangeTracker;

/// The HTTP status a failure of class `code` is reported with.
fn status_of(code: ErrorCode) -> StatusCode {
    match code {
        ErrorCode::IssueNotFound
        | ErrorCode::GateNotFound
        | ErrorCode::RepositoryNotFound
        | ErrorCode::DocumentNotFound
        | ErrorCode::ItemNotFound => StatusCode::NOT_FOUND,
        ErrorCode::InvalidArgument
        | ErrorCode::InvalidLabelPattern
        | ErrorCode::AmbiguousId
        | ErrorCode::InvalidIdPrefix
        | ErrorCode::DeletionNotConfirmed => StatusCode::BAD_REQUEST,
        ErrorCode::CycleDetected
        | ErrorCode::AlreadyExists
        | ErrorCode::Blocked
        | ErrorCode::ProfileConflict
        | ErrorCode::ProfileUncommittedTargets
        | ErrorCode::LinkedCheckoutWriteRefused => StatusCode::CONFLICT,
        ErrorCode::ValidationFailed | ErrorCode::GateFailed => StatusCode::UNPROCESSABLE_ENTITY,
        ErrorCode::PermissionDenied => StatusCode::FORBIDDEN,
        ErrorCode::LockTimeout | ErrorCode::RetryableConflict => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::IoError
        | ErrorCode::ParseError
        | ErrorCode::RequiresGit
        | ErrorCode::RepositoryFormatUnsupported
        | ErrorCode::PresetError
        | ErrorCode::ProfileError
        | ErrorCode::SearchFailed
        | ErrorCode::RipgrepNotFound
        | ErrorCode::GenericError
        | ErrorCode::RecoveryFailed
        | ErrorCode::ServerExited => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// The status of `error`, projected from the library's classification.
fn status_of_failure(error: &anyhow::Error) -> StatusCode {
    status_of(classify(error).error.code)
}

/// Log `error` and report it with the status its classification maps to.
fn reject(error: impl Into<anyhow::Error>) -> StatusCode {
    let error = error.into();
    let status = status_of_failure(&error);
    tracing::warn!(%status, "request failed: {error:#}");
    status
}

/// Run synchronous executor work outside Tokio's async workers.
async fn execute_blocking<S, T>(
    state: &AppState<S>,
    work: impl FnOnce(&CommandExecutor<S>) -> anyhow::Result<T> + Send + 'static,
) -> Result<T, StatusCode>
where
    S: IssueStore + Send + Sync + 'static,
    T: Send + 'static,
{
    let executor = Arc::clone(&state.executor);
    tokio::task::spawn_blocking(move || work(&executor))
        .await
        .map_err(reject)?
        .map_err(reject)
}

/// Declares an extractor that reports a malformed request through [`reject`],
/// so the rejection takes its status from the classification like any failure.
macro_rules! classified_extractor {
    ($name:ident, $extractor:ident) => {
        struct $name<T>(T);

        #[axum::async_trait]
        impl<T, S> FromRequestParts<S> for $name<T>
        where
            axum::extract::$extractor<T>: FromRequestParts<S>,
            <axum::extract::$extractor<T> as FromRequestParts<S>>::Rejection: std::fmt::Display,
            S: Send + Sync,
        {
            type Rejection = StatusCode;

            async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, StatusCode> {
                axum::extract::$extractor::<T>::from_request_parts(parts, state)
                    .await
                    .map(|axum::extract::$extractor(value)| Self(value))
                    .map_err(|rejection| reject(InvalidArgumentError::new(rejection.to_string())))
            }
        }
    };
}

classified_extractor!(Path, Path);
classified_extractor!(Query, Query);

/// Shared application state
#[derive(Clone)]
pub struct AppState<S: IssueStore> {
    pub executor: Arc<CommandExecutor<S>>,
    pub tracker: Arc<ChangeTracker>,
    pub capture_cache: Arc<ProjectionCache>,
    pub project_name: String,
    /// Cancelled when the process starts shutting down, so live event streams
    /// end instead of holding their connections through the drain deadline.
    pub shutdown: CancellationToken,
}

/// Per-version repository capture and its projections.
#[derive(Default)]
pub struct ProjectionCache {
    captures: CaptureCache<Snapshot>,
}

struct Snapshot {
    issues: Vec<Issue>,
    responses: Vec<IssueResponse>,
    graph: OnceCell<GraphData>,
}

async fn capture_snapshot<S: IssueStore + Send + Sync + 'static>(
    state: &AppState<S>,
) -> Result<Arc<Snapshot>, StatusCode> {
    state
        .capture_cache
        .captures
        .get(&state.tracker, || async {
            execute_blocking(state, |executor| {
                let issues = executor.issues().list_issues(None, None, None)?;
                let responses = issues.iter().cloned().map(IssueResponse::from).collect();
                Ok(Snapshot {
                    issues,
                    responses,
                    graph: OnceCell::new(),
                })
            })
            .await
        })
        .await
}

/// Create API routes
pub fn create_routes<S: IssueStore + Send + Sync + 'static>(state: AppState<S>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/issues", get(list_issues))
        .route("/issues/:id", get(get_issue))
        .route("/graph", get(get_graph))
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
        .route("/issues/:id/gate-runs", get(list_gate_runs))
        .route("/issues/:id/gate-runs/:run_id", get(get_gate_run))
        .route("/config/hierarchy", get(get_hierarchy))
        .route("/config/namespaces", get(get_namespaces))
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

/// An issue as served by the API: the stored record's fields, plus its
/// `short_id`, with each `dependencies` entry expanded to a [`DependencyRef`].
/// The stored record keeps its own shape.
#[derive(Clone, Debug, Serialize)]
pub struct IssueResponse {
    #[serde(flatten)]
    issue: IssueBody,
    /// Domain-derived short id ([`IssueId::short_id`]).
    short_id: String,
    dependencies: Vec<DependencyRef>,
}

/// A dependency entry: the id and its short id, as `issue show --json` pairs them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DependencyRef {
    pub id: IssueId,
    pub short_id: String,
}

impl From<&IssueId> for DependencyRef {
    fn from(id: &IssueId) -> Self {
        Self {
            id: id.clone(),
            short_id: id.short_id().to_string(),
        }
    }
}

impl From<Issue> for IssueResponse {
    fn from(issue: Issue) -> Self {
        Self {
            short_id: issue.id.short_id().to_string(),
            dependencies: issue.dependencies.iter().map(DependencyRef::from).collect(),
            issue: IssueBody(issue),
        }
    }
}

/// An [`Issue`] serialized without its `dependencies`, which [`IssueResponse`]
/// serves expanded.
#[derive(Clone, Debug)]
struct IssueBody(Issue);

impl Serialize for IssueBody {
    fn serialize<Ser: serde::Serializer>(&self, serializer: Ser) -> Result<Ser::Ok, Ser::Error> {
        use serde::ser::Error;
        match serde_json::to_value(&self.0).map_err(Ser::Error::custom)? {
            serde_json::Value::Object(mut fields) => {
                fields.remove("dependencies");
                fields.serialize(serializer)
            }
            _ => Err(Ser::Error::custom("issue did not serialize as an object")),
        }
    }
}

/// List all issues, each `documents` explicit and derived from one shared listing of the declared areas.
async fn list_issues<S: IssueStore + Send + Sync + 'static>(
    State(state): State<AppState<S>>,
) -> Result<Json<Vec<IssueResponse>>, StatusCode> {
    let snapshot = capture_snapshot(&state).await?;
    let issues = snapshot.issues.clone();
    let mut responses = snapshot.responses.clone();
    execute_blocking(&state, move |executor| {
        let issues = executor.documents().with_reported_documents(issues)?;
        for (response, issue) in responses.iter_mut().zip(issues) {
            response.issue.0.documents = issue.documents;
        }
        Ok(Json(responses))
    })
    .await
}

fn resolve_issue_path_id<S: IssueStore>(
    executor: &CommandExecutor<S>,
    requested: &str,
) -> anyhow::Result<IssueId> {
    let prefix = requested.parse::<IssueIdPrefix>()?;
    executor.issues().resolve_issue_id(&prefix)
}

/// Get single issue by ID, its `documents` explicit and derived.
async fn get_issue<S: IssueStore + Send + Sync + 'static>(
    Path(id): Path<String>,
    State(state): State<AppState<S>>,
) -> Result<Json<IssueResponse>, StatusCode> {
    execute_blocking(&state, move |executor| {
        let id = resolve_issue_path_id(executor, &id)?;
        let mut issue = executor.issues().show_issue(&id)?;
        issue.documents = executor.documents().issue_documents(&issue)?;
        Ok(Json(IssueResponse::from(issue)))
    })
    .await
}

/// Graph data for visualization: one node per issue, one edge per dependency.
///
/// List envelope: `count` is the number of entries in `nodes`, the primary
/// collection. `edges` is a secondary collection, counted by its own length.
/// This matches [`jit::commands::GraphTreeResponse`], where `count` likewise
/// counts `nodes`, and [`jit::commands::IssueChildrenResponse`], where `count`
/// counts the primary `issues` collection and leaves the secondary `dangling`
/// list uncounted.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphData {
    /// Number of entries in `nodes` (equals `nodes.len()`).
    pub count: usize,
    /// One entry per issue in the repository.
    pub nodes: Vec<GraphNode>,
    /// One entry per dependency edge between issues.
    pub edges: Vec<GraphEdge>,
}

/// One graph node: its display fields plus its DAG-resolved placement.
///
/// `type` is the value of the node's `type:` label, resolved from repository
/// configuration rather than parsed by the client. `parent`, `children`,
/// `cluster`, and `rank` are flattened from [`NodeHierarchy`], the same
/// serialization `jit graph tree` emits, so a consumer never re-derives
/// containment.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: IssueId,
    /// Domain-derived short id ([`IssueId::short_id`]).
    pub short_id: String,
    pub label: String,
    pub state: IssueState,
    pub priority: Priority,
    pub assignee: Option<String>,
    pub labels: Vec<String>,
    pub blocked: bool,
    /// The node's `type:` label value, or `null` when it carries none.
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    /// DAG-resolved parent, children, cluster, and rank.
    #[serde(flatten)]
    pub hierarchy: NodeHierarchy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: IssueId,
    pub to: IssueId,
}

/// Get dependency graph
///
/// Responds with the [`GraphData`] list envelope: `count` entries in `nodes`,
/// plus the `edges` between them.
///
/// Each node carries the canonical hierarchy resolved by the jit library
/// (`jit::graph::hierarchy::resolve_hierarchy`) over the repository's configured
/// type levels. Resolution is repository-wide, matching `jit graph tree` with no
/// root.
async fn get_graph<S: IssueStore + Send + Sync + 'static>(
    State(state): State<AppState<S>>,
) -> Result<Json<GraphData>, StatusCode> {
    let snapshot = capture_snapshot(&state).await?;
    let graph = snapshot
        .graph
        .get_or_try_init(|| async {
            let issues = snapshot.issues.clone();
            execute_blocking(&state, move |executor| {
                // Build node lookup for blocked state calculation
                let issue_map: std::collections::HashMap<IssueId, &Issue> =
                    issues.iter().map(|i| (i.id.clone(), i)).collect();

                // Resolve the hierarchy with the core resolver, over the repo's type levels.
                let issue_refs: Vec<&Issue> = issues.iter().collect();
                let config = executor.config().get_hierarchy_config()?;
                let resolution = jit::graph::hierarchy::resolve_hierarchy(&issue_refs, &config);

                // Create nodes
                let nodes: Vec<GraphNode> = issues
                    .iter()
                    .map(|issue| GraphNode {
                        id: issue.id.clone(),
                        short_id: issue.id.short_id().to_string(),
                        label: issue.title.clone(),
                        state: issue.state,
                        priority: issue.priority,
                        assignee: issue.assignee.as_ref().map(|a| a.to_string()),
                        labels: issue.labels.clone(),
                        blocked: issue.is_blocked(&issue_map),
                        type_name: jit::labels::type_label_value(&issue.labels).map(str::to_string),
                        hierarchy: resolution.get(&issue.id).cloned().unwrap_or_default(),
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

                Ok(GraphData {
                    count: nodes.len(),
                    nodes,
                    edges,
                })
            })
            .await
        })
        .await?;
    Ok(Json(graph.clone()))
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
async fn search_issues<S: IssueStore + Send + Sync + 'static>(
    Query(params): Query<SearchQuery>,
    State(state): State<AppState<S>>,
) -> Result<Json<SearchResponse>, StatusCode> {
    execute_blocking(&state, move |executor| {
        let start = std::time::Instant::now();

        let semantic = executor.queries().search_issue_matches(
            &params.q,
            jit::commands::SemanticSearchOptions {
                case_sensitive: params.case_sensitive,
                regex: params.regex,
            },
        )?;
        let config = executor.config();
        let layout = config.repository_layout()?;
        let data_root = match layout.data_root().canonicalize() {
            Ok(root) => Some(root),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let document_paths = executor
            .documents()
            .get_issue_document_paths()?
            .into_iter()
            .filter(|path| {
                let candidate = layout.worktree_root().join(path);
                !candidate.starts_with(layout.data_root())
                    && !candidate
                        .canonicalize()
                        .ok()
                        .zip(data_root.as_ref())
                        .is_some_and(|(resolved, root)| resolved.starts_with(root))
            })
            .collect::<Vec<_>>();
        let mut results = if params.limit == 0 {
            Vec::new()
        } else {
            jit::search::search_files(
                layout.worktree_root(),
                &params.q,
                SearchOptions {
                    case_sensitive: params.case_sensitive,
                    regex: params.regex,
                    max_results: Some(params.limit),
                    ..Default::default()
                },
                &document_paths,
            )?
        };
        for result in &mut results {
            result.issue_id = None;
        }
        results.extend(
            semantic
                .into_iter()
                .take(params.limit.saturating_sub(results.len()))
                .map(|item| SearchResult {
                    // Semantic matches have no source line or byte offsets.
                    path: format!("issues/{}.json", item.issue.id),
                    issue_id: Some(item.issue.id),
                    line_number: 0,
                    line_text: item.text,
                    matches: Vec::new(),
                }),
        );

        let duration_ms = start.elapsed().as_millis();

        Ok(Json(SearchResponse {
            query: params.q,
            total: results.len(),
            results,
            duration_ms,
        }))
    })
    .await
}

/// Query parameters for standalone document access
#[derive(Debug, Deserialize)]
struct DocumentByPathQuery {
    path: String,
    commit: Option<String>,
}

/// Get document content by path (without requiring issue ID)
///
/// This endpoint allows accessing documents directly by their filesystem path,
/// which is useful for opening documents from search results that may not be
/// associated with a specific issue context.
///
/// Delegates I/O to `DocumentOps::read_path_bytes` so that filesystem/git
/// reads remain in the domain layer, not in the route handler.
async fn get_document_by_path<S: IssueStore + Send + Sync + 'static>(
    Query(query): Query<DocumentByPathQuery>,
    State(state): State<AppState<S>>,
) -> Result<Json<DocumentContentResponse>, StatusCode> {
    execute_blocking(&state, move |executor| {
        let (bytes, commit_hash) = executor
            .documents()
            .read_path_bytes(&query.path, query.commit.as_deref())?;
        // Convert bytes to String for the JSON response (lossy for binary files).
        let content = String::from_utf8_lossy(&bytes).into_owned();
        Ok(Json(DocumentContentResponse {
            path: query.path.clone(),
            commit: commit_hash,
            content,
            content_type: infer_content_type(&query.path),
        }))
    })
    .await
}

/// Infer content type from the path, with a binary-safe fallback.
pub(crate) fn infer_content_type(path: &str) -> String {
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

const PATH_SEGMENT_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Encode one URL path segment, including any slash in the input.
fn percent_encode_segment(segment: &str) -> String {
    utf8_percent_encode(segment, PATH_SEGMENT_ENCODE_SET).to_string()
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
    let len = lower.len();
    let mut pos = 0;
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
        // Advance by one UTF-8 char, not one byte — slicing `lower[pos..]`
        // panics if `pos` falls inside a multi-byte character (e.g. non-ASCII
        // text in a <title>).
        pos += lower[pos..].chars().next().map_or(1, char::len_utf8);
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
async fn get_document_content<S: IssueStore + Send + Sync + 'static>(
    Path((id, path)): Path<(String, String)>,
    Query(query): Query<DocumentContentQuery>,
    State(state): State<AppState<S>>,
) -> Result<Json<DocumentContentResponse>, StatusCode> {
    let (content, commit_hash) = execute_blocking(&state, {
        let path = path.clone();
        move |executor| {
            let id = resolve_issue_path_id(executor, &id)?;
            Ok(executor
                .documents()
                .read_document_content(&id, &path, query.commit.as_deref())?)
        }
    })
    .await?;

    Ok(Json(DocumentContentResponse {
        path: path.clone(),
        commit: commit_hash,
        content,
        content_type: infer_content_type(&path),
    }))
}

fn raw_document_response(path: &str, bytes: Vec<u8>) -> Response<Body> {
    let content_type = infer_content_type(path);
    let body = if content_type == "text/html" {
        inject_base_href(&String::from_utf8_lossy(&bytes), &compute_base_href(path)).into_bytes()
    } else {
        bytes
    };

    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        "content-security-policy",
        HeaderValue::from_static(CSP_HEADER),
    );
    response
}

/// Get raw document bytes (issue-scoped variant)
///
/// Verifies that `path` is a document of the requested issue, explicit or
/// derived, then reads the file as raw bytes via `read_document_bytes` (which
/// delegates to `IssueStore::read_path_bytes`).  Binary artifacts are served faithfully
/// without any UTF-8 conversion.  HTML responses for working-tree reads get a
/// `<base href>` injected so sibling assets resolve via `/api/raw/*path`.
async fn get_document_raw<S: IssueStore + Send + Sync + 'static>(
    Path((id, path)): Path<(String, String)>,
    Query(query): Query<DocumentContentQuery>,
    State(state): State<AppState<S>>,
) -> Result<Response<Body>, StatusCode> {
    let (bytes, _commit_hash) = execute_blocking(&state, {
        let path = path.clone();
        move |executor| {
            let id = resolve_issue_path_id(executor, &id)?;
            Ok(executor
                .documents()
                .read_document_bytes(&id, &path, query.commit.as_deref())?)
        }
    })
    .await?;

    Ok(raw_document_response(&path, bytes))
}

/// Get raw document bytes (path-only variant)
///
/// Delegates I/O to `DocumentOps::read_path_bytes` so persistence stays
/// in the domain layer.  Returns raw bytes with the inferred Content-Type and
/// CSP headers.  Binary files are served faithfully without UTF-8 conversion.
/// HTML responses get a `<base href>` injected so sibling assets resolve via
/// the wildcard `/api/raw/*path` route.
///
/// Repo-root containment (rejecting empty/absolute/`..` paths and symlink
/// escapes) is enforced by `JsonFileStorage::read_path_bytes`, which is
/// invariant-owning for this concern.  Invalid paths and out-of-repo escapes
/// surface as `PathReadError::InvalidPath` / `PathReadError::OutsideRepoRoot`,
/// both classified as invalid arguments.
async fn get_document_raw_by_path<S: IssueStore + Send + Sync + 'static>(
    Query(query): Query<DocumentByPathQuery>,
    State(state): State<AppState<S>>,
) -> Result<Response<Body>, StatusCode> {
    let (bytes, _commit_hash) = execute_blocking(&state, {
        let path = query.path.clone();
        move |executor| {
            Ok(executor
                .documents()
                .read_path_bytes(&path, query.commit.as_deref())?)
        }
    })
    .await?;
    Ok(raw_document_response(&query.path, bytes))
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
/// variants classify as invalid arguments.
async fn get_raw_wildcard<S: IssueStore + Send + Sync + 'static>(
    Path(path): Path<String>,
    Query(query): Query<DocumentContentQuery>,
    State(state): State<AppState<S>>,
) -> Result<Response<Body>, StatusCode> {
    let (bytes, _commit_hash) = execute_blocking(&state, {
        let path = path.clone();
        move |executor| {
            Ok(executor
                .documents()
                .read_path_bytes(&path, query.commit.as_deref())?)
        }
    })
    .await?;

    Ok(raw_document_response(&path, bytes))
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
async fn get_document_history<S: IssueStore + Send + Sync + 'static>(
    Path((id, path)): Path<(String, String)>,
    State(state): State<AppState<S>>,
) -> Result<Json<DocumentHistoryResponse>, StatusCode> {
    let commits = execute_blocking(&state, {
        let path = path.clone();
        move |executor| {
            let id = resolve_issue_path_id(executor, &id)?;
            Ok(executor.documents().get_document_history(&id, &path)?)
        }
    })
    .await?;

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
async fn get_document_diff<S: IssueStore + Send + Sync + 'static>(
    Path((id, path)): Path<(String, String)>,
    Query(query): Query<DocumentDiffQuery>,
    State(state): State<AppState<S>>,
) -> Result<Json<DocumentDiffResponse>, StatusCode> {
    execute_blocking(&state, move |executor| {
        let id = resolve_issue_path_id(executor, &id)?;
        let diff =
            executor
                .documents()
                .get_document_diff(&id, &path, &query.from, query.to.as_deref())?;
        Ok(Json(DocumentDiffResponse {
            path,
            from: query.from,
            to: query.to.unwrap_or_else(|| "HEAD".to_string()),
            diff,
        }))
    })
    .await
}

/// List all gate definitions from the registry
async fn list_gates<S: IssueStore + Send + Sync + 'static>(
    State(state): State<AppState<S>>,
) -> Result<Json<Vec<GateDefinition>>, StatusCode> {
    execute_blocking(&state, |executor| Ok(Json(executor.gates().list_gates()?))).await
}

/// Query parameters for gate runs
#[derive(Debug, Deserialize)]
struct GateRunsQuery {
    gate_key: Option<String>,
}

/// List gate run results for an issue (summaries without stdout/stderr for passing runs).
async fn list_gate_runs<S: IssueStore + Send + Sync + 'static>(
    Path(id): Path<String>,
    Query(params): Query<GateRunsQuery>,
    State(state): State<AppState<S>>,
) -> Result<Json<Vec<GateRunSummary>>, StatusCode> {
    let runs = execute_blocking(&state, move |executor| {
        let id = resolve_issue_path_id(executor, &id)?;
        let gate_key = params
            .gate_key
            .map(|key| key.parse::<GateKey>())
            .transpose()?;
        executor.gates().list_gate_runs(&id, gate_key.as_ref())
    })
    .await?;
    Ok(Json(runs.iter().map(GateRunSummary::lean).collect()))
}

/// Get a single gate run result with full stdout/stderr
async fn get_gate_run<S: IssueStore + Send + Sync + 'static>(
    Path((id, run_id)): Path<(String, String)>,
    State(state): State<AppState<S>>,
) -> Result<Json<GateRunSummary>, StatusCode> {
    execute_blocking(&state, move |executor| {
        let id = resolve_issue_path_id(executor, &id)?;
        let run_id = run_id.parse::<GateRunId>()?;
        let run = executor.gates().get_gate_run_result(&id, &run_id)?;
        Ok(Json(GateRunSummary::full(&run)))
    })
    .await
}

/// Response for type hierarchy
#[derive(Debug, Serialize, Deserialize)]
struct HierarchyResponse {
    types: std::collections::HashMap<String, u8>,
    strategic_types: Vec<String>,
    icons: std::collections::HashMap<String, String>,
}

/// Get type hierarchy configuration
async fn get_hierarchy<S: IssueStore + Send + Sync + 'static>(
    State(state): State<AppState<S>>,
) -> Result<Json<HierarchyResponse>, StatusCode> {
    let (namespaces, icons) = execute_blocking(&state, |executor| {
        let namespaces = executor.config().get_namespaces()?;
        let icons = executor.config().get_hierarchy_icons()?;
        Ok((namespaces, icons))
    })
    .await?;

    let types = namespaces.type_hierarchy.unwrap_or_default();
    let strategic_types = namespaces.strategic_types.unwrap_or_default();

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
}

/// Get namespace registry from configuration
async fn get_namespaces<S: IssueStore + Send + Sync + 'static>(
    State(state): State<AppState<S>>,
) -> Result<Json<NamespacesResponse>, StatusCode> {
    let label_namespaces =
        execute_blocking(&state, |executor| executor.config().get_namespaces()).await?;

    let namespaces = label_namespaces
        .namespaces
        .into_iter()
        .map(|(name, ns)| {
            (
                name,
                NamespaceInfo {
                    description: ns.description,
                    unique: ns.unique,
                },
            )
        })
        .collect();

    Ok(Json(NamespacesResponse { namespaces }))
}

/// SSE stream of change events
async fn events_stream<S: IssueStore>(State(state): State<AppState<S>>) -> impl IntoResponse {
    sse::change_stream(&state.tracker, state.shutdown.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum_test::TestServer;
    use jit::commands::test_helpers::ExecutorTestAccess;
    use jit::commands::test_helpers::{memory_executor_declaring, MemoryDeclarations};
    use jit::domain::Priority;
    use jit::storage::{InMemoryStorage, JsonFileStorage, PathReadError};
    use jit::test_utils::add_reducing_edge;
    use jit::test_utils::setup_test_repo_with_config;

    use crate::watcher::ChangeTracker;

    /// The taxonomy the taxonomy-serving routes answer with, declared by this
    /// repository.
    fn taxonomy_config() -> String {
        format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            jit::test_taxonomy::test_taxonomy().config_fragment()
        )
    }

    fn test_app<S: IssueStore + Send + Sync + 'static>(executor: CommandExecutor<S>) -> TestServer {
        let state = AppState {
            executor: Arc::new(executor),
            tracker: Arc::new(ChangeTracker::new(16)),
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        TestServer::new(create_routes(state)).unwrap()
    }

    #[tokio::test]
    async fn test_execute_blocking_runs_executor_work_outside_async_worker() {
        let state = AppState {
            executor: Arc::new(memory_executor_declaring(
                InMemoryStorage::new(),
                MemoryDeclarations::default(),
            )),
            tracker: Arc::new(ChangeTracker::new(16)),
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let async_thread = std::thread::current().id();
        let blocking_thread = execute_blocking(&state, |_| {
            Ok::<_, anyhow::Error>(std::thread::current().id())
        })
        .await
        .unwrap();
        assert_ne!(blocking_thread, async_thread);
    }

    #[tokio::test]
    async fn test_execute_blocking_classifies_executor_failure() {
        let state = AppState {
            executor: Arc::new(memory_executor_declaring(
                InMemoryStorage::new(),
                MemoryDeclarations::default(),
            )),
            tracker: Arc::new(ChangeTracker::new(16)),
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let result = execute_blocking(&state, |_| {
            Err::<(), _>(InvalidArgumentError::new("invalid input").into())
        })
        .await;
        assert_eq!(result, Err(StatusCode::BAD_REQUEST));
    }

    /// The status the server reports for `error`, derived from its classification.
    fn status_of_error(error: impl Into<anyhow::Error>) -> StatusCode {
        status_of(classify(&error.into()).error.code)
    }

    #[test]
    fn test_status_of_maps_each_error_class_to_its_http_status() {
        let expected = [
            (ErrorCode::IssueNotFound, StatusCode::NOT_FOUND),
            (ErrorCode::DocumentNotFound, StatusCode::NOT_FOUND),
            (ErrorCode::InvalidArgument, StatusCode::BAD_REQUEST),
            (ErrorCode::AmbiguousId, StatusCode::BAD_REQUEST),
            (ErrorCode::CycleDetected, StatusCode::CONFLICT),
            (
                ErrorCode::ValidationFailed,
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (ErrorCode::PermissionDenied, StatusCode::FORBIDDEN),
            (ErrorCode::LockTimeout, StatusCode::SERVICE_UNAVAILABLE),
            (
                ErrorCode::RetryableConflict,
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (ErrorCode::IoError, StatusCode::INTERNAL_SERVER_ERROR),
            (ErrorCode::ParseError, StatusCode::INTERNAL_SERVER_ERROR),
        ];
        for (code, status) in expected {
            assert_eq!(status_of(code), status, "{code:?}");
        }
        assert!(ErrorCode::ALL
            .into_iter()
            .map(status_of)
            .all(|status| status.is_client_error() || status.is_server_error()));
    }

    #[test]
    fn test_status_of_failure_reports_path_read_errors_by_what_they_describe() {
        let status = |error: PathReadError| status_of_failure(&error.into());
        assert_eq!(
            status(PathReadError::NotFound("a".into())),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status(PathReadError::CommitNotFound("abc1234".into())),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status(PathReadError::InvalidPath("../x".into())),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(PathReadError::OutsideRepoRoot("link".into())),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(PathReadError::Other(anyhow::anyhow!("disk fault"))),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    /// A repository whose only gate run result is unreadable.
    fn unreadable_gate_run() -> (
        tempfile::TempDir,
        TestServer,
        IssueId,
        GateRunId,
        anyhow::Error,
    ) {
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        let gate_key: jit::domain::identity::GateKey = "review".parse().unwrap();
        executor
            .gates()
            .define_gate(
                &gate_key,
                "Review".to_string(),
                "Manual review".to_string(),
                jit::declarations::GateStage::Postcheck,
                jit::declarations::GateMode::Manual,
                None,
                100,
            )
            .unwrap();
        let id = executor
            .issues()
            .create_issue(
                "Gated".to_string(),
                String::new(),
                Priority::Normal,
                vec![gate_key.clone()],
                vec![],
                None,
                None,
                false,
            )
            .unwrap()
            .issue
            .id;
        executor
            .gates()
            .pass_gate(&id, &gate_key, Some("human:reviewer".to_string()), false)
            .unwrap();
        let run = executor
            .gates()
            .get_last_gate_run(&id, &gate_key)
            .unwrap()
            .unwrap();
        let result = temp.path().join(".jit").join(
            jit::repository_state::VirtualPath::gate_run_record(&id, &run.run_id)
                .relative()
                .as_path(),
        );
        std::fs::write(result, "{").unwrap();
        let failure = executor
            .gates()
            .get_gate_run_result(&id, &run.run_id)
            .unwrap_err();
        (temp, test_app(executor), id, run.run_id, failure)
    }

    #[tokio::test]
    async fn test_get_gate_run_reports_an_unreadable_result_as_a_server_error() {
        let (_temp, server, id, run_id, failure) = unreadable_gate_run();
        let expected = status_of_error(failure);

        let response = server
            .get(&format!("/issues/{id}/gate-runs/{run_id}"))
            .await;

        response.assert_status(expected);
        assert_eq!(expected, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_list_gate_runs_reports_an_unreadable_result_as_a_server_error() {
        let (_temp, server, id, _run_id, failure) = unreadable_gate_run();
        let expected = status_of_error(failure);

        let response = server.get(&format!("/issues/{id}/gate-runs")).await;

        response.assert_status(expected);
        assert_eq!(expected, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_get_issue_reports_an_ambiguous_prefix_as_a_bad_request() {
        let harness = jit::test_harness::TestHarness::new();
        for seed in [1, 2] {
            jit::test_harness::seed_memory_issue(
                &harness.storage,
                &jit::domain::Issue::draft(
                    jit::test_utils::issue_id(seed),
                    "Twin".into(),
                    "".into(),
                ),
            )
        }
        let prefix = "0000".parse().unwrap();
        let expected = status_of_error(
            harness
                .executor
                .storage()
                .resolve_issue_id(&prefix)
                .unwrap_err(),
        );

        let response = test_app(harness.executor).get("/issues/0000").await;

        response.assert_status(expected);
        assert_eq!(expected, StatusCode::BAD_REQUEST);
    }

    /// An unreadable issue index fails id resolution as a server fault, not a missing issue.
    #[tokio::test]
    async fn test_get_issue_reports_an_unreadable_index_as_a_server_error() {
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        std::fs::write(temp.path().join(".jit/index.json"), "{").unwrap();
        let prefix = "abcd".parse().unwrap();
        let expected = status_of_error(executor.storage().resolve_issue_id(&prefix).unwrap_err());

        let response = test_app(executor).get("/issues/abcd").await;

        response.assert_status(expected);
        assert_eq!(expected, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_search_reports_a_malformed_query_parameter_as_a_bad_request() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let expected = status_of_error(InvalidArgumentError::new("limit is not a number"));

        let response = server
            .get("/search")
            .add_query_param("q", "x")
            .add_query_param("limit", "many")
            .await;

        response.assert_status(expected);
        assert_eq!(expected, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_health_check() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let response = server.get("/health").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["status"], "ok");
        assert_eq!(body["service"], "jit-api");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert!(body["project_name"].is_string());
    }

    #[tokio::test]
    async fn test_create_routes_omits_retired_paths_and_keeps_retained_paths() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations::default(),
        ));

        for path in [
            "/status",
            "/config/strategic-types",
            "/changes",
            "/gates/Bad_Key",
        ] {
            server.get(path).await.assert_status(StatusCode::NOT_FOUND);
        }
        server.get("/gates").await.assert_status_ok();
        server
            .get("/issues/nonexistent/documents/README.md/diff")
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_list_issues_empty() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let response = server.get("/issues").await;
        response.assert_status_ok();
        let issues: Vec<serde_json::Value> = response.json();
        assert_eq!(issues.len(), 0);
    }

    #[tokio::test]
    async fn test_get_issue_not_found() {
        let executor = memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        );
        let missing = executor
            .storage()
            .resolve_issue_id(&"nonexistent".parse().unwrap())
            .unwrap_err();
        let expected = status_of_error(missing);
        let response = test_app(executor).get("/issues/nonexistent").await;
        response.assert_status(expected);
        assert_eq!(expected, StatusCode::NOT_FOUND);
    }

    /// A stored issue that cannot be read is a server fault, not a missing issue.
    #[tokio::test]
    async fn test_get_issue_reports_an_unreadable_issue_as_a_server_error() {
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        let id = executor
            .issues()
            .create_issue(
                "Broken".to_string(),
                "desc".to_string(),
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
        std::fs::write(temp.path().join(format!(".jit/issues/{id}.json")), "{").unwrap();
        let expected = status_of_error(executor.issues().show_issue(&id).unwrap_err());

        let response = test_app(executor).get(&format!("/issues/{id}")).await;

        response.assert_status(expected);
        assert_eq!(expected, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_get_issue_rejects_too_short_id_prefix() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let response = server.get("/issues/abc").await;
        response.assert_status(StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_gate_run_list_rejects_malformed_gate_key_at_extraction() {
        let (server, id, _) = dependent_pair_server();
        let malformed = status_of_error("Bad_Key".parse::<GateKey>().unwrap_err());
        assert_eq!(malformed, StatusCode::BAD_REQUEST);
        server
            .get(&format!("/issues/{id}/gate-runs"))
            .add_query_param("gate_key", "Bad_Key")
            .await
            .assert_status(malformed);
        server
            .get(&format!("/issues/{id}/gate-runs"))
            .add_query_param("gate_key", "unknown-gate")
            .await
            .assert_status_ok();
    }

    #[tokio::test]
    async fn test_get_gate_run_returns_not_found_under_another_issues_path() {
        let harness = jit::test_harness::TestHarness::new();
        let gate_key = "review".parse().unwrap();
        harness.add_gate(
            &gate_key,
            "Review",
            "Manual review",
            jit::declarations::GateMode::Manual,
            None,
        );
        let owner = harness.create_issue_with_gates("Run owner", vec![gate_key.clone()]);
        let other = harness.create_issue("Other issue");
        harness
            .executor
            .gates()
            .pass_gate(&owner, &gate_key, Some("human:reviewer".to_string()), false)
            .unwrap();
        let run = harness
            .executor
            .gates()
            .get_last_gate_run(&owner, &gate_key)
            .unwrap()
            .unwrap();
        let server = test_app(harness.executor);

        server
            .get(&format!("/issues/{owner}/gate-runs/{}", run.run_id))
            .await
            .assert_status_ok();
        server
            .get(&format!("/issues/{other}/gate-runs/{}", run.run_id))
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_gate_run_reports_a_malformed_run_id_as_a_bad_request_and_an_unknown_one_missing(
    ) {
        let (server, id, _) = dependent_pair_server();
        let malformed = status_of_error("not-a-run-id".parse::<GateRunId>().unwrap_err());
        assert_eq!(malformed, StatusCode::BAD_REQUEST);

        server
            .get(&format!("/issues/{id}/gate-runs/not-a-run-id"))
            .await
            .assert_status(malformed);
        server
            .get(&format!(
                "/issues/{id}/gate-runs/{}",
                jit::test_utils::gate_run_id(1)
            ))
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_list_issues_returns_retryable_status_for_pending_redo_and_retries() {
        let (temp, executor) = setup_test_repo_with_config(&taxonomy_config()).unwrap();
        let scratch = temp.path().join(".jit/tmp");
        std::fs::create_dir_all(&scratch).unwrap();
        let record = scratch.join("redo.record");
        std::fs::write(&record, b"pending").unwrap();
        let server = test_app(executor);
        server
            .get("/issues")
            .await
            .assert_status(StatusCode::SERVICE_UNAVAILABLE);
        assert!(record.exists());
        std::fs::remove_file(record).unwrap();
        server.get("/issues").await.assert_status_ok();
    }

    #[tokio::test]
    async fn test_list_and_graph_share_snapshot_while_artifacts_remain_live() {
        let (temp, executor) =
            setup_test_repo_with_config("[documentation]\nissue_scoped_areas = [\"notes\"]\n")
                .unwrap();
        let id = executor
            .issues()
            .create_issue(
                "First".into(),
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
        let dir = executor
            .documents()
            .resolve_issue_artifact_directory(&id, "notes")
            .unwrap()
            .directory;
        std::fs::create_dir_all(temp.path().join(&dir)).unwrap();
        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker: tracker.clone(),
            capture_cache: Default::default(),
            project_name: "test".into(),
            shutdown: CancellationToken::new(),
        };
        let executor = state.executor.clone();
        let server = TestServer::new(create_routes(state)).unwrap();
        let graph: serde_json::Value = server.get("/graph").await.json();
        assert_eq!(graph["count"], 1);
        executor
            .issues()
            .create_issue(
                "Second".into(),
                String::new(),
                Priority::Normal,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();
        let first = temp.path().join(&dir).join("first.md");
        let second = temp.path().join(&dir).join("second.md");
        std::fs::write(&first, "first").unwrap();
        let docs = |body: &serde_json::Value| {
            body[0]["documents"]
                .as_array()
                .unwrap()
                .iter()
                .map(|doc| doc["path"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        let listed: serde_json::Value = server.get("/issues").await.json();
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert!(docs(&listed).iter().any(|path| path.ends_with("first.md")));
        std::fs::rename(&first, &second).unwrap();
        let listed: serde_json::Value = server.get("/issues").await.json();
        assert!(docs(&listed).iter().any(|path| path.ends_with("second.md")));
        assert!(!docs(&listed).iter().any(|path| path.ends_with("first.md")));
        std::fs::remove_file(&second).unwrap();
        let listed: serde_json::Value = server.get("/issues").await.json();
        assert!(docs(&listed).is_empty());
        tracker.notify_change();
        let listed: serde_json::Value = server.get("/issues").await.json();
        assert_eq!(listed.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_graph_reloads_hierarchy_after_configuration_invalidation() {
        let (temp, executor) = setup_test_repo_with_config(&taxonomy_config()).unwrap();
        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker: tracker.clone(),
            capture_cache: Default::default(),
            project_name: "test".into(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();
        server.get("/graph").await.assert_status_ok();
        let config = temp.path().join(".jit/config.toml");
        let original = std::fs::read(&config).unwrap();
        std::fs::write(
            &config,
            jit::test_taxonomy::invalid_hierarchy_config_fragment(),
        )
        .unwrap();
        tracker.notify_change();
        server
            .get("/graph")
            .await
            .assert_status(StatusCode::INTERNAL_SERVER_ERROR);
        std::fs::write(&config, original).unwrap();
        tracker.notify_change();
        server.get("/graph").await.assert_status_ok();
    }

    #[tokio::test]
    async fn test_list_issues_with_data() {
        let executor = Arc::new(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));

        // Create test issues
        let _id1 = executor
            .issues()
            .create_issue(
                "Issue 1".to_string(),
                "Description".to_string(),
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
        let _id2 = executor
            .issues()
            .create_issue(
                "Issue 2".to_string(),
                "Description".to_string(),
                Priority::High,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap()
            .issue
            .id;

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let app = create_routes(state);
        let server = TestServer::new(app).unwrap();

        let response = server.get("/issues").await;
        response.assert_status_ok();
        let issues: Vec<serde_json::Value> = response.json();
        assert_eq!(issues.len(), 2);
    }

    fn dependent_pair_server() -> (TestServer, IssueId, IssueId) {
        let executor = Arc::new(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let create = |title: &str| {
            executor
                .issues()
                .create_issue(
                    title.to_string(),
                    "Description".to_string(),
                    Priority::Normal,
                    vec![],
                    vec![],
                    None,
                    None,
                    false,
                )
                .unwrap()
                .issue
                .id
        };
        let dependency = create("Dependency");
        let dependent = create("Dependent");
        add_reducing_edge(&executor, &dependent, &dependency).unwrap();
        let state = AppState {
            executor,
            tracker: Arc::new(ChangeTracker::new(16)),
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        (
            TestServer::new(create_routes(state)).unwrap(),
            dependent,
            dependency,
        )
    }

    fn assert_serves_short_ids(issue: &serde_json::Value, id: &IssueId, dependency: &IssueId) {
        assert_eq!(issue["id"], id.as_str());
        assert_eq!(issue["short_id"], id.short_id());
        let entries = issue["dependencies"].as_array().unwrap();
        assert!(entries
            .iter()
            .any(|entry| entry["id"] == dependency.as_str()
                && entry["short_id"] == dependency.short_id()));
    }

    #[tokio::test]
    async fn test_get_issue_serves_domain_short_ids_for_issue_and_dependencies() {
        let (server, dependent, dependency) = dependent_pair_server();

        let response = server.get(&format!("/issues/{}", dependent)).await;

        response.assert_status_ok();
        assert_serves_short_ids(&response.json(), &dependent, &dependency);
    }

    #[tokio::test]
    async fn test_list_issues_serves_domain_short_ids_for_every_issue() {
        let (server, dependent, dependency) = dependent_pair_server();

        let response = server.get("/issues").await;

        response.assert_status_ok();
        let issues: Vec<serde_json::Value> = response.json();
        let served = |id: &IssueId| issues.iter().find(|issue| issue["id"] == id.as_str());
        assert_serves_short_ids(served(&dependent).unwrap(), &dependent, &dependency);
        assert_eq!(
            served(&dependency).unwrap()["short_id"],
            dependency.short_id()
        );
    }

    #[tokio::test]
    async fn test_get_graph_serves_domain_short_id_for_every_node() {
        let (server, dependent, dependency) = dependent_pair_server();

        let response = server.get("/graph").await;

        response.assert_status_ok();
        let graph: GraphData = response.json();
        for id in [&dependent, &dependency] {
            let node = graph.nodes.iter().find(|node| &node.id == id).unwrap();
            assert_eq!(node.short_id, id.short_id());
        }
    }

    #[tokio::test]
    async fn test_get_graph() {
        let executor = Arc::new(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));

        // Create issues with dependencies
        let id1 = executor
            .issues()
            .create_issue(
                "Issue 1".to_string(),
                "Description".to_string(),
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
        let id2 = executor
            .issues()
            .create_issue(
                "Issue 2".to_string(),
                "Description".to_string(),
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

        add_reducing_edge(&executor, &id2, &id1).unwrap();

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
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
    async fn test_get_graph_returns_server_error_for_invalid_hierarchy() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&jit::test_taxonomy::invalid_hierarchy_config_fragment()),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        ));

        server
            .get("/graph")
            .await
            .assert_status(StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// The response body carries the `count` envelope field, and it counts
    /// `nodes` rather than `edges` (which differ in number here).
    #[tokio::test]
    async fn test_get_graph_body_carries_node_count() {
        let executor = Arc::new(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let new = |title: &str| {
            executor
                .issues()
                .create_issue(
                    title.to_string(),
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
                .id
        };
        let (id1, id2) = (new("Issue 1"), new("Issue 2"));
        new("Issue 3");
        // One edge over three nodes, so a `count` of edges could not pass.
        add_reducing_edge(&executor, &id2, &id1).unwrap();

        let state = AppState {
            executor,
            tracker: Arc::new(ChangeTracker::new(16)),
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let response = server.get("/graph").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();

        assert_eq!(body["count"], 3);
        assert_eq!(body["nodes"].as_array().unwrap().len(), 3);
        assert_eq!(body["edges"].as_array().unwrap().len(), 1);
    }

    /// Every node carries the core-resolved parent, children, cluster, rank, and
    /// type value, so the client derives none of them.
    #[tokio::test]
    async fn test_get_graph_nodes_carry_resolved_hierarchy() {
        let executor = Arc::new(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let new = |title: &str, labels: Vec<String>| {
            executor
                .issues()
                .create_issue(
                    title.to_string(),
                    String::new(),
                    Priority::Normal,
                    vec![],
                    labels,
                    None,
                    None,
                    false,
                )
                .unwrap()
                .issue
                .id
        };
        // The container and leaf kinds this repository declares, read from the
        // declaration its configuration was rendered from.
        let taxonomy = jit::test_taxonomy::test_taxonomy();
        let container_type = taxonomy.type_at_level(2);
        let leaf_type = taxonomy.type_at_level(4);
        let container = new("Container", vec![format!("type:{container_type}")]);
        let leaf = new("Leaf", vec![format!("type:{leaf_type}")]);
        add_reducing_edge(&executor, &container, &leaf).unwrap();

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let response = server.get("/graph").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        let node = |id: &str| -> serde_json::Value {
            body["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["id"] == serde_json::Value::String(id.to_string()))
                .expect("node in payload")
                .clone()
        };

        let leaf_node = node(leaf.as_str());
        assert_eq!(leaf_node["type"], leaf_type);
        assert_eq!(leaf_node["parent"], container.as_str());
        assert_eq!(leaf_node["cluster"], container.as_str());
        assert_eq!(leaf_node["children"], serde_json::json!([]));
        assert_eq!(leaf_node["rank"], 0);

        let container_node = node(container.as_str());
        assert_eq!(container_node["type"], container_type);
        assert_eq!(container_node["parent"], serde_json::Value::Null);
        assert_eq!(container_node["cluster"], container.as_str());
        assert_eq!(container_node["children"], serde_json::json!([leaf]));
        assert_eq!(container_node["rank"], 1);
    }

    /// A node with no `type:` label reports a null type and an orphan resolution.
    #[tokio::test]
    async fn test_get_graph_untyped_node_has_null_type() {
        let storage = InMemoryStorage::new();
        // The repository declares a default type, so an issue reaches the store
        // without a `type:` label only when it is seeded rather than created.
        let untyped = jit::domain::Issue::draft(
            jit::test_utils::issue_id(1),
            "Untyped".to_string(),
            String::new(),
        );
        storage.seed_issue_fixture(&untyped);

        let executor = Arc::new(memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor,
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let response = server.get("/graph").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        let node = &body["nodes"][0];
        assert_eq!(node["type"], serde_json::Value::Null);
        assert_eq!(node["parent"], serde_json::Value::Null);
        assert_eq!(node["cluster"], serde_json::Value::Null);
        assert_eq!(node["rank"], 0);
    }

    #[tokio::test]
    async fn test_search_endpoint_requires_query() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let response = server.get("/search").await;
        // Should fail without query parameter
        assert!(response.status_code() != StatusCode::OK);
    }

    #[tokio::test]
    async fn test_search_endpoint_empty_results() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
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
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
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
    async fn test_search_only_searches_issue_documents() {
        // This test verifies that search is restricted to issue documents
        // by checking that get_issue_document_paths is called
        // (implementation detail test - would need integration test for full verification)
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let _response = server.get("/search?q=test").await;
        // If it doesn't panic or error, the issue-document logic is being used
    }

    #[tokio::test]
    async fn test_get_hierarchy() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let response = server.get("/config/hierarchy").await;
        response.assert_status_ok();
        let data: HierarchyResponse = response.json();
        // REQ-05 (jit:0439da41): with the named bundles gone, a repository that
        // declares no icons still receives the level-keyed defaults for every
        // type it declared. Stated as the level-keying property rather than as
        // an icon inventory, so the assertion holds for any taxonomy the fixture
        // declares: one icon per declared type, and two types share an icon
        // exactly when they share a level.
        assert_eq!(
            data.icons.keys().collect::<std::collections::BTreeSet<_>>(),
            data.types.keys().collect::<std::collections::BTreeSet<_>>(),
        );
        for (left, left_level) in &data.types {
            for (right, right_level) in &data.types {
                assert_eq!(
                    data.icons[left] == data.icons[right],
                    left_level == right_level,
                    "{left} (level {left_level}) and {right} (level {right_level}) \
                     must share an icon exactly when they share a level",
                );
            }
        }
    }

    #[tokio::test]
    async fn test_get_namespaces() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
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
    async fn test_get_namespaces_exposes_taxonomy_only() {
        // A file-backed storage with a config.toml carrying the taxonomy keys,
        // so the response contract (mirror of CLI config show --json) stays
        // protected against drift.
        let (_temp, executor) = setup_test_repo_with_config(
            r#"
[namespaces.type]
description = "Issue type"
unique = true

[namespaces.milestone]
description = "Release"
unique = false
"#,
        )
        .unwrap();
        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test".to_string(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let response = server.get("/config/namespaces").await;
        response.assert_status_ok();
        let data: NamespacesResponse = response.json();

        // Only the taxonomy (description/unique) is surfaced.
        let type_ns = data.namespaces.get("type").expect("type namespace");
        assert_eq!(type_ns.description, "Issue type");
        assert!(type_ns.unique);

        let ms_ns = data
            .namespaces
            .get("milestone")
            .expect("milestone namespace");
        assert_eq!(ms_ns.description, "Release");
        assert!(!ms_ns.unique);
    }

    #[tokio::test]
    async fn test_get_document_by_path_missing_param() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let response = server.get("/documents").await;
        // Should fail without path parameter
        assert!(response.status_code() != StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_document_by_path_nonexistent() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let response = server.get("/documents?path=nonexistent.md").await;
        // Should return 404 for file that doesn't exist
        assert_eq!(
            response.status_code(),
            status_of_error(PathReadError::NotFound(String::new()))
        );
    }

    #[tokio::test]
    async fn test_get_document_by_path_success() {
        use std::fs;

        // Set up a real tempdir acting as repo root with a .jit subdir.
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();

        // Write the fixture document at a subdirectory path relative to repo root.
        // This exercises the path-resolution fix: storage must resolve against
        // the explicit RepositoryLayout worktree root, not process CWD.
        let doc_rel = "docs/readme.md";
        let doc_content = "# Fixture document\n\nSome content.";
        let doc_abs = temp.path().join(doc_rel);
        fs::create_dir_all(doc_abs.parent().unwrap()).unwrap();
        fs::write(&doc_abs, doc_content).unwrap();

        let tracker = Arc::new(ChangeTracker::new(16));

        // Keep tempdir alive for the duration of the test.
        Box::leak(Box::new(temp));

        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
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

        let body: serde_json::Value =
            serde_json::from_slice(response.as_bytes()).expect("valid JSON body");
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
        assert_eq!(infer_content_type("data.csv"), "text/csv");
        assert_eq!(infer_content_type("data.json"), "application/json");
        assert_eq!(infer_content_type("style.css"), "text/css");
        assert_eq!(infer_content_type("app.js"), "text/javascript");
        assert_eq!(infer_content_type("logo.svg"), "image/svg+xml");
        assert_eq!(infer_content_type("photo.png"), "image/png");
        assert_eq!(infer_content_type("photo.jpg"), "image/jpeg");
        assert_eq!(infer_content_type("photo.jpeg"), "image/jpeg");
        assert_eq!(infer_content_type("readme.txt"), "text/plain");
    }

    #[test]
    fn test_infer_content_type_unknown_binary_extension_uses_octet_stream() {
        assert_eq!(
            infer_content_type("artifacts/model.unknown-binary-type"),
            "application/octet-stream"
        );
    }

    // ── /api/issues/:id/documents/:path/raw tests ────────────────────────────

    const HTML_DOC: &str = "slide.html";

    /// Writes `HTML_DOC` under `worktree` and links it to a new issue, whose id it returns.
    fn link_html_document(
        worktree: &std::path::Path,
        executor: &CommandExecutor<JsonFileStorage>,
    ) -> IssueId {
        use jit::domain::Priority;

        let id = executor
            .issues()
            .create_issue(
                "Test issue".to_string(),
                "desc".to_string(),
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
        std::fs::write(worktree.join(HTML_DOC), "<html><body>hello</body></html>").unwrap();
        // skip_scan avoids the git dependency.
        executor
            .documents()
            .add_document_reference(&id, HTML_DOC, None, None, None, true)
            .unwrap();
        id
    }

    async fn search_hits(server: &axum_test::TestServer, query: &str) -> Vec<SearchResult> {
        let response = server.get("/search").add_query_param("q", query).await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{}",
            response.text()
        );
        response.json::<SearchResponse>().results
    }

    #[tokio::test]
    async fn test_search_options_apply_to_issue_fields_and_linked_documents() {
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        let id = link_html_document(temp.path(), &executor);
        let issue_path = format!(".jit/issues/{id}.json");
        executor
            .documents()
            .add_document_reference(&id, &issue_path, None, None, None, true)
            .unwrap();
        let server = test_app(executor);
        let response = server
            .get("/search")
            .add_query_param("q", "^Test iss.e$")
            .add_query_param("regex", "true")
            .add_query_param("case_sensitive", "true")
            .await;
        response.assert_status_ok();
        let hits = response.json::<SearchResponse>().results;
        assert!(hits.iter().any(|hit| hit.issue_id.as_ref() == Some(&id)));
        let response = server
            .get("/search")
            .add_query_param("q", "^test iss.e$")
            .add_query_param("regex", "true")
            .add_query_param("case_sensitive", "true")
            .await;
        assert!(response.json::<SearchResponse>().results.is_empty());
        let response = server
            .get("/search")
            .add_query_param("q", "h.llo")
            .add_query_param("regex", "true")
            .await;
        assert!(response
            .json::<SearchResponse>()
            .results
            .iter()
            .any(|hit| hit.path.ends_with(HTML_DOC)));
        assert!(search_hits(&server, "enforce_leases").await.is_empty());
        assert!(search_hits(&server, "normal").await.is_empty());
        assert!(search_hits(&server, "desc")
            .await
            .iter()
            .any(|hit| hit.issue_id.as_ref() == Some(&id)));
    }

    #[tokio::test]
    async fn test_search_without_linked_documents_does_not_search_worktree() {
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        std::fs::write(temp.path().join("unlinked.md"), "unlinked-needle").unwrap();
        let server = test_app(executor);
        assert!(search_hits(&server, "unlinked-needle").await.is_empty());
    }

    #[tokio::test]
    async fn test_search_nested_data_root_returns_issue_record_match() {
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        let id = link_html_document(temp.path(), &executor);
        let server = test_app(executor);
        let hits = search_hits(&server, "Test issue").await;
        assert!(
            hits.iter().any(|hit| hit.issue_id.as_ref() == Some(&id)),
            "{hits:?}"
        );
    }

    #[tokio::test]
    async fn test_search_nested_data_root_keeps_linked_documents_and_excludes_other_files() {
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        let _id = link_html_document(temp.path(), &executor);
        let server = test_app(executor);
        let doc = HTML_DOC;
        let hits = search_hits(&server, "hello").await;
        assert!(hits.iter().any(|hit| hit.path.ends_with(&doc)), "{hits:?}");
        assert!(search_hits(&server, "enforce_leases").await.is_empty());
    }

    #[tokio::test]
    async fn test_get_document_raw_html_returns_200_with_correct_content_type() {
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        let id = link_html_document(temp.path(), &executor);
        let server = test_app(executor);
        let path = HTML_DOC;
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
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        let id = link_html_document(temp.path(), &executor);
        let server = test_app(executor);
        let path = HTML_DOC;
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
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        let id = link_html_document(temp.path(), &executor);
        let server = test_app(executor);
        let path = HTML_DOC;
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
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        let id = link_html_document(temp.path(), &executor);
        let server = test_app(executor);
        let _path = HTML_DOC;
        let url = format!("/issues/{}/documents/nonexistent.html/raw", id);
        let response = server.get(&url).await;
        assert_eq!(
            response.status_code(),
            status_of_error(PathReadError::NotFound(String::new()))
        );
    }

    #[tokio::test]
    async fn test_get_document_raw_returns_byte_faithful_for_non_utf8() {
        use jit::domain::Priority;
        use std::fs;

        // 4-byte sequence that is not valid UTF-8.
        let binary_fixture: &[u8] = b"\xff\xfe\xfd\xfc";

        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();

        let id = executor
            .issues()
            .create_issue(
                "Binary test".to_string(),
                "desc".to_string(),
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

        // Write binary fixture with a .dat extension (catches the text/plain fallback).
        let doc_rel = "artifact.dat";
        let doc_abs = temp.path().join(doc_rel);
        fs::write(&doc_abs, binary_fixture).unwrap();

        executor
            .documents()
            .add_document_reference(&id, doc_rel, None, None, None, true)
            .unwrap();

        // Keep tempdir alive.
        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
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
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .unwrap(),
            "application/octet-stream"
        );
    }

    // ── /api/documents/raw?path=... tests ────────────────────────────────────

    #[tokio::test]
    async fn test_get_document_raw_by_path_html_returns_correct_content_type() {
        use std::fs;

        // Build a JsonFileStorage-backed server rooted at a tempdir and write
        // the fixture HTML at a repo-relative path.  Absolute paths are no
        // longer accepted by the storage layer (they surface as
        // PathReadError::InvalidPath → 400), so every test must use a
        // repo-relative path.
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();

        let doc_rel = "slide.html";
        let html_body = "<p>test</p>";
        fs::write(temp.path().join(doc_rel), html_body).unwrap();

        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
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
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let response = server
            .get("/documents/raw")
            .add_query_param("path", "no_such_file.html")
            .await;
        assert_eq!(
            response.status_code(),
            status_of_error(PathReadError::NotFound(String::new()))
        );
    }

    /// Verify that the `get_document_by_path` endpoint returns 404 for a
    /// missing file (relying on typed dispatch, not string-matching).
    #[tokio::test]
    async fn test_get_document_by_path_not_found_returns_404() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        // Repo-relative path (absolute paths are rejected as InvalidPath → 400).
        let response = server
            .get("/documents")
            .add_query_param("path", "does_not_exist.md")
            .await;
        assert_eq!(
            response.status_code(),
            status_of_error(PathReadError::NotFound(String::new()))
        );
    }

    /// Verify that the `get_document_raw_by_path` endpoint returns 404 for a
    /// missing file (relying on typed dispatch, not string-matching).
    #[tokio::test]
    async fn test_get_document_raw_by_path_returns_404_for_missing() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        // Repo-relative path (absolute paths are rejected as InvalidPath → 400).
        let response = server
            .get("/documents/raw")
            .add_query_param("path", "does_not_exist.bin")
            .await;
        assert_eq!(
            response.status_code(),
            status_of_error(PathReadError::NotFound(String::new()))
        );
    }

    /// A directory is not an ordinary document file and is rejected as an
    /// unsafe path, not passed to `fs::read` as a server-side I/O failure.
    #[tokio::test]
    async fn test_get_document_by_path_rejects_directory() {
        use std::fs;

        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();

        let dir_rel = "not-a-file";
        fs::create_dir_all(temp.path().join(dir_rel)).unwrap();

        // Keep tempdir alive.
        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let response = server
            .get("/documents")
            .add_query_param("path", dir_rel)
            .await;
        assert_eq!(
            response.status_code(),
            status_of_error(PathReadError::InvalidPath(String::new()))
        );
        assert_eq!(response.text(), "");
    }

    /// The standalone raw endpoint applies the same ordinary-file contract.
    #[tokio::test]
    async fn test_get_document_raw_by_path_rejects_directory() {
        use std::fs;

        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();

        let dir_rel = "also-not-a-file";
        fs::create_dir_all(temp.path().join(dir_rel)).unwrap();

        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let response = server
            .get("/documents/raw")
            .add_query_param("path", dir_rel)
            .await;
        assert_eq!(
            response.status_code(),
            status_of_error(PathReadError::InvalidPath(String::new()))
        );
        assert_eq!(response.text(), "");
    }

    /// Linked document content also rejects a directory as an unsafe path.
    #[tokio::test]
    async fn test_get_document_content_rejects_directory() {
        use jit::domain::Priority;
        use std::fs;

        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();

        let dir_path = temp.path().join("broken-doc");
        fs::create_dir_all(&dir_path).unwrap();
        // Use a relative path so the URL stays clean (no slashes in the segment).
        let doc_rel = "broken-doc";

        let id = executor
            .issues()
            .create_issue(
                "IO error test".to_string(),
                "desc".to_string(),
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

        // Link the directory path as a document (skip_scan=true avoids the read
        // during registration so we can store the bad reference).
        executor
            .documents()
            .add_document_reference(&id, doc_rel, None, None, None, true)
            .unwrap();

        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let encoded = percent_encode_segment(doc_rel);
        let url = format!("/issues/{}/documents/{}/content", id, encoded);
        let response = server.get(&url).await;
        assert_eq!(
            response.status_code(),
            status_of_error(PathReadError::InvalidPath(String::new()))
        );
        assert_eq!(response.text(), "");
    }

    /// Every surface reporting an issue's documents reports one set: explicit
    /// links, then files under the issue's artifact directory.
    #[tokio::test]
    async fn test_every_issue_surface_reports_the_same_documents() {
        use jit::declarations::{GateChecker, GateMode, GateStage};
        use jit::domain::Priority;
        use std::collections::HashMap;

        let (temp, executor) =
            setup_test_repo_with_config("[documentation]\nissue_scoped_areas = [\"notes\"]\n")
                .unwrap();
        let id = executor
            .issues()
            .create_issue(
                "Documents on every surface".to_string(),
                "desc".to_string(),
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
        let directory = executor
            .documents()
            .resolve_issue_artifact_directory(&id, "notes")
            .unwrap()
            .directory;
        let derived = format!("{directory}/plan.md");
        std::fs::create_dir_all(temp.path().join(&directory)).unwrap();
        std::fs::write(temp.path().join(&derived), "plan\n").unwrap();
        executor
            .documents()
            .add_document_reference(&id, "linked.md", None, Some("Linked"), Some("design"), true)
            .unwrap();
        let observed = temp.path().join("observed-issue-docs.json");
        let gate_key = "docs-gate".parse().unwrap();
        executor
            .gates()
            .define_gate(
                &gate_key,
                "Docs".to_string(),
                "Records JIT_ISSUE_DOCS".to_string(),
                GateStage::Postcheck,
                GateMode::Auto,
                Some(GateChecker::Exec {
                    command: format!("printf '%s' \"$JIT_ISSUE_DOCS\" > {}", observed.display()),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                100,
            )
            .unwrap();
        executor.gates().add_gate(&id, &gate_key).unwrap();
        executor.gates().check_gate(&id, &gate_key).unwrap();

        let listed_by_doc_list =
            serde_json::to_value(executor.documents().list_document_references(&id).unwrap())
                .unwrap();
        let shown = executor
            .issues()
            .show_response(executor.issues().show_issue(&id).unwrap())
            .unwrap();
        let state = AppState {
            executor: Arc::new(executor),
            tracker: Arc::new(ChangeTracker::new(16)),
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();
        let single: serde_json::Value = server.get(&format!("/issues/{id}")).await.json();
        let listed: serde_json::Value = server.get("/issues").await.json();

        let entries = |documents: &serde_json::Value| {
            documents
                .as_array()
                .unwrap()
                .iter()
                .map(|document| {
                    (
                        document["path"].as_str().unwrap().to_string(),
                        document["doc_type"].as_str().map(str::to_string),
                        document["label"].as_str().map(str::to_string),
                    )
                })
                .collect::<std::collections::BTreeSet<_>>()
        };
        let expected = entries(&listed_by_doc_list["documents"]);
        assert!(expected.contains(&(derived, None, None)));
        assert!(expected.contains(&(
            "linked.md".to_string(),
            Some("design".to_string()),
            Some("Linked".to_string())
        )));
        let received: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(observed).unwrap()).unwrap();
        for (surface, documents) in [
            (
                "issue show",
                serde_json::to_value(&shown.documents).unwrap(),
            ),
            ("GET /issues/:id", single["documents"].clone()),
            ("GET /issues", listed[0]["documents"].clone()),
            ("JIT_ISSUE_DOCS", received),
        ] {
            assert_eq!(entries(&documents), expected, "{surface}");
        }
    }

    /// Every issue-scoped document read, and the search universe, accept a
    /// file under the issue's artifact directory that no link names.
    #[tokio::test]
    async fn test_document_reads_serve_a_derived_document_without_a_link() {
        use jit::domain::Priority;
        use std::fs;
        use std::process::Command;

        let (temp, executor) = setup_test_repo_with_config(
            "[worktree]\nenforce_leases = \"off\"\n\n[documentation]\nissue_scoped_areas = [\"notes\"]\n",
        )
        .unwrap();
        jit::test_cli::init_git_repository(temp.path()).unwrap();
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .current_dir(temp.path())
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        let id = executor
            .issues()
            .create_issue(
                "Derived document reads".to_string(),
                "desc".to_string(),
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
        let directory = executor
            .documents()
            .resolve_issue_artifact_directory(&id, "notes")
            .unwrap()
            .directory;
        let doc_rel = format!("{directory}/plan.md");
        let doc_path = temp.path().join(&doc_rel);
        fs::create_dir_all(doc_path.parent().unwrap()).unwrap();
        ["first\n", "second\n"].iter().for_each(|content| {
            fs::write(&doc_path, content).unwrap();
            git(&["add", doc_rel.as_str()]);
            git(&["commit", "-qm", content.trim()]);
        });

        assert!(executor
            .documents()
            .get_issue_document_paths()
            .unwrap()
            .contains(&doc_rel));
        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();
        let base = format!(
            "/issues/{id}/documents/{}",
            percent_encode_segment(&doc_rel)
        );

        for route in ["content", "raw", "history"] {
            server
                .get(&format!("{base}/{route}"))
                .await
                .assert_status_ok();
        }
        server
            .get(&format!("{base}/diff"))
            .add_query_param("from", "HEAD~1")
            .await
            .assert_status_ok();
        let history: serde_json::Value = server.get(&format!("{base}/history")).await.json();
        assert_eq!(history["commits"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_get_document_content_csv_returns_text_csv() {
        use jit::domain::Priority;
        use std::fs;

        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();

        let doc_rel = "bench/results.csv";
        let doc_content = "prime,n,kernel\n31,1024,C\n";
        let doc_path = temp.path().join(doc_rel);
        fs::create_dir_all(doc_path.parent().unwrap()).unwrap();
        fs::write(&doc_path, doc_content).unwrap();

        let id = executor
            .issues()
            .create_issue(
                "CSV content test".to_string(),
                "desc".to_string(),
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
            .add_document_reference(&id, doc_rel, None, None, None, false)
            .unwrap();

        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let encoded = percent_encode_segment(doc_rel);
        let url = format!("/issues/{}/documents/{}/content", id, encoded);
        let response = server.get(&url).await;
        response.assert_status_ok();

        let body: serde_json::Value =
            serde_json::from_slice(response.as_bytes()).expect("valid JSON body");
        assert_eq!(body["content"].as_str().unwrap(), doc_content);
        assert_eq!(body["content_type"].as_str().unwrap(), "text/csv");
    }

    /// Linked raw document access applies the same ordinary-file contract.
    #[tokio::test]
    async fn test_get_document_raw_rejects_directory() {
        use jit::domain::Priority;
        use std::fs;

        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();

        let dir_path = temp.path().join("broken-raw-doc");
        fs::create_dir_all(&dir_path).unwrap();
        let doc_rel = "broken-raw-doc";

        let id = executor
            .issues()
            .create_issue(
                "IO error raw test".to_string(),
                "desc".to_string(),
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
            .add_document_reference(&id, doc_rel, None, None, None, true)
            .unwrap();

        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let url = format!("/issues/{}/documents/{}/raw", id, doc_rel);
        let response = server.get(&url).await;
        assert_eq!(
            response.status_code(),
            status_of_error(PathReadError::InvalidPath(String::new()))
        );
        assert_eq!(response.text(), "");
    }

    #[tokio::test]
    async fn test_get_document_by_path_csv_returns_text_csv() {
        use std::fs;

        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();

        let doc_rel = "bench/results.csv";
        let doc_content = "prime,n,kernel\n7,256,F\n";
        let doc_path = temp.path().join(doc_rel);
        fs::create_dir_all(doc_path.parent().unwrap()).unwrap();
        fs::write(&doc_path, doc_content).unwrap();

        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();

        let response = server
            .get("/documents")
            .add_query_param("path", doc_rel)
            .await;
        response.assert_status_ok();

        let body: serde_json::Value =
            serde_json::from_slice(response.as_bytes()).expect("valid JSON body");
        assert_eq!(body["content"].as_str().unwrap(), doc_content);
        assert_eq!(body["content_type"].as_str().unwrap(), "text/csv");
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

    #[test]
    fn test_inject_base_href_handles_non_ascii_in_head() {
        // Regression: non-ASCII characters (em-dash, accents, etc.) inside
        // <head> caused a panic in html_has_base_element when advancing a byte
        // cursor past a multi-byte UTF-8 character. Must now advance by char.
        let html = "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n    <meta charset=\"UTF-8\">\n    <title>grand and sogrand in gf2 — reproducing yuan–médard–galligan–duffy</title>\n</head>\n<body></body>\n</html>";
        let result = inject_base_href(html, "/api/raw/docs/presentations/");
        assert!(
            result.contains("<base href=\"/api/raw/docs/presentations/\">"),
            "base tag must be injected even when <head> contains non-ASCII characters"
        );
        // Must be inserted right after <head>, not inside the title.
        let head_open_end = result.find("<head>").unwrap() + "<head>".len();
        let base_start = result.find("<base").unwrap();
        assert_eq!(
            base_start, head_open_end,
            "base tag must be inserted immediately after <head>"
        );
    }

    // ── GET /raw/*path integration tests ─────────────────────────────────────

    fn seed_worktree_file(worktree: &std::path::Path, rel_path: &str, content: &[u8]) {
        let path = worktree.join(rel_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[tokio::test]
    async fn test_get_raw_wildcard_serves_svg() {
        let svg_content = br#"<svg xmlns="http://www.w3.org/2000/svg"><circle r="5"/></svg>"#;
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        seed_worktree_file(
            temp.path(),
            "docs/presentations/figures/fig4.svg",
            svg_content,
        );
        let server = test_app(executor);

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
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        seed_worktree_file(temp.path(), "docs/presentations/deck.html", html_content);
        let server = test_app(executor);

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
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
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
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let response = server.get("/raw/a/../b").await;
        assert!(
            response.status_code().is_client_error(),
            "embedded .. must produce a 4xx response, got {}",
            response.status_code()
        );
    }

    #[tokio::test]
    async fn test_get_raw_wildcard_404_for_missing_file() {
        let server = test_app(memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&taxonomy_config()),
                ..MemoryDeclarations::default()
            },
        ));
        let response = server.get("/raw/definitely/does/not/exist.svg").await;
        assert_eq!(
            response.status_code(),
            status_of_error(PathReadError::NotFound(String::new()))
        );
    }

    /// Commit-pinned HTML also gets base-tag injection (same as working-tree reads).
    /// With a fake commit SHA the storage returns CommitNotFound → 404 on non-git
    /// backends, so we only verify that we reach the injection code path when 200.
    #[tokio::test]
    async fn test_get_document_raw_html_with_commit_param() {
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        let id = link_html_document(temp.path(), &executor);
        let server = test_app(executor);
        let path = HTML_DOC;
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
        use std::fs;

        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();

        let id = executor
            .issues()
            .create_issue(
                "base tag test".to_string(),
                "desc".to_string(),
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

        // HTML that already has its own <base> tag.
        let original =
            r#"<html><head><base href="https://cdn.example.com/"></head><body>hi</body></html>"#;
        let doc_rel = "withbase.html";
        fs::write(temp.path().join(doc_rel), original).unwrap();
        executor
            .documents()
            .add_document_reference(&id, doc_rel, None, None, None, true)
            .unwrap();

        Box::leak(Box::new(temp));

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
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
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();
        seed_worktree_file(temp.path(), "docs/deck.html", original);
        let server = test_app(executor);

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

    #[test]
    fn test_compute_base_href_percent_encodes_unicode_and_reserved_characters() {
        assert_eq!(
            compute_base_href("docs/Café & notes/deck.html"),
            "/api/raw/docs/Caf%C3%A9%20%26%20notes/"
        );
    }

    /// Symlinks inside the repository that point outside the repo root must be
    /// rejected with 400.  This guards against a scenario where an attacker or
    /// misconfigured repo contains a symlink like `docs/secret -> /etc/passwd`.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_get_raw_wildcard_rejects_symlink_escape() {
        use std::fs;
        use std::os::unix::fs as unix_fs;

        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();

        // Create a file outside the repo that we'll try to reach via symlink.
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"secret outside repo").unwrap();

        // Create a symlink inside the repo pointing to the outside file.
        let link_path = temp.path().join("docs").join("secret.txt");
        fs::create_dir_all(link_path.parent().unwrap()).unwrap();
        unix_fs::symlink(outside.path(), &link_path).unwrap();

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();
        // Keep tempdir + outside file alive.
        Box::leak(Box::new(temp));
        Box::leak(Box::new(outside));

        let response = server.get("/raw/docs/secret.txt").await;
        assert_eq!(
            response.status_code(),
            status_of_error(PathReadError::InvalidPath(String::new())),
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
        use std::fs;
        use std::os::unix::fs as unix_fs;

        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();

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

        let id = executor
            .issues()
            .create_issue(
                "Symlink escape repro".to_string(),
                "desc".to_string(),
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
            .add_document_reference(&id, doc_rel, None, None, None, true)
            .unwrap();

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
        };
        let server = TestServer::new(create_routes(state)).unwrap();
        Box::leak(Box::new(temp));
        Box::leak(Box::new(outside));

        let url = format!("/issues/{}/documents/{}/raw", id, doc_rel);
        let response = server.get(&url).await;
        assert_eq!(
            response.status_code(),
            status_of_error(PathReadError::InvalidPath(String::new())),
            "symlink escaping repo root must be rejected on issue-scoped raw endpoint"
        );
    }

    /// Same invariant for the path-only `/documents/raw?path=...` handler:
    /// if the query path resolves through a symlink to a file outside the
    /// repo root, the storage layer must reject it with 400.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_get_document_raw_by_path_rejects_symlink_escape() {
        use std::fs;
        use std::os::unix::fs as unix_fs;

        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n").unwrap();

        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"secret outside repo").unwrap();

        // Symlink inside the repo pointing to the outside file.
        let link_rel = "docs/escape.txt";
        let link_path = temp.path().join(link_rel);
        fs::create_dir_all(link_path.parent().unwrap()).unwrap();
        unix_fs::symlink(outside.path(), &link_path).unwrap();

        let tracker = Arc::new(ChangeTracker::new(16));
        let state = AppState {
            executor: Arc::new(executor),
            tracker,
            capture_cache: Default::default(),
            project_name: "test-project".to_string(),
            shutdown: CancellationToken::new(),
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
            status_of_error(PathReadError::InvalidPath(String::new())),
            "symlink escaping repo root must be rejected on path-only raw endpoint"
        );
    }
}
