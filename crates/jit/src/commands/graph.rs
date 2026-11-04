//! Graph visualization and traversal

use super::*;
use crate::domain::identity::IssueId;
use crate::domain::list_envelope::{collection_keys, ListEnvelope};
use crate::domain::MinimalIssue;
use crate::graph::Direction;
use crate::visualization::{pretty_document, GraphExportFullResponse, GraphExportSummaryResponse};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

collection_keys! {
    /// Collection key of [`GraphDepsTreeResponse`].
    GraphDepsNodesKey = "nodes" as "GraphDepsTreeResponse";
    /// Collection key of [`GraphRootsResponse`].
    RootsKey = "roots" as "GraphRootsResponse";
    /// Collection key of [`GraphTreeResponse`].
    GraphTreeNodesKey = "nodes" as "GraphTreeResponse";
    /// Collection key of [`DivergenceResponse`].
    MembershipDivergencesKey = "divergences" as "DivergenceResponse";
}

/// Tree node for hierarchical dependency display
#[derive(Debug, Serialize, JsonSchema, Clone)]
pub struct DependencyTreeNode {
    /// Issue ID
    pub id: IssueId,
    /// Short ID (first 8 chars)
    pub short_id: String,
    /// Issue title
    pub title: String,
    /// Current state
    pub state: State,
    /// Priority
    pub priority: Priority,
    /// Depth level in tree (1 = immediate child, 2 = grandchild, etc.)
    pub level: u32,
    /// Whether this node appears multiple times in the tree (shared dependency)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared: Option<bool>,
    /// Pre-archive origin state, present exactly for an `Archived` node.
    /// Carried so dependency consumers can compute effective
    /// terminality (`jit:45a140ae`): an `archived` node with a terminal origin
    /// still satisfies its dependents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_from: Option<State>,
    /// Child dependencies
    pub children: Vec<DependencyTreeNode>,
}

impl DependencyTreeNode {
    /// Create from MinimalIssue
    pub fn from_minimal(issue: &MinimalIssue, level: u32) -> Self {
        Self {
            short_id: issue.short_id(),
            id: issue.id.clone(),
            title: issue.title.clone(),
            state: issue.state,
            priority: issue.priority,
            level,
            shared: None,
            archived_from: issue.archived_from,
            children: Vec::new(),
        }
    }
}

/// Response for `graph deps` with tree structure.
///
/// `count` is the number of top-level entries in `nodes` (the immediate
/// dependency nodes; each may carry nested `children`). This differs from
/// `summary.total`, which counts every unique dependency across the whole tree.
pub type GraphDepsTreeResponse =
    ListEnvelope<GraphDepsNodesKey, DependencyTreeNode, DependencyTreeScope>;

/// Fields of [`GraphDepsTreeResponse`] beside its `nodes`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DependencyTreeScope {
    pub issue_id: IssueId,
    /// Depth of traversal (1 = immediate, 0 = unlimited)
    pub depth: u32,
    /// Summary statistics
    pub summary: DependencySummary,
}

/// Summary statistics for dependencies
#[derive(Debug, Serialize, JsonSchema)]
pub struct DependencySummary {
    /// Total number of unique dependencies
    pub total: usize,
    /// Count by state (keys are canonical snake_case state names)
    pub by_state: std::collections::HashMap<State, usize>,
}

/// Response for `graph roots` command
pub type GraphRootsResponse = ListEnvelope<RootsKey, MinimalIssue>;

/// One node's resolved hierarchy facts, as rendered by `graph tree`.
///
/// The `type` key is the value of the node's `type:` label (absent when it has
/// none). The `parent`, `children`, `cluster`, and `rank` keys are flattened
/// from [`NodeHierarchy`](crate::graph::hierarchy::NodeHierarchy), the
/// DAG-authoritative resolution produced by
/// [`resolve_hierarchy`](crate::graph::hierarchy::resolve_hierarchy).
///
/// # Examples
///
/// ```
/// use jit::graph::hierarchy::NodeHierarchy;
/// use jit::commands::HierarchyNodeView;
///
/// let epic_id = jit::test_utils::issue_id(0x4000_8000_0000_0000_0001_1234_5678);
/// let task_id = jit::test_utils::issue_id(0x4000_8000_0000_0000_0002_8765_4321);
/// let view = HierarchyNodeView {
///     id: epic_id.clone(),
///     short_id: epic_id.short_id().to_string(),
///     title: "Auth epic".into(),
///     type_name: Some("epic".into()),
///     hierarchy: NodeHierarchy {
///         parent: None,
///         children: vec![task_id.clone()],
///         cluster: Some(epic_id.clone()),
///         rank: 1,
///     },
/// };
/// let json = serde_json::to_value(&view).unwrap();
/// // `type_name` serializes under the `type` key.
/// assert_eq!(json["type"], "epic");
/// // The resolution fields sit flat on the node.
/// assert_eq!(json["children"][0], task_id.as_str());
/// assert_eq!(json["cluster"], epic_id.as_str());
/// assert_eq!(json["rank"], 1);
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct HierarchyNodeView {
    /// Full issue id.
    pub id: IssueId,
    /// Short id (first 8 chars).
    pub short_id: String,
    /// Issue title.
    pub title: String,
    /// The node's `type:` label value, if any.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    /// DAG-resolved parent, children, cluster, and rank, flattened onto the node.
    #[serde(flatten)]
    pub hierarchy: crate::graph::hierarchy::NodeHierarchy,
}

/// Response for `graph tree` command.
///
/// `count` is the number of entries in `nodes`. `root` echoes the optional root
/// id the view was scoped to (`null` for the whole repository). Each node
/// carries its DAG-resolved parent, children, cluster, and rank.
///
/// # Examples
///
/// ```
/// use jit::graph::hierarchy::NodeHierarchy;
/// use jit::commands::{GraphTreeResponse, HierarchyNodeView, TreeRoot};
///
/// let epic_id = jit::test_utils::issue_id(0x4000_8000_0000_0000_0001_1234_5678);
/// let task_id = jit::test_utils::issue_id(0x4000_8000_0000_0000_0002_8765_4321);
/// let response = GraphTreeResponse::with_extras(
///     vec![HierarchyNodeView {
///         id: task_id.clone(),
///         short_id: task_id.short_id().to_string(),
///         title: "A task".into(),
///         type_name: Some("task".into()),
///         hierarchy: NodeHierarchy {
///             parent: Some(epic_id.clone()),
///             children: vec![],
///             cluster: Some(epic_id.clone()),
///             rank: 0,
///         },
///     }],
///     TreeRoot { root: None },
/// );
/// let json = serde_json::to_value(&response).unwrap();
/// assert_eq!(json["count"], 1);
/// assert_eq!(json["root"], serde_json::Value::Null);
/// assert_eq!(json["nodes"][0]["parent"], epic_id.as_str());
/// ```
pub type GraphTreeResponse = ListEnvelope<GraphTreeNodesKey, HierarchyNodeView, TreeRoot>;

/// The scope of a [`GraphTreeResponse`]. `nodes` are ordered by ascending short id.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TreeRoot {
    /// The root id the tree was scoped to, or `null` for the whole repository.
    pub root: Option<IssueId>,
}

/// One reported membership-vs-DAG divergence, as rendered by `query divergence`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DivergenceView {
    /// Full id of the issue carrying the unsupported membership label.
    pub id: IssueId,
    /// Short id (first 8 chars).
    pub short_id: String,
    /// Issue title.
    pub title: String,
    /// The full `namespace:value` membership label.
    pub label: String,
    /// The label namespace.
    pub namespace: String,
    /// The label value.
    pub value: String,
}

/// Response for `query divergence` command.
///
/// Each entry in `divergences` is a membership label whose claim the dependency
/// DAG does not back (the issue carries the label but is not in the closure of
/// the container that owns it), ordered by `(issue_id, label)`.
pub type DivergenceResponse = ListEnvelope<MembershipDivergencesKey, DivergenceView>;

impl DivergenceResponse {
    pub(crate) fn from_issues(
        issues: &[Issue],
        config: &crate::domain::type_taxonomy::HierarchyConfig,
    ) -> Self {
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let divergences =
            crate::graph::hierarchy::detect_membership_divergences(&issue_refs, config);

        let by_id: HashMap<&IssueId, &Issue> = issues.iter().map(|i| (&i.id, i)).collect();
        let views = divergences
            .into_iter()
            .map(|d| {
                let issue = by_id.get(&d.issue_id);
                DivergenceView {
                    short_id: d.issue_id.short_id().to_string(),
                    title: issue.map(|i| i.title.clone()).unwrap_or_default(),
                    id: d.issue_id,
                    label: d.label,
                    namespace: d.namespace,
                    value: d.value,
                }
            })
            .collect::<Vec<_>>();

        Self::new(views)
    }
}

/// The success response of `jit graph export`: one of the JSON documents, the
/// written-file report under `--output`, or the text of a DOT, Mermaid or batch
/// export. Text is only ever rendered in human mode, since `--json` selects a
/// JSON format.
#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub enum GraphExportOutput {
    Summary(GraphExportSummaryResponse),
    Full(GraphExportFullResponse),
    Written(GraphExportWritten),
    Text(GraphExportText),
}

/// The document text of a DOT, Mermaid or batch export.
#[derive(Serialize, JsonSchema)]
#[serde(transparent)]
pub struct GraphExportText(pub String);

/// The `--output` report: the path the export was written to.
#[derive(Serialize, JsonSchema)]
pub struct GraphExportWritten {
    pub path: String,
}

impl GraphExportOutput {
    /// The exact bytes an `--output` file receives: the document text without
    /// its trailing newline. A written report has no document.
    pub fn document_text(&self) -> Option<String> {
        match self {
            Self::Summary(document) => Some(pretty_document(document)),
            Self::Full(document) => Some(pretty_document(document)),
            Self::Text(text) => Some(text.0.clone()),
            Self::Written(_) => None,
        }
    }
}

/// Serialization format for `jit graph export`.
///
/// Deriving [`clap::ValueEnum`] lets clap reject an unknown format at parse time
/// (with the accepted values listed). The value names are the lowercase variant
/// names: `dot`, `mermaid`, `json`, `batch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum GraphExportFormat {
    /// Graphviz DOT format.
    Dot,
    /// Mermaid diagram format.
    Mermaid,
    /// JSON node/edge format.
    Json,
    /// `batch-create` input schema: a JSON array of issue definitions, the
    /// structural inverse of `jit issue batch-create`. Handled on a separate
    /// path ([`export_graph_batch`](GraphOps::export_graph_batch)); the
    /// graph-rendering [`export_graph`](GraphOps::export_graph) never
    /// receives it.
    Batch,
}

/// A dependency edge dropped from batch output because it crosses the export's
/// membership scope: `from` (a batch node) depends on `to` (a real issue outside
/// the scope). Both are short ids. Surfaced so no edge is dropped silently.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BoundaryEdge {
    /// Short id of the in-scope batch node the edge originates from.
    pub from: String,
    /// Short id of the out-of-scope dependency the edge points to.
    pub to: String,
}

/// The result of a batch-shape export: the batch-create definitions plus the
/// scope-boundary edges that were excluded from them.
///
/// [`defs`](Self::defs) serializes to exactly the JSON array `jit issue
/// batch-create --from-json` consumes.
#[derive(Debug, Clone)]
pub struct BatchExport {
    /// The exported issue definitions, ordered by short id.
    pub defs: Vec<BatchIssueDef>,
    /// Edges excluded because they cross the membership scope boundary
    /// (REQ-06), ordered by `(from, to)`.
    pub boundary_edges: Vec<BoundaryEdge>,
}

/// Whether a label survives batch export: kept unless it is the `type:*` label
/// (lifted into the def's `type` field) or its namespace is identity-bound.
fn keep_generic_label(label: &str, stripped_ns: &HashSet<String>) -> bool {
    let namespace = label.split(':').next().unwrap_or(label);
    namespace != crate::labels::TYPE_NAMESPACE && !stripped_ns.contains(namespace)
}

/// Graph and dependency operations of a [`CommandExecutor`].
pub struct GraphOps<'a, S: IssueStore> {
    pub(super) exec: &'a CommandExecutor<S>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// The graph family.
    pub fn graph(&self) -> GraphOps<'_, S> {
        GraphOps { exec: self }
    }
}

impl<S: IssueStore> GraphOps<'_, S> {
    /// Publish rendered graph bytes to an explicitly selected filesystem path.
    ///
    /// Repository-contained destinations use one captured recoverable delta;
    /// destinations outside both selected roots retain the atomic external-file
    /// behavior. Stdout export does not call this method and remains read-only.
    pub fn publish_graph_export(
        &self,
        invocation_dir: &std::path::Path,
        requested: &std::path::Path,
        bytes: &[u8],
    ) -> Result<()>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{
            classify_repository_export, CaptureBudget, RepositoryExportDestination,
            RepositoryExportIntent,
        };

        let layout = self.exec.require_layout()?;
        match classify_repository_export(&layout, invocation_dir, requested)? {
            RepositoryExportDestination::External(path) => {
                crate::storage::external_publish::write_external_export_atomic(&path, bytes)
            }
            RepositoryExportDestination::Repository(target) => {
                let intent = RepositoryExportIntent::new(target, bytes.to_vec());
                let budget = CaptureBudget {
                    max_listings: 1,
                    max_bytes: super::CAPTURE_MAX_BYTES,
                    max_depth: 128,
                };
                self.exec
                    .publish_repository_export(&layout, &intent, budget)
            }
        }
    }

    /// Build a dependency tree with specified depth
    ///
    /// Returns a tree structure that preserves parent-child relationships
    /// and marks shared dependencies (diamonds in the DAG).
    ///
    /// Every issue is loaded once up front; the unfolding itself
    /// ([`expand`](DependencyGraph::expand)) runs in memory over the graph.
    pub fn build_dependency_tree(
        &self,
        issue_id: &IssueId,
        depth: u32,
    ) -> Result<Vec<DependencyTreeNode>> {
        use crate::graph::Expansion;

        let full_id = issue_id.clone();
        let issues = self.exec.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        let forest = graph.expand(&full_id, Direction::Dependencies, depth);
        // An issue unfolded along several paths is a diamond in the DAG.
        let occurrences = Expansion::occurrences(&forest);
        let to_tree_node = |issue: &Issue, level: u32, children: Vec<DependencyTreeNode>| {
            let shared = occurrences.get(&issue.id).is_some_and(|&n| n > 1);
            DependencyTreeNode {
                shared: shared.then_some(true),
                children,
                ..DependencyTreeNode::from_minimal(&MinimalIssue::from(issue), level)
            }
        };

        Ok(forest
            .iter()
            .map(|occurrence| occurrence.fold(&to_tree_node))
            .collect())
    }

    /// Show what an issue depends on with depth control.
    ///
    /// # Arguments
    ///
    /// * `issue_id` - Issue ID to show dependencies for
    /// * `depth` - Maximum depth to traverse (1 = immediate, 0 = unlimited)
    pub fn show_dependencies_with_depth(
        &self,
        issue_id: &IssueId,
        depth: u32,
    ) -> Result<Vec<Issue>> {
        self.traverse_dependency_edges(issue_id, Direction::Dependencies, depth)
    }

    /// Show what depends on an issue with depth control.
    ///
    /// # Arguments
    ///
    /// * `issue_id` - Issue ID to show dependents for
    /// * `depth` - Maximum depth to traverse (1 = immediate, 0 = unlimited)
    pub fn show_rdeps_with_depth(&self, issue_id: &IssueId, depth: u32) -> Result<Vec<Issue>> {
        self.traverse_dependency_edges(issue_id, Direction::Dependents, depth)
    }

    /// Walk the repository's dependency graph from `issue_id`, following edges in
    /// `direction` up to `depth` (0 = unlimited), and return the issues reached.
    fn traverse_dependency_edges(
        &self,
        issue_id: &IssueId,
        direction: Direction,
        depth: u32,
    ) -> Result<Vec<Issue>> {
        let full_id = issue_id.clone();
        let issues = self.exec.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        Ok(graph
            .traverse(&full_id, direction, depth)
            .into_iter()
            .cloned()
            .collect())
    }

    pub fn show_roots(&self) -> Result<Vec<Issue>> {
        let issues = self.exec.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        let roots = graph.get_roots();
        Ok(roots.into_iter().cloned().collect())
    }

    /// Render the dependency graph in `format`, optionally scoped to a container.
    ///
    /// `full` selects the complete-record JSON node shape
    /// ([`export_json_full`](crate::visualization::export_json_full)) instead of
    /// the default summary shape; it applies ONLY to
    /// [`GraphExportFormat::Json`]. The caller (CLI) rejects `full` with a
    /// non-JSON format as a usage error before reaching here, so the `dot`/
    /// `mermaid` arms ignore it.
    ///
    /// `scope = Some(container)` restricts the listed nodes to the container's
    /// DAG-authoritative containment membership
    /// ([`membership_closure`](crate::graph::hierarchy::HierarchyResolution::membership_closure));
    /// `scope = None` lists the whole repository. Hierarchy resolution stays
    /// repository-wide either way, so a scoped node's `parent`/`cluster` may
    /// point at a container outside the subtree (matching `graph tree`).
    ///
    /// [`GraphExportFormat::Batch`] is handled by
    /// [`export_graph_batch`](Self::export_graph_batch) and is not a valid
    /// argument here.
    pub fn export_graph(
        &self,
        format: GraphExportFormat,
        full: bool,
        scope: Option<&IssueId>,
    ) -> Result<GraphExportOutput> {
        let issues = self.exec.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();

        // Resolve the hierarchy once when it is needed — for scoping and/or the
        // full JSON node shape. Resolution is always repository-wide.
        let needs_full_json = matches!((format, full), (GraphExportFormat::Json, true));
        let resolution = if scope.is_some() || needs_full_json {
            let config = crate::config_manager::get_hierarchy_config(&self.exec.storage)?;
            Some(crate::graph::hierarchy::resolve_hierarchy(
                &issue_refs,
                &config,
            ))
        } else {
            None
        };

        let member_ids = match &scope {
            Some(container) => {
                let container_id = *container;
                Some(
                    resolution
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("Missing hierarchy resolution for scope"))?
                        .membership_closure(container_id)
                        .into_iter()
                        .map(|node| &node.id)
                        .collect::<HashSet<_>>(),
                )
            }
            None => None,
        };
        let selected: Vec<&Issue> = issue_refs
            .iter()
            .copied()
            .filter(|i| member_ids.as_ref().is_none_or(|m| m.contains(&i.id)))
            .collect();
        let graph = DependencyGraph::new(&selected);

        Ok(match format {
            GraphExportFormat::Dot => {
                GraphExportOutput::Text(GraphExportText(crate::visualization::export_dot(&graph)))
            }
            GraphExportFormat::Mermaid => GraphExportOutput::Text(GraphExportText(
                crate::visualization::export_mermaid(&graph),
            )),
            GraphExportFormat::Json if full => {
                GraphExportOutput::Full(crate::visualization::full_document(
                    &graph,
                    resolution.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("Missing hierarchy resolution for full JSON")
                    })?,
                ))
            }
            GraphExportFormat::Json => {
                GraphExportOutput::Summary(crate::visualization::summary_document(&graph))
            }
            GraphExportFormat::Batch => {
                return Err(anyhow::anyhow!(
                    "batch format is handled by export_graph_batch, not export_graph"
                ));
            }
        })
    }

    /// Export the graph in the `batch-create` input schema — the structural
    /// inverse of [`batch_create_from_json`](crate::commands::IssueOps::batch_create_from_json).
    ///
    /// `scope = Some(container)` captures that container's containment subtree;
    /// `scope = None` captures the whole graph. Each in-scope, non-bracket node
    /// becomes a [`BatchIssueDef`] keyed by its short id, carrying only
    /// structural fields (title, description, type, priority, surviving generic
    /// labels, gates, and in-scope `depends_on` keys) — no lifecycle fields, and
    /// every in-scope node regardless of state.
    ///
    /// Three projection policies, all config-derived (never hardcoded,
    /// `@/inv/domain-agnostic`):
    ///
    /// - **Identity-bound labels stripped**: `type:*` is lifted into the `type`
    ///   field; membership namespaces (`[type_hierarchy.label_associations]`)
    ///   and the coverage rule's `satisfies-namespace`/`container-from-label`
    ///   namespaces are dropped. Generic labels survive.
    /// - **Bracket nodes excluded**: nodes whose type is a template planning- or
    ///   breakdown-role node type are dropped together with every edge touching
    ///   them, keeping the seed template-compatible.
    /// - **Boundary edges reported**: a dependency on a real issue outside the
    ///   membership scope is excluded and recorded in
    ///   [`BatchExport::boundary_edges`] rather than dropped silently.
    ///
    /// The returned [`BatchExport::defs`] serializes to exactly the JSON array
    /// `batch-create` consumes.
    pub fn export_graph_batch(&self, scope: Option<&IssueId>) -> Result<BatchExport> {
        let issues = self.exec.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();

        // Membership scope (the whole graph when unscoped).
        let member_ids: Option<HashSet<&IssueId>> = match &scope {
            Some(container) => {
                let container_id = *container;
                let config = crate::config_manager::get_hierarchy_config(&self.exec.storage)?;
                let resolution = crate::graph::hierarchy::resolve_hierarchy(&issue_refs, &config);
                Some(
                    resolution
                        .membership_closure(container_id)
                        .into_iter()
                        .map(|node| &node.id)
                        .collect(),
                )
            }
            None => None,
        };
        let in_scope = |id: &IssueId| member_ids.as_ref().is_none_or(|m| m.contains(id));

        // Config-derived projection inputs.
        let stripped_ns = self.identity_bound_namespaces()?;
        let bracket_types = self.bracket_node_types()?;
        let is_bracket = |issue: &Issue| {
            crate::labels::type_label_value(&issue.labels)
                .is_some_and(|t| bracket_types.contains(t))
        };
        let by_id: HashMap<&IssueId, &Issue> = issues.iter().map(|i| (&i.id, i)).collect();
        let short_of = |id: &IssueId| -> String { id.short_id().to_string() };

        // Batch node set: in-scope, non-bracket nodes, ordered by short id.
        let mut nodes: Vec<&Issue> = issue_refs
            .iter()
            .copied()
            .filter(|i| in_scope(&i.id) && !is_bracket(i))
            .collect();
        nodes.sort_by_key(|i| i.short_id());
        let node_ids: HashSet<&IssueId> = nodes.iter().map(|i| &i.id).collect();

        let mut boundary_edges: Vec<BoundaryEdge> = Vec::new();
        let defs: Vec<BatchIssueDef> = nodes
            .iter()
            .map(|issue| {
                let mut depends_on: Vec<String> = Vec::new();
                for dep in &issue.dependencies {
                    if node_ids.contains(dep) {
                        // Internal edge: reference the dependency by its key.
                        depends_on.push(short_of(dep));
                    } else if by_id.get(dep).is_some_and(|d| is_bracket(d)) {
                        // Edge to a bracket node: dropped with the bracket (REQ-05).
                    } else if by_id.contains_key(dep) {
                        // A real issue outside the membership scope: a boundary
                        // edge — excluded but reported, never dropped silently.
                        boundary_edges.push(BoundaryEdge {
                            from: issue.short_id(),
                            to: short_of(dep),
                        });
                    }
                    // else: dangling id — the integrity check owns broken edges.
                }
                depends_on.sort();

                let labels: Vec<String> = issue
                    .labels
                    .iter()
                    .filter(|label| keep_generic_label(label, &stripped_ns))
                    .cloned()
                    .collect();

                BatchIssueDef {
                    key: issue.short_id(),
                    title: issue.title.clone(),
                    description: issue.description.clone(),
                    r#type: crate::labels::type_label_value(&issue.labels).map(str::to_string),
                    priority: Some(issue.priority),
                    labels,
                    gates: issue.gates_required.clone(),
                    depends_on,
                    planning: None,
                }
            })
            .collect();

        boundary_edges.sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
        Ok(BatchExport {
            defs,
            boundary_edges,
        })
    }

    /// The set of label namespaces that are identity-bound and therefore
    /// stripped from batch export (REQ-04). Derived from configuration only:
    /// membership namespaces declared in `[type_hierarchy.label_associations]`,
    /// plus the `satisfies-namespace` and `container-from-label` values of every
    /// `label-coverage` rule in the effective ruleset.
    fn identity_bound_namespaces(&self) -> Result<HashSet<String>> {
        use crate::declarations::rules::Assertion;

        let mut namespaces: HashSet<String> = HashSet::new();

        let hierarchy = crate::config_manager::get_hierarchy_config(&self.exec.storage)?;
        for (_type_name, namespace) in hierarchy.membership_namespaces() {
            namespaces.insert(namespace.clone());
        }

        for rule in &self.exec.effective_rules()?.rules {
            if let Assertion::LabelCoverage { config } = &rule.assert {
                namespaces.extend(
                    [
                        &config.authored_satisfies_namespace,
                        &config.container_from_label,
                    ]
                    .into_iter()
                    .flatten()
                    .cloned(),
                );
            }
        }

        Ok(namespaces)
    }

    /// The set of issue type names that serve a template planning- or
    /// breakdown-role node (REQ-05). Collected across every registered template
    /// via [`GraphTemplate::planning_type`]/[`breakdown_type`], resolved through
    /// the repository's [`RoleBindings`] — no role or type literal.
    ///
    /// [`GraphTemplate::planning_type`]: crate::templates::GraphTemplate::planning_type
    /// [`breakdown_type`]: crate::templates::GraphTemplate::breakdown_type
    /// [`RoleBindings`]: crate::templates::RoleBindings
    fn bracket_node_types(&self) -> Result<HashSet<String>> {
        let registry = &self.exec.cached_config()?.templates;
        Ok(registry
            .templates
            .iter()
            .flat_map(|t| {
                [
                    t.planning_type(&registry.roles),
                    t.breakdown_type(&registry.roles),
                ]
            })
            .flatten()
            .map(str::to_string)
            .collect())
    }

    /// Resolve the canonical hierarchy for `graph tree`, optionally scoped to a
    /// container's subtree.
    ///
    /// With `root = None` every issue is included; with `root = Some(id)` the
    /// view is the root plus its transitive dependency closure (the DAG subtree
    /// it contains). Each node still carries its repository-wide resolved
    /// parent/children/cluster/rank — scoping filters which nodes are listed, not
    /// how they resolve — so a scoped node's `parent` may point at a container
    /// outside the subtree. Nodes are ordered by ascending short id. Membership
    /// follows the dependency DAG only (labels are advisory), via
    /// [`resolve_hierarchy`](crate::graph::hierarchy::resolve_hierarchy).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::commands::CommandExecutor;
    /// use jit::domain::Priority;
    /// use jit::storage::discover_repository_layout;
    /// use jit::test_utils::setup_test_repo_with_taxonomy;
    ///
    /// // Resolution reads the hierarchy the repository's `config.toml` declares.
    /// let (repo, storage, taxonomy) = setup_test_repo_with_taxonomy().unwrap();
    /// let layout = discover_repository_layout(repo.path(), repo.path().join(".jit")).unwrap();
    /// let executor = CommandExecutor::new(storage).with_layout(layout);
    /// let new = |title: &str, labels: Vec<String>| {
    ///     executor
    ///         .issues().create_issue(title.into(), String::new(), Priority::Normal,
    ///             vec![], labels, None, None, false)
    ///         .unwrap()
    ///         .issue.id
    /// };
    ///
    /// let epic = new("Epic", vec![format!("type:{}", taxonomy.type_at_level(2))]);
    /// let task = new("Task", vec![format!("type:{}", taxonomy.type_at_level(4))]);
    /// executor
    ///     .graph()
    ///     .add_dependencies_with_policy(
    ///         &epic,
    ///         std::slice::from_ref(&task),
    ///         jit::commands::RedundancyPolicy::Reduce,
    ///     )
    ///     .unwrap();
    ///
    /// let response = executor.graph().resolve_hierarchy_tree(None).unwrap();
    /// assert_eq!(response.items.len(), 2);
    /// let epic_view = response.items.iter().find(|n| n.id == epic).unwrap();
    /// assert_eq!(epic_view.hierarchy.children, vec![task.clone()]);
    /// assert_eq!(epic_view.hierarchy.parent, None);
    /// ```
    pub fn resolve_hierarchy_tree(
        &self,
        root: Option<&IssueId>,
    ) -> Result<crate::commands::GraphTreeResponse> {
        use crate::commands::{GraphTreeResponse, HierarchyNodeView};

        let issues = self.exec.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let config = crate::config_manager::get_hierarchy_config(&self.exec.storage)?;
        let resolution = crate::graph::hierarchy::resolve_hierarchy(&issue_refs, &config);

        // When scoped to a root, keep the root plus its transitive dependency
        // closure (the DAG subtree it contains). Resolution itself stays
        // repository-wide; only the listed node set is filtered.
        let root_id = root.cloned();
        let scope: Option<std::collections::HashSet<IssueId>> = root_id.as_ref().map(|root_id| {
            let graph = DependencyGraph::new(&issue_refs);
            let mut set: std::collections::HashSet<IssueId> = graph
                .get_transitive_dependencies(root_id)
                .into_iter()
                .map(|i| i.id.clone())
                .collect();
            set.insert(root_id.clone());
            set
        });
        let in_scope = |id: &IssueId| -> bool { scope.as_ref().is_none_or(|set| set.contains(id)) };

        let mut nodes: Vec<HierarchyNodeView> = issues
            .iter()
            .filter(|issue| in_scope(&issue.id))
            .map(|issue| HierarchyNodeView {
                short_id: issue.short_id(),
                id: issue.id.clone(),
                title: issue.title.clone(),
                type_name: crate::labels::type_label_value(&issue.labels).map(str::to_string),
                hierarchy: resolution.get(&issue.id).cloned().unwrap_or_default(),
            })
            .collect();
        nodes.sort_by(|a, b| a.short_id.cmp(&b.short_id));

        Ok(GraphTreeResponse::with_extras(
            nodes,
            crate::commands::TreeRoot { root: root_id },
        ))
    }

    /// Report membership labels that disagree with DAG-resolved containment
    /// (`query divergence`).
    ///
    /// Thin orchestration over
    /// [`detect_membership_divergences`](crate::graph::hierarchy::detect_membership_divergences):
    /// loads every issue and the repo's type hierarchy, then projects each
    /// divergence to the display shape (adding the issue's short id and title).
    /// The result is ordered by `(issue_id, label)`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::commands::CommandExecutor;
    /// use jit::domain::Priority;
    /// use jit::storage::discover_repository_layout;
    /// use jit::test_utils::setup_test_repo_with_taxonomy;
    ///
    /// let (repo, storage, taxonomy) = setup_test_repo_with_taxonomy().unwrap();
    /// let layout = discover_repository_layout(repo.path(), repo.path().join(".jit")).unwrap();
    /// let executor = CommandExecutor::new(storage).with_layout(layout);
    /// // An epic that contains nothing, plus a task that claims to belong to it.
    /// let container_type = taxonomy.type_at_level(2);
    /// let leaf_type = taxonomy.type_at_level(4);
    /// let membership_namespace = taxonomy
    ///     .label_associations
    ///     .get(container_type)
    ///     .expect("the test taxonomy declares container membership");
    /// let membership = format!("{membership_namespace}:auth");
    /// executor
    ///     .issues().create_issue("Auth".into(), String::new(), Priority::Normal, vec![],
    ///         vec![format!("type:{container_type}"), membership.clone()], None, None, false)
    ///     .unwrap();
    /// executor
    ///     .issues().create_issue("Stray".into(), String::new(), Priority::Normal, vec![],
    ///         vec![format!("type:{leaf_type}"), membership.clone()], None, None, false)
    ///     .unwrap();
    ///
    /// let report = executor.graph().detect_divergences().unwrap();
    /// assert_eq!(report.items.len(), 1);
    /// assert_eq!(report.items[0].label, membership);
    /// ```
    pub fn detect_divergences(&self) -> Result<DivergenceResponse> {
        let issues = self.exec.storage.list_issues()?;
        let config = crate::config_manager::get_hierarchy_config(&self.exec.storage)?;
        Ok(DivergenceResponse::from_issues(&issues, &config))
    }
}

/// Compute summary statistics from a dependency tree
pub fn compute_dependency_summary(
    tree: &[DependencyTreeNode],
) -> crate::commands::DependencySummary {
    let mut unique_ids = HashSet::new();
    let mut by_state: HashMap<crate::domain::State, usize> = HashMap::new();

    collect_stats(tree, &mut unique_ids, &mut by_state);

    crate::commands::DependencySummary {
        total: unique_ids.len(),
        by_state,
    }
}

/// Collect statistics from dependency tree recursively
fn collect_stats(
    tree: &[DependencyTreeNode],
    unique_ids: &mut HashSet<IssueId>,
    by_state: &mut HashMap<crate::domain::State, usize>,
) {
    for node in tree {
        if unique_ids.insert(node.id.clone()) {
            // Only count each unique ID once
            *by_state.entry(node.state).or_insert(0) += 1;
        }
        collect_stats(&node.children, unique_ids, by_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::DependencyTreeNode;
    use crate::domain::{Priority, State};

    #[test]
    fn test_resolve_hierarchy_tree_propagates_invalid_hierarchy_error() {
        let (repo, storage, _) = crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
        let config = crate::test_taxonomy::invalid_hierarchy_config_fragment();
        std::fs::write(storage.root().join("config.toml"), config).unwrap();
        let executor = crate::test_utils::file_executor(repo.path()).unwrap();

        let error = executor
            .graph()
            .resolve_hierarchy_tree(None)
            .expect_err("invalid hierarchy must fail graph resolution");

        assert_eq!(
            error.downcast_ref::<crate::domain::type_taxonomy::ConfigError>(),
            Some(&crate::domain::type_taxonomy::ConfigError::InvalidLevel(0))
        );
    }

    fn make_node(id: &IssueId, state: State) -> DependencyTreeNode {
        DependencyTreeNode {
            id: id.clone(),
            short_id: id.short_id().to_string(),
            title: format!("Node {}", id),
            state,
            priority: Priority::Normal,
            level: 1,
            shared: None,
            archived_from: None,
            children: vec![],
        }
    }

    #[test]
    fn test_compute_dependency_summary_keys_by_state_enum() {
        let tree = vec![
            make_node(
                &crate::test_utils::issue_id(0x9e53_59a5_8c6f_d2b0_438f_8263_9a3b_eda4),
                State::Done,
            ),
            make_node(
                &crate::test_utils::issue_id(0x1a8b_5963_9831_82d7_1ca2_a3a0_89c1_f9f1),
                State::Done,
            ),
            make_node(
                &crate::test_utils::issue_id(0xdaac_5d84_92d4_d5f9_ba87_32e3_b23c_207c),
                State::InProgress,
            ),
        ];

        let summary = compute_dependency_summary(&tree);

        assert_eq!(summary.total, 3);
        assert_eq!(summary.by_state.get(&State::Done), Some(&2));
        assert_eq!(summary.by_state.get(&State::InProgress), Some(&1));
        assert_eq!(summary.by_state.get(&State::Ready), None);
    }

    #[test]
    fn test_compute_dependency_summary_deduplicates_shared_ids() {
        // Same ID appearing as both a direct and nested dep should count once.
        let mut child = make_node(
            &crate::test_utils::issue_id(0x315a_50d0_9915_3228_5331_aff3_5bd5_5d8a),
            State::Done,
        );
        child.level = 2;
        let mut parent = make_node(
            &crate::test_utils::issue_id(0x3ce2_5e38_8dd9_a1a8_fe34_cc5c_fa69_be22),
            State::InProgress,
        );
        parent.children = vec![child];
        let tree = vec![
            make_node(
                &crate::test_utils::issue_id(0x315a_50d0_9915_3228_5331_aff3_5bd5_5d8a),
                State::Done,
            ),
            parent,
        ];

        let summary = compute_dependency_summary(&tree);

        assert_eq!(summary.total, 2, "shared node should only be counted once");
        assert_eq!(summary.by_state.get(&State::Done), Some(&1));
    }

    #[test]
    fn test_dependency_summary_json_uses_snake_case_state_keys() {
        // Regression guard: keys must be "in_progress" not "inprogress".
        let tree = vec![
            make_node(
                &crate::test_utils::issue_id(0x54d5_5acd_bd26_cc73_4e22_92e3_f6b2_86c5),
                State::Done,
            ),
            make_node(
                &crate::test_utils::issue_id(0x907e_5586_9d02_2c3d_23d4_d8ae_3ba0_9e56),
                State::InProgress,
            ),
        ];
        let summary = compute_dependency_summary(&tree);
        let json = serde_json::to_string(&summary).expect("summary serializes");

        assert!(
            json.contains("\"in_progress\""),
            "expected snake_case key \"in_progress\" in JSON, got: {}",
            json
        );
        assert!(
            json.contains("\"done\""),
            "expected key \"done\" in JSON, got: {}",
            json
        );
        assert!(
            !json.contains("\"inprogress\""),
            "must not contain Debug-format key \"inprogress\", got: {}",
            json
        );
    }
}
