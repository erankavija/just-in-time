//! Human rendering and export responses of the `jit graph` commands.

use super::{
    plain_issue_rows, render_issue_list, GraphDepsTreeResponse, GraphDownstreamResponse,
    GraphRootsResponse, GraphTreeResponse, HumanRender, OutputMode,
};
use crate::commands::{DependencyTreeNode, GraphExportOutput, GraphExportWritten};
use crate::domain::identity::IssueId;
use crate::domain::State;
use std::fmt::Write;

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
        render_issue_list(
            &format!("Reverse dependencies of {}:", self.extras.issue_id),
            plain_issue_rows(&self.items),
            &[],
            mode.quiet,
        )
    }
}

impl HumanRender for GraphRootsResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        render_issue_list(
            "Root issues (no dependencies):",
            plain_issue_rows(&self.items),
            &[],
            mode.quiet,
        )
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
