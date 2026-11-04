//! `jit dep` handlers.

use super::{resolve_issue, CommandContext, HandlerResult, HandlerStore, Outcome};
use crate::cli::DepCommands;
use crate::commands::RedundancyPolicy;
use crate::output::{DependencyAddResponse, DependencyRemoveResponse};

fn add<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    from_id: String,
    to_ids: Vec<String>,
    reduce: bool,
) -> HandlerResult<DependencyAddResponse> {
    let from_id = resolve_issue(ctx.executor, &from_id)?;
    let policy = if reduce {
        RedundancyPolicy::Reduce
    } else {
        RedundancyPolicy::Reject
    };
    let result = ctx
        .executor
        .graph()
        .add_requested_dependencies(&from_id, &to_ids, policy)?;
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
    let from_id = resolve_issue(ctx.executor, &from_id)?;
    let (result, unmatched) = ctx
        .executor
        .graph()
        .remove_requested_dependencies(&from_id, &to_ids)?;
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
