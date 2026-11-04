//! `jit issue` handlers.

use super::{resolve_issue, CommandContext, HandlerResult, HandlerStore, Outcome};
use crate::cli::{IssueCommands, IssueFilterArgs};
use crate::commands::bulk_update::UpdateOperations;
use crate::commands::{
    BatchDryRunOutcome, BatchIssueDef, DescriptionUpdate, IssueLocalUpdate, IssueMutation,
    ItemShowResult,
};
use crate::domain::identity::{GateKey, IssueIdPrefix};
use crate::domain::{Assignee, ContentFormat, Issue, MinimalIssue, Priority, State};
use crate::errors::InvalidArgumentError;
use crate::output::{
    emit_outcome, BatchCreateResponse, ContainerProgressResponse, ExitCode, IssueBatchCreateOutput,
    IssueChildrenResponse, IssueCreateResponse, IssueDeleteResponse, IssueLifecycleResponse,
    IssueListFullResponse, IssueListResponse, IssueListing, IssueProjection,
    IssueSearchFullResponse, IssueSearchQuery, IssueSearchResponse, IssueSearchResults,
    IssueShowListResponse, IssueShowOutput, IssueShowResponse, IssueShowSummaryResponse,
    IssueStatusListResponse, IssueStatusOutput, IssueStatusResponse, IssueUpdateOutput,
    IssueUpdateResponse,
};
use crate::query_engine::QueryFilter;
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

/// Read `--description-file` / `--append-description-file` content verbatim;
/// `-` reads stdin.
fn read_description_source(path: &str) -> anyhow::Result<String> {
    if path == "-" {
        std::io::read_to_string(std::io::stdin()).context("Failed to read description from stdin")
    } else {
        std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read description file: {path}"))
    }
}

fn parse_gate_keys(keys: Vec<String>) -> anyhow::Result<Vec<GateKey>> {
    Ok(keys
        .iter()
        .map(|key| key.parse())
        .collect::<Result<_, _>>()?)
}

#[allow(clippy::too_many_arguments)]
fn create<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    title: String,
    description: Option<String>,
    description_file: Option<String>,
    priority: String,
    issue_type: Option<String>,
    gate: Vec<String>,
    label: Vec<String>,
    content_format: Option<String>,
    force: bool,
) -> HandlerResult<IssueCreateResponse> {
    let gate = parse_gate_keys(gate)?;
    let priority = priority.parse::<Priority>()?;
    let content_format = content_format
        .map(|format| format.parse::<ContentFormat>())
        .transpose()?;
    let description = match (description, description_file) {
        (Some(text), _) => text,
        (None, Some(path)) => read_description_source(&path)
            .map_err(|error| InvalidArgumentError::new(format!("{error:#}")))?,
        (None, None) => String::new(),
    };
    let creation = ctx.executor.issues().create_issue(
        title,
        description,
        priority,
        gate,
        label,
        content_format,
        issue_type,
        force,
    )?;
    let issue = creation.issue;
    let message = format!("Created issue {} - {}", issue.short_id(), issue.title);
    // A new issue has no dependencies and no gate runs.
    let response = IssueCreateResponse {
        issue: IssueShowResponse::from_issue(issue, vec![], &[]),
        findings: creation.findings,
    };
    Ok(Outcome::success_with(response, message))
}

fn batch_create<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    from_json: std::path::PathBuf,
    dry_run: bool,
) -> HandlerResult<IssueBatchCreateOutput> {
    let contents = std::fs::read_to_string(&from_json)
        .with_context(|| format!("Failed to read batch file {}", from_json.display()))?;
    let defs: Vec<BatchIssueDef> = serde_json::from_str(&contents).with_context(|| {
        format!(
            "Failed to parse batch file {} as a JSON array of issue definitions",
            from_json.display()
        )
    })?;
    let issues = ctx.executor.issues();
    if dry_run {
        let outcome: BatchDryRunOutcome = issues.validate_batch_from_json(&defs)?;
        return Ok(IssueBatchCreateOutput::DryRun(outcome).into());
    }
    let created = BatchCreateResponse::from(issues.batch_create_from_json(defs)?);
    let message = format!("Created {} issue(s)", created.ids.len());
    Ok(Outcome::success_with(
        IssueBatchCreateOutput::Created(created),
        message,
    ))
}

/// The guards of `--filter` batch mode, which applies only fields shared by
/// every matched issue.
fn check_update_selection(
    id: Option<&String>,
    filter: Option<&String>,
    per_issue: &[(bool, &str)],
) -> anyhow::Result<()> {
    let refusal = match (id, filter) {
        (None, None) => Some("Must specify either issue ID or --filter for batch mode"),
        (Some(_), Some(_)) => Some("Cannot specify both ID and --filter (mutually exclusive)"),
        (None, Some(_)) => per_issue
            .iter()
            .find_map(|(given, refusal)| given.then_some(*refusal)),
        (Some(_), None) => None,
    };
    refusal.map_or(Ok(()), |refusal| {
        Err(InvalidArgumentError::new(refusal).into())
    })
}

#[allow(clippy::too_many_arguments)]
fn update<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    id: Option<String>,
    filter: Option<String>,
    title: Option<String>,
    description: Option<String>,
    description_file: Option<String>,
    append_description: Option<String>,
    append_description_file: Option<String>,
    priority: Option<String>,
    state: Option<String>,
    issue_type: Option<String>,
    label: Vec<String>,
    remove_label: Vec<String>,
    add_gate: Vec<String>,
    remove_gate: Vec<String>,
    assignee: Option<String>,
    unassign: bool,
    content_format: Option<String>,
    force: bool,
) -> HandlerResult<IssueUpdateOutput> {
    let add_gates = parse_gate_keys(add_gate)?;
    let remove_gates = parse_gate_keys(remove_gate)?;
    let assignee = assignee
        .map(|value| value.parse::<Assignee>())
        .transpose()?;
    let describes = description.is_some()
        || description_file.is_some()
        || append_description.is_some()
        || append_description_file.is_some();
    check_update_selection(
        id.as_ref(),
        filter.as_ref(),
        &[
            (
                content_format.is_some(),
                "--content-format is not supported with --filter (batch mode); set it per issue",
            ),
            (
                issue_type.is_some(),
                "--type is not supported with --filter (batch mode); set it per issue",
            ),
            (
                describes,
                "description flags (--description/--description-file/--append-description/--append-description-file) are not supported with --filter (batch mode); update descriptions per issue",
            ),
        ],
    )?;
    let priority = priority.map(|p| p.parse::<Priority>()).transpose()?;
    let state = state.map(|s| s.parse::<State>()).transpose()?;

    let Some(id) = id else {
        let filter = QueryFilter::parse(filter.as_deref().unwrap_or_default())?;
        let operations = UpdateOperations {
            state,
            add_labels: label,
            remove_labels: remove_label,
            assignee,
            unassign,
            priority,
            add_gates,
            remove_gates,
        };
        let result = ctx
            .executor
            .issues()
            .apply_bulk_update(&filter, &operations, force)?;
        let message = format!("Modified {} issue(s)", result.summary.total_modified);
        let warnings = result
            .warnings
            .iter()
            .map(|(id, warning)| format!("{}: {warning}", id.short_id()))
            .collect::<Vec<_>>();
        return Ok(
            Outcome::success_with(IssueUpdateOutput::Bulk(result), message).with_warnings(warnings),
        );
    };

    // The description flags are mutually exclusive (clap-enforced).
    let description = match (
        description,
        description_file,
        append_description,
        append_description_file,
    ) {
        (Some(text), ..) => Some(DescriptionUpdate::Replace(text)),
        (_, Some(path), ..) => Some(DescriptionUpdate::Replace(read_description_source(&path)?)),
        (_, _, Some(text), _) => Some(DescriptionUpdate::Append(text)),
        (.., Some(path)) => Some(DescriptionUpdate::Append(read_description_source(&path)?)),
        _ => None,
    };
    // Absent leaves the override unchanged; `inherit`/`default` clears it.
    let content_format = match content_format.as_deref() {
        None => None,
        Some("inherit" | "default") => Some(None),
        Some(format) => Some(Some(format.parse::<ContentFormat>()?)),
    };
    let IssueMutation { issue, warnings } = ctx.executor.issues().update_issue_resolved(
        &id.parse::<IssueIdPrefix>()?,
        title,
        description,
        priority,
        state,
        label,
        remove_label,
        content_format,
        issue_type,
        IssueLocalUpdate {
            add_gates,
            remove_gates,
            assignee,
            unassign,
        },
        force,
    )?;
    let message = format!("Updated issue {} to {}", issue.short_id(), issue.state);
    Ok(Outcome::success_with(
        IssueUpdateOutput::One(IssueUpdateResponse::from(&issue)),
        message,
    )
    .with_warnings(warnings))
}

fn delete<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
) -> HandlerResult<IssueDeleteResponse> {
    // Deletion is refused unless the operator confirms it (jit:0daba57d).
    let allowed = std::env::var("JIT_ALLOW_DELETION").is_ok_and(|value| value == "1");
    let deletion = ctx
        .executor
        .issues()
        .delete_issue(&id.parse::<IssueIdPrefix>()?, allowed)?;
    let message = format!("Deleted issue {}", deletion.id.short_id());
    let response = IssueDeleteResponse {
        id: deletion.id,
        deleted: true,
    };
    Ok(Outcome::success_with(response, message).with_warnings(deletion.warnings))
}

/// A lifecycle mutation's outcome: `message` under `--json`, `line(id)` in
/// human mode.
fn lifecycle(
    IssueMutation { issue, warnings }: IssueMutation,
    message: impl FnOnce(&Issue) -> String,
    line: impl FnOnce(&Issue) -> String,
) -> HandlerResult<IssueLifecycleResponse> {
    let (message, line) = (message(&issue), line(&issue));
    Ok(
        Outcome::success_with(IssueLifecycleResponse::new(issue, line), message)
            .with_warnings(warnings),
    )
}

fn assignee_of(issue: &Issue) -> String {
    issue
        .assignee
        .as_ref()
        .map_or_else(|| "unknown".to_string(), ToString::to_string)
}

fn assign<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
    assignee: String,
) -> HandlerResult<IssueLifecycleResponse> {
    let mutation = ctx
        .executor
        .issues()
        .assign_issue(&id.parse()?, assignee.parse()?)?;
    lifecycle(
        mutation,
        |issue| {
            format!(
                "Assigned issue {} to {}",
                issue.short_id(),
                assignee_of(issue)
            )
        },
        |issue| format!("Assigned issue: {}", issue.id),
    )
}

fn claim<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
    assignee: String,
    assign_only: bool,
) -> HandlerResult<IssueLifecycleResponse> {
    let (id, assignee) = (id.parse()?, assignee.parse()?);
    let issues = ctx.executor.issues();
    if assign_only {
        return lifecycle(
            issues.assign_issue(&id, assignee)?,
            |issue| {
                format!(
                    "Assigned issue {} to {} (assign-only)",
                    issue.short_id(),
                    assignee_of(issue)
                )
            },
            |issue| format!("Assigned issue {} (assign-only)", issue.id),
        );
    }
    lifecycle(
        issues.claim_issue(&id, assignee)?,
        |issue| format!("Claimed issue {}", issue.short_id()),
        |issue| format!("Claimed issue: {}", issue.id),
    )
}

fn unassign<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
) -> HandlerResult<IssueLifecycleResponse> {
    lifecycle(
        ctx.executor.issues().unassign_issue(&id.parse()?)?,
        |issue| format!("Unassigned issue {}", issue.short_id()),
        |issue| format!("Unassigned issue: {}", issue.id),
    )
}

fn reject<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
    reason: Option<String>,
) -> HandlerResult<IssueLifecycleResponse> {
    lifecycle(
        ctx.executor
            .issues()
            .reject_issue(&id.parse()?, reason.as_deref())?,
        |issue| format!("Rejected issue {}", issue.short_id()),
        |issue| match &reason {
            Some(reason) => format!("Rejected issue: {} (reason: {reason})", issue.id),
            None => format!("Rejected issue: {}", issue.id),
        },
    )
}

fn release<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
    reason: String,
) -> HandlerResult<IssueLifecycleResponse> {
    let issue = ctx.executor.issues().release_issue(&id.parse()?, &reason)?;
    lifecycle(
        IssueMutation {
            issue,
            warnings: Vec::new(),
        },
        |issue| format!("Released issue {}", issue.short_id()),
        |issue| format!("Released issue: {} (reason: {reason})", issue.id),
    )
}

fn claim_next<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    assignee: String,
    filter: Option<String>,
) -> HandlerResult<IssueLifecycleResponse> {
    lifecycle(
        ctx.executor
            .issues()
            .claim_next(assignee.parse()?, filter)?,
        |issue| format!("Claimed issue {}", issue.short_id()),
        |issue| format!("Claimed issue: {}", issue.id),
    )
}

command_group! {
    command: IssueCommands,
    storage: [HandlerStore],
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
        "issue_create" => IssueCommands::Create { positional_title, title, description, description_file, priority, issue_type, gate, label, content_format, force } => create(positional_title.or(title).unwrap_or_default(), description, description_file, priority, issue_type, gate, label, content_format, force) -> IssueCreateResponse,
        "issue_batch-create" => IssueCommands::BatchCreate { from_json, dry_run } => batch_create(from_json, dry_run) -> IssueBatchCreateOutput
            as union [BatchCreateResponse, BatchDryRunOutcome]
            "Default shape: the ids map from each entry's symbolic key to its created issue's full id. With --dry-run: the validation result; nothing is created.",
        "issue_update" => IssueCommands::Update { id, filter, title, description, description_file, append_description, append_description_file, priority, state, issue_type, label, remove_label, add_gate, remove_gate, assignee, unassign, content_format, force } => update(id, filter, title, description, description_file, append_description, append_description_file, priority, state, issue_type, label, remove_label, add_gate, remove_gate, assignee, unassign, content_format, force) -> IssueUpdateOutput
            as union [IssueUpdateResponse, crate::commands::BulkUpdateResult]
            "With an issue id: the updated issue's lean confirmation. With --filter: the batch result (matched, modified, skipped and failed ids with a summary).",
        "issue_delete" => IssueCommands::Delete { id } => delete(id) -> IssueDeleteResponse,
        "issue_assign" => IssueCommands::Assign { id, assignee } => assign(id, assignee) -> IssueLifecycleResponse,
        "issue_claim" => IssueCommands::Claim { id, assignee, assign_only } => claim(id, assignee, assign_only) -> IssueLifecycleResponse,
        "issue_unassign" => IssueCommands::Unassign { id } => unassign(id) -> IssueLifecycleResponse,
        "issue_reject" => IssueCommands::Reject { id, reason } => reject(id, reason) -> IssueLifecycleResponse,
        "issue_release" => IssueCommands::Release { id, reason } => release(id, reason) -> IssueLifecycleResponse,
        "issue_claim-next" => IssueCommands::ClaimNext { assignee, filter } => claim_next(assignee, filter) -> IssueLifecycleResponse,
    }
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
    fn test_registered_output_schema_types_every_issue_leaf_and_unions_the_multi_shape_ones() {
        for (path, union) in [
            ("issue_show", true),
            ("issue_list", true),
            ("list", true),
            ("issue_search", true),
            ("issue_status", true),
            ("issue_children", false),
            ("issue_progress", false),
            ("issue_create", false),
            ("issue_batch-create", true),
            ("issue_update", true),
            ("issue_delete", false),
            ("issue_assign", false),
            ("issue_claim", false),
            ("issue_unassign", false),
            ("issue_reject", false),
            ("issue_release", false),
            ("issue_claim-next", false),
        ] {
            let schema = registered_output_schema(path)
                .unwrap()
                .success_schema
                .unwrap();
            assert_eq!(schema.get("oneOf").is_some(), union, "{path}");
        }
    }
}
