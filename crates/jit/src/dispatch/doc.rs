//! `jit doc` handlers.

use super::{CommandContext, Outcome};
use crate::cli::{AssetCommands, DocCommands};
use crate::commands::{
    AssetListResult, DocumentAddResult, DocumentContentResult, DocumentDiffResult, DocumentHistory,
    DocumentListResult, DocumentRemoveResult, LinkCheckResult,
};
use crate::document::DocumentScope;
use crate::domain::identity::{IssueId, IssueIdPrefix};
use crate::output::{
    ArtifactConformanceResponse, ArtifactDirectoryResponse, ErrorCode, OutputContext,
};
use crate::storage::{IssueStore, RepositoryStateStore};

fn resolve_issue<S: IssueStore>(ctx: &CommandContext<'_, S>, id: &str) -> anyhow::Result<IssueId> {
    ctx.executor
        .storage()
        .resolve_issue_id(&id.parse::<IssueIdPrefix>()?)
}

fn success<R>(response: R, message: impl Into<String>) -> Outcome<R> {
    Outcome::Success {
        response,
        message: Some(message.into()),
    }
}

fn print_warnings<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    warnings: &[String],
) -> anyhow::Result<()> {
    let output = OutputContext::new(ctx.mode.quiet, ctx.mode.json);
    warnings
        .iter()
        .try_for_each(|warning| Ok(output.print_warning(warning)?))
}

#[allow(clippy::too_many_arguments)]
fn add<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    path: &str,
    commit: Option<&str>,
    label: Option<&str>,
    doc_type: Option<&str>,
    skip_scan: bool,
) -> anyhow::Result<Outcome<DocumentAddResult>> {
    let (result, warnings) = ctx.executor.documents().add_document_reference(
        &resolve_issue(ctx, id)?,
        path,
        commit,
        label,
        doc_type,
        skip_scan,
    )?;
    print_warnings(ctx, &warnings)?;
    let verb = if result.updated { "Updated" } else { "Added" };
    let message = format!("{verb} document reference on issue {}", result.issue_id);
    Ok(success(result, message))
}

fn list<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
) -> anyhow::Result<Outcome<DocumentListResult>> {
    let result = ctx
        .executor
        .documents()
        .list_document_references(&resolve_issue(ctx, id)?)?;
    let message = format!("{} document(s) attached", result.count());
    Ok(success(result, message))
}

fn remove<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    path: &str,
) -> anyhow::Result<Outcome<DocumentRemoveResult>> {
    let (result, warnings) = ctx
        .executor
        .documents()
        .remove_document_reference(&resolve_issue(ctx, id)?, path)?;
    print_warnings(ctx, &warnings)?;
    let message = format!(
        "Removed document reference {} from issue {}",
        result.path, result.issue_id
    );
    Ok(success(result, message))
}

fn show<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    path: &str,
    at: Option<&str>,
) -> anyhow::Result<Outcome<DocumentContentResult>> {
    let result =
        ctx.executor
            .documents()
            .show_document_content(&resolve_issue(ctx, id)?, path, at)?;
    Ok(success(result, "Document content retrieved"))
}

fn dir<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    area: &str,
) -> anyhow::Result<Outcome<ArtifactDirectoryResponse>> {
    let response = ctx
        .executor
        .documents()
        .resolve_issue_artifact_directory(&resolve_issue(ctx, id)?, area)?;
    Ok(Outcome::Success {
        response,
        message: None,
    })
}

fn conformance<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
) -> anyhow::Result<Outcome<ArtifactConformanceResponse>> {
    Ok(Outcome::Success {
        response: ctx.executor.documents().report_artifact_conformance()?,
        message: None,
    })
}

fn history<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    path: &str,
) -> anyhow::Result<Outcome<DocumentHistory>> {
    let result = ctx
        .executor
        .documents()
        .document_history(&resolve_issue(ctx, id)?, path)?;
    let message = format!("{} commits in history", result.commits.len());
    Ok(success(result, message))
}

fn diff<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    path: &str,
    from: &str,
    to: Option<&str>,
) -> anyhow::Result<Outcome<DocumentDiffResult>> {
    let result =
        ctx.executor
            .documents()
            .document_diff(&resolve_issue(ctx, id)?, path, from, to)?;
    Ok(success(result, "Document diff retrieved"))
}

fn assets_list<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    path: &str,
    rescan: bool,
) -> anyhow::Result<Outcome<AssetListResult>> {
    let result =
        ctx.executor
            .documents()
            .list_document_assets(&resolve_issue(ctx, id)?, path, rescan)?;
    print_warnings(ctx, &result.extras.warnings)?;
    let message = format!("{} assets found", result.extras.summary.total);
    Ok(success(result, message))
}

/// Broken links are a findings outcome; warnings alone succeed.
fn check_links<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    scope: &str,
) -> anyhow::Result<Outcome<LinkCheckResult>> {
    let scope = DocumentScope::parse(scope, |id| resolve_issue(ctx, id))?;
    let result = ctx.executor.documents().check_document_links(&scope)?;
    let summary = &result.summary;
    let message = if summary.errors == 0 && summary.warnings == 0 {
        "All links valid".to_owned()
    } else {
        format!(
            "{} error(s), {} warning(s)",
            summary.errors, summary.warnings
        )
    };
    Ok(if result.valid {
        success(result, message)
    } else {
        Outcome::Findings {
            code: ErrorCode::ValidationFailed,
            message,
            report: Box::new(result),
        }
    })
}

command_group! {
    command: DocCommands,
    storage: [IssueStore + RepositoryStateStore],
    leaves: {
        "doc_add" => DocCommands::Add { id, path, commit, label, doc_type, skip_scan }
            => add(&id, &path, commit.as_deref(), label.as_deref(), doc_type.as_deref(), skip_scan)
            -> DocumentAddResult,
        "doc_list" => DocCommands::List { id } => list(&id) -> DocumentListResult,
        "doc_remove" => DocCommands::Remove { id, path } => remove(&id, &path) -> DocumentRemoveResult,
        "doc_show" => DocCommands::Show { id, path, at }
            => show(&id, &path, at.as_deref()) -> DocumentContentResult,
        "doc_dir" => DocCommands::Dir { id, area } => dir(&id, &area) -> ArtifactDirectoryResponse,
        "doc_conformance" => DocCommands::Conformance => conformance() -> ArtifactConformanceResponse,
        "doc_history" => DocCommands::History { id, path } => history(&id, &path) -> DocumentHistory,
        "doc_diff" => DocCommands::Diff { id, path, from, to }
            => diff(&id, &path, &from, to.as_deref()) -> DocumentDiffResult,
        "doc_assets_list" => DocCommands::Assets { command: AssetCommands::List { id, path, rescan } }
            => assets_list(&id, &path, rescan) -> AssetListResult,
        "doc_check-links" => DocCommands::CheckLinks { scope } => check_links(&scope) -> LinkCheckResult,
    }
}
