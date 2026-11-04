//! `jit archive` handlers.
//!
//! Execution is implemented only for the JSON-file store, so the group is bound
//! to it rather than generic over [`IssueStore`](crate::storage::IssueStore).

use super::{resolve_issue, CommandContext, HandlerResult};
use crate::cli::ArchiveCommands;
use crate::domain::artifact_execution::ArchiveExecutionResult;
use crate::domain::artifact_plan::{ArchiveCandidates, ArtifactPlan};
use crate::output::ArchiveTargetResponse;
use crate::storage::JsonFileStorage;

fn candidates(ctx: &CommandContext<'_, JsonFileStorage>) -> HandlerResult<ArchiveCandidates> {
    Ok(ctx.executor.archive().archive_candidates()?.into())
}

fn document(
    ctx: &CommandContext<'_, JsonFileStorage>,
    path: String,
    execute: bool,
) -> HandlerResult<ArchiveTargetResponse> {
    let archive = ctx.executor.archive();
    let response = if execute {
        ArchiveTargetResponse::Executed(archive.execute_archive_document(&path)?)
    } else {
        ArchiveTargetResponse::Preview(archive.preview_archive_document(&path)?)
    };
    Ok(response.into())
}

fn container(
    ctx: &CommandContext<'_, JsonFileStorage>,
    requested: String,
    execute: bool,
) -> HandlerResult<ArchiveTargetResponse> {
    let id = resolve_issue(ctx.executor, &requested)?;
    let archive = ctx.executor.archive();
    let response = if execute {
        ArchiveTargetResponse::Executed(archive.execute_archive_container(&id)?)
    } else {
        ArchiveTargetResponse::Preview(archive.preview_archive_container(&id)?)
    };
    Ok(response.into())
}

command_group! {
    command: ArchiveCommands,
    storage: JsonFileStorage,
    leaves: {
        "archive_candidates" => ArchiveCommands::Candidates => candidates() -> ArchiveCandidates,
        "archive_document" => ArchiveCommands::Document { path, execute } => document(path, execute) -> ArchiveTargetResponse
            as union [ArtifactPlan, ArchiveExecutionResult] "Default: the ArtifactPlan preview. With --execute: the ArchiveExecutionResult of the run.",
        "archive_container" => ArchiveCommands::Container { id, execute } => container(id, execute) -> ArchiveTargetResponse
            as union [ArtifactPlan, ArchiveExecutionResult] "Default: the ArtifactPlan preview. With --execute: the ArchiveExecutionResult of the run.",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_registered_output_schema_archive_leaves_name_their_typed_responses() {
        let registered = |path: &str| {
            crate::dispatch::registered_output_schema(path)
                .unwrap_or_else(|| panic!("{path} has a registered schema"))
        };

        assert_eq!(
            registered("archive_candidates").success,
            "ArchiveCandidates"
        );
        for path in ["archive_document", "archive_container"] {
            let schema = registered(path);
            assert_eq!(schema.success, "ArtifactPlan");
            assert!(schema.success_schema.unwrap()["oneOf"].is_array());
        }
    }
}
