//! `jit issue` read handlers.

use super::{CommandContext, Outcome};
use crate::cli::{IssueCommands, IssueFilterArgs};
use crate::domain::identity::{IssueId, IssueIdPrefix};
use crate::domain::MinimalIssue;
use crate::errors::InvalidArgumentError;
use crate::output::{
    ContainerProgressResponse, IssueChildrenResponse, IssueListFullResponse, IssueListResponse,
    IssueListing, IssueProjection, IssueSearchFullResponse, IssueSearchQuery, IssueSearchResponse,
    IssueSearchResults, IssueShowListResponse, IssueShowOutput, IssueShowResponse,
    IssueShowSummaryResponse, IssueStatusListResponse, IssueStatusOutput, IssueStatusResponse,
};
use crate::schema::OutputSchema;
use crate::storage::IssueStore;

fn success<R>(response: R, message: impl Into<Option<String>>) -> Outcome<R> {
    Outcome::Success {
        response,
        message: message.into(),
    }
}

fn resolve<S: IssueStore>(ctx: &CommandContext<'_, S>, requested: &str) -> anyhow::Result<IssueId> {
    ctx.executor
        .storage()
        .resolve_issue_id(&requested.parse::<IssueIdPrefix>()?)
}

fn show_response<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    requested: &str,
) -> anyhow::Result<IssueShowResponse> {
    let issues = ctx.executor.issues();
    issues.show_response(issues.show_issue(&resolve(ctx, requested)?)?)
}

fn show<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    ids: Vec<String>,
    summary: bool,
    field: Option<String>,
    fields: Vec<String>,
) -> anyhow::Result<Outcome<IssueShowOutput>> {
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
        return Ok(success(
            IssueShowOutput::Many(IssueShowListResponse::new(shown)),
            None,
        ));
    };
    // A `<issue>/<self-id>` argument addresses an item; issue ids never
    // contain '/'.
    if !projecting && !summary && requested.contains('/') {
        let item = ctx.executor.items().show_item(requested)?;
        return Ok(success(IssueShowOutput::Item(item), None));
    }
    let issues = ctx.executor.issues();
    let issue = issues.show_issue(&resolve(ctx, requested)?)?;
    if summary && ctx.mode.json && !projecting {
        let summary = IssueShowSummaryResponse::from(&issue);
        let message = format!(
            "Issue {}: {} [{}]",
            summary.short_id, summary.title, summary.state
        );
        return Ok(success(IssueShowOutput::Summary(summary), message));
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
        return Ok(success(IssueShowOutput::Projection(projection), None));
    }
    let message = format!(
        "Issue {}: {} [{}]",
        shown.id.short_id(),
        shown.title,
        shown.state
    );
    Ok(success(IssueShowOutput::Issue(Box::new(shown)), message))
}

/// `jit issue list`, `jit list`, `jit query all` and bare `jit query`.
pub(super) fn list<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    filters: IssueFilterArgs,
    full: bool,
) -> anyhow::Result<Outcome<IssueListing>> {
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
    Ok(success(listing, message))
}

fn search<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    query: Option<String>,
    filters: IssueFilterArgs,
    full: bool,
) -> anyhow::Result<Outcome<IssueSearchResults>> {
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
    Ok(success(results, message))
}

fn status<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    ids: Vec<String>,
) -> anyhow::Result<Outcome<IssueStatusOutput>> {
    let issues = ctx.executor.issues();
    let statuses = ids
        .iter()
        .map(|id| issues.issue_status_response(issues.show_issue(&resolve(ctx, id)?)?))
        .collect::<anyhow::Result<Vec<IssueStatusResponse>>>()?;
    // A single id stays a bare object, like `issue show`.
    let output = match <[IssueStatusResponse; 1]>::try_from(statuses) {
        Ok([only]) => IssueStatusOutput::One(only),
        Err(many) => IssueStatusOutput::Many(IssueStatusListResponse::new(many)),
    };
    Ok(success(output, None))
}

fn children<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
) -> anyhow::Result<Outcome<IssueChildrenResponse>> {
    let response = ctx.executor.issues().issue_children(&resolve(ctx, &id)?)?;
    Ok(success(response, None))
}

fn progress<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
) -> anyhow::Result<Outcome<ContainerProgressResponse>> {
    let response = ctx.executor.issues().issue_progress(&resolve(ctx, &id)?)?;
    Ok(success(response, None))
}

command_group! {
    command: IssueCommands,
    storage: [IssueStore],
    leaves: {
        "issue_show" => IssueCommands::Show { ids, summary, field, fields } => show(ids, summary, field, fields) -> IssueShowOutput,
        "issue_list" => IssueCommands::List { filters, full } => list(filters, full) -> IssueListing,
        "issue_search" => IssueCommands::Search { query, filters, full } => search(query, filters, full) -> IssueSearchResults,
        "issue_status" => IssueCommands::Status { ids } => status(ids) -> IssueStatusOutput,
        "issue_children" => IssueCommands::Children { id } => children(id) -> IssueChildrenResponse,
        "issue_progress" => IssueCommands::Progress { id } => progress(id) -> ContainerProgressResponse,
    },
    unmigrated: _ => Err(anyhow::anyhow!("not an issue read command"))
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
    fn test_registered_output_schema_names_a_typed_response_for_every_issue_read_leaf() {
        let names = [
            ("issue_show", "IssueShowOutput"),
            ("issue_list", "IssueListing"),
            ("list", "IssueListing"),
            ("issue_search", "IssueSearchResults"),
            ("issue_status", "IssueStatusOutput"),
            ("issue_children", "IssueChildrenResponse"),
            ("issue_progress", "ContainerProgressResponse"),
        ];
        for (path, name) in names {
            let registered = registered_output_schema(path).unwrap();
            assert_eq!(registered.success, name, "{path}");
        }
    }
}
