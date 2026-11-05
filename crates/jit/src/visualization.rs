//! Visualization and export functionality for Issue dependency graphs.
//!
//! This module provides functions to export Issue dependency graphs to various formats
//! like DOT (Graphviz) and Mermaid. These functions are Issue-specific and access
//! Issue fields like title and state for rendering.

use crate::domain::identity::IssueId;
use crate::domain::{Issue, Priority, State};
use crate::graph::hierarchy::NodeHierarchy;
use crate::graph::DependencyGraph;
use schemars::JsonSchema;
use serde::Serialize;

/// One edge in a `jit graph export --format json` document: a dependency from
/// `from` (the dependent issue) to `to` (the prerequisite id). Shared verbatim
/// by both the summary and `--full` shapes.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphExportEdge {
    pub from: IssueId,
    pub to: IssueId,
}

/// A node in the default (summary) `jit graph export --format json` shape.
///
/// Lean projection for orchestration loops: it deliberately omits the gate list
/// (and every other heavy field). A consumer that needs an issue's gates asks
/// for `--full`, whose node is the complete stored record ([`GraphExportFullNode`]).
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphExportSummaryNode {
    pub id: IssueId,
    pub short_id: String,
    pub title: String,
    pub state: State,
    pub priority: Priority,
    pub labels: Vec<String>,
}

/// A node in the `jit graph export --format json --full` shape: the complete
/// stored [`Issue`] record — so the gate list appears under the storage names
/// `gates_required` / `gates_status`, exactly as `.jit/issues/<id>.json` carries
/// it — with the resolved-hierarchy facts (`parent`, `children`, `cluster`,
/// `rank`) flattened alongside.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphExportFullNode {
    #[serde(flatten)]
    pub issue: Issue,
    #[serde(flatten)]
    pub hierarchy: NodeHierarchy,
}

/// The `{ nodes, edges }` document emitted by the default `jit graph export
/// --format json` shape.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphExportSummaryResponse {
    pub nodes: Vec<GraphExportSummaryNode>,
    pub edges: Vec<GraphExportEdge>,
}

/// The `{ nodes, edges }` document emitted by `jit graph export --format json
/// --full`. Declared alongside [`GraphExportSummaryResponse`] in `jit --schema`
/// so a consumer can tell the gate-bearing full record from the gate-free
/// summary without reading the source.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphExportFullResponse {
    pub nodes: Vec<GraphExportFullNode>,
    pub edges: Vec<GraphExportEdge>,
}

/// Build the shared edge list from a node set: one [`GraphExportEdge`] per
/// stored dependency, so both export shapes derive edges identically.
fn build_export_edges(all_nodes: &[&Issue]) -> Vec<GraphExportEdge> {
    let node_ids = exported_node_ids(all_nodes);
    all_nodes
        .iter()
        .flat_map(|issue| {
            issue
                .dependencies
                .iter()
                .filter(|dep_id| node_ids.contains(dep_id))
                .map(|dep_id| GraphExportEdge {
                    from: issue.id.clone(),
                    to: dep_id.clone(),
                })
        })
        .collect()
}

/// The id set of the nodes actually in the export.
///
/// Every exporter emits only edges whose BOTH endpoints are exported nodes
/// (jit:3e12ffbd REQ-01): a scoped export must not leak references to
/// out-of-scope issues as dangling edges. Unscoped exports are unaffected —
/// repository validation keeps every dependency resolvable, so the filter
/// passes every edge through.
fn exported_node_ids<'a>(all_nodes: &'a [&Issue]) -> std::collections::HashSet<&'a IssueId> {
    all_nodes.iter().map(|issue| &issue.id).collect()
}

/// Export Issue dependency graph as DOT format for Graphviz
///
/// Generates a directed graph visualization with:
/// - Node labels showing issue ID and title
/// - Color-coded nodes based on issue state
/// - Dependency edges between issues
pub fn export_dot(graph: &DependencyGraph<Issue>) -> String {
    let mut output = String::from("digraph issues {\n");
    output.push_str("  rankdir=LR;\n");
    output.push_str("  node [shape=box, style=rounded];\n\n");

    let all_nodes = graph.nodes_by_id();

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

    // Add edges between exported nodes only (no dangling references under scope).
    let node_ids = exported_node_ids(&all_nodes);
    for issue in &all_nodes {
        for dep in &issue.dependencies {
            if node_ids.contains(dep) {
                output.push_str(&format!("  \"{}\" -> \"{}\";\n", issue.id, dep));
            }
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
pub fn export_mermaid(graph: &DependencyGraph<Issue>) -> String {
    let mut output = String::from("graph LR\n");

    let all_nodes = graph.nodes_by_id();

    // Add nodes with state styling
    for issue in &all_nodes {
        let label = format!("{}:<br/>{}", issue.id, issue.title);
        output.push_str(&format!(
            "  {}[\"{}\"]:::{}\n",
            issue.id,
            label,
            issue.state.as_str()
        ));
    }

    output.push('\n');

    // Add edges between exported nodes only (no dangling references under scope).
    let node_ids = exported_node_ids(&all_nodes);
    for issue in &all_nodes {
        for dep in &issue.dependencies {
            if node_ids.contains(dep) {
                output.push_str(&format!("  {} --> {}\n", issue.id, dep));
            }
        }
    }

    // One class per lifecycle state, named by its canonical token
    output.push('\n');
    output.push_str(
        &State::ALL
            .into_iter()
            .map(|state| {
                format!(
                    "  classDef {} fill:{},stroke:#333\n",
                    state.as_str(),
                    mermaid_fill(state)
                )
            })
            .collect::<String>(),
    );

    output
}

/// Fill colour for a state's mermaid class; exhaustive so a new state cannot go unstyled.
fn mermaid_fill(state: State) -> &'static str {
    match state {
        State::Backlog => "#e0e0e0",
        State::Ready => "#add8e6",
        State::InProgress => "#ffff99",
        State::Gated => "#ffd8a8",
        State::Done => "#90ee90",
        State::Rejected => "#ffc0cb",
        State::Archived => "#808080",
    }
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
    pretty_document(&summary_document(graph))
}

/// The summary document of [`export_json`], as a typed response.
pub fn summary_document(graph: &DependencyGraph<Issue>) -> GraphExportSummaryResponse {
    let all_nodes = graph.nodes_by_id();

    let nodes: Vec<GraphExportSummaryNode> = all_nodes
        .iter()
        .map(|issue| GraphExportSummaryNode {
            id: issue.id.clone(),
            short_id: issue.id.short_id().to_string(),
            title: issue.title.clone(),
            state: issue.state,
            priority: issue.priority,
            labels: issue.labels.clone(),
        })
        .collect();

    GraphExportSummaryResponse {
        nodes,
        edges: build_export_edges(&all_nodes),
    }
}

/// The pretty JSON text of a document; serialization of these shapes cannot fail.
pub fn pretty_document<T: Serialize>(document: &T) -> String {
    serde_json::to_string_pretty(document).unwrap_or_else(|_| "{}".to_string())
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
/// The DAG-authoritative hierarchy from `resolution` is flattened onto each node
/// as `parent`, `children`, `cluster`, and `rank` — the same four fields, from
/// the same [`NodeHierarchy`]
/// serialization, that `jit graph tree` emits. These appear only in the `--full`
/// shape; the default summary shape is untouched.
///
/// Resolution is repository-wide, so a node's `parent`, `cluster`, or `children`
/// name ids by their canonical placement in the whole graph. A consumer that
/// filters the emitted node set can therefore hold references to nodes it did
/// not keep.
pub fn export_json_full(
    graph: &DependencyGraph<Issue>,
    resolution: &crate::graph::hierarchy::HierarchyResolution<'_, Issue>,
) -> String {
    pretty_document(&full_document(graph, resolution))
}

/// The complete-record document of [`export_json_full`], as a typed response.
pub fn full_document(
    graph: &DependencyGraph<Issue>,
    resolution: &crate::graph::hierarchy::HierarchyResolution<'_, Issue>,
) -> GraphExportFullResponse {
    let all_nodes = graph.nodes_by_id();

    let nodes: Vec<GraphExportFullNode> = all_nodes
        .iter()
        .map(|issue| GraphExportFullNode {
            issue: (*issue).clone(),
            hierarchy: resolution.get(&issue.id).cloned().unwrap_or_default(),
        })
        .collect();

    GraphExportFullResponse {
        nodes,
        edges: build_export_edges(&all_nodes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed_issue(title: &str, level: u8) -> crate::domain::Issue {
        let mut issue = crate::test_utils::fixture_issue(title.into(), String::new());
        issue.labels = vec![format!(
            "type:{}",
            crate::test_taxonomy::test_taxonomy().type_at_level(level)
        )];
        issue
    }

    #[test]
    fn test_export_dot_format() {
        let issue1 = crate::test_utils::fixture_issue(
            "API Design".to_string(),
            "Design REST API".to_string(),
        );
        let mut issue2 = crate::test_utils::fixture_issue(
            "Backend".to_string(),
            "Implement backend".to_string(),
        );
        issue2.dependencies.push(issue1.id.clone());

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        let dot = export_dot(&graph);

        assert!(dot.contains("digraph issues"));
        assert!(dot.contains("rankdir=LR"));
        assert!(dot.contains(issue1.id.as_str()));
        assert!(dot.contains(issue2.id.as_str()));
        assert!(dot.contains("API Design"));
        assert!(dot.contains("Backend"));
        assert!(dot.contains(&format!("\"{}\" -> \"{}\"", issue2.id, issue1.id)));
    }

    #[test]
    fn test_export_mermaid_format() {
        let issue1 =
            crate::test_utils::fixture_issue("Setup".to_string(), "Initial setup".to_string());
        let mut issue2 =
            crate::test_utils::fixture_issue("Deploy".to_string(), "Deploy to prod".to_string());
        issue2.dependencies.push(issue1.id.clone());

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        let mermaid = export_mermaid(&graph);

        assert!(mermaid.contains("graph LR"));
        assert!(mermaid.contains(issue1.id.as_str()));
        assert!(mermaid.contains(issue2.id.as_str()));
        assert!(mermaid.contains("Setup"));
        assert!(mermaid.contains("Deploy"));
        assert!(mermaid.contains(&format!("{} --> {}", issue2.id, issue1.id)));
    }

    #[test]
    fn test_export_mermaid_declares_one_class_per_state_and_every_node_class() {
        let issues: Vec<Issue> = State::ALL
            .into_iter()
            .enumerate()
            .map(|(i, state)| {
                let mut issue = crate::test_utils::fixture_issue(
                    format!("Issue {i}"),
                    "State fixture".to_string(),
                );
                issue.state = state;
                issue
            })
            .collect();
        let refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&refs);

        let mermaid = export_mermaid(&graph);

        let mut declared: Vec<&str> = mermaid
            .lines()
            .filter_map(|line| line.trim().strip_prefix("classDef "))
            .filter_map(|rest| rest.split_whitespace().next())
            .collect();
        let mut expected: Vec<&str> = State::ALL.iter().map(|s| s.as_str()).collect();
        declared.sort_unstable();
        expected.sort_unstable();
        assert_eq!(declared, expected);

        let node_classes = mermaid
            .lines()
            .filter_map(|line| line.rsplit_once(":::").map(|(_, class)| class.trim()));
        for class in node_classes {
            assert!(
                declared.contains(&class),
                "node class {class} is not declared"
            );
        }
    }

    #[test]
    fn test_export_dot_with_different_states() {
        let mut issue1 =
            crate::test_utils::fixture_issue("Done Action".to_string(), "Completed".to_string());
        issue1.state = State::Done;

        let mut issue2 = crate::test_utils::fixture_issue(
            "In Progress".to_string(),
            "Working on it".to_string(),
        );
        issue2.state = State::InProgress;

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        let dot = export_dot(&graph);

        assert!(dot.contains("lightgreen")); // Done state
        assert!(dot.contains("yellow")); // in_progress state
    }

    #[test]
    fn test_export_handles_special_characters() {
        let issue = crate::test_utils::fixture_issue(
            "Title with \"quotes\"".to_string(),
            "Test".to_string(),
        );
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
        let issue =
            crate::test_utils::fixture_issue("Summary".to_string(), "Body text".to_string());
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
        let mut issue =
            crate::test_utils::fixture_issue("Full".to_string(), "Body text".to_string());
        issue.state = State::Done;
        issue.gates_required.push("tests".parse().unwrap());
        issue.gates_status.insert(
            "tests".parse().unwrap(),
            GateState {
                status: GateStatus::Passed,
                updated_by: None,
                updated_at: chrono::Utc::now(),
                run_id: None,
            },
        );
        issue.mark_first_ready(chrono::Utc::now());
        issue.mark_done(chrono::Utc::now());

        let issues = vec![&issue];
        let graph = DependencyGraph::new(&issues);
        let resolution = crate::graph::hierarchy::resolve_hierarchy(
            &issues,
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
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
        let dep = crate::test_utils::fixture_issue("Dep".to_string(), String::new());
        let mut issue = crate::test_utils::fixture_issue("Root".to_string(), String::new());
        issue.dependencies.push(dep.id.clone());

        let issues = vec![&dep, &issue];
        let graph = DependencyGraph::new(&issues);
        let resolution = crate::graph::hierarchy::resolve_hierarchy(
            &issues,
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
        );

        let summary: serde_json::Value = serde_json::from_str(&export_json(&graph)).unwrap();
        let full: serde_json::Value =
            serde_json::from_str(&export_json_full(&graph, &resolution)).unwrap();
        assert_eq!(summary["edges"], full["edges"]);
        assert_eq!(summary["edges"][0]["from"], issue.id.as_str());
        assert_eq!(summary["edges"][0]["to"], dep.id.as_str());
    }

    /// The `--full` node carries the four DAG-resolved hierarchy fields; the
    /// summary node never does.
    #[test]
    fn test_export_json_full_carries_resolved_hierarchy_fields() {
        let mut initiative = typed_issue("Initiative", 2);
        let action = typed_issue("Action", 4);
        initiative.dependencies.push(action.id.clone());

        let issues = vec![&initiative, &action];
        let graph = DependencyGraph::new(&issues);
        let resolution = crate::graph::hierarchy::resolve_hierarchy(
            &issues,
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
        );

        let full: serde_json::Value =
            serde_json::from_str(&export_json_full(&graph, &resolution)).unwrap();
        let node_by_id = |id: &IssueId| -> serde_json::Value {
            full["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["id"] == id.as_str())
                .unwrap()
                .clone()
        };

        let action_node = node_by_id(&action.id);
        assert_eq!(action_node["parent"], initiative.id.as_str());
        // The initiative is a root container, so the whole subtree clusters to it.
        assert_eq!(action_node["cluster"], initiative.id.as_str());
        assert_eq!(action_node["children"], serde_json::json!([]));
        assert_eq!(action_node["rank"], 0);

        let initiative_node = node_by_id(&initiative.id);
        assert_eq!(initiative_node["parent"], serde_json::Value::Null);
        assert_eq!(initiative_node["children"], serde_json::json!([action.id]));
        assert_eq!(initiative_node["rank"], 1);

        // The summary shape stays free of the hierarchy fields.
        let summary: serde_json::Value = serde_json::from_str(&export_json(&graph)).unwrap();
        for field in ["parent", "children", "cluster", "rank"] {
            assert!(summary["nodes"][0].get(field).is_none());
        }
    }

    /// Typed hierarchy facts retain the string wire format on both surfaces.
    #[test]
    fn test_export_json_full_hierarchy_serialization_matches_tree_view() {
        use crate::commands::HierarchyNodeView;

        #[derive(Serialize)]
        struct WireNode<T> {
            #[serde(flatten)]
            identity: T,
            #[serde(flatten)]
            hierarchy: NodeHierarchy<String>,
        }

        #[derive(Serialize)]
        struct TreeIdentity<'a> {
            id: &'a str,
            short_id: &'a str,
            title: &'a str,
            #[serde(rename = "type")]
            type_name: Option<&'a str>,
        }

        #[derive(Serialize)]
        struct WireExport<'a> {
            nodes: Vec<WireNode<&'a Issue>>,
            edges: Vec<GraphExportEdge>,
        }

        let mut initiative = typed_issue("Initiative", 2);
        let action = typed_issue("Action", 4);
        initiative.dependencies.push(action.id.clone());

        let issues = vec![&initiative, &action];
        let graph = DependencyGraph::new(&issues);
        let resolution = crate::graph::hierarchy::resolve_hierarchy(
            &issues,
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
        );
        let string_facts = |issue: &Issue| {
            let facts = resolution.get(&issue.id).unwrap();
            NodeHierarchy {
                parent: facts.parent.as_ref().map(ToString::to_string),
                children: facts.children.iter().map(ToString::to_string).collect(),
                cluster: facts.cluster.as_ref().map(ToString::to_string),
                rank: facts.rank,
            }
        };

        for issue in &issues {
            let type_name = crate::labels::type_label_value(&issue.labels);
            let tree_node = HierarchyNodeView {
                id: issue.id.clone(),
                short_id: issue.short_id(),
                title: issue.title.clone(),
                type_name: type_name.map(str::to_string),
                hierarchy: resolution.get(&issue.id).unwrap().clone(),
            };
            let expected = WireNode {
                identity: TreeIdentity {
                    id: issue.id.as_str(),
                    short_id: issue.id.short_id(),
                    title: &issue.title,
                    type_name,
                },
                hierarchy: string_facts(issue),
            };
            assert_eq!(
                serde_json::to_string(&tree_node).unwrap(),
                serde_json::to_string(&expected).unwrap(),
            );
        }

        let all_nodes = graph.nodes_by_id();
        let expected = WireExport {
            nodes: all_nodes
                .iter()
                .map(|issue| WireNode {
                    identity: *issue,
                    hierarchy: string_facts(issue),
                })
                .collect(),
            edges: build_export_edges(&all_nodes),
        };
        assert_eq!(
            export_json_full(&graph, &resolution),
            serde_json::to_string_pretty(&expected).unwrap(),
        );
    }

    fn exported_ids(json: &str) -> std::collections::BTreeSet<IssueId> {
        serde_json::from_str::<serde_json::Value>(json).unwrap()["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| serde_json::from_value::<IssueId>(n["id"].clone()).unwrap())
            .collect()
    }

    #[test]
    fn test_export_json_scope_members_depending_only_on_outside_issues_all_appear() {
        let outside = crate::test_utils::fixture_issue("Outside".into(), String::new());
        let mut task = crate::test_utils::fixture_issue("Task".into(), String::new());
        task.dependencies.push(outside.id.clone());
        let mut epic = crate::test_utils::fixture_issue("Epic".into(), String::new());
        epic.dependencies.push(task.id.clone());

        let scope = vec![&task, &epic];
        let graph = DependencyGraph::new(&scope);

        let expected = [&task, &epic].map(|i| i.id.clone()).into_iter().collect();
        assert_eq!(exported_ids(&export_json(&graph)), expected);
    }

    proptest::proptest! {
        /// Node `i` depends only on nodes `< i`; `scope` selects any subset.
        #[test]
        fn prop_export_lists_exactly_the_selected_nodes_in_a_stable_order(
            masks in proptest::collection::vec(0u32..128, 1..8),
            scope_bits in 0u32..256,
        ) {
            let mut issues: Vec<Issue> = masks
                .iter()
                .map(|_| crate::test_utils::fixture_issue("N".into(), String::new()))
                .collect();
            let ids: Vec<IssueId> = issues.iter().map(|i| i.id.clone()).collect();
            for (index, mask) in masks.iter().enumerate() {
                issues[index].dependencies = (0..index)
                    .filter(|earlier| mask & (1 << earlier) != 0)
                    .map(|earlier| ids[earlier].clone())
                    .collect();
            }
            let selected: Vec<&Issue> = issues
                .iter()
                .enumerate()
                .filter(|(index, _)| scope_bits & (1 << index) != 0)
                .map(|(_, issue)| issue)
                .collect();
            let expected: std::collections::BTreeSet<IssueId> =
                selected.iter().map(|i| i.id.clone()).collect();

            let graph = DependencyGraph::new(&selected);
            let json = export_json(&graph);
            proptest::prop_assert_eq!(exported_ids(&json), expected);

            let mut reversed = selected.clone();
            reversed.reverse();
            let rebuilt = DependencyGraph::new(&reversed);
            proptest::prop_assert_eq!(&json, &export_json(&rebuilt));
            proptest::prop_assert_eq!(export_dot(&graph), export_dot(&rebuilt));
            proptest::prop_assert_eq!(export_mermaid(&graph), export_mermaid(&rebuilt));
        }
    }
}
