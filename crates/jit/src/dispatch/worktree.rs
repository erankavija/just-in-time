//! Worktree inspection handlers.

use super::{CommandContext, HandlerResult, Outcome};
use crate::cli::WorktreeCommands;
use crate::commands::worktree::{
    execute_worktree_info, execute_worktree_list, execute_worktree_store_divergence, WorktreeInfo,
};
use crate::output::{StoreComparison, WorktreeListResponse, WorktreeStoreDivergenceResponse};
use crate::storage::JsonFileStorage;

fn info(ctx: &CommandContext<'_, JsonFileStorage>) -> HandlerResult<WorktreeInfo> {
    let (info, warnings) =
        execute_worktree_info(&ctx.executor.storage().configured_worktree_paths()?)?;
    let message = format!("Worktree {} on branch {}", info.worktree_id, info.branch);
    Ok(Outcome::success_with(info, message).with_warnings(warnings))
}

fn list(ctx: &CommandContext<'_, JsonFileStorage>) -> HandlerResult<WorktreeListResponse> {
    let (worktrees, warnings) =
        execute_worktree_list(&ctx.executor.storage().configured_worktree_paths()?)?;
    let message = format!("{} worktree(s)", worktrees.len());
    Ok(
        Outcome::success_with(WorktreeListResponse::new(worktrees), message)
            .with_warnings(warnings),
    )
}

fn store_divergence(
    ctx: &CommandContext<'_, JsonFileStorage>,
) -> HandlerResult<WorktreeStoreDivergenceResponse> {
    let report =
        execute_worktree_store_divergence(&ctx.executor.storage().configured_worktree_paths()?)?;
    let message = format!("{} divergence(s)", report.divergences.len());
    Ok(Outcome::success_with(
        WorktreeStoreDivergenceResponse::with_extras(
            report.divergences,
            StoreComparison {
                checkout_store: report.checkout_store,
                reference_store: report.reference_store,
            },
        ),
        message,
    ))
}

command_group! {
    command: WorktreeCommands,
    storage: JsonFileStorage,
    leaves: {
        "worktree_info" => WorktreeCommands::Info => info() -> WorktreeInfo,
        "worktree_list" => WorktreeCommands::List => list() -> WorktreeListResponse,
        "worktree_store-divergence" => WorktreeCommands::StoreDivergence => store_divergence() -> WorktreeStoreDivergenceResponse,
    },
    unmigrated: WorktreeCommands::Exec { .. }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::OutputSchema;

    #[test]
    fn test_worktree_registration_exposes_each_typed_success_schema() {
        for (path, expected) in [
            ("worktree_info", OutputSchema::of::<WorktreeInfo>()),
            ("worktree_list", OutputSchema::of::<WorktreeListResponse>()),
            (
                "worktree_store-divergence",
                OutputSchema::of::<WorktreeStoreDivergenceResponse>(),
            ),
        ] {
            let registered = crate::dispatch::registered_output_schema(path).unwrap();
            assert_eq!(registered.success_schema, expected.success_schema);
            assert_eq!(registered.success, expected.success);
        }
    }
}
