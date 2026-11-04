//! Gate definitions, presets, status and evaluation.

use super::{
    resolve_gate_key, resolve_gate_key_for, resolve_issue, CommandContext, HandlerResult,
    HandlerStore, Outcome,
};
use crate::cli::{GateCommands, PresetCommands};
use crate::commands::{FieldEdit, GateStatusArgumentsTransposedError, GateUpdate};
use crate::declarations::{GateChecker, GateMode, GateStage};
use crate::domain::identity::{GateKey, IssueId};
use crate::domain::{GateRunResult, GateRunStatus, GateStatus};
use crate::errors::InvalidArgumentError;
use crate::output::failure::classify;
use crate::output::gate::{
    ForIssue, GateAddResponse, GateDefinitionResponse, GateEvaluateAllResponse,
    GateEvaluateResponse, GateFailResponse, GateMutationResponse, GateNeverRun,
    GatePresetApplyResponse, GatePresetCreateResponse, GatePresetError, GatePresetShowResponse,
    GatePresetSuccess, GateStatusOutput,
};
use crate::output::{
    BatchTarget, ErrorCode, GateCheckAllResponse, GateCheckSummary, GateDefinition,
    GateEvaluateManyResponse, GateEvaluateManyResult, GateFindingsResponse, GateFlatReportResponse,
    GateListResponse, GatePresetListResponse, GateRunHistoryResponse, GateRunSummary,
    GateStatusEntry,
};
use crate::storage::IssueStore;
use std::collections::HashMap;
use std::str::FromStr;

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
    Ok(Outcome::from(GateMutationResponse::new(key, "Removed")).with_warnings(warnings))
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
        Outcome::from(GateAddResponse::new(id, result.added, result.already_exist))
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

/// The last `tail` lines of `text`, or all of it without `--tail`.
fn tail_lines(text: &str, tail: Option<usize>) -> String {
    tail.map_or_else(
        || text.to_owned(),
        |n| {
            let lines: Vec<&str> = text.lines().collect();
            lines[lines.len().saturating_sub(n)..].join("\n")
        },
    )
}

/// `id` resolved to an issue; a gate key in its place with an issue in the
/// gate position is reported as transposed.
fn resolve_status_issue<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: &str,
    gate_key: &str,
) -> anyhow::Result<IssueId> {
    resolve_issue(ctx.executor.storage(), id).map_err(|error| {
        if ctx
            .executor
            .gates()
            .are_gate_status_arguments_transposed(id, gate_key)
        {
            GateStatusArgumentsTransposedError {
                issue: gate_key.to_owned(),
                gate_key: id.to_owned(),
            }
            .into()
        } else {
            error
        }
    })
}

fn never_run(key: GateKey, issue_id: IssueId) -> Outcome<GateStatusOutput> {
    let never_run = GateNeverRun { key, issue_id };
    let message = never_run.text();
    Outcome::success_with(GateStatusOutput::NeverRun(never_run), message)
}

#[allow(clippy::too_many_arguments)]
fn gate_status<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
    gate_key: Option<String>,
    gate_flag: Option<String>,
    all: bool,
    limit: Option<usize>,
    status: Option<String>,
    stdout: bool,
    stderr: bool,
    tail: Option<usize>,
    findings: bool,
) -> HandlerResult<GateStatusOutput> {
    let history = all || limit.is_some();
    let flat = stdout || stderr || tail.is_some();
    if [history, flat, findings]
        .into_iter()
        .filter(|on| *on)
        .count()
        > 1
    {
        return Err(InvalidArgumentError::new(
            "history (--all/--limit), flat-output (--stdout/--stderr/--tail), and findings \
             (--findings) views are mutually exclusive.",
        )
        .into());
    }
    if status.is_some() && !history {
        return Err(InvalidArgumentError::new(
            "--status only applies to the history view; pass --all or --limit.",
        )
        .into());
    }
    let gates = ctx.executor.gates();
    if history {
        let filter = match (gate_key, gate_flag) {
            (None, None) => None,
            (positional, flag) => Some(resolve_gate_key_for(positional, flag, "gate status")?),
        };
        let wanted = status
            .map(|value| {
                let value = value.to_ascii_lowercase();
                GateRunStatus::from_str(&value).map_err(|_| {
                    InvalidArgumentError::new(format!(
                        "unknown --status value '{value}'; expected one of: {}.",
                        GateRunStatus::ALL.map(GateRunStatus::as_str).join(", ")
                    ))
                })
            })
            .transpose()?;
        let issue_id = resolve_issue(ctx.executor.storage(), &id)?;
        let runs = gates
            .list_gate_runs(&issue_id, filter.as_ref())?
            .into_iter()
            .filter(|run| wanted.is_none_or(|status| run.status == status))
            .take(limit.unwrap_or(usize::MAX))
            .map(|run| GateRunSummary::full(&run))
            .collect::<Vec<_>>();
        let message = format!("{} gate run(s) listed for issue {issue_id}", runs.len());
        let history = GateStatusOutput::History(ForIssue {
            issue_id,
            response: GateRunHistoryResponse::new(runs),
        });
        return Ok(Outcome::success_with(history, message));
    }
    if findings || flat {
        let key = resolve_gate_key_for(gate_key, gate_flag, "gate status")?;
        let issue_id = resolve_issue(ctx.executor.storage(), &id)?;
        return Ok(match gates.get_last_gate_run(&issue_id, &key)? {
            None => never_run(key, issue_id),
            Some(run) if findings => {
                GateStatusOutput::Findings(GateFindingsResponse::of_run(&run)).into()
            }
            // `--stderr` alone shows only stderr; the tail-only form includes stdout.
            Some(run) => GateStatusOutput::Report(GateFlatReportResponse {
                key,
                run_id: run.run_id.clone(),
                stdout: (stdout || !stderr).then(|| tail_lines(&run.stdout, tail)),
                stderr: stderr.then(|| tail_lines(&run.stderr, tail)),
            })
            .into(),
        });
    }
    let key = resolve_gate_key(gate_key, gate_flag, "gate status")?;
    let issue_id = resolve_status_issue(ctx, &id, &key)?;
    let key: GateKey = key.parse()?;
    Ok(match gates.get_last_gate_run(&issue_id, &key)? {
        None => never_run(key, issue_id),
        Some(run) => {
            let message = format!("Gate '{key}': {}", run.status);
            Outcome::success_with(
                GateStatusOutput::Latest(Box::new(GateRunSummary::full(&run))),
                message,
            )
        }
    })
}

fn status_all<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
    full: bool,
) -> HandlerResult<ForIssue<GateCheckAllResponse>> {
    let issue_id = resolve_issue(ctx.executor.storage(), &id)?;
    let gates = ctx.executor.gates();
    let (results, _) = gates.get_last_gate_runs_for_issue(&issue_id)?;
    // Readiness across every required gate, automated and manual.
    let statuses = gates.get_required_gate_statuses_for_issue(&issue_id)?;
    let total = statuses.len();
    let passed = statuses
        .iter()
        .filter(|(_, status)| *status == GateStatus::Passed)
        .count();
    let not_run: Vec<GateKey> = statuses
        .iter()
        .filter(|(_, status)| *status == GateStatus::Pending)
        .map(|(key, _)| key.clone())
        .collect();
    let all_passed = total == passed;
    let message = if all_passed {
        format!("{passed}/{total} required gates passed")
    } else {
        format!(
            "{passed}/{total} required gates passed ({} pending, {} failed)",
            not_run.len(),
            total - passed - not_run.len()
        )
    };
    // Human output always shows each run's report text.
    let summarize = |run: &GateRunResult| {
        if full || !ctx.mode.json {
            GateRunSummary::full(run)
        } else {
            GateRunSummary::lean(run)
        }
    };
    let response = ForIssue {
        issue_id,
        response: GateCheckAllResponse::with_extras(
            statuses
                .into_iter()
                .map(|(key, status)| GateStatusEntry { key, status })
                .collect(),
            GateCheckSummary {
                results: results.iter().map(summarize).collect(),
                passed,
                total,
                not_run,
                all_passed,
            },
        ),
    };
    Ok(if all_passed {
        Outcome::success_with(response, message)
    } else {
        Outcome::findings(ErrorCode::GateFailed, message, response)
    })
}

fn evaluate<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
    gate_key: Option<String>,
    gate_flag: Option<String>,
    by: Option<String>,
    force: bool,
) -> HandlerResult<GateEvaluateResponse> {
    let key = resolve_gate_key_for(gate_key, gate_flag, "gate evaluate")?;
    let full_id = resolve_issue(ctx.executor.storage(), &id)?;
    let outcome = ctx.executor.gates().pass_gate(&full_id, &key, by, force)?;
    Ok(Outcome::from(GateEvaluateResponse::new(id, key)).with_warnings(outcome.warnings))
}

fn evaluate_all<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
    by: Option<String>,
    force: bool,
) -> HandlerResult<GateEvaluateAllResponse> {
    let full_id = resolve_issue(ctx.executor.storage(), &id)?;
    let results = ctx
        .executor
        .gates()
        .pass_all_gates(&full_id, by, force)?
        .results;
    let warnings = results
        .iter()
        .flat_map(|entry| {
            let key = &entry.gate_key;
            entry
                .warnings
                .iter()
                .map(move |warning| format!("({key}): {warning}"))
        })
        .collect::<Vec<_>>();
    let keys = results.into_iter().map(|entry| entry.gate_key).collect();
    Ok(Outcome::from(GateEvaluateAllResponse::new(id, keys)).with_warnings(warnings))
}

/// Each requested issue in argument order, an unresolved id failing its own
/// entry; any failure makes the batch findings under the first failure's code.
/// Another agent's lease on any issue refuses the whole batch.
fn evaluate_many<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    gate_key: String,
    ids: Vec<String>,
    by: Option<String>,
    force: bool,
) -> HandlerResult<GateEvaluateManyResponse> {
    let key: GateKey = gate_key.parse()?;
    let resolved: Vec<(String, anyhow::Result<IssueId>)> = ids
        .into_iter()
        .map(|requested| {
            let id = resolve_issue(ctx.executor.storage(), &requested);
            (requested, id)
        })
        .collect();
    let full_ids: Vec<IssueId> = resolved
        .iter()
        .filter_map(|(_, id)| id.as_ref().ok().cloned())
        .collect();
    // A foreign lease is an authorization failure, not a per-issue error.
    full_ids
        .iter()
        .try_for_each(|id| ctx.executor.refuse_foreign_lease(id).map(drop))?;
    let mut evaluated = ctx
        .executor
        .gates()
        .pass_gate_many(&full_ids, &key, by, force)
        .results
        .into_iter();
    let mut warnings = Vec::new();
    let mut first_failure = None;
    let mut results = Vec::with_capacity(resolved.len());
    for (requested, id) in resolved {
        let (target, result) = match id {
            Ok(_) => match evaluated.next() {
                Some(entry) => (BatchTarget::Resolved(entry.issue_id), entry.result),
                None => continue,
            },
            Err(error) => (BatchTarget::Unresolved(requested), Err(error)),
        };
        let (entry_warnings, entry) = match result {
            Ok(outcome) => (
                outcome.warnings,
                GateEvaluateManyResult::passed(target, key.clone()),
            ),
            Err(error) => {
                let failure = classify(&error);
                first_failure.get_or_insert(failure.error.code);
                let entry = GateEvaluateManyResult::failed(target, key.clone(), &failure)?;
                (failure.warnings, entry)
            }
        };
        let subject = entry.target.as_str();
        warnings.extend(
            entry_warnings
                .into_iter()
                .map(|warning| format!("({subject}): {warning}")),
        );
        results.push(entry);
    }
    let failed = results.iter().filter(|entry| entry.error.is_some()).count();
    let message = format!(
        "Evaluated gate '{key}' for {} issue(s): {} passed, {failed} failed",
        results.len(),
        results.len() - failed
    );
    let response = GateEvaluateManyResponse::new(results);
    let outcome = match first_failure {
        None => Outcome::success_with(response, message),
        Some(code) => Outcome::findings(code, message, response),
    };
    Ok(outcome.with_warnings(warnings))
}

fn fail<S: HandlerStore>(
    ctx: &CommandContext<'_, S>,
    id: String,
    gate_key: String,
    by: Option<String>,
) -> HandlerResult<GateFailResponse> {
    let key = gate_key.parse()?;
    let full_id = resolve_issue(ctx.executor.storage(), &id)?;
    let warnings = ctx.executor.gates().fail_gate(&full_id, &key, by)?;
    Ok(Outcome::from(GateFailResponse::new(id, key)).with_warnings(warnings))
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
        "gate_status" => GateCommands::Status { id, gate_key, gate_flag, all, limit, status, stdout, stderr, tail, findings } => gate_status(id, gate_key, gate_flag, all, limit, status, stdout, stderr, tail, findings) -> GateStatusOutput
            as union [GateRunSummary, GateRunHistoryResponse, GateFlatReportResponse, GateFindingsResponse, GateNeverRun]
            "Default shape: the gate's latest run. With --all or --limit: the count-wrapped run history. With --stdout, --stderr or --tail: the stored report text. With --findings: the structured findings and verdict. GateNeverRun when the selected gate has not run.",
        "gate_status-all" => GateCommands::StatusAll { id, full } => status_all(id, full) -> ForIssue<GateCheckAllResponse>,
        "gate_evaluate" => GateCommands::Evaluate { id, gate_key, gate_flag, by, force } => evaluate(id, gate_key, gate_flag, by, force) -> GateEvaluateResponse,
        "gate_evaluate-all" => GateCommands::EvaluateAll { id, by, force } => evaluate_all(id, by, force) -> GateEvaluateAllResponse,
        "gate_evaluate-many" => GateCommands::EvaluateMany { gate_key, ids, by, force } => evaluate_many(gate_key, ids, by, force) -> GateEvaluateManyResponse,
        "gate_fail" => GateCommands::Fail { id, gate_key, by } => fail(id, gate_key, by) -> GateFailResponse,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{memory_executor, seed_gate_registry, seed_issue};
    use crate::commands::CommandExecutor;
    use crate::declarations::{GateDefinition as Declared, GateMode, GateStage};
    use crate::output::failure::classify;
    use crate::output::{ExitCode, OutputMode};
    use crate::schema::OutputSchema;
    use crate::storage::InMemoryStorage;

    /// An executor whose one issue requires the manual gate `tests`; returns the issue id.
    fn requiring_manual_gate() -> (CommandExecutor<InMemoryStorage>, String) {
        let executor = memory_executor(InMemoryStorage::new());
        let mut registry = executor.storage().load_gate_registry().unwrap();
        let key: GateKey = "tests".parse().unwrap();
        registry.gates.insert(
            key.clone(),
            Declared {
                version: 1,
                key: key.clone(),
                title: "Tests".to_owned(),
                description: String::new(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
                inputs: None,
                priority: 100,
                reserved: HashMap::new(),
            },
        );
        seed_gate_registry(executor.storage(), &registry);
        let issue = crate::test_utils::fixture_issue("Gated".to_owned(), String::new());
        let id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor.gates().add_gate(&id, &key).unwrap();
        (executor, id.to_string())
    }

    fn run(
        executor: &CommandExecutor<InMemoryStorage>,
        command: GateCommands,
    ) -> anyhow::Result<(ExitCode, serde_json::Value)> {
        let ctx = CommandContext {
            executor,
            mode: OutputMode {
                json: true,
                quiet: false,
            },
        };
        let mut out = Vec::new();
        let status = dispatch(command, &ctx, &mut out, &mut Vec::new())?;
        Ok((status, serde_json::from_slice(&out).unwrap()))
    }

    fn status_args(id: &str, gate_key: Option<&str>) -> GateCommands {
        GateCommands::Status {
            id: id.to_owned(),
            gate_key: gate_key.map(str::to_owned),
            gate_flag: None,
            all: false,
            limit: None,
            status: None,
            stdout: false,
            stderr: false,
            tail: None,
            findings: false,
        }
    }

    #[test]
    fn test_dispatch_status_all_with_an_unrun_gate_fails_with_the_report_in_error_details() {
        let (executor, id) = requiring_manual_gate();

        let (status, printed) =
            run(&executor, GateCommands::StatusAll { id, full: false }).unwrap();

        assert_eq!(status.code(), 4);
        assert_eq!(printed["error"]["code"], "GATE_FAILED");
        let details = &printed["error"]["details"];
        assert_eq!(details["all_passed"], false);
        assert_eq!(details["not_run"], serde_json::json!(["tests"]));
        assert_eq!(details["gates"][0]["status"], "pending");
    }

    #[test]
    fn test_dispatch_evaluate_many_with_failing_entries_fails_with_the_first_failures_code_and_results(
    ) {
        let (executor, id) = requiring_manual_gate();
        let missing = crate::test_utils::issue_id(0x99).to_string();
        let ids = ["!", id.as_str(), missing.as_str()]
            .map(str::to_owned)
            .to_vec();

        let (status, printed) = run(
            &executor,
            GateCommands::EvaluateMany {
                gate_key: "tests".to_owned(),
                ids,
                by: Some("human:reviewer".to_owned()),
                force: false,
            },
        )
        .unwrap();

        let first = ErrorCode::InvalidIdPrefix;
        assert_eq!(printed["error"]["code"], first.as_str());
        assert_eq!(status, first.exit_code());
        let results = printed["error"]["details"]["results"].as_array().unwrap();
        let statuses: Vec<_> = results.iter().map(|entry| &entry["status"]).collect();
        assert_eq!(statuses, ["failed", "passed", "failed"]);
        assert_eq!(results[0]["error"]["code"], first.as_str());
        assert_eq!(results[1]["issue_id"], id);
        assert_eq!(
            results[2]["error"]["code"],
            ErrorCode::IssueNotFound.as_str()
        );
    }

    #[test]
    fn test_dispatch_status_of_a_gate_never_run_prints_a_typed_object_with_the_explanation() {
        let (executor, id) = requiring_manual_gate();

        let (status, printed) = run(&executor, status_args(&id, Some("tests"))).unwrap();

        assert_eq!(status, ExitCode::Success);
        assert_eq!(printed["key"], "tests");
        assert_eq!(printed["issue_id"], id);
        assert!(printed["message"]
            .as_str()
            .unwrap()
            .contains("has not been run yet"));
    }

    #[test]
    fn test_dispatch_status_of_an_unknown_issue_fails_with_the_classifiers_code() {
        let (executor, _) = requiring_manual_gate();
        let missing = crate::test_utils::issue_id(0x99).to_string();

        let error = run(&executor, status_args(&missing, Some("tests"))).unwrap_err();

        let envelope = classify(&error);
        assert_eq!(envelope.error.code, ErrorCode::IssueNotFound);
        assert_ne!(envelope.error.code, ErrorCode::GateCheckError);
    }

    #[test]
    fn test_dispatch_status_with_transposed_arguments_is_an_invalid_argument() {
        let (executor, id) = requiring_manual_gate();

        let error = run(&executor, status_args("tests", Some(&id))).unwrap_err();

        assert!(error.is::<GateStatusArgumentsTransposedError>());
        let envelope = classify(&error);
        assert_eq!(envelope.error.code, ErrorCode::InvalidArgument);
        assert_eq!(envelope.exit_code().code(), 2);
    }

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
            ("gate_evaluate", OutputSchema::of::<GateEvaluateResponse>()),
            (
                "gate_evaluate-all",
                OutputSchema::of::<GateEvaluateAllResponse>(),
            ),
            (
                "gate_evaluate-many",
                OutputSchema::of::<GateEvaluateManyResponse>(),
            ),
            ("gate_fail", OutputSchema::of::<GateFailResponse>()),
        ] {
            let actual = crate::dispatch::registered_output_schema(path).unwrap();
            assert_eq!(actual.success, expected.success, "{path}");
            assert_eq!(actual.success_schema, expected.success_schema, "{path}");
        }
    }
}
