//! Human rendering and export responses of the `jit graph` commands.

use super::{
    plain_issue_rows, render_issue_list, GraphDepsTreeResponse, GraphDownstreamResponse,
    GraphRootsResponse, GraphTreeResponse, HumanRender, OutputMode,
};
use crate::domain::identity::IssueId;
use crate::domain::{MinimalIssue, State};
use crate::output::DependencyTreeNode;
use crate::visualization::{pretty_document, GraphExportFullResponse, GraphExportSummaryResponse};
use schemars::JsonSchema;
use serde::Serialize;
use std::fmt::Write;

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

impl HumanRender for GraphExportOutput {
    fn render_human(&self, mode: OutputMode) -> String {
        match (self, self.document_text()) {
            (Self::Written(written), _) => written.render_human(mode),
            (_, Some(text)) => format!("{text}\n"),
            (_, None) => String::new(),
        }
    }
}

impl HumanRender for GraphExportWritten {
    fn render_human(&self, mode: OutputMode) -> String {
        if mode.quiet {
            String::new()
        } else {
            format!("Graph exported to: {}\n", self.path)
        }
    }
}

impl HumanRender for GraphDepsTreeResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        let scope = &self.extras;
        let depth = match scope.depth {
            0 => "all transitive".to_string(),
            1 => "immediate".to_string(),
            n => format!("depth {n}"),
        };
        let mut out = String::new();
        if !mode.quiet {
            let _ = writeln!(out, "Dependencies of {} ({depth}):", scope.issue_id);
        }
        if self.items.is_empty() {
            out.push_str("  (none)\n");
        } else {
            let done = scope.summary.by_state.get(&State::Done).unwrap_or(&0);
            if scope.summary.total > 0 {
                let _ = write!(
                    out,
                    "  Summary: {done}/{} complete\n\n",
                    scope.summary.total
                );
            }
            push_dependency_tree(&mut out, &self.items, "", true);
        }
        out
    }
}

/// The `graph deps` tree with tree symbols (├─, └─, │).
fn push_dependency_tree(
    out: &mut String,
    nodes: &[DependencyTreeNode],
    prefix: &str,
    is_root: bool,
) {
    let count = nodes.len();
    for (i, node) in nodes.iter().enumerate() {
        let is_last = i == count - 1;
        let (connector, child_prefix) = if is_root {
            ("  ", "  ")
        } else if is_last {
            ("└─ ", "   ")
        } else {
            ("├─ ", "│  ")
        };
        let shared_marker = if node.shared.unwrap_or(false) {
            " (shared)"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "{prefix}{connector}{} - {} [{}]{shared_marker}",
            node.short_id,
            node.title,
            node.state.as_str()
        );
        if !node.children.is_empty() {
            push_dependency_tree(
                out,
                &node.children,
                &format!("{prefix}{child_prefix}"),
                false,
            );
        }
    }
}

impl HumanRender for GraphDownstreamResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        issue_list_text(
            &format!("Reverse dependencies of {}:", self.extras.issue_id),
            &self.items,
            mode,
        )
    }
}

impl HumanRender for GraphRootsResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        issue_list_text("Root issues (no dependencies):", &self.items, mode)
    }
}

/// A list's human text: an empty quiet list prints nothing.
fn issue_list_text(header: &str, items: &[MinimalIssue], mode: OutputMode) -> String {
    let rendered = render_issue_list(header, plain_issue_rows(items), &[], mode.quiet);
    if rendered.is_empty() {
        String::new()
    } else {
        format!("{rendered}\n")
    }
}

impl HumanRender for GraphTreeResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        let mut out = String::new();
        if !mode.quiet {
            let scope = match &self.extras.root {
                Some(id) => format!("subtree of {}", id.short_id()),
                None => "repository".to_string(),
            };
            let _ = writeln!(out, "Resolved hierarchy ({scope}):");
        }
        if self.items.is_empty() {
            out.push_str("  (none)\n");
        }
        for node in &self.items {
            let parent = node
                .hierarchy
                .parent
                .as_ref()
                .map(IssueId::short_id)
                .unwrap_or("-");
            let _ = writeln!(
                out,
                "  {} | parent={parent} children={} rank={} | {}",
                node.short_id,
                node.hierarchy.children.len(),
                node.hierarchy.rank,
                node.title
            );
        }
        out
    }
}
