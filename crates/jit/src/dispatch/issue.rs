//! `jit issue` read handlers.

use super::{resolve_issue, CommandContext, HandlerResult, Outcome};
use crate::cli::{IssueCommands, IssueFilterArgs};
use crate::commands::ItemShowResult;
use crate::domain::MinimalIssue;
use crate::errors::InvalidArgumentError;
use crate::output::{
    emit_outcome, ContainerProgressResponse, ExitCode, IssueChildrenResponse,
    IssueListFullResponse, IssueListResponse, IssueListing, IssueProjection,
    IssueSearchFullResponse, IssueSearchQuery, IssueSearchResponse, IssueSearchResults,
    IssueShowListResponse, IssueShowOutput, IssueShowResponse, IssueShowSummaryResponse,
    IssueStatusListResponse, IssueStatusOutput, IssueStatusResponse,
};
use crate::schema::OutputSchema;
use crate::storage::IssueStore;
use anyhow::Context as _;

fn show_response<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    requested: &str,
) -> anyhow::Result<IssueShowResponse> {
    let issues = ctx.executor.issues();
    let id = resolve_issue(ctx.executor.storage(), requested)?;
    let issue = issues
        .show_issue(&id)
        .with_context(|| format!("Failed to load issue {id}"))?;
    issues.show_response(issue)
}

fn show<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    ids: Vec<String>,
    summary: bool,
    field: Option<String>,
    fields: Vec<String>,
) -> HandlerResult<IssueShowOutput> {
    let projecting = field.is_some() || !fields.is_empty();
    // Projection targets a single issue.
    if projecting && ids.len() > 1 {
        return Err(InvalidArgumentError::new(format!(
            "invalid arguments: --field/--fields require exactly one issue id (got {})",
            ids.len()
        ))
        .into());
    }
    let [requested] = ids.as_slice() else {
        let shown = ids
            .iter()
            .map(|id| show_response(ctx, id))
            .collect::<anyhow::Result<Vec<_>>>()?;
        return Ok(IssueShowOutput::Many(IssueShowListResponse::new(shown)).into());
    };
    // A `<issue>/<self-id>` argument addresses an item; issue ids never
    // contain '/'.
    if !projecting && !summary && requested.contains('/') {
        let item = ctx.executor.items().show_item(requested)?;
        return Ok(IssueShowOutput::Item(item).into());
    }
    let issues = ctx.executor.issues();
    let issue = issues.show_issue(&resolve_issue(ctx.executor.storage(), requested)?)?;
    if summary && ctx.mode.json && !projecting {
        let summary = IssueShowSummaryResponse::from(&issue);
        let message = format!(
            "Issue {}: {} [{}]",
            summary.short_id, summary.title, summary.state
        );
        return Ok(Outcome::success_with(
            IssueShowOutput::Summary(summary),
            message,
        ));
    }
    let shown = issues.show_response(issue)?;
    if projecting {
        let value = serde_json::to_value(&shown)?;
        let unknown = |name: &str| {
            InvalidArgumentError::new(format!(
                "invalid field: unknown field '{name}' for issue show"
            ))
        };
        let projection = match field {
            Some(name) => IssueProjection::field(&value, &name).ok_or_else(|| unknown(&name))?,
            None => IssueProjection::fields(&value, &fields).map_err(|e| unknown(&e.0))?,
        };
        return Ok(IssueShowOutput::Projection(projection).into());
    }
    let message = format!(
        "Issue {}: {} [{}]",
        shown.id.short_id(),
        shown.title,
        shown.state
    );
    Ok(Outcome::success_with(
        IssueShowOutput::Issue(Box::new(shown)),
        message,
    ))
}

/// `jit issue list`, `jit list`, `jit query all` and bare `jit query`.
pub(super) fn list<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    filters: IssueFilterArgs,
    full: bool,
) -> HandlerResult<IssueListing> {
    let filters = filters.parse()?;
    let issues = ctx.executor.queries().query_all(
        filters.state,
        filters.assignee,
        filters.priority,
        filters.labels,
    )?;
    let message = format!("Found {} issue(s)", issues.len());
    let listing = if full {
        IssueListing::Full(IssueListFullResponse::new(issues))
    } else {
        IssueListing::Summary(IssueListResponse::new(
            issues.iter().map(MinimalIssue::from).collect(),
        ))
    };
    Ok(Outcome::success_with(listing, message))
}

fn search<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    query: Option<String>,
    filters: IssueFilterArgs,
    full: bool,
) -> HandlerResult<IssueSearchResults> {
    let filters = filters.parse()?;
    // A query-less search needs a filter; otherwise it would dump every issue.
    let has_filter = filters.state.is_some()
        || filters.assignee.is_some()
        || filters.priority.is_some()
        || !filters.labels.is_empty();
    if query.is_none() && !has_filter {
        return Err(InvalidArgumentError::new(
            "invalid arguments: provide a search query or at least one filter (--label/--state/--assignee/--priority)",
        )
        .into());
    }
    let issues = ctx.executor.queries().search_issues_with_filters(
        query.as_deref().unwrap_or(""),
        filters.priority,
        filters.state,
        filters.assignee,
        filters.labels,
    )?;
    let count = issues.len();
    let message = match &query {
        Some(q) => format!("Found {count} issue(s) matching '{q}'"),
        None => format!("Found {count} issue(s)"),
    };
    let echoed = IssueSearchQuery { query };
    let results = if full {
        IssueSearchResults::Full(IssueSearchFullResponse::with_extras(issues, echoed))
    } else {
        IssueSearchResults::Summary(IssueSearchResponse::with_extras(
            issues.iter().map(MinimalIssue::from).collect(),
            echoed,
        ))
    };
    Ok(Outcome::success_with(results, message))
}

fn status<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    ids: Vec<String>,
) -> HandlerResult<IssueStatusOutput> {
    let issues = ctx.executor.issues();
    let statuses = ids
        .iter()
        .map(|id| {
            issues.issue_status_response(
                issues.show_issue(&resolve_issue(ctx.executor.storage(), id)?)?,
            )
        })
        .collect::<anyhow::Result<Vec<IssueStatusResponse>>>()?;
    // A single id stays a bare object, like `issue show`.
    let output = match <[IssueStatusResponse; 1]>::try_from(statuses) {
        Ok([only]) => IssueStatusOutput::One(only),
        Err(many) => IssueStatusOutput::Many(IssueStatusListResponse::new(many)),
    };
    Ok(output.into())
}

fn children<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
) -> HandlerResult<IssueChildrenResponse> {
    let response = ctx
        .executor
        .issues()
        .issue_children(&resolve_issue(ctx.executor.storage(), &id)?)?;
    Ok(response.into())
}

fn progress<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
) -> HandlerResult<ContainerProgressResponse> {
    let response = ctx
        .executor
        .issues()
        .issue_progress(&resolve_issue(ctx.executor.storage(), &id)?)?;
    Ok(response.into())
}

command_group! {
    command: IssueCommands,
    storage: [IssueStore],
    leaves: {
        "issue_show" => IssueCommands::Show { ids, summary, field, fields } => show(ids, summary, field, fields) -> IssueShowOutput
            as union [IssueShowResponse, IssueShowSummaryResponse, IssueShowListResponse, ItemShowResult, IssueProjection]
            "Single id: a bare IssueShowResponse (IssueShowSummaryResponse with --summary; the addressed item for an <issue>/<self-id> argument; the projected value with --field/--fields). Multiple ids: the {count, issues} envelope of full IssueShowResponse objects in argument order.",
        "issue_list" => IssueCommands::List { filters, full } => list(filters, full) -> IssueListing
            as union [IssueListResponse, IssueListFullResponse]
            "Default shape: lean MinimalIssue entries (id, short_id, title, state, priority) with no gate fields. With --full: complete stored issue records, whose gate list is carried under gates_required / gates_status.",
        "issue_search" => IssueCommands::Search { query, filters, full } => search(query, filters, full) -> IssueSearchResults
            as union [IssueSearchResponse, IssueSearchFullResponse]
            "Default shape: lean MinimalIssue entries with no gate fields, plus the echoed query. With --full: complete stored issue records, whose gate list is carried under gates_required / gates_status.",
        "issue_status" => IssueCommands::Status { ids } => status(ids) -> IssueStatusOutput
            as union [IssueStatusResponse, IssueStatusListResponse]
            "Single id: a bare IssueStatusResponse. Multiple ids: the {count, issues} envelope of IssueStatusResponse objects in argument order.",
        "issue_children" => IssueCommands::Children { id } => children(id) -> IssueChildrenResponse,
        "issue_progress" => IssueCommands::Progress { id } => progress(id) -> ContainerProgressResponse,
    },
    unmigrated: IssueCommands::Create { .. }
        | IssueCommands::BatchCreate { .. }
        | IssueCommands::Update { .. }
        | IssueCommands::Delete { .. }
        | IssueCommands::Assign { .. }
        | IssueCommands::Claim { .. }
        | IssueCommands::Unassign { .. }
        | IssueCommands::Reject { .. }
        | IssueCommands::Release { .. }
        | IssueCommands::ClaimNext { .. }
}

/// Run the `list` handler and render it, for `jit query all` and bare `jit query`.
pub fn emit_list<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    filters: IssueFilterArgs,
    full: bool,
    out: &mut dyn std::io::Write,
    err: &mut dyn std::io::Write,
) -> anyhow::Result<ExitCode> {
    emit_outcome(&list(ctx, filters, full)?, ctx.mode, out, err)
}

/// The top-level `list` alias shares `issue list`'s response.
pub fn alias_output_schema(cmd_path: &str) -> Option<OutputSchema> {
    (cmd_path == "list")
        .then(|| output_schema("issue_list"))
        .flatten()
}

#[cfg(test)]
mod tests {
    use crate::dispatch::registered_output_schema;

    #[test]
    fn test_registered_output_schema_types_every_issue_read_leaf_and_unions_the_multi_shape_ones() {
        for (path, union) in [
            ("issue_show", true),
            ("issue_list", true),
            ("list", true),
            ("issue_search", true),
            ("issue_status", true),
            ("issue_children", false),
            ("issue_progress", false),
        ] {
            let schema = registered_output_schema(path)
                .unwrap()
                .success_schema
                .unwrap();
            assert_eq!(schema.get("oneOf").is_some(), union, "{path}");
        }
    }
}
