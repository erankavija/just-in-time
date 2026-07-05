//! Graph visualization and traversal

use super::*;
use crate::output::DependencyTreeNode;
use std::collections::{HashMap, HashSet};

/// Serialization format for `jit graph export`.
///
/// Deriving [`clap::ValueEnum`] lets clap reject an unknown format at parse time
/// (with the accepted values listed), replacing the previous runtime
/// `to_lowercase()` match. The value names are the lowercase variant names:
/// `dot`, `mermaid`, `json`.
///
/// # Examples
///
/// ```
/// use jit::commands::GraphExportFormat;
/// use clap::ValueEnum;
///
/// assert_eq!(
///     GraphExportFormat::from_str("mermaid", true).unwrap(),
///     GraphExportFormat::Mermaid
/// );
/// // Unknown formats are rejected.
/// assert!(GraphExportFormat::from_str("yaml", true).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum GraphExportFormat {
    /// Graphviz DOT format.
    Dot,
    /// Mermaid diagram format.
    Mermaid,
    /// JSON node/edge format.
    Json,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Build a dependency tree with specified depth
    ///
    /// Returns a tree structure that preserves parent-child relationships
    /// and marks shared dependencies (diamonds in the DAG).
    pub fn build_dependency_tree(
        &self,
        issue_id: &str,
        depth: u32,
    ) -> Result<Vec<DependencyTreeNode>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let root_issue = self.storage.load_issue(&full_id)?;

        // Track all seen nodes to detect shared dependencies
        let mut seen_ids: HashMap<String, usize> = HashMap::new();

        // Build tree recursively
        let mut tree = Vec::new();
        for dep_id in &root_issue.dependencies {
            if let Ok(dep_issue) = self.storage.load_issue(dep_id) {
                let node = self.build_tree_node(&dep_issue, 1, depth, &mut seen_ids)?;
                tree.push(node);
            }
        }

        // Mark nodes that appear multiple times as shared
        mark_shared_nodes(&mut tree, &seen_ids);

        Ok(tree)
    }

    /// Recursively build a tree node
    fn build_tree_node(
        &self,
        issue: &Issue,
        current_level: u32,
        max_depth: u32,
        seen_ids: &mut HashMap<String, usize>,
    ) -> Result<DependencyTreeNode> {
        use crate::domain::MinimalIssue;

        // Track this node
        *seen_ids.entry(issue.id.clone()).or_insert(0) += 1;

        let minimal = MinimalIssue::from(issue);
        let mut node = DependencyTreeNode::from_minimal(&minimal, current_level);

        // Recurse into children if we haven't reached max depth
        if max_depth == 0 || current_level < max_depth {
            for dep_id in &issue.dependencies {
                if let Ok(dep_issue) = self.storage.load_issue(dep_id) {
                    let child_node =
                        self.build_tree_node(&dep_issue, current_level + 1, max_depth, seen_ids)?;
                    node.children.push(child_node);
                }
            }
        }

        Ok(node)
    }
    /// Show what an issue depends on with depth control.
    ///
    /// # Arguments
    ///
    /// * `issue_id` - Issue ID to show dependencies for
    /// * `depth` - Maximum depth to traverse (1 = immediate, 0 = unlimited)
    pub fn show_dependencies_with_depth(&self, issue_id: &str, depth: u32) -> Result<Vec<Issue>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;

        if depth == 1 {
            // Immediate dependencies only
            let deps: Vec<Issue> = issue
                .dependencies
                .iter()
                .filter_map(|dep_id| self.storage.load_issue(dep_id).ok())
                .collect();
            return Ok(deps);
        }

        // Depth-limited or unlimited traversal
        let mut result = Vec::new();
        let mut to_process: Vec<(String, u32)> = issue
            .dependencies
            .iter()
            .map(|id| (id.clone(), 1))
            .collect();
        let mut processed = std::collections::HashSet::new();

        while let Some((dep_id, current_depth)) = to_process.pop() {
            if processed.contains(&dep_id) {
                continue;
            }
            processed.insert(dep_id.clone());

            if let Ok(dep_issue) = self.storage.load_issue(&dep_id) {
                result.push(dep_issue.clone());

                // Add children if we haven't reached max depth
                if depth == 0 || current_depth < depth {
                    for child_id in &dep_issue.dependencies {
                        to_process.push((child_id.clone(), current_depth + 1));
                    }
                }
            }
        }

        Ok(result)
    }

    pub fn show_rdeps_with_depth(&self, issue_id: &str, depth: u32) -> Result<Vec<Issue>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        if depth == 0 {
            // Unlimited transitive traversal
            let dependents = graph.get_transitive_dependents(&full_id);
            return Ok(dependents.into_iter().cloned().collect());
        }

        if depth == 1 {
            // Immediate dependents only
            let dependents = graph.get_dependents(&full_id);
            return Ok(dependents.into_iter().cloned().collect());
        }

        // Depth-limited BFS
        let mut result = Vec::new();
        let mut to_process: Vec<(String, u32)> = graph
            .get_dependents(&full_id)
            .iter()
            .map(|issue| (issue.id.to_string(), 1))
            .collect();
        let mut processed = HashSet::new();

        while let Some((dep_id, current_depth)) = to_process.pop() {
            if processed.contains(&dep_id) {
                continue;
            }
            processed.insert(dep_id.clone());

            if let Ok(dep_issue) = self.storage.load_issue(&dep_id) {
                result.push(dep_issue.clone());
                if current_depth < depth {
                    for parent in graph.get_dependents(&dep_id) {
                        to_process.push((parent.id.to_string(), current_depth + 1));
                    }
                }
            }
        }

        Ok(result)
    }

    pub fn show_roots(&self) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        let roots = graph.get_roots();
        Ok(roots.into_iter().cloned().collect())
    }

    /// Render the whole-repository dependency graph in `format`.
    ///
    /// `full` selects the complete-record JSON node shape
    /// ([`export_json_full`](crate::visualization::export_json_full)) instead of
    /// the default summary shape; it applies ONLY to
    /// [`GraphExportFormat::Json`]. The caller (CLI) rejects `full` with a
    /// non-JSON format as a usage error before reaching here, so the `dot`/
    /// `mermaid` arms ignore it.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use jit::commands::{CommandExecutor, GraphExportFormat};
    /// # use jit::storage::JsonFileStorage;
    /// # fn run(executor: &CommandExecutor<JsonFileStorage>) -> anyhow::Result<()> {
    /// // Complete issue records plus the edge list.
    /// let json = executor.export_graph(GraphExportFormat::Json, true)?;
    /// # let _ = json;
    /// # Ok(())
    /// # }
    /// ```
    pub fn export_graph(&self, format: GraphExportFormat, full: bool) -> Result<String> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        Ok(match format {
            GraphExportFormat::Dot => crate::visualization::export_dot(&graph),
            GraphExportFormat::Mermaid => crate::visualization::export_mermaid(&graph),
            GraphExportFormat::Json if full => {
                // The full node shape carries the DAG-resolved parent + cluster;
                // resolution reads the repo's configured type hierarchy.
                let config = crate::hierarchy_templates::get_hierarchy_config(&self.storage)?;
                let resolution = crate::graph::hierarchy::resolve_hierarchy(&issue_refs, &config);
                crate::visualization::export_json_full(&graph, &resolution)
            }
            GraphExportFormat::Json => crate::visualization::export_json(&graph),
        })
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
    /// let executor = CommandExecutor::new(storage);
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
    /// assert_eq!(epic_view.children, vec![task.clone()]);
    /// assert_eq!(epic_view.parent, None);
    /// ```
    pub fn resolve_hierarchy_tree(
        &self,
        root: Option<&str>,
    ) -> Result<crate::output::GraphTreeResponse> {
        use crate::output::{GraphTreeResponse, HierarchyNodeView};

        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let config = crate::hierarchy_templates::get_hierarchy_config(&self.storage)?;
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
            .map(|issue| {
                let facts = resolution.get(&issue.id);
                HierarchyNodeView {
                    short_id: issue.short_id(),
                    id: issue.id.clone(),
                    title: issue.title.clone(),
                    type_name: crate::labels::type_label_value(&issue.labels).map(str::to_string),
                    parent: facts.and_then(|f| f.parent.clone()),
                    children: facts.map(|f| f.children.clone()).unwrap_or_default(),
                    cluster: facts.and_then(|f| f.cluster.clone()),
                    rank: facts.map(|f| f.rank).unwrap_or(0),
                }
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
    /// let executor = CommandExecutor::new(storage);
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
        let config = crate::hierarchy_templates::get_hierarchy_config(&self.storage)?;
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

/// Mark nodes that appear multiple times in the tree as shared
fn mark_shared_nodes(tree: &mut [DependencyTreeNode], seen_counts: &HashMap<String, usize>) {
    for node in tree {
        if seen_counts.get(&node.id).copied().unwrap_or(0) > 1 {
            node.shared = Some(true);
        }
        mark_shared_nodes(&mut node.children, seen_counts);
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
