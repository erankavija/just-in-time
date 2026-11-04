//! `jit init` handler.

use super::Outcome;
use crate::cli::InitArgs;
use crate::commands::{
    open_repository, parse_profile_selectors, parse_profile_variable_assignments, OpenedRepository,
    ProfileVariableOptions, RepositoryInvocation,
};
use crate::config_manager::ConfigManager;
use crate::output::InitResponse;
use std::env;
use std::path::PathBuf;

/// Initialization opens the repository itself, since it creates the data root
/// the other commands require, then fills the scaffold through one mutation
/// session; the machine-local worktree identity follows the publication.
fn init(args: &InitArgs) -> anyhow::Result<Outcome<InitResponse>> {
    let repository_root = env::current_dir()?;
    let OpenedRepository {
        retained_session: _session,
        executor,
        data_root,
        worktree_paths,
        write_override_annotation: _annotation,
        ..
    } = open_repository(RepositoryInvocation {
        current_dir: repository_root.clone(),
        data_dir_override: env::var("JIT_DATA_DIR").ok().map(PathBuf::from),
        is_init: true,
        opens_mutation_session: true,
        coordinates_claims_first: false,
        is_validation_exempt: true,
        is_recover: false,
        write_override: ConfigManager::invocation_write_override()?,
    })?;
    let profiles = executor.profiles();
    let selection = profiles.select_profiles(
        &parse_profile_selectors(&args.profile)?,
        &ProfileVariableOptions {
            values_file: args.values_file.clone(),
            assignments: parse_profile_variable_assignments(&args.set)?,
        },
    )?;
    let init_result = profiles.initialize_fresh_repository(&repository_root, Some(&selection))?;
    let (identity, warnings) = profiles.initialize_worktree_identity(&worktree_paths)?;
    let response = InitResponse {
        repository_root: repository_root.display().to_string(),
        data_dir: data_root.display().to_string(),
        repository_id: identity.map(|identity| identity.worktree_id.to_string()),
        gitattributes_status: init_result.gitattributes,
        created_paths: init_result.created_paths,
        modified_paths: init_result.modified_paths,
        profile: init_result.profile,
    };
    let message = response.message();
    Ok(Outcome::success_with(response, message).with_warnings(&warnings))
}

command_group! {
    command: InitArgs,
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
