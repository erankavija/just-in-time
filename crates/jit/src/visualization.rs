//! Visualization and export functionality for Issue dependency graphs.
//!
//! This module provides functions to export Issue dependency graphs to various formats
//! like DOT (Graphviz) and Mermaid. These functions are Issue-specific and access
//! Issue fields like title and state for rendering.

use crate::domain::{Issue, State};
use crate::graph::DependencyGraph;

/// Export Issue dependency graph as DOT format for Graphviz
///
/// Generates a directed graph visualization with:
/// - Node labels showing issue ID and title
/// - Color-coded nodes based on issue state
/// - Dependency edges between issues
///
/// # Example
/// ```
/// use jit::{Issue, graph::DependencyGraph};
/// use jit::visualization;
///
/// let issue1 = Issue::new("Setup".to_string(), "Initial setup".to_string());
/// let mut issue2 = Issue::new("Deploy".to_string(), "Deploy app".to_string());
/// issue2.dependencies.push(issue1.id.clone());
///
/// let issues = vec![&issue1, &issue2];
/// let graph = DependencyGraph::new(&issues);
/// let dot = visualization::export_dot(&graph);
/// assert!(dot.contains("digraph issues"));
/// ```
pub fn export_dot(graph: &DependencyGraph<Issue>) -> String {
    let mut output = String::from("digraph issues {\n");
    output.push_str("  rankdir=LR;\n");
    output.push_str("  node [shape=box, style=rounded];\n\n");

    // Get all nodes from the graph
    let mut all_nodes = Vec::new();

    // Collect all nodes (this is a bit awkward, but we need to iterate all nodes)
    // We'll use a helper to traverse the graph
    collect_all_nodes(graph, &mut all_nodes);

    // Add nodes with labels
    for issue in &all_nodes {
        let label = format!("{}\\n{}", issue.id, issue.title.replace('"', "\\\""));
        let color = match issue.state {
            State::Backlog => "lightgray",
            State::Ready => "lightblue",
            State::InProgress => "yellow",
            State::Gated => "orange",
            State::Done => "lightgreen",
            State::Archived => "gray",
        };
        output.push_str(&format!(
            "  \"{}\" [label=\"{}\", fillcolor={}, style=\"rounded,filled\"];\n",
            issue.id, label, color
        ));
    }

    output.push('\n');

    // Add edges
    for issue in &all_nodes {
        for dep in &issue.dependencies {
            output.push_str(&format!("  \"{}\" -> \"{}\";\n", issue.id, dep));
        }
    }

    output.push_str("}\n");
    output
}

/// Export Issue dependency graph as Mermaid format
///
/// Generates a Mermaid flowchart with:
/// - Node labels showing issue ID and title
/// - CSS classes for state-based styling
/// - Dependency arrows between issues
///
/// # Example
/// ```
/// use jit::{Issue, graph::DependencyGraph};
/// use jit::visualization;
///
/// let issue1 = Issue::new("Design".to_string(), "Design API".to_string());
/// let mut issue2 = Issue::new("Implement".to_string(), "Build API".to_string());
/// issue2.dependencies.push(issue1.id.clone());
///
/// let issues = vec![&issue1, &issue2];
/// let graph = DependencyGraph::new(&issues);
/// let mermaid = visualization::export_mermaid(&graph);
/// assert!(mermaid.contains("graph LR"));
/// ```
pub fn export_mermaid(graph: &DependencyGraph<Issue>) -> String {
    let mut output = String::from("graph LR\n");

    // Collect all nodes
    let mut all_nodes = Vec::new();
    collect_all_nodes(graph, &mut all_nodes);

    // Add nodes with state styling
    for issue in &all_nodes {
        let label = format!("{}:<br/>{}", issue.id, issue.title);
        let style_class = match issue.state {
            State::Backlog => "backlog",
            State::Ready => "ready",
            State::InProgress => "inprogress",
            State::Gated => "gated",
            State::Done => "done",
            State::Archived => "archived",
        };
        output.push_str(&format!(
            "  {}[\"{}\"]:::{}\n",
            issue.id, label, style_class
        ));
    }

    output.push('\n');

    // Add edges
    for issue in &all_nodes {
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

// Helper function to collect all nodes from the graph
// Since we don't have direct access to internal nodes, we build the set
// by traversing from roots and collecting all dependents
fn collect_all_nodes<'a>(graph: &DependencyGraph<'a, Issue>, nodes: &mut Vec<&'a Issue>) {
    use std::collections::HashSet;

    let roots = graph.get_roots();
    let mut visited = HashSet::new();
    let mut stack: Vec<&Issue> = roots.clone();

    while let Some(node) = stack.pop() {
        if visited.insert(node.id.clone()) {
            nodes.push(node);
            // Get all nodes that depend on this one
            let dependents = graph.get_dependents(&node.id);
            stack.extend(dependents);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_dot_format() {
        let issue1 = Issue::new("API Design".to_string(), "Design REST API".to_string());
        let mut issue2 = Issue::new("Backend".to_string(), "Implement backend".to_string());
        issue2.dependencies.push(issue1.id.clone());

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        let dot = export_dot(&graph);

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

        let mermaid = export_mermaid(&graph);

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
        let mut issue1 = Issue::new("Done Task".to_string(), "Completed".to_string());
        issue1.state = State::Done;

        let mut issue2 = Issue::new("In Progress".to_string(), "Working on it".to_string());
        issue2.state = State::InProgress;

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        let dot = export_dot(&graph);

        assert!(dot.contains("lightgreen")); // Done state
        assert!(dot.contains("yellow")); // InProgress state
    }

    #[test]
    fn test_export_handles_special_characters() {
        let issue = Issue::new("Title with \"quotes\"".to_string(), "Test".to_string());
        let issues = vec![&issue];
        let graph = DependencyGraph::new(&issues);

        let dot = export_dot(&graph);

        assert!(dot.contains("\\\""));
        assert!(!dot.contains("Title with \"quotes\""));
    }
}
