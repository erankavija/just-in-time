//! `jit init` handler.

use super::{CommandContext, Outcome};
use crate::cli::InitArgs;
use crate::commands::{
    parse_profile_selectors, parse_profile_variable_assignments, ProfileVariableOptions,
};
use crate::output::{InitResponse, OutputContext};
use crate::storage::{IssueStore, RepositoryStateStore};
use std::env;

/// Initialization fills the scaffold through one mutation session; the
/// machine-local worktree identity follows the publication.
fn init<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    args: &InitArgs,
) -> anyhow::Result<Outcome<InitResponse>> {
    let profiles = ctx.executor.profiles();
    let selection = profiles.select_profiles(
        &parse_profile_selectors(&args.profile)?,
        &ProfileVariableOptions {
            values_file: args.values_file.clone(),
            assignments: parse_profile_variable_assignments(&args.set)?,
        },
    )?;
    let repository_root = env::current_dir()?;
    let init_result = profiles.initialize_fresh_repository(&repository_root, Some(&selection))?;
    let (identity, warnings) =
        profiles.initialize_worktree_identity(ctx.executor.require_worktree_paths()?)?;
    let output = OutputContext::new(ctx.mode.quiet, ctx.mode.json);
    warnings
        .iter()
        .try_for_each(|warning| output.print_warning(warning))?;
    let response = InitResponse {
        repository_root: repository_root.display().to_string(),
        data_dir: ctx.executor.storage().root().display().to_string(),
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
    command: InitArgs,
    storage: [IssueStore + RepositoryStateStore],
    leaves: {
        "init" => args => init(&args) -> InitResponse,
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
