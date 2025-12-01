//! Dependency graph operations and validation.
//!
//! Provides DAG enforcement, cycle detection, and graph traversal operations.

use crate::domain::Issue;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Errors that can occur during graph operations
#[derive(Debug, Error, PartialEq)]
pub enum GraphError {
    /// A cycle was detected in the dependency graph
    #[error("Cycle detected: adding dependency would create a cycle")]
    CycleDetected,
    /// Referenced issue does not exist
    #[error("Issue not found: {0}")]
    IssueNotFound(String),
}

/// Dependency graph for issues with cycle detection and traversal
pub struct DependencyGraph<'a> {
    issues: HashMap<String, &'a Issue>,
}

impl<'a> DependencyGraph<'a> {
    /// Create a new dependency graph from a list of issues
    pub fn new(issues: &[&'a Issue]) -> Self {
        let issues_map = issues
            .iter()
            .map(|issue| (issue.id.clone(), *issue))
            .collect();

        Self { issues: issues_map }
    }

    /// Validate that adding a dependency would not create a cycle
    pub fn validate_add_dependency(&self, issue_id: &str, dep_id: &str) -> Result<(), GraphError> {
        if !self.issues.contains_key(issue_id) {
            return Err(GraphError::IssueNotFound(issue_id.to_string()));
        }
        if !self.issues.contains_key(dep_id) {
            return Err(GraphError::IssueNotFound(dep_id.to_string()));
        }

        // Check if adding this dependency would create a cycle
        // We simulate adding the edge and check for cycles
        if self.would_create_cycle(issue_id, dep_id) {
            return Err(GraphError::CycleDetected);
        }

        Ok(())
    }

    fn would_create_cycle(&self, from: &str, to: &str) -> bool {
        // Adding edge from -> to creates a cycle if there's already a path to -> from
        // In other words, if 'from' is reachable from 'to'
        self.is_reachable(to, from)
    }

    fn is_reachable(&self, start: &str, target: &str) -> bool {
        let mut visited = HashSet::new();
        let mut stack = vec![start];

        while let Some(current) = stack.pop() {
            if current == target {
                return true;
            }

            if visited.contains(current) {
                continue;
            }
            visited.insert(current);

            if let Some(issue) = self.issues.get(current) {
                for dep in &issue.dependencies {
                    stack.push(dep.as_str());
                }
            }
        }

        false
    }

    /// Get all root issues (issues with no dependencies)
    pub fn get_roots(&self) -> Vec<&'a Issue> {
        self.issues
            .values()
            .filter(|issue| issue.dependencies.is_empty())
            .copied()
            .collect()
    }

    /// Get all issues that directly depend on the given issue
    pub fn get_dependents(&self, issue_id: &str) -> Vec<&'a Issue> {
        self.issues
            .values()
            .filter(|issue| issue.dependencies.contains(&issue_id.to_string()))
            .copied()
            .collect()
    }

    /// Get all issues that transitively depend on the given issue
    pub fn get_transitive_dependents(&self, issue_id: &str) -> Vec<&'a Issue> {
        let mut result = HashSet::new();
        let mut stack = vec![issue_id];
        let mut visited = HashSet::new();

        while let Some(current) = stack.pop() {
            if visited.contains(current) {
                continue;
            }
            visited.insert(current);

            let dependents = self.get_dependents(current);
            for dependent in dependents {
                result.insert(dependent.id.as_str());
                stack.push(&dependent.id);
            }
        }

        result
            .into_iter()
            .filter_map(|id| self.issues.get(id).copied())
            .collect()
    }

    /// Export graph as DOT format for Graphviz
    pub fn export_dot(&self) -> String {
        let mut output = String::from("digraph issues {\n");
        output.push_str("  rankdir=LR;\n");
        output.push_str("  node [shape=box, style=rounded];\n\n");

        // Add nodes with labels
        for issue in self.issues.values() {
            let label = format!("{}\\n{}", issue.id, issue.title.replace('"', "\\\""));
            let color = match issue.state {
                crate::domain::State::Open => "lightgray",
                crate::domain::State::Ready => "lightblue",
                crate::domain::State::InProgress => "yellow",
                crate::domain::State::Done => "lightgreen",
                crate::domain::State::Archived => "gray",
            };
            output.push_str(&format!(
                "  \"{}\" [label=\"{}\", fillcolor={}, style=\"rounded,filled\"];\n",
                issue.id, label, color
            ));
        }

        output.push('\n');

        // Add edges
        for issue in self.issues.values() {
            for dep in &issue.dependencies {
                output.push_str(&format!("  \"{}\" -> \"{}\";\n", issue.id, dep));
            }
        }

        output.push_str("}\n");
        output
    }

    /// Export graph as Mermaid format
    pub fn export_mermaid(&self) -> String {
        let mut output = String::from("graph LR\n");

        // Add nodes with state styling
        for issue in self.issues.values() {
            let label = format!("{}:<br/>{}", issue.id, issue.title);
            let style_class = match issue.state {
                crate::domain::State::Open => "open",
                crate::domain::State::Ready => "ready",
                crate::domain::State::InProgress => "inprogress",
                crate::domain::State::Done => "done",
                crate::domain::State::Archived => "archived",
            };
            output.push_str(&format!(
                "  {}[\"{}\"]:::{}\n",
                issue.id, label, style_class
            ));
        }

        output.push('\n');

        // Add edges
        for issue in self.issues.values() {
            for dep in &issue.dependencies {
                output.push_str(&format!("  {} --> {}\n", issue.id, dep));
            }
        }

        // Add style classes
        output.push_str("\n  classDef open fill:#e0e0e0,stroke:#333\n");
        output.push_str("  classDef ready fill:#add8e6,stroke:#333\n");
        output.push_str("  classDef inprogress fill:#ffff99,stroke:#333\n");
        output.push_str("  classDef done fill:#90ee90,stroke:#333\n");
        output.push_str("  classDef archived fill:#808080,stroke:#333\n");

        output
    }

    /// Validate that the graph is a DAG (no cycles)
    pub fn validate_dag(&self) -> Result<(), GraphError> {
        // Check for cycles using DFS
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for id in self.issues.keys() {
            if !visited.contains(id.as_str())
                && self.has_cycle_dfs(id, &mut visited, &mut rec_stack)
            {
                return Err(GraphError::CycleDetected);
            }
        }

        Ok(())
    }

    fn has_cycle_dfs(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
    ) -> bool {
        visited.insert(node.to_string());
        rec_stack.insert(node.to_string());

        if let Some(issue) = self.issues.get(node) {
            for dep in &issue.dependencies {
                if !visited.contains(dep.as_str()) {
                    if self.has_cycle_dfs(dep, visited, rec_stack) {
                        return true;
                    }
                } else if rec_stack.contains(dep.as_str()) {
                    return true;
                }
            }
        }

        rec_stack.remove(node);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_add_dependency_success() {
        let issue1 = Issue::new("Issue 1".to_string(), "Desc".to_string());
        let issue2 = Issue::new("Issue 2".to_string(), "Desc".to_string());

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        assert!(graph
            .validate_add_dependency(&issue1.id, &issue2.id)
            .is_ok());
    }

    #[test]
    fn test_validate_add_dependency_with_nonexistent_issue() {
        let issue1 = Issue::new("Issue 1".to_string(), "Desc".to_string());

        let issues = vec![&issue1];
        let graph = DependencyGraph::new(&issues);

        let result = graph.validate_add_dependency(&issue1.id, "nonexistent");
        assert_eq!(
            result,
            Err(GraphError::IssueNotFound("nonexistent".to_string()))
        );
    }

    #[test]
    fn test_validate_add_dependency_detects_direct_cycle() {
        let mut issue1 = Issue::new("Issue 1".to_string(), "Desc".to_string());
        let issue2 = Issue::new("Issue 2".to_string(), "Desc".to_string());

        // issue1 depends on issue2
        issue1.dependencies.push(issue2.id.clone());

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        // Trying to make issue2 depend on issue1 would create a cycle
        let result = graph.validate_add_dependency(&issue2.id, &issue1.id);
        assert_eq!(result, Err(GraphError::CycleDetected));
    }

    #[test]
    fn test_validate_add_dependency_detects_indirect_cycle() {
        let mut issue1 = Issue::new("Issue 1".to_string(), "Desc".to_string());
        let mut issue2 = Issue::new("Issue 2".to_string(), "Desc".to_string());
        let issue3 = Issue::new("Issue 3".to_string(), "Desc".to_string());

        // issue1 -> issue2 -> issue3
        issue1.dependencies.push(issue2.id.clone());
        issue2.dependencies.push(issue3.id.clone());

        let issues = vec![&issue1, &issue2, &issue3];
        let graph = DependencyGraph::new(&issues);

        // Trying to make issue3 depend on issue1 would create a cycle
        let result = graph.validate_add_dependency(&issue3.id, &issue1.id);
        assert_eq!(result, Err(GraphError::CycleDetected));
    }

    #[test]
    fn test_get_roots_returns_issues_with_no_dependencies() {
        let issue1 = Issue::new("Root 1".to_string(), "Desc".to_string());
        let mut issue2 = Issue::new("Dependent".to_string(), "Desc".to_string());
        let issue3 = Issue::new("Root 2".to_string(), "Desc".to_string());

        issue2.dependencies.push(issue1.id.clone());

        let issues = vec![&issue1, &issue2, &issue3];
        let graph = DependencyGraph::new(&issues);

        let roots = graph.get_roots();
        assert_eq!(roots.len(), 2);
        assert!(roots.iter().any(|i| i.id == issue1.id));
        assert!(roots.iter().any(|i| i.id == issue3.id));
    }

    #[test]
    fn test_get_dependents_returns_direct_dependents() {
        let issue1 = Issue::new("Dependency".to_string(), "Desc".to_string());
        let mut issue2 = Issue::new("Dependent 1".to_string(), "Desc".to_string());
        let mut issue3 = Issue::new("Dependent 2".to_string(), "Desc".to_string());

        issue2.dependencies.push(issue1.id.clone());
        issue3.dependencies.push(issue1.id.clone());

        let issues = vec![&issue1, &issue2, &issue3];
        let graph = DependencyGraph::new(&issues);

        let dependents = graph.get_dependents(&issue1.id);
        assert_eq!(dependents.len(), 2);
        assert!(dependents.iter().any(|i| i.id == issue2.id));
        assert!(dependents.iter().any(|i| i.id == issue3.id));
    }

    #[test]
    fn test_get_transitive_dependents() {
        let issue1 = Issue::new("Root".to_string(), "Desc".to_string());
        let mut issue2 = Issue::new("Level 1".to_string(), "Desc".to_string());
        let mut issue3 = Issue::new("Level 2".to_string(), "Desc".to_string());

        issue2.dependencies.push(issue1.id.clone());
        issue3.dependencies.push(issue2.id.clone());

        let issues = vec![&issue1, &issue2, &issue3];
        let graph = DependencyGraph::new(&issues);

        let transitive = graph.get_transitive_dependents(&issue1.id);
        assert_eq!(transitive.len(), 2);
        assert!(transitive.iter().any(|i| i.id == issue2.id));
        assert!(transitive.iter().any(|i| i.id == issue3.id));
    }

    #[test]
    fn test_validate_dag_success_for_valid_graph() {
        let issue1 = Issue::new("Issue 1".to_string(), "Desc".to_string());
        let mut issue2 = Issue::new("Issue 2".to_string(), "Desc".to_string());
        let mut issue3 = Issue::new("Issue 3".to_string(), "Desc".to_string());

        issue2.dependencies.push(issue1.id.clone());
        issue3.dependencies.push(issue2.id.clone());

        let issues = vec![&issue1, &issue2, &issue3];
        let graph = DependencyGraph::new(&issues);

        assert!(graph.validate_dag().is_ok());
    }

    #[test]
    fn test_validate_dag_detects_cycle() {
        let mut issue1 = Issue::new("Issue 1".to_string(), "Desc".to_string());
        let mut issue2 = Issue::new("Issue 2".to_string(), "Desc".to_string());

        issue1.dependencies.push(issue2.id.clone());
        issue2.dependencies.push(issue1.id.clone());

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        assert_eq!(graph.validate_dag(), Err(GraphError::CycleDetected));
    }

    #[test]
    fn test_export_dot_format() {
        let issue1 = Issue::new("API Design".to_string(), "Design REST API".to_string());
        let mut issue2 = Issue::new("Backend".to_string(), "Implement backend".to_string());
        issue2.dependencies.push(issue1.id.clone());

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        let dot = graph.export_dot();

        assert!(dot.contains("digraph issues"));
        assert!(dot.contains("rankdir=LR"));
        assert!(dot.contains(&issue1.id));
        assert!(dot.contains(&issue2.id));
        assert!(dot.contains("API Design"));
        assert!(dot.contains("Backend"));
        assert!(dot.contains(&format!("\"{}\" -> \"{}\"", issue2.id, issue1.id)));
    }

    #[test]
    fn test_export_mermaid_format() {
        let issue1 = Issue::new("Setup".to_string(), "Initial setup".to_string());
        let mut issue2 = Issue::new("Deploy".to_string(), "Deploy to prod".to_string());
        issue2.dependencies.push(issue1.id.clone());

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        let mermaid = graph.export_mermaid();

        assert!(mermaid.contains("graph LR"));
        assert!(mermaid.contains(&issue1.id));
        assert!(mermaid.contains(&issue2.id));
        assert!(mermaid.contains("Setup"));
        assert!(mermaid.contains("Deploy"));
        assert!(mermaid.contains(&format!("{} --> {}", issue2.id, issue1.id)));
        assert!(mermaid.contains("classDef open"));
    }

    #[test]
    fn test_export_dot_with_different_states() {
        use crate::domain::State;

        let mut issue1 = Issue::new("Done Task".to_string(), "Completed".to_string());
        issue1.state = State::Done;

        let mut issue2 = Issue::new("In Progress".to_string(), "Working on it".to_string());
        issue2.state = State::InProgress;

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        let dot = graph.export_dot();

        assert!(dot.contains("lightgreen")); // Done state
        assert!(dot.contains("yellow")); // InProgress state
    }

    #[test]
    fn test_export_handles_special_characters() {
        let issue = Issue::new("Title with \"quotes\"".to_string(), "Test".to_string());
        let issues = vec![&issue];
        let graph = DependencyGraph::new(&issues);

        let dot = graph.export_dot();

        assert!(dot.contains("\\\""));
        assert!(!dot.contains("Title with \"quotes\""));
    }
}
