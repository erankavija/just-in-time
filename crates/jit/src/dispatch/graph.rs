//! `jit graph` handlers.

use super::{count_message, resolve_issue, CommandContext, HandlerResult, HandlerStore, Outcome};
use crate::cli::GraphCommands;
use crate::commands::graph::compute_dependency_summary;
use crate::commands::GraphExportFormat;
use crate::commands::{
    DependencyTreeScope, GraphDepsTreeResponse, GraphExportOutput, GraphExportText,
    GraphExportWritten, GraphRootsResponse, GraphTreeResponse,
};
use crate::domain::MinimalIssue;
use crate::errors::InvalidArgumentError;
use crate::output::{GraphDownstreamResponse, IssueScope};
use crate::storage::IssueStore;
use crate::visualization::{GraphExportFullResponse, GraphExportSummaryResponse};
use std::path::Path;

/// The arguments of the top-level `jit rdeps <id>` alias, which runs the
/// `graph rdeps` leaf.
pub struct RdepsArgs {
    pub id: String,
    pub depth: u32,
}

fn deps<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
    depth: u32,
) -> HandlerResult<GraphDepsTreeResponse> {
    let id = resolve_issue(ctx.executor, &id)?;
    let tree = ctx.executor.graph().build_dependency_tree(&id, depth)?;
    let summary = compute_dependency_summary(&tree);
    let message = count_message(summary.total, "", " dependencies");
    Ok(Outcome::success_with(
        GraphDepsTreeResponse::with_extras(
            tree,
            DependencyTreeScope {
                issue_id: id,
                depth,
                summary,
            },
        ),
        message,
    ))
}

fn rdeps<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
    depth: u32,
) -> HandlerResult<GraphDownstreamResponse> {
    let id = resolve_issue(ctx.executor, &id)?;
    let issues = ctx.executor.graph().show_rdeps_with_depth(&id, depth)?;
    let message = count_message(issues.len(), "", " dependents");
    let dependents: Vec<MinimalIssue> = issues.iter().map(MinimalIssue::from).collect();
    Ok(Outcome::success_with(
        GraphDownstreamResponse::with_extras(dependents, IssueScope { issue_id: id }),
        message,
    ))
}

fn roots<S: IssueStore>(ctx: &CommandContext<'_, S>) -> HandlerResult<GraphRootsResponse> {
    let issues = ctx.executor.graph().show_roots()?;
    let message = count_message(issues.len(), "", " root issues");
    let roots: Vec<MinimalIssue> = issues.iter().map(MinimalIssue::from).collect();
    Ok(Outcome::success_with(
        GraphRootsResponse::new(roots),
        message,
    ))
}

fn tree<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    root: Option<String>,
) -> HandlerResult<GraphTreeResponse> {
    let root = root
        .map(|root| resolve_issue(ctx.executor, &root))
        .transpose()?;
    let response = ctx.executor.graph().resolve_hierarchy_tree(root.as_ref())?;
    let message = count_message(response.count(), "", " nodes");
    Ok(Outcome::success_with(response, message))
}

fn export<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    format: Option<GraphExportFormat>,
    full: bool,
    scope: Option<String>,
    output: Option<String>,
) -> HandlerResult<GraphExportOutput> {
    let json = ctx.mode.json;
    // `--json` is sugar for `--format json`; combining it with an explicit
    // conflicting `--format` (dot/mermaid/batch) is a usage error (exit 2).
    if json && matches!(format, Some(f) if f != GraphExportFormat::Json) {
        return Err(InvalidArgumentError::new(
            "--json conflicts with --format dot/mermaid/batch; use one or the other",
        )
        .into());
    }
    let format = if json {
        GraphExportFormat::Json
    } else {
        format.unwrap_or(GraphExportFormat::Dot)
    };
    // `--full` applies only to `--format json` (or `--json`).
    if full && format != GraphExportFormat::Json {
        return Err(InvalidArgumentError::new("--full is only valid with --format json").into());
    }
    let scope = scope
        .map(|scope| resolve_issue(ctx.executor, &scope))
        .transpose()?;

    // `batch` runs a distinct projection; boundary edges are reported as warnings
    // so no crossing edge is dropped silently (REQ-06).
    let (document, warnings) = if format == GraphExportFormat::Batch {
        let export = ctx.executor.graph().export_graph_batch(scope.as_ref())?;
        // The old arm printed this block to stderr; as one warning it keeps the
        // edges together and is reported under the `Warning:` prefix.
        let warnings: Vec<String> = (!export.boundary_edges.is_empty())
            .then(|| {
                export.boundary_edges.iter().fold(
                    format!(
                        "Excluded {} edge(s) crossing the scope boundary:",
                        export.boundary_edges.len()
                    ),
                    |block, edge| format!("{block}\n  {} -> {}", edge.from, edge.to),
                )
            })
            .into_iter()
            .collect();
        let text = GraphExportText(serde_json::to_string_pretty(&export.defs)?);
        (GraphExportOutput::Text(text), warnings)
    } else {
        (
            ctx.executor
                .graph()
                .export_graph(format, full, scope.as_ref())?,
            Vec::new(),
        )
    };

    let response = match (output, document.document_text()) {
        (Some(path), Some(text)) => {
            ctx.executor.graph().publish_graph_export(
                &std::env::current_dir()?,
                Path::new(&path),
                text.as_bytes(),
            )?;
            GraphExportOutput::Written(GraphExportWritten { path })
        }
        _ => document,
    };
    Ok(Outcome::from(response).with_warnings(warnings))
}

command_group! {
    command: GraphCommands,
    storage: [HandlerStore],
    leaves: {
        "graph_deps" => GraphCommands::Deps { id, depth } => deps(id, depth) -> GraphDepsTreeResponse,
        "graph_rdeps" => GraphCommands::Rdeps { id, depth } => rdeps(id, depth) -> GraphDownstreamResponse,
        "graph_roots" => GraphCommands::Roots => roots() -> GraphRootsResponse,
        "graph_tree" => GraphCommands::Tree { root } => tree(root) -> GraphTreeResponse,
        "graph_export" => GraphCommands::Export { format, full, scope, output } => export(format, full, scope, output)
            -> GraphExportOutput
            as union [GraphExportSummaryResponse, GraphExportFullResponse, GraphExportWritten]
            "Default shape: lean summary nodes (id, short_id, title, state, priority, labels) with no gate fields. With --full: complete stored issue records, whose gate list is carried under gates_required / gates_status, plus the resolved-hierarchy fields (parent, children, cluster, rank). With --output: the path the export was written to.",
    }
}

/// The top-level `rdeps` alias, registered under its own schema path.
pub mod rdeps {
    use super::*;

    command_group! {
        command: RdepsArgs,
        storage: [IssueStore],
        leaves: {
            "rdeps" => args => rdeps(args.id, args.depth) -> GraphDownstreamResponse,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::dispatch::registered_output_schema;
    use crate::schema::CommandSchema;

    #[test]
    fn test_output_schema_graph_rdeps_names_downstream_envelope() {
        let registered = registered_output_schema("graph_rdeps").unwrap();

        assert_eq!(registered.success, "GraphDownstreamResponse");
    }

    #[test]
    fn test_output_schema_top_level_rdeps_names_downstream_envelope() {
        let registered = registered_output_schema("rdeps").unwrap();

        assert_eq!(registered.success, "GraphDownstreamResponse");
    }

    #[test]
    fn test_get_output_schema_for_command_top_level_rdeps_matches_graph_rdeps() {
        let top_level = CommandSchema::get_output_schema_for_command("rdeps").unwrap();
        let graph = CommandSchema::get_output_schema_for_command("graph_rdeps").unwrap();

        assert_eq!(top_level.success, graph.success);
        assert_eq!(top_level.success_schema, graph.success_schema);
    }

    #[test]
    fn test_output_schema_graph_export_is_union_of_documents_and_written_report() {
        let registered = registered_output_schema("graph_export").unwrap();
        let members = registered.success_schema.unwrap()["oneOf"]
            .as_array()
            .unwrap()
            .len();

        assert_eq!(members, 3);
        assert_eq!(registered.success, "GraphExportSummaryResponse");
    }
}
