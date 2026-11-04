//! Gate definitions and presets; execution and status remain in the CLI.

use super::{resolve_issue, CommandContext, HandlerResult, HandlerStore, Outcome};
use crate::cli::{GateCommands, PresetCommands};
use crate::commands::{FieldEdit, GateUpdate};
use crate::declarations::{GateChecker, GateMode, GateStage};
use crate::domain::identity::GateKey;
use crate::errors::InvalidArgumentError;
use crate::output::gate::{
    GateAddResponse, GateDefinitionResponse, GateMutationResponse, GatePresetApplyResponse,
    GatePresetCreateResponse, GatePresetError, GatePresetShowResponse, GatePresetSuccess,
};
use crate::output::{ErrorCode, GateDefinition, GateListResponse, GatePresetListResponse};
use crate::storage::IssueStore;
use std::collections::HashMap;

fn parse_env(env: Vec<String>) -> anyhow::Result<HashMap<String, String>> {
    env.into_iter()
        .map(|pair| {
            pair.split_once('=')
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .ok_or_else(|| {
                    InvalidArgumentError::new(format!(
                        "Invalid --env format '{}': expected KEY=VALUE",
                        pair
                    ))
                    .into()
                })
        })
        .collect()
}

fn field_edit<T>(set: Option<T>, clear: bool, flag: &str) -> anyhow::Result<FieldEdit<T>> {
    match (set, clear) {
        (Some(_), true) => Err(InvalidArgumentError::new(format!(
            "--{flag} and --clear-{flag} are mutually exclusive; provide only one."
        ))
        .into()),
        (Some(value), false) => Ok(FieldEdit::Set(value)),
        (None, true) => Ok(FieldEdit::Clear),
        (None, false) => Ok(FieldEdit::Keep),
    }
}

#[allow(clippy::too_many_arguments)]
fn define<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    key: String,
    title: String,
    description: String,
    stage: GateStage,
    mode: Option<GateMode>,
    auto: bool,
    checker_command: Option<String>,
    timeout: u64,
    working_dir: Option<String>,
    pass_context: bool,
    prompt: Option<String>,
    prompt_file: Option<String>,
    env: Vec<String>,
    priority: u32,
) -> HandlerResult<GateMutationResponse> {
    let key = key.parse()?;
    let mode = if auto {
        GateMode::Auto
    } else {
        match mode {
            Some(GateMode::Manual) if checker_command.is_some() => {
                return Err(InvalidArgumentError::new(format!(
                    "--mode manual conflicts with --checker-command for gate '{}': a manual gate cannot have a checker. Drop --checker-command, or omit --mode to define an automated gate.", key
                )).into());
            }
            Some(explicit) => explicit,
            None if checker_command.is_some() => GateMode::Auto,
            None => GateMode::Manual,
        }
    };
    let env = parse_env(env)?;
    let checker = checker_command.map(|command| GateChecker::Exec {
        command,
        timeout_seconds: timeout,
        working_dir,
        env,
        pass_context,
        prompt,
        prompt_file,
    });
    ctx.executor
        .gates()
        .define_gate(&key, title, description, stage, mode, checker, priority)?;
    Ok(GateMutationResponse::new(key, "Defined").into())
}

#[allow(clippy::too_many_arguments)]
fn update<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    key: String,
    title: Option<String>,
    description: Option<String>,
    stage: Option<GateStage>,
    mode: Option<GateMode>,
    auto: bool,
    checker_command: Option<String>,
    timeout: Option<u64>,
    working_dir: Option<String>,
    clear_working_dir: bool,
    pass_context: Option<bool>,
    prompt: Option<String>,
    clear_prompt: bool,
    prompt_file: Option<String>,
    clear_prompt_file: bool,
    env: Vec<String>,
    clear_env: bool,
    priority: Option<u32>,
) -> HandlerResult<GateDefinitionResponse> {
    let key = key.parse()?;
    let working_dir = field_edit(working_dir, clear_working_dir, "working-dir")?;
    let prompt = field_edit(prompt, clear_prompt, "prompt")?;
    let prompt_file = field_edit(prompt_file, clear_prompt_file, "prompt-file")?;
    let env = if env.is_empty() {
        None
    } else {
        Some(parse_env(env)?)
    };
    let update = GateUpdate {
        title,
        description,
        stage,
        mode: if auto { Some(GateMode::Auto) } else { mode },
        priority,
        checker_command,
        timeout,
        working_dir,
        pass_context,
        prompt,
        prompt_file,
        env: field_edit(env, clear_env, "env")?,
    };
    if update.is_empty() {
        return Err(InvalidArgumentError::new(
            "no fields to update; provide at least one field to change \
             (e.g. --title, --description, --mode, --checker-command)",
        )
        .into());
    }
    let (gate, warnings) = ctx.executor.gates().update_gate(&key, update)?;
    let message = format!("Updated gate '{}'", gate.key);
    Ok(
        Outcome::success_with(GateDefinitionResponse::updated(gate), message)
            .with_warnings(warnings),
    )
}

fn remove<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    key: String,
) -> HandlerResult<GateMutationResponse> {
    let key = key.parse()?;
    let warnings = ctx.executor.gates().remove_gate_definition(&key)?;
    Ok(Outcome::success(GateMutationResponse::new(key, "Removed")).with_warnings(warnings))
}

fn add<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
    gate_keys: Vec<String>,
) -> HandlerResult<GateAddResponse> {
    let keys = gate_keys
        .into_iter()
        .map(|key| key.parse())
        .collect::<Result<Vec<GateKey>, _>>()?;
    let full_id = resolve_issue(ctx.executor.storage(), &id)?;
    let (result, warnings) = ctx.executor.gates().add_gates(&full_id, &keys)?;
    Ok(
        Outcome::success(GateAddResponse::new(id, result.added, result.already_exist))
            .with_warnings(warnings),
    )
}

fn list<S: IssueStore>(ctx: &CommandContext<'_, S>) -> HandlerResult<GateListResponse> {
    let gates = ctx.executor.gates().list_gates()?;
    let message = format!("{} gate definition(s)", gates.len());
    Ok(Outcome::success_with(
        GateListResponse::new(gates.into_iter().map(GateDefinition::from).collect()),
        message,
    ))
}

fn show<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    key: String,
) -> HandlerResult<GateDefinitionResponse> {
    let gate = ctx.executor.gates().show_gate_definition(&key.parse()?)?;
    let message = format!("Gate {}: {}", gate.key, gate.title);
    Ok(Outcome::success_with(
        GateDefinitionResponse::shown(gate),
        message,
    ))
}

fn preset_list<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
) -> HandlerResult<GatePresetListResponse> {
    let presets = ctx.executor.gates().list_gate_presets()?;
    let message = format!("{} preset(s)", presets.len());
    Ok(Outcome::success_with(
        GatePresetListResponse::new(presets),
        message,
    ))
}

fn preset_show<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    name: String,
) -> HandlerResult<GatePresetShowResponse> {
    let preset = ctx.executor.gates().show_gate_preset(&name)?;
    let message = format!("Preset {}: {}", preset.name, preset.description);
    Ok(Outcome::success_with(preset.into(), message))
}

fn preset_create<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    from_issue: String,
    name: String,
) -> HandlerResult<GatePresetCreateResponse> {
    let id = resolve_issue(ctx.executor.storage(), &from_issue)?;
    let path = ctx.executor.gates().create_gate_preset(&name, &id)?;
    let message = format!("Created preset '{}'", name);
    Ok(Outcome::success_with(
        GatePresetCreateResponse {
            name,
            path: path.display().to_string(),
        },
        message,
    ))
}

fn preset_apply<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    name: String,
    ids: Vec<String>,
    timeout: Option<u64>,
    no_precheck: bool,
    no_postcheck: bool,
    except: Vec<String>,
) -> HandlerResult<GatePresetApplyResponse> {
    let except = except
        .into_iter()
        .map(|key| key.parse())
        .collect::<Result<Vec<GateKey>, _>>()?;
    let mut report = GatePresetApplyResponse {
        preset: name,
        success: Vec::new(),
        errors: Vec::new(),
    };
    let mut warnings = Vec::new();
    let mut lock_timed_out = false;
    for id in ids {
        let resolved = resolve_issue(ctx.executor.storage(), &id);
        if let Ok(issue) = &resolved {
            // A foreign lease is an authorization failure, not a per-gate error.
            ctx.executor.refuse_foreign_lease(issue)?;
        }
        match resolved.and_then(|id| {
            ctx.executor.gates().apply_gate_preset(
                &id,
                &report.preset,
                timeout,
                no_precheck,
                no_postcheck,
                &except,
            )
        }) {
            Ok((result, issue_warnings)) => {
                warnings.extend(
                    issue_warnings
                        .into_iter()
                        .map(|warning| format!("({id}): {warning}")),
                );
                report.success.push(GatePresetSuccess {
                    issue_id: id,
                    gates_added: result.added,
                    already_existed: result.already_exist,
                });
            }
            Err(error) => {
                lock_timed_out |= crate::storage::is_lock_timeout(&error);
                report.errors.push(GatePresetError {
                    issue_id: id,
                    error: error.to_string(),
                });
            }
        }
    }
    let outcome = if report.errors.is_empty() {
        let message = format!(
            "Applied preset '{}' to {} issue(s)",
            report.preset,
            report.success.len()
        );
        Outcome::success_with(report, message)
    } else {
        Outcome::findings(
            if lock_timed_out {
                ErrorCode::LockTimeout
            } else {
                ErrorCode::PresetError
            },
            format!(
                "Failed to apply preset '{}' to {} issue(s)",
                report.preset,
                report.errors.len()
            ),
            report,
        )
    };
    Ok(outcome.with_warnings(warnings))
}

command_group! {
    command: GateCommands,
    storage: [HandlerStore],
    leaves: {
        "gate_define" => GateCommands::Define { key, title, description, stage, mode, auto, checker_command, timeout, working_dir, pass_context, prompt, prompt_file, env, priority } => define(key, title, description, stage, mode, auto, checker_command, timeout, working_dir, pass_context, prompt, prompt_file, env, priority) -> GateMutationResponse,
        "gate_update" => GateCommands::Update { key, title, description, stage, mode, auto, checker_command, timeout, working_dir, clear_working_dir, pass_context, prompt, clear_prompt, prompt_file, clear_prompt_file, env, clear_env, priority } => update(key, title, description, stage, mode, auto, checker_command, timeout, working_dir, clear_working_dir, pass_context, prompt, clear_prompt, prompt_file, clear_prompt_file, env, clear_env, priority) -> GateDefinitionResponse,
        "gate_remove" => GateCommands::Remove { key } => remove(key) -> GateMutationResponse,
        "gate_add" => GateCommands::Add { id, gate_keys } => add(id, gate_keys) -> GateAddResponse,
        "gate_list" => GateCommands::List => list() -> GateListResponse,
        "gate_show" => GateCommands::Show { key } => show(key) -> GateDefinitionResponse,
        "gate_preset_list" => GateCommands::Preset(PresetCommands::List) => preset_list() -> GatePresetListResponse,
        "gate_preset_show" => GateCommands::Preset(PresetCommands::Show { name }) => preset_show(name) -> GatePresetShowResponse,
        "gate_preset_apply" => GateCommands::Preset(PresetCommands::Apply { name, ids, timeout, no_precheck, no_postcheck, except }) => preset_apply(name, ids, timeout, no_precheck, no_postcheck, except) -> GatePresetApplyResponse,
        "gate_preset_create" => GateCommands::Preset(PresetCommands::Create { from_issue, name }) => preset_create(from_issue, name) -> GatePresetCreateResponse,
    },
    unmigrated: GateCommands::Status { .. } | GateCommands::StatusAll { .. } |
        GateCommands::Evaluate { .. } | GateCommands::EvaluateAll { .. } |
        GateCommands::EvaluateMany { .. } | GateCommands::Fail { .. }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::OutputSchema;

    #[test]
    fn test_output_schema_gate_leaves_publish_registered_response_types() {
        for (path, expected) in [
            ("gate_define", OutputSchema::of::<GateMutationResponse>()),
            ("gate_update", OutputSchema::of::<GateDefinitionResponse>()),
            ("gate_remove", OutputSchema::of::<GateMutationResponse>()),
            ("gate_add", OutputSchema::of::<GateAddResponse>()),
            ("gate_list", OutputSchema::of::<GateListResponse>()),
            ("gate_show", OutputSchema::of::<GateDefinitionResponse>()),
            (
                "gate_preset_list",
                OutputSchema::of::<GatePresetListResponse>(),
            ),
            (
                "gate_preset_show",
                OutputSchema::of::<GatePresetShowResponse>(),
            ),
            (
                "gate_preset_apply",
                OutputSchema::of::<GatePresetApplyResponse>(),
            ),
            (
                "gate_preset_create",
                OutputSchema::of::<GatePresetCreateResponse>(),
            ),
        ] {
            let actual = crate::dispatch::registered_output_schema(path).unwrap();
            assert_eq!(actual.success, expected.success, "{path}");
            assert_eq!(actual.success_schema, expected.success_schema, "{path}");
        }
    }
}
