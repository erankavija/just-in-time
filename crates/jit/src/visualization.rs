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
/// use jit::visualization;
/// use jit::{graph::DependencyGraph, Issue};
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
            State::Rejected => "pink",
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
/// use jit::visualization;
/// use jit::{graph::DependencyGraph, Issue};
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
            State::Rejected => "rejected",
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

/// Export Issue dependency graph as JSON for programmatic analysis (summary shape)
///
/// Emits `{ "nodes": [...], "edges": [...] }` where each node includes
/// `id`, `short_id`, `title`, `state`, `priority`, and `labels`, and each
/// edge is `{ "from": source_id, "to": dep_id }`.
///
/// This is the default `jit graph export --format json` shape, kept lean for
/// orchestration loops. For complete node records use [`export_json_full`]
/// (`--full`); the two share an identical `edges` list.
pub fn export_json(graph: &DependencyGraph<Issue>) -> String {
    let mut all_nodes = Vec::new();
    collect_all_nodes(graph, &mut all_nodes);

    let nodes: Vec<serde_json::Value> = all_nodes
        .iter()
        .map(|issue| {
            let short_id: String = issue.id.chars().take(8).collect();
            serde_json::json!({
                "id": issue.id,
                "short_id": short_id,
                "title": issue.title,
                "state": issue.state,
                "priority": issue.priority,
                "labels": issue.labels,
            })
        })
        .collect();

    render_graph_json(nodes, &all_nodes)
}

/// Export Issue dependency graph as JSON with COMPLETE node records (`--full`).
///
/// Emits `{ "nodes": [...], "edges": [...] }` with the same `edges` list as
/// [`export_json`], but each node is the full serialized [`Issue`] record —
/// every field the on-disk issue file carries: `id`, `title`, `description`,
/// `state`, `priority`, `assignee`, `dependencies`, `gates_required`,
/// `gates_status` (each gate's key mapped to its `status`/`updated_by`/
/// `updated_at`), `context`, `documents`, `labels`, `created_at`, `updated_at`,
/// and the lifecycle timestamps (`first_ready_at`, `claimed_at`, `done_at`) when
/// present. This gives bulk consumers complete records in one call without
/// globbing `.jit/issues/*.json`.
///
/// Two additive fields carry the DAG-authoritative hierarchy from `resolution`:
/// `resolved_parent` (the node's nearest dominating container id, or `null`) and
/// `cluster` (its strategic root container id, or `null`). These appear only in
/// the `--full` shape; the default summary shape is untouched.
///
/// # Example
/// ```
/// use jit::visualization;
/// use jit::graph::hierarchy::resolve_hierarchy;
/// use jit::type_hierarchy::HierarchyConfig;
/// use jit::{graph::DependencyGraph, Issue};
///
/// let mut epic = Issue::new("Epic".to_string(), String::new());
/// epic.labels = vec!["type:epic".to_string()];
/// let mut task = Issue::new("Task".to_string(), String::new());
/// task.labels = vec!["type:task".to_string()];
/// epic.dependencies.push(task.id.clone());
///
/// let issues = vec![&epic, &task];
/// let graph = DependencyGraph::new(&issues);
/// let resolution = resolve_hierarchy(&issues, &HierarchyConfig::default());
/// let json = visualization::export_json_full(&graph, &resolution);
/// assert!(json.contains("\"description\""));
/// assert!(json.contains("\"resolved_parent\""));
/// assert!(json.contains("\"cluster\""));
/// ```
pub fn export_json_full(
    graph: &DependencyGraph<Issue>,
    resolution: &crate::graph::hierarchy::HierarchyResolution,
) -> String {
    let mut all_nodes = Vec::new();
    collect_all_nodes(graph, &mut all_nodes);

    let nodes: Vec<serde_json::Value> = all_nodes
        .iter()
        .map(|issue| {
            let mut value = serde_json::to_value(issue).unwrap_or(serde_json::Value::Null);
            if let Some(obj) = value.as_object_mut() {
                let facts = resolution.get(&issue.id);
                obj.insert(
                    "resolved_parent".to_string(),
                    facts
                        .and_then(|f| f.parent.clone())
                        .map(serde_json::Value::String)
                        .unwrap_or(serde_json::Value::Null),
                );
                obj.insert(
                    "cluster".to_string(),
                    facts
                        .and_then(|f| f.cluster.clone())
                        .map(serde_json::Value::String)
                        .unwrap_or(serde_json::Value::Null),
                );
            }
            value
        })
        .collect();

    render_graph_json(nodes, &all_nodes)
}

/// Assemble the `{ "nodes", "edges" }` document, pretty-printed.
///
/// The `edges` list is derived from the same node set both export shapes share,
/// so the two shapes differ ONLY in node contents (summary vs. full record).
fn render_graph_json(nodes: Vec<serde_json::Value>, all_nodes: &[&Issue]) -> String {
    let edges: Vec<serde_json::Value> = all_nodes
        .iter()
        .flat_map(|issue| {
            issue
                .dependencies
                .iter()
                .map(|dep_id| serde_json::json!({ "from": issue.id, "to": dep_id }))
        })
        .collect();

    serde_json::to_string_pretty(&serde_json::json!({
        "nodes": nodes,
        "edges": edges,
    }))
    .unwrap_or_else(|_| "{}".to_string())
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

    /// REQ-03 byte-compat guard: the DEFAULT (summary) JSON node exposes exactly
    /// the six documented keys and nothing else — no full-record fields leak in.
    #[test]
    fn test_export_json_summary_node_has_exactly_summary_keys() {
        let issue = Issue::new("Summary".to_string(), "Body text".to_string());
        let issues = vec![&issue];
        let graph = DependencyGraph::new(&issues);

        let doc: serde_json::Value = serde_json::from_str(&export_json(&graph)).unwrap();
        let node = &doc["nodes"][0];
        let keys: std::collections::BTreeSet<&str> = node
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();

        let expected: std::collections::BTreeSet<&str> =
            ["id", "short_id", "title", "state", "priority", "labels"]
                .into_iter()
                .collect();
        assert_eq!(keys, expected);
        // Full-record fields are absent from the summary shape.
        assert!(node.get("description").is_none());
        assert!(node.get("gates_status").is_none());
    }

    /// `--full` node is the complete issue record: it carries the fields the
    /// summary shape omits, including the lifecycle timestamps when present.
    #[test]
    fn test_export_json_full_node_is_complete_record() {
        use crate::domain::{GateState, GateStatus, State};
        let mut issue = Issue::new("Full".to_string(), "Body text".to_string());
        issue.state = State::Done;
        issue.gates_required.push("tests".to_string());
        issue.gates_status.insert(
            "tests".to_string(),
            GateState {
                status: GateStatus::Passed,
                updated_by: None,
                updated_at: chrono::Utc::now(),
            },
        );
        issue.mark_first_ready(chrono::Utc::now());
        issue.mark_done(chrono::Utc::now());

        let issues = vec![&issue];
        let graph = DependencyGraph::new(&issues);
        let resolution = crate::graph::hierarchy::resolve_hierarchy(
            &issues,
            &crate::type_hierarchy::HierarchyConfig::default(),
        );
        let doc: serde_json::Value =
            serde_json::from_str(&export_json_full(&graph, &resolution)).unwrap();
        let node = &doc["nodes"][0];

        assert_eq!(node["description"], "Body text");
        assert_eq!(node["gates_required"][0], "tests");
        assert_eq!(node["gates_status"]["tests"]["status"], "passed");
        assert!(node.get("created_at").is_some());
        assert!(node.get("updated_at").is_some());
        assert!(node.get("first_ready_at").is_some());
        assert!(node.get("done_at").is_some());
        // Absent lifecycle fields stay omitted (skip_serializing_if).
        assert!(node.get("claimed_at").is_none());
    }

    /// Both shapes emit an identical `edges` list — `--full` only changes node
    /// contents.
    #[test]
    fn test_export_json_shapes_share_edges() {
        let dep = Issue::new("Dep".to_string(), String::new());
        let mut issue = Issue::new("Root".to_string(), String::new());
        issue.dependencies.push(dep.id.clone());

        let issues = vec![&dep, &issue];
        let graph = DependencyGraph::new(&issues);
        let resolution = crate::graph::hierarchy::resolve_hierarchy(
            &issues,
            &crate::type_hierarchy::HierarchyConfig::default(),
        );

        let summary: serde_json::Value = serde_json::from_str(&export_json(&graph)).unwrap();
        let full: serde_json::Value =
            serde_json::from_str(&export_json_full(&graph, &resolution)).unwrap();
        assert_eq!(summary["edges"], full["edges"]);
        assert_eq!(summary["edges"][0]["from"], issue.id);
        assert_eq!(summary["edges"][0]["to"], dep.id);
    }

    /// The `--full` node carries the additive DAG-resolved hierarchy fields
    /// (`resolved_parent`, `cluster`); the summary node never does.
    #[test]
    fn test_export_json_full_carries_resolved_hierarchy_fields() {
        let mut epic = Issue::new("Epic".to_string(), String::new());
        epic.labels = vec!["type:epic".to_string()];
        let mut task = Issue::new("Task".to_string(), String::new());
        task.labels = vec!["type:task".to_string()];
        epic.dependencies.push(task.id.clone());

        let issues = vec![&epic, &task];
        let graph = DependencyGraph::new(&issues);
        let resolution = crate::graph::hierarchy::resolve_hierarchy(
            &issues,
            &crate::type_hierarchy::HierarchyConfig::default(),
        );

        let full: serde_json::Value =
            serde_json::from_str(&export_json_full(&graph, &resolution)).unwrap();
        let task_node = full["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == serde_json::Value::String(task.id.clone()))
            .unwrap();
        assert_eq!(task_node["resolved_parent"], epic.id);
        // The epic is a root container, so the whole subtree clusters to it.
        assert_eq!(task_node["cluster"], epic.id);

        let epic_node = full["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == serde_json::Value::String(epic.id.clone()))
            .unwrap();
        assert_eq!(epic_node["resolved_parent"], serde_json::Value::Null);

        // The summary shape stays free of the hierarchy fields.
        let summary: serde_json::Value = serde_json::from_str(&export_json(&graph)).unwrap();
        assert!(summary["nodes"][0].get("resolved_parent").is_none());
        assert!(summary["nodes"][0].get("cluster").is_none());
    }
}
