//! Graph visualization and traversal

use super::*;
use crate::output::DependencyTreeNode;
use std::collections::{HashMap, HashSet};

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

    pub fn show_downstream(&self, issue_id: &str) -> Result<Vec<Issue>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        let dependents = graph.get_transitive_dependents(&full_id);
        Ok(dependents.into_iter().cloned().collect())
    }

    pub fn show_roots(&self) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        let roots = graph.get_roots();
        Ok(roots.into_iter().cloned().collect())
    }

    pub fn export_graph(&self, format: &str) -> Result<String> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        match format.to_lowercase().as_str() {
            "dot" => Ok(crate::visualization::export_dot(&graph)),
            "mermaid" => Ok(crate::visualization::export_mermaid(&graph)),
            _ => Err(anyhow!(
                "Unsupported format: {}. Use 'dot' or 'mermaid'",
                format
            )),
        }
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
    let mut by_state: HashMap<String, usize> = HashMap::new();

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
    by_state: &mut HashMap<String, usize>,
) {
    for node in tree {
        if unique_ids.insert(node.id.clone()) {
            // Only count each unique ID once
            let state_key = format!("{:?}", node.state).to_lowercase();
            *by_state.entry(state_key).or_insert(0) += 1;
        }
        collect_stats(&node.children, unique_ids, by_state);
    }
}
