//! `jit init` handler.

use super::{CommandContext, Outcome};
use crate::commands::{
    parse_profile_selectors, parse_profile_variable_assignments, ProfileVariableOptions,
};
use crate::output::{InitResponse, OutputContext};
use crate::storage::{IssueStore, RepositoryStateStore};
use std::path::Path;

/// The parsed arguments of `jit init` from the directory it runs in.
pub struct Invocation<'a> {
    pub profile: &'a [String],
    pub values_file: Option<&'a Path>,
    pub set: &'a [String],
    pub repository_root: &'a Path,
}

/// Initialization fills the scaffold through one mutation session; the
/// machine-local worktree identity follows the publication.
fn init<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    invocation: &Invocation<'_>,
) -> anyhow::Result<Outcome<InitResponse>> {
    let profiles = ctx.executor.profiles();
    let selection = profiles.select_profiles(
        &parse_profile_selectors(invocation.profile)?,
        &ProfileVariableOptions {
            values_file: invocation.values_file.map(Path::to_path_buf),
            assignments: parse_profile_variable_assignments(invocation.set)?,
        },
    )?;
    let init_result =
        profiles.initialize_fresh_repository(invocation.repository_root, Some(&selection))?;
    let (identity, warnings) =
        profiles.initialize_worktree_identity(ctx.executor.require_worktree_paths()?)?;
    let output = OutputContext::new(ctx.mode.quiet, ctx.mode.json);
    warnings
        .iter()
        .try_for_each(|warning| output.print_warning(warning))?;
    let data_dir = ctx
        .executor
        .repository_layout()?
        .data_root()
        .display()
        .to_string();
    let response = InitResponse {
        repository_root: invocation.repository_root.display().to_string(),
        data_dir,
        repository_id: identity.map(|identity| identity.worktree_id.to_string()),
        gitattributes_status: init_result.gitattributes,
        created_paths: init_result.created_paths,
        modified_paths: init_result.modified_paths,
        profile: init_result.profile,
        quiet: ctx.mode.quiet,
    };
    let message = Some(response.message());
    Ok(Outcome::Success { response, message })
}

command_group! {
    command: Invocation<'_>,
    storage: [IssueStore + RepositoryStateStore],
    leaves: {
        "init" => invocation => init(&invocation) -> InitResponse,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_registered_output_schema_init_is_named_for_its_response_type() {
        let registered = crate::dispatch::registered_output_schema("init").unwrap();

        assert_eq!(registered.success, "InitResponse");
    }
}
