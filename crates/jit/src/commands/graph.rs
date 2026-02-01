//! Graph visualization and traversal

use super::*;

impl<S: IssueStore> CommandExecutor<S> {
    /// Show what an issue depends on (immediate or transitive dependencies).
    ///
    /// # Arguments
    ///
    /// * `issue_id` - Issue ID to show dependencies for
    /// * `transitive` - If true, show all transitive dependencies; if false, only immediate
    pub fn show_dependencies(&self, issue_id: &str, transitive: bool) -> Result<Vec<Issue>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;

        if !transitive {
            // Immediate dependencies only (depth 1)
            let deps: Vec<Issue> = issue
                .dependencies
                .iter()
                .filter_map(|dep_id| self.storage.load_issue(dep_id).ok())
                .collect();
            return Ok(deps);
        }

        // Transitive dependencies (all levels)
        let mut result = Vec::new();
        let mut to_process = issue.dependencies.clone();
        let mut processed = std::collections::HashSet::new();

        while let Some(dep_id) = to_process.pop() {
            if processed.contains(&dep_id) {
                continue;
            }
            processed.insert(dep_id.clone());

            if let Ok(dep_issue) = self.storage.load_issue(&dep_id) {
                to_process.extend(dep_issue.dependencies.clone());
                result.push(dep_issue);
            }
        }

        Ok(result)
    }

    pub fn show_graph(&self, issue_id: &str) -> Result<Vec<Issue>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;
        let mut result = vec![issue.clone()];

        // Recursively get dependencies
        let mut to_process = issue.dependencies.clone();
        let mut processed = std::collections::HashSet::new();

        while let Some(dep_id) = to_process.pop() {
            if processed.contains(&dep_id) {
                continue;
            }
            processed.insert(dep_id.clone());

            if let Ok(dep_issue) = self.storage.load_issue(&dep_id) {
                to_process.extend(dep_issue.dependencies.clone());
                result.push(dep_issue);
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
