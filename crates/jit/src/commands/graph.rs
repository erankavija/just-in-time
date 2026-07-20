//! Graph visualization and traversal

use super::*;
use crate::graph::Direction;
use crate::output::DependencyTreeNode;
use std::collections::{HashMap, HashSet};

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
    /// path ([`export_graph_batch`](CommandExecutor::export_graph_batch)); the
    /// graph-rendering [`export_graph`](CommandExecutor::export_graph) never
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

/// The `batch-create` priority string for `priority` (the value
/// [`Priority::from_str`](std::str::FromStr) round-trips), so an exported def
/// re-imports at the same priority.
fn priority_str(priority: crate::domain::Priority) -> &'static str {
    use crate::domain::Priority;
    match priority {
        Priority::Low => "low",
        Priority::Normal => "normal",
        Priority::High => "high",
        Priority::Critical => "critical",
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Build a dependency tree with specified depth
    ///
    /// Returns a tree structure that preserves parent-child relationships
    /// and marks shared dependencies (diamonds in the DAG).
    ///
    /// Every issue is loaded once up front; the unfolding itself
    /// ([`expand`](DependencyGraph::expand)) runs in memory over the graph.
    pub fn build_dependency_tree(
        &self,
        issue_id: &str,
        depth: u32,
    ) -> Result<Vec<DependencyTreeNode>> {
        use crate::domain::MinimalIssue;
        use crate::graph::Expansion;

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        let forest = graph.expand(&full_id, Direction::Dependencies, depth);
        // An issue unfolded along several paths is a diamond in the DAG.
        let occurrences = Expansion::occurrences(&forest);
        let to_tree_node = |issue: &Issue, level: u32, children: Vec<DependencyTreeNode>| {
            let shared = occurrences.get(issue.id.as_str()).is_some_and(|&n| n > 1);
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
    pub fn show_dependencies_with_depth(&self, issue_id: &str, depth: u32) -> Result<Vec<Issue>> {
        self.traverse_dependency_edges(issue_id, Direction::Dependencies, depth)
    }

    /// Show what depends on an issue with depth control.
    ///
    /// # Arguments
    ///
    /// * `issue_id` - Issue ID to show dependents for
    /// * `depth` - Maximum depth to traverse (1 = immediate, 0 = unlimited)
    pub fn show_rdeps_with_depth(&self, issue_id: &str, depth: u32) -> Result<Vec<Issue>> {
        self.traverse_dependency_edges(issue_id, Direction::Dependents, depth)
    }

    /// Walk the repository's dependency graph from `issue_id`, following edges in
    /// `direction` up to `depth` (0 = unlimited), and return the issues reached.
    fn traverse_dependency_edges(
        &self,
        issue_id: &str,
        direction: Direction,
        depth: u32,
    ) -> Result<Vec<Issue>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        Ok(graph
            .traverse(&full_id, direction, depth)
            .into_iter()
            .cloned()
            .collect())
    }

    pub fn show_roots(&self) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
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
        scope: Option<&str>,
    ) -> Result<String> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();

        // Resolve the hierarchy once when it is needed — for scoping and/or the
        // full JSON node shape. Resolution is always repository-wide.
        let needs_full_json = matches!((format, full), (GraphExportFormat::Json, true));
        let resolution = if scope.is_some() || needs_full_json {
            let config = crate::config_manager::get_hierarchy_config(&self.storage)?;
            Some(crate::graph::hierarchy::resolve_hierarchy(
                &issue_refs,
                &config,
            ))
        } else {
            None
        };

        let member_ids = match scope {
            Some(container) => {
                let container_id = self.storage.resolve_issue_id(container)?;
                Some(
                    resolution
                        .as_ref()
                        .expect("resolution is built whenever scope is Some")
                        .membership_closure(&container_id),
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
            GraphExportFormat::Dot => crate::visualization::export_dot(&graph),
            GraphExportFormat::Mermaid => crate::visualization::export_mermaid(&graph),
            GraphExportFormat::Json if full => crate::visualization::export_json_full(
                &graph,
                resolution
                    .as_ref()
                    .expect("resolution is built for the full JSON shape"),
            ),
            GraphExportFormat::Json => crate::visualization::export_json(&graph),
            GraphExportFormat::Batch => {
                return Err(anyhow::anyhow!(
                    "batch format is handled by export_graph_batch, not export_graph"
                ));
            }
        })
    }

    /// Export the graph in the `batch-create` input schema — the structural
    /// inverse of [`batch_create_from_json`](Self::batch_create_from_json).
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
    pub fn export_graph_batch(&self, scope: Option<&str>) -> Result<BatchExport> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();

        // Membership scope (the whole graph when unscoped).
        let member_ids: Option<HashSet<String>> = match scope {
            Some(container) => {
                let container_id = self.storage.resolve_issue_id(container)?;
                let config = crate::config_manager::get_hierarchy_config(&self.storage)?;
                let resolution = crate::graph::hierarchy::resolve_hierarchy(&issue_refs, &config);
                Some(resolution.membership_closure(&container_id))
            }
            None => None,
        };
        let in_scope = |id: &str| member_ids.as_ref().is_none_or(|m| m.contains(id));

        // Config-derived projection inputs.
        let stripped_ns = self.identity_bound_namespaces()?;
        let bracket_types = self.bracket_node_types()?;
        let is_bracket = |issue: &Issue| {
            crate::labels::type_label_value(&issue.labels)
                .is_some_and(|t| bracket_types.contains(t))
        };
        let by_id: HashMap<&str, &Issue> = issues.iter().map(|i| (i.id.as_str(), i)).collect();
        let short_of = |id: &str| -> String { id.chars().take(8).collect() };

        // Batch node set: in-scope, non-bracket nodes, ordered by short id.
        let mut nodes: Vec<&Issue> = issue_refs
            .iter()
            .copied()
            .filter(|i| in_scope(&i.id) && !is_bracket(i))
            .collect();
        nodes.sort_by_key(|i| i.short_id());
        let node_ids: HashSet<&str> = nodes.iter().map(|i| i.id.as_str()).collect();

        let mut boundary_edges: Vec<BoundaryEdge> = Vec::new();
        let defs: Vec<BatchIssueDef> = nodes
            .iter()
            .map(|issue| {
                let mut depends_on: Vec<String> = Vec::new();
                for dep in &issue.dependencies {
                    if node_ids.contains(dep.as_str()) {
                        // Internal edge: reference the dependency by its key.
                        depends_on.push(short_of(dep));
                    } else if by_id.get(dep.as_str()).is_some_and(|d| is_bracket(d)) {
                        // Edge to a bracket node: dropped with the bracket (REQ-05).
                    } else if by_id.contains_key(dep.as_str()) {
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
                    priority: Some(priority_str(issue.priority).to_string()),
                    labels,
                    gates: issue.gates_required.clone(),
                    depends_on,
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

        let hierarchy = crate::config_manager::get_hierarchy_config(&self.storage)?;
        for (_type_name, namespace) in hierarchy.membership_namespaces() {
            namespaces.insert(namespace.clone());
        }

        for rule in &self.effective_rules()?.rules {
            if let Assertion::LabelCoverage { config } = &rule.assert {
                for key in ["satisfies-namespace", "container-from-label"] {
                    if let Some(ns) = config.get(key).and_then(|v| v.as_str()) {
                        namespaces.insert(ns.to_string());
                    }
                }
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
        let registry = &self.cached_config()?.templates;
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
    /// use jit::storage::{InMemoryStorage, IssueStore};
    ///
    /// let storage = InMemoryStorage::new();
    /// storage.init().unwrap();
    /// let layout = storage.repository_layout();
    /// let executor = CommandExecutor::new(storage).with_layout(layout);
    /// let new = |title: &str, labels: Vec<String>| {
    ///     executor
    ///         .create_issue(title.into(), String::new(), Priority::Normal,
    ///             vec![], labels, None, None, false)
    ///         .unwrap()
    ///         .0
    /// };
    ///
    /// let epic = new("Epic", vec!["type:epic".into()]);
    /// let task = new("Task", vec!["type:task".into()]);
    /// executor.add_dependency(&epic, &task).unwrap();
    ///
    /// let response = executor.resolve_hierarchy_tree(None).unwrap();
    /// assert_eq!(response.count, 2);
    /// let epic_view = response.nodes.iter().find(|n| n.id == epic).unwrap();
    /// assert_eq!(epic_view.hierarchy.children, vec![task.clone()]);
    /// assert_eq!(epic_view.hierarchy.parent, None);
    /// ```
    pub fn resolve_hierarchy_tree(
        &self,
        root: Option<&str>,
    ) -> Result<crate::output::GraphTreeResponse> {
        use crate::output::{GraphTreeResponse, HierarchyNodeView};

        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let config = crate::config_manager::get_hierarchy_config(&self.storage)?;
        let resolution = crate::graph::hierarchy::resolve_hierarchy(&issue_refs, &config);

        // When scoped to a root, keep the root plus its transitive dependency
        // closure (the DAG subtree it contains). Resolution itself stays
        // repository-wide; only the listed node set is filtered.
        let root_id = match root {
            Some(r) => Some(self.storage.resolve_issue_id(r)?),
            None => None,
        };
        let scope: Option<std::collections::HashSet<String>> = root_id.as_deref().map(|root_id| {
            let graph = DependencyGraph::new(&issue_refs);
            let mut set: std::collections::HashSet<String> = graph
                .get_transitive_dependencies(root_id)
                .into_iter()
                .map(|i| i.id.clone())
                .collect();
            set.insert(root_id.to_string());
            set
        });
        let in_scope = |id: &str| -> bool { scope.as_ref().is_none_or(|set| set.contains(id)) };

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

        Ok(GraphTreeResponse {
            root: root_id,
            count: nodes.len(),
            nodes,
        })
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
    /// use jit::storage::{InMemoryStorage, IssueStore};
    ///
    /// let storage = InMemoryStorage::new();
    /// storage.init().unwrap();
    /// let layout = storage.repository_layout();
    /// let executor = CommandExecutor::new(storage).with_layout(layout);
    /// // An epic that contains nothing, plus a task that claims to belong to it.
    /// executor
    ///     .create_issue("Auth".into(), String::new(), Priority::Normal, vec![],
    ///         vec!["type:epic".into(), "epic:auth".into()], None, None, false)
    ///     .unwrap();
    /// executor
    ///     .create_issue("Stray".into(), String::new(), Priority::Normal, vec![],
    ///         vec!["type:task".into(), "epic:auth".into()], None, None, false)
    ///     .unwrap();
    ///
    /// let report = executor.detect_divergences().unwrap();
    /// assert_eq!(report.count, 1);
    /// assert_eq!(report.divergences[0].label, "epic:auth");
    /// ```
    pub fn detect_divergences(&self) -> Result<crate::output::DivergenceResponse> {
        use crate::output::{DivergenceResponse, DivergenceView};

        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let config = crate::config_manager::get_hierarchy_config(&self.storage)?;
        let divergences =
            crate::graph::hierarchy::detect_membership_divergences(&issue_refs, &config);

        let by_id: HashMap<&str, &Issue> = issues.iter().map(|i| (i.id.as_str(), i)).collect();
        let views = divergences
            .into_iter()
            .map(|d| {
                let issue = by_id.get(d.issue_id.as_str());
                DivergenceView {
                    short_id: d.issue_id.chars().take(8).collect(),
                    title: issue.map(|i| i.title.clone()).unwrap_or_default(),
                    id: d.issue_id,
                    label: d.label,
                    namespace: d.namespace,
                    value: d.value,
                }
            })
            .collect::<Vec<_>>();

        Ok(DivergenceResponse {
            count: views.len(),
            divergences: views,
        })
    }
}

/// Compute summary statistics from a dependency tree
pub fn compute_dependency_summary(tree: &[DependencyTreeNode]) -> crate::output::DependencySummary {
    let mut unique_ids = HashSet::new();
    let mut by_state: HashMap<crate::domain::State, usize> = HashMap::new();

    collect_stats(tree, &mut unique_ids, &mut by_state);

    crate::output::DependencySummary {
        total: unique_ids.len(),
        by_state,
    }
}

/// Collect statistics from dependency tree recursively
fn collect_stats(
    tree: &[DependencyTreeNode],
    unique_ids: &mut HashSet<String>,
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
    use crate::domain::{Priority, State};
    use crate::output::DependencyTreeNode;

    fn make_node(id: &str, state: State) -> DependencyTreeNode {
        DependencyTreeNode {
            id: id.to_string(),
            short_id: id[..8.min(id.len())].to_string(),
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
            make_node("id-done-1", State::Done),
            make_node("id-done-2", State::Done),
            make_node("id-in-progress", State::InProgress),
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
        let mut child = make_node("shared-id", State::Done);
        child.level = 2;
        let mut parent = make_node("parent-id", State::InProgress);
        parent.children = vec![child];
        let tree = vec![make_node("shared-id", State::Done), parent];

        let summary = compute_dependency_summary(&tree);

        assert_eq!(summary.total, 2, "shared node should only be counted once");
        assert_eq!(summary.by_state.get(&State::Done), Some(&1));
    }

    #[test]
    fn test_dependency_summary_json_uses_snake_case_state_keys() {
        // Regression guard: keys must be "in_progress" not "inprogress".
        let tree = vec![
            make_node("a", State::Done),
            make_node("b", State::InProgress),
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
