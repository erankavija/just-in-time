//! `jit archive` handlers.
//!
//! Execution is implemented only for the JSON-file store, so the group is bound
//! to it rather than generic over [`IssueStore`](crate::storage::IssueStore).

use super::{resolve_issue, CommandContext, HandlerResult};
use crate::cli::ArchiveCommands;
use crate::domain::archive_advisory::{ArchiveExecutionReport, ArchivePreview};
use crate::domain::artifact_plan::ArchiveCandidates;
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
        "archive_candidates" => ArchiveCommands::Candidates => candidates() raises [RetryableConflict] -> ArchiveCandidates,
        "archive_document" => ArchiveCommands::Document { path, execute } => document(path, execute) raises [RetryableConflict] -> ArchiveTargetResponse
            as union [ArchivePreview, ArchiveExecutionReport] "Default: the ArchivePreview plan. With --execute: the ArchiveExecutionReport of the run. Both carry an advisory citation section.",
        "archive_container" => ArchiveCommands::Container { id, execute } => container(id, execute) raises [RetryableConflict] -> ArchiveTargetResponse
            as union [ArchivePreview, ArchiveExecutionReport] "Default: the ArchivePreview plan. With --execute: the ArchiveExecutionReport of the run. Both carry an advisory citation section.",
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
            assert_eq!(schema.success, "ArchivePreview");
            assert!(schema.success_schema.unwrap()["oneOf"].is_array());
        }
    }
}
