//! `jit doc` handlers.

use super::{count_message, resolve_issue, CommandContext, HandlerResult, HandlerStore, Outcome};
use crate::cli::{AssetCommands, DocCommands};
use crate::commands::{ArtifactConformanceResponse, ArtifactDirectoryResponse};
use crate::commands::{
    AssetListResult, DocumentAddResult, DocumentContentResult, DocumentDiffResult, DocumentHistory,
    DocumentListResult, DocumentRemoveResult, LinkCheckResult,
};
use crate::document::DocumentScope;
use crate::domain::identity::IssueId;
use crate::output::ErrorCode;
use crate::storage::IssueStore;

fn issue<S: IssueStore>(ctx: &CommandContext<'_, S>, id: &str) -> anyhow::Result<IssueId> {
    resolve_issue(ctx.executor.storage(), id)
}

#[allow(clippy::too_many_arguments)]
fn add<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    path: &str,
    commit: Option<&str>,
    label: Option<&str>,
    doc_type: Option<&str>,
    skip_scan: bool,
) -> HandlerResult<DocumentAddResult> {
    let (result, warnings) = ctx.executor.documents().add_document_reference(
        &issue(ctx, id)?,
        path,
        commit,
        label,
        doc_type,
        skip_scan,
    )?;
    let verb = if result.updated { "Updated" } else { "Added" };
    let message = format!("{verb} document reference on issue {}", result.issue_id);
    Ok(Outcome::success_with(result, message).with_warnings(warnings))
}

fn list<S: IssueStore>(ctx: &CommandContext<'_, S>, id: &str) -> HandlerResult<DocumentListResult> {
    let result = ctx
        .executor
        .documents()
        .list_document_references(&issue(ctx, id)?)?;
    let message = count_message(result.count(), "", " document(s) attached");
    Ok(Outcome::success_with(result, message))
}

fn remove<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    path: &str,
) -> HandlerResult<DocumentRemoveResult> {
    let (result, warnings) = ctx
        .executor
        .documents()
        .remove_document_reference(&issue(ctx, id)?, path)?;
    let message = format!(
        "Removed document reference {} from issue {}",
        result.path, result.issue_id
    );
    Ok(Outcome::success_with(result, message).with_warnings(warnings))
}

fn show<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    path: &str,
    at: Option<&str>,
) -> HandlerResult<DocumentContentResult> {
    let result = ctx
        .executor
        .documents()
        .show_document_content(&issue(ctx, id)?, path, at)?;
    Ok(Outcome::success_with(result, "Document content retrieved"))
}

fn dir<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    area: &str,
) -> HandlerResult<ArtifactDirectoryResponse> {
    let directory = ctx
        .executor
        .documents()
        .resolve_issue_artifact_directory(&issue(ctx, id)?, area)?;
    Ok(directory.into())
}

fn conformance<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
) -> HandlerResult<ArtifactConformanceResponse> {
    Ok(ctx
        .executor
        .documents()
        .report_artifact_conformance()?
        .into())
}

fn history<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    path: &str,
) -> HandlerResult<DocumentHistory> {
    let result = ctx
        .executor
        .documents()
        .document_history(&issue(ctx, id)?, path)?;
    let message = count_message(result.commits.len(), "", " commits in history");
    Ok(Outcome::success_with(result, message))
}

fn diff<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    path: &str,
    from: &str,
    to: Option<&str>,
) -> HandlerResult<DocumentDiffResult> {
    let result = ctx
        .executor
        .documents()
        .document_diff(&issue(ctx, id)?, path, from, to)?;
    Ok(Outcome::success_with(result, "Document diff retrieved"))
}

fn assets_list<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    path: &str,
    rescan: bool,
) -> HandlerResult<AssetListResult> {
    let (result, warnings) =
        ctx.executor
            .documents()
            .list_document_assets(&issue(ctx, id)?, path, rescan)?;
    let message = count_message(result.extras.summary.total, "", " assets found");
    Ok(Outcome::success_with(result, message).with_warnings(warnings))
}

/// Broken links are a findings outcome; warnings alone succeed.
fn check_links<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    scope: &str,
) -> HandlerResult<LinkCheckResult> {
    let scope = DocumentScope::parse(scope, |id| issue(ctx, id))?;
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
        Outcome::success_with(result, message)
    } else {
        Outcome::findings(ErrorCode::ValidationFailed, message, result)
    })
}

command_group! {
    command: DocCommands,
    storage: [HandlerStore],
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
        "doc_check-links" => DocCommands::CheckLinks { scope } => check_links(&scope)
            findings ValidationFailed "One or more documents have broken links. Documents with only risky-link warnings exit `0`: warnings are reported, never fatal." -> LinkCheckResult,
    }
}
