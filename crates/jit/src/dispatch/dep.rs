//! `jit dep` handlers.

use super::{resolve_issue, CommandContext, HandlerResult, HandlerStore, Outcome};
use crate::cli::DepCommands;
use crate::commands::RedundancyPolicy;
use crate::domain::identity::{IssueId, IssueIdPrefix};
use crate::output::{DependencyAddResponse, DependencyRemoveResponse};
use crate::storage::IssueStore;
use anyhow::Result;

fn add<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    from_id: String,
    to_ids: Vec<String>,
    reduce: bool,
) -> HandlerResult<DependencyAddResponse> {
    let from_id = resolve_issue(ctx.executor.storage(), &from_id)?;
    let to_ids = resolve_dependency_args(ctx.executor.storage(), &from_id, &to_ids)?;
    let policy = if reduce {
        RedundancyPolicy::Reduce
    } else {
        RedundancyPolicy::Reject
    };
    let result = ctx
        .executor
        .graph()
        .add_dependencies_with_policy(&from_id, &to_ids, policy)?;
    let message = format!(
        "Added {} dependencies to issue {from_id}",
        result.added.len()
    );
    Ok(Outcome::success_with(
        DependencyAddResponse {
            from_id,
            added: result.added,
            already_exist: result.already_exist,
            skipped: result.skipped,
        },
        message,
    )
    .with_warnings(result.warnings))
}

fn rm<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    from_id: String,
    to_ids: Vec<String>,
) -> HandlerResult<DependencyRemoveResponse> {
    let from_id = resolve_issue(ctx.executor.storage(), &from_id)?;
    let (to_ids, unmatched) =
        resolve_dependency_removal_args(ctx.executor.storage(), &from_id, &to_ids)?;
    let result = ctx
        .executor
        .graph()
        .remove_dependencies(&from_id, &to_ids)?;
    let message = format!(
        "Removed {} dependencies from issue {from_id}",
        result.removed.len()
    );
    Ok(Outcome::success_with(
        DependencyRemoveResponse {
            from_id,
            removed: result.removed,
            not_found: result
                .not_found
                .into_iter()
                .map(|id| id.to_string())
                .chain(unmatched)
                .collect(),
        },
        message,
    )
    .with_warnings(result.warnings))
}

command_group! {
    command: DepCommands,
    storage: [HandlerStore],
    leaves: {
        "dep_add" => DepCommands::Add { from_id, to_ids, reduce } => add(from_id, to_ids, reduce)
            raises [CycleDetected, ValidationFailed] -> DependencyAddResponse,
        "dep_rm" => DepCommands::Rm { from_id, to_ids } => rm(from_id, to_ids) -> DependencyRemoveResponse,
    }
}

/// Resolve a `jit dep add` batch's targets, naming every unresolvable one in a
/// single [`DependencyBatchRejectedError`](crate::errors::DependencyBatchRejectedError).
fn resolve_dependency_args<S: IssueStore>(
    storage: &S,
    from_id: &IssueId,
    requested: &[String],
) -> Result<Vec<IssueId>> {
    let (resolved, rejected): (Vec<_>, Vec<_>) = requested
        .iter()
        .map(|id| (id, resolve_issue(storage, id)))
        .partition(|(_, result)| result.is_ok());
    if rejected.is_empty() {
        resolved.into_iter().map(|(_, result)| result).collect()
    } else {
        Err(crate::errors::DependencyBatchRejectedError::new(
            from_id.clone(),
            rejected
                .into_iter()
                .filter_map(|(id, result)| result.err().map(|error| (id.clone(), error)))
                .collect(),
        )
        .into())
    }
}

/// Resolve `jit dep rm` targets against `from_id`'s own stored dependencies,
/// so an edge whose target was deleted stays removable (jit:f847df3f). Returns
/// the ids the targets name, in order, and the targets naming none; each target
/// matches only the edges no earlier target named.
///
/// # Errors
///
/// [`InvalidIdPrefixError`](crate::domain::identity::InvalidIdPrefixError) before
/// any read, or [`AmbiguousIdError`](crate::storage::AmbiguousIdError).
fn resolve_dependency_removal_args<S: IssueStore>(
    storage: &S,
    from_id: &IssueId,
    requested: &[String],
) -> Result<(Vec<IssueId>, Vec<String>)> {
    let prefixes = requested
        .iter()
        .map(|id| id.parse::<IssueIdPrefix>())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let stored = storage.load_issue(from_id)?.dependencies;
    prefixes.into_iter().try_fold(
        (Vec::new(), Vec::new()),
        |(mut matched, mut unmatched): (Vec<IssueId>, Vec<String>), prefix| {
            let remaining = stored
                .iter()
                .filter(|id| !matched.contains(id))
                .cloned()
                .collect::<Vec<_>>();
            match crate::storage::resolve_dependency_id(&prefix, &remaining) {
                Ok(id) => matched.push(id),
                Err(error) if error.is::<crate::storage::IssueNotFoundError>() => {
                    unmatched.push(prefix.as_str().to_string())
                }
                Err(error) => return Err(error),
            }
            Ok((matched, unmatched))
        },
    )
}
