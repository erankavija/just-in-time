//! `jit profile` handlers.

use super::{CommandContext, Outcome};
use crate::cli::ProfileCommands;
use crate::commands::{
    parse_profile_selectors, parse_profile_variable_assignments, ProfileVariableOptions,
};
use crate::errors::InvalidArgumentError;
use crate::output::ErrorCode;
use crate::profile::{
    ProfileAddResult, ProfileComposedApplyResult, ProfilePackResult, ProfileShowResult,
};
use crate::storage::{IssueStore, RepositoryStateStore};
use std::path::PathBuf;

fn show<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    profile: Vec<String>,
) -> anyhow::Result<Outcome<ProfileShowResult>> {
    Ok(Outcome::success(
        ctx.executor
            .profiles()
            .show_profiles(&parse_profile_selectors(&profile)?)?,
    ))
}

fn diff<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    profile: Vec<String>,
    values_file: Option<PathBuf>,
    set: Vec<String>,
) -> anyhow::Result<Outcome<ProfileComposedApplyResult>> {
    let selectors = parse_profile_selectors(&profile)?;
    let assignments = parse_profile_variable_assignments(&set)?;
    if selectors.is_empty() {
        return Err(InvalidArgumentError::new(
            "profile diff requires at least one --profile DIR selector",
        )
        .into());
    }
    let selection = ctx.executor.profiles().select_profiles(
        &selectors,
        &ProfileVariableOptions {
            values_file,
            assignments,
        },
    )?;
    let result = ctx.executor.profiles().diff_profiles(&selection)?;
    let conflicted = result.conflicted().count();
    Ok(if conflicted > 0 {
        Outcome::findings(
            ErrorCode::ValidationFailed,
            format!(
                "{conflicted} of {} profile(s) decided targets that cannot be published",
                result.count()
            ),
            result,
        )
    } else {
        let message = format!("{} profile(s) can be published", result.count());
        Outcome::success_with(result, message)
    })
}

fn apply<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    profile: Vec<String>,
    values_file: Option<PathBuf>,
    set: Vec<String>,
    dry_run: bool,
) -> anyhow::Result<Outcome<ProfileComposedApplyResult>> {
    let selectors = parse_profile_selectors(&profile)?;
    let assignments = parse_profile_variable_assignments(&set)?;
    if selectors.is_empty() {
        return Err(InvalidArgumentError::new(
            "profile apply requires at least one --profile DIR selector",
        )
        .into());
    }
    let selection = ctx.executor.profiles().select_profiles(
        &selectors,
        &ProfileVariableOptions {
            values_file,
            assignments,
        },
    )?;
    Ok(Outcome::success(if dry_run {
        ctx.executor.profiles().plan_profiles(&selection)?
    } else {
        ctx.executor.profiles().apply_profiles(&selection)?
    }))
}

fn pack<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    source: PathBuf,
    output: PathBuf,
    dry_run: bool,
) -> anyhow::Result<Outcome<ProfilePackResult>> {
    let invocation_dir = std::env::current_dir()?;
    let (result, warnings) = if dry_run {
        ctx.executor
            .profiles()
            .plan_profile_pack(&invocation_dir, &source, &output)?
    } else {
        ctx.executor
            .profiles()
            .pack_profile_package(&invocation_dir, &source, &output)?
    };
    Ok(Outcome::success(result).with_warnings(warnings))
}

fn add<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    archive: PathBuf,
    destination: PathBuf,
    dry_run: bool,
) -> anyhow::Result<Outcome<ProfileAddResult>> {
    let invocation_dir = std::env::current_dir()?;
    Ok(Outcome::success(if dry_run {
        ctx.executor
            .profiles()
            .plan_profile_add(&invocation_dir, &archive, &destination)?
    } else {
        ctx.executor
            .profiles()
            .add_profile_package(&invocation_dir, &archive, &destination)?
    }))
}

command_group! {
    command: ProfileCommands,
    storage: [IssueStore + RepositoryStateStore],
    leaves: {
        "profile_show" => ProfileCommands::Show { profile } => show(profile) -> ProfileShowResult,
        "profile_diff" => ProfileCommands::Diff { profile, values_file, set } => diff(profile, values_file, set) -> ProfileComposedApplyResult,
        "profile_apply" => ProfileCommands::Apply { profile, values_file, set, dry_run } => apply(profile, values_file, set, dry_run) -> ProfileComposedApplyResult,
        "profile_pack" => ProfileCommands::Pack { source, output, dry_run } => pack(source, output, dry_run) -> ProfilePackResult,
        "profile_add" => ProfileCommands::Add { archive, destination, dry_run } => add(archive, destination, dry_run) -> ProfileAddResult,
    }
}
