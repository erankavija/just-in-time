//! `jit snapshot export` handler.

use super::{resolve_issue, CommandContext, Outcome};
use crate::cli::SnapshotExportArgs;
use crate::output::snapshot::SnapshotExportResponse;
use crate::snapshot::{SnapshotFormat, SnapshotScope};
use crate::storage::{IssueStore, RepositoryStateStore};
use anyhow::Context;
use std::path::Path;

fn export<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    args: SnapshotExportArgs,
) -> anyhow::Result<Outcome<SnapshotExportResponse>> {
    let snapshot_scope =
        SnapshotScope::parse(&args.scope, |id| resolve_issue(ctx.executor.storage(), id))?;
    let snapshot_format = SnapshotFormat::parse(&args.format)
        .with_context(|| format!("Invalid format: {}", args.format))?;
    let source_mode = ctx.executor.archive().determine_snapshot_source_mode(
        args.at.as_deref(),
        args.working_tree,
        args.committed_only,
    )?;

    if !args.force {
        ctx.executor.validation().validate_silent()?;
    }

    let (result, warnings) = ctx.executor.archive().export_snapshot(
        &std::env::current_dir()?,
        &snapshot_scope,
        &source_mode,
        &snapshot_format,
        args.out.as_deref().map(Path::new),
    )?;
    let message = format!("Exported {} issues to {}", result.issue_count, result.path);

    Ok(Outcome::success_with(SnapshotExportResponse(result), message).with_warnings(warnings))
}

command_group! {
    command: SnapshotExportArgs,
    storage: [IssueStore + RepositoryStateStore],
    leaves: {
        "snapshot_export" => args => export(args) -> SnapshotExportResponse,
    }
}

#[cfg(test)]
mod tests {
    use super::output_schema;

    #[test]
    fn test_snapshot_export_registers_output_owned_response_schema() {
        let schema = output_schema("snapshot_export").unwrap();

        assert_eq!(schema.success, "SnapshotExportResponse");
    }
}
