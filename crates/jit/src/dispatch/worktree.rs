//! Worktree inspection handlers.

use super::{count_message, CommandContext, HandlerResult, Outcome};
use crate::cli::WorktreeCommands;
use crate::errors::InvalidArgumentError;
use crate::output::{
    FieldProjection, StoreComparison, WorktreeExecResponse, WorktreeInfoOutput,
    WorktreeListResponse, WorktreeStoreDivergenceResponse,
};
use crate::storage::JsonFileStorage;
use anyhow::Context;

fn info(
    ctx: &CommandContext<'_, JsonFileStorage>,
    field: Option<String>,
) -> HandlerResult<WorktreeInfoOutput> {
    let (info, warnings) = ctx.executor.worktrees().info()?;
    if let Some(name) = field {
        let value = serde_json::to_value(&info)?;
        let projection = FieldProjection::field(&value, &name).ok_or_else(|| {
            InvalidArgumentError::new(format!(
                "invalid field: unknown field '{name}' for worktree info"
            ))
        })?;
        return Ok(
            Outcome::from(WorktreeInfoOutput::Projection(projection)).with_warnings(warnings)
        );
    }
    let message = format!("Worktree {} on branch {}", info.worktree_id, info.branch);
    Ok(Outcome::success_with(WorktreeInfoOutput::Info(info), message).with_warnings(warnings))
}

fn list(ctx: &CommandContext<'_, JsonFileStorage>) -> HandlerResult<WorktreeListResponse> {
    let (worktrees, warnings) = ctx.executor.worktrees().list()?;
    let message = count_message(worktrees.len(), "", " worktree(s)");
    Ok(
        Outcome::success_with(WorktreeListResponse::new(worktrees), message)
            .with_warnings(warnings),
    )
}

fn store_divergence(
    ctx: &CommandContext<'_, JsonFileStorage>,
) -> HandlerResult<WorktreeStoreDivergenceResponse> {
    let report = ctx.executor.worktrees().store_divergence()?;
    let message = count_message(report.divergences.len(), "", " divergence(s)");
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

fn exec(
    ctx: &CommandContext<'_, JsonFileStorage>,
    command: Vec<String>,
) -> HandlerResult<WorktreeExecResponse> {
    let (program, args) = command.split_first().context("no command to run")?;
    let status = ctx.executor.worktrees().exec(program, args)?;
    Ok(Outcome::Exited(
        status
            .code()
            .unwrap_or(crate::output::ExitCode::GenericError.code()),
    ))
}

command_group! {
    command: WorktreeCommands,
    storage: JsonFileStorage,
    leaves: {
        "worktree_info" => WorktreeCommands::Info { field } => info(field) raises [RequiresGit] -> WorktreeInfoOutput,
        "worktree_list" => WorktreeCommands::List => list() -> WorktreeListResponse,
        "worktree_store-divergence" => WorktreeCommands::StoreDivergence => store_divergence() -> WorktreeStoreDivergenceResponse,
        "worktree_exec" => WorktreeCommands::Exec { command } => exec(command) -> WorktreeExecResponse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::OutputSchema;

    #[test]
    fn test_worktree_registration_exposes_each_typed_success_schema() {
        for (path, expected) in [
            ("worktree_info", OutputSchema::of::<WorktreeInfoOutput>()),
            ("worktree_list", OutputSchema::of::<WorktreeListResponse>()),
            (
                "worktree_store-divergence",
                OutputSchema::of::<WorktreeStoreDivergenceResponse>(),
            ),
            ("worktree_exec", OutputSchema::of::<WorktreeExecResponse>()),
        ] {
            let registered = crate::dispatch::registered_output_schema(path).unwrap();
            assert_eq!(registered.success_schema, expected.success_schema);
            assert_eq!(registered.success, expected.success);
        }
    }
}
