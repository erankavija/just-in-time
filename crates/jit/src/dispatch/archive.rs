//! `jit archive` handlers.
//!
//! Execution is implemented only for the JSON-file store, so the group is bound
//! to it rather than generic over [`IssueStore`](crate::storage::IssueStore).

use super::{CommandContext, Outcome};
use crate::cli::ArchiveCommands;
use crate::domain::artifact_plan::ArchiveCandidates;
use crate::domain::identity::IssueIdPrefix;
use crate::output::ArchiveTargetResponse;
use crate::storage::{IssueStore, JsonFileStorage};

fn candidates(
    ctx: &CommandContext<'_, JsonFileStorage>,
) -> anyhow::Result<Outcome<ArchiveCandidates>> {
    Ok(Outcome::Success {
        response: ctx.executor.archive().archive_candidates()?,
        message: None,
    })
}

fn document(
    ctx: &CommandContext<'_, JsonFileStorage>,
    path: String,
    execute: bool,
) -> anyhow::Result<Outcome<ArchiveTargetResponse>> {
    let archive = ctx.executor.archive();
    let response = if execute {
        ArchiveTargetResponse::Executed(archive.execute_archive_document(&path)?)
    } else {
        ArchiveTargetResponse::Preview(archive.preview_archive_document(&path)?)
    };
    Ok(Outcome::Success {
        response,
        message: None,
    })
}

fn container(
    ctx: &CommandContext<'_, JsonFileStorage>,
    requested: String,
    execute: bool,
) -> anyhow::Result<Outcome<ArchiveTargetResponse>> {
    let id = ctx
        .executor
        .storage()
        .resolve_issue_id(&requested.parse::<IssueIdPrefix>()?)?;
    let archive = ctx.executor.archive();
    let response = if execute {
        ArchiveTargetResponse::Executed(archive.execute_archive_container(&id)?)
    } else {
        ArchiveTargetResponse::Preview(archive.preview_archive_container(&id)?)
    };
    Ok(Outcome::Success {
        response,
        message: None,
    })
}

command_group! {
    command: ArchiveCommands,
    storage: JsonFileStorage,
    leaves: {
        "archive_candidates" => ArchiveCommands::Candidates => candidates() -> ArchiveCandidates,
        "archive_document" => ArchiveCommands::Document { path, execute } => document(path, execute) -> ArchiveTargetResponse,
        "archive_container" => ArchiveCommands::Container { id, execute } => container(id, execute) -> ArchiveTargetResponse,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_registered_output_schema_archive_leaves_name_their_typed_responses() {
        let schema_name = |path: &str| {
            crate::dispatch::registered_output_schema(path)
                .unwrap_or_else(|| panic!("{path} has a registered schema"))
                .success
        };

        assert_eq!(schema_name("archive_candidates"), "ArchiveCandidates");
        assert_eq!(schema_name("archive_document"), "ArchiveTargetResponse");
        assert_eq!(schema_name("archive_container"), "ArchiveTargetResponse");
    }
}
