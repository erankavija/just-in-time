//! Just-In-Time Issue Tracker
//!
//! A repository-local CLI issue tracker with dependency graph enforcement and quality gating.
//! Designed for deterministic, machine-friendly outputs and process automation.
//!
//! # Features
//!
//! - Dependency graph modeling with cycle detection
//! - Quality gate enforcement before state transitions
//! - Event logging for full audit trail
//! - Priority-based issue management
//! - Agent coordination support

// Binary-specific module (not in library)
mod output_macros;

use anyhow::{Context, Result};
use clap::Parser;
use jit::cli::{Cli, Commands, GateCommands, IssueCommands, ProfileCommands};
use jit::commands::{CommandExecutor, DescriptionUpdate, ProfileVariableOptions};
use jit::dispatch::{
    apply, archive, claim, config, dep, doc, events, graph, hooks, init, invariant, item, label,
    project, resolve_issue, search, serve, validate, version, CommandContext,
};
use jit::domain::identity::{GateKey, IssueId, IssueIdPrefix};
use jit::domain::{Assignee, GateRunResult, Priority, State};
use jit::output::failure::{classify, command_json_error, profile_json_error};
use jit::output::{
    plain_issue_rows, print_issue_list, ErrorCode, ExitCode, IssueColumn, JsonError, JsonOutput,
    OutputContext, OutputMode,
};
use jit::storage::{IssueStore, JsonFileStorage};
use std::env;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;

/// Render completed findings under the machine-readable contract.
///
/// Success keeps its command-specific report shape. A failure moves that same report under the canonical envelope's
/// `error.details`; the registered code is the sole authority for the process
/// status. The owning command writes exactly one payload, so the top-level
/// renderer cannot append a second document.
fn render_findings_json(
    details: serde_json::Value,
    message: String,
    failure_code: Option<ErrorCode>,
) -> Result<()> {
    if let Some(code) = failure_code {
        let output = JsonError::new(code, message).with_details(details);
        println!("{}", output.to_json_string()?);
        std::process::exit(output.exit_code().code());
    }

    let output = JsonOutput::success(details).with_message(message);
    println!("{}", output.to_json_string()?);
    Ok(())
}

/// Render a failed `gate evaluate` / `gate evaluate-all` outcome and terminate appropriately.
///
/// Shared by the `gate evaluate` and `gate evaluate-all` handlers so both classify the
/// same error the same way. In `--json` mode it prints a structured
/// [`JsonError`](jit::output::JsonError) and exits with its mapped code:
/// `GatePassFailed` becomes a checker failure (`GATE_FAILED`, exit 4, verdict
/// `fail`) or a runner error (`IO_ERROR`, exit 10, verdict `error`) per the
/// carried [`GateRunStatus`](jit::domain::GateRunStatus); `GateNotRequiredError`
/// becomes `INVALID_ARGUMENT` (exit 2); an unresolved id becomes
/// [`ErrorCode::IssueNotFound`] (exit 3); anything else [`ErrorCode::GateError`]. In non-JSON mode it
/// surfaces any gate-failure warnings and returns `Err(e)` so the top-level
/// handler maps the exit code via [`classify`], so both output modes
/// agree.
fn render_gate_pass_error(
    e: anyhow::Error,
    requested: &str,
    output_ctx: &OutputContext,
    json: bool,
) -> Result<()> {
    if !json {
        if let Some(gate_failure) = e.downcast_ref::<jit::commands::GatePassFailed>() {
            for warning in &gate_failure.warnings {
                output_ctx.print_warning(warning)?;
            }
        }
        return Err(e);
    }

    let json_error = gate_pass_json_error(&e, requested);
    println!("{}", json_error.to_json_string()?);
    std::process::exit(json_error.exit_code().code());
}

/// Report a `gate status` failure: under `--json` print its
/// `GATE_CHECK_ERROR` envelope and exit; otherwise return it.
fn report_gate_check_error<T>(e: anyhow::Error, json: bool) -> Result<T> {
    if json {
        let json_error = command_json_error(&e, ErrorCode::GateCheckError);
        println!("{}", json_error.to_json_string()?);
        std::process::exit(json_error.exit_code().code());
    }
    Err(e)
}

/// Build the structured error used by both single-issue and multi-issue gate
/// evaluation. Keeping the classifier shared preserves the `gate evaluate`
/// taxonomy when a batch contains checker, runner, lookup, or argument errors.
fn gate_pass_json_error(e: &anyhow::Error, requested: &str) -> jit::output::JsonError {
    use jit::commands::{GateNotRequiredError, GatePassFailed, ManualGateAttestationRequiredError};
    if e.is::<GatePassFailed>()
        || e.is::<GateNotRequiredError>()
        || e.is::<ManualGateAttestationRequiredError>()
    {
        classify(e)
    } else if e.is::<jit::storage::IssueNotFoundError>() {
        // Pre-verdict lookup error: the request text, not the resolved id,
        // names what did not resolve.
        JsonError::issue_not_found(requested)
    } else {
        command_json_error(e, ErrorCode::GateError)
    }
}

fn parse_gate_key(value: String, json: bool) -> Result<GateKey> {
    value
        .parse::<GateKey>()
        .map_err(|error| invalid_argument(error.to_string(), json))
}

fn parse_gate_keys(values: Vec<String>, json: bool) -> Result<Vec<GateKey>> {
    values
        .into_iter()
        .map(|value| parse_gate_key(value, json))
        .collect()
}

/// Build an `INVALID_ARGUMENT` error, rendering it `--json`-aware.
///
/// A `--json` caller emits a machine-readable
/// `INVALID_ARGUMENT` JSON error (exit 2) and the process exits; a non-`--json`
/// caller receives the typed [`InvalidArgumentError`](jit::errors::InvalidArgumentError)
/// so the top-level handler classifies it (exit 2) and prints the plain message.
/// Never returns when `json` is true.
fn invalid_argument(message: String, json: bool) -> anyhow::Error {
    if json {
        let json_error =
            jit::output::JsonError::new(jit::output::ErrorCode::InvalidArgument, message);
        if let Ok(s) = json_error.to_json_string() {
            println!("{}", s);
        }
        std::process::exit(json_error.exit_code().code());
    }
    jit::errors::InvalidArgumentError::new(message).into()
}

/// One invocation of `jit profile apply`, as the CLI parsed it.
struct ProfileApplyInvocation<'a> {
    /// Repeatable raw `--profile` selectors in occurrence order.
    profile: &'a [String],
    /// Optional `--values-file` path.
    values_file: Option<std::path::PathBuf>,
    /// Repeatable raw `--set NAME=VALUE` assignments in occurrence order.
    set: &'a [String],
    /// Whether this run is a rehearsal.
    dry_run: bool,
    /// Whether output is machine-readable.
    json: bool,
}

/// Dispatch `jit profile apply` through its rehearsal or its publication.
fn run_profile_apply(
    executor: &CommandExecutor<JsonFileStorage>,
    invocation: ProfileApplyInvocation<'_>,
) -> Result<()> {
    let ProfileApplyInvocation {
        profile,
        values_file,
        set,
        dry_run,
        json,
    } = invocation;
    let selectors = jit::commands::parse_profile_selectors(profile)?;
    let assignments = jit::commands::parse_profile_variable_assignments(set)?;
    if selectors.is_empty() {
        return Err(invalid_argument(
            "profile apply requires at least one --profile DIR selector".to_string(),
            json,
        ));
    }
    let options = ProfileVariableOptions {
        values_file,
        assignments,
    };
    let results = profile_result(
        executor
            .profiles()
            .select_profiles(&selectors, &options)
            .and_then(|selection| {
                if dry_run {
                    executor.profiles().plan_profiles(&selection)
                } else {
                    executor.profiles().apply_profiles(&selection)
                }
            }),
        json,
    )?;
    if json {
        println!("{}", JsonOutput::success(&results).to_json_string()?);
    } else {
        print_profile_results(&results);
    }
    Ok(())
}

/// Render every entry of one apply, rehearsal or difference run.
fn print_profile_results(results: &jit::profile::ProfileComposedApplyResult) {
    for profile in &results.items {
        print_profile_heading(
            &profile.id,
            &profile.version,
            &profile.origin,
            &profile_status_label(profile.status, "apply", "applied"),
        );
        print_profile_decisions(&profile.targets, &profile.contributions);
        if let Some(refusal) = &profile.refusal {
            println!("  refused: {refusal}");
        }
    }
}

/// Name one profile status in the verb its command answers in.
fn profile_status_label(
    status: jit::profile::ProfileApplicationStatus,
    verb: &str,
    past: &str,
) -> String {
    use jit::profile::ProfileApplicationStatus;
    match status {
        ProfileApplicationStatus::Unchanged => "unchanged".to_string(),
        ProfileApplicationStatus::WouldApply => format!("would {verb}"),
        ProfileApplicationStatus::WouldConflict => "would conflict".to_string(),
        ProfileApplicationStatus::Applied => past.to_string(),
    }
}

/// Render one decision: what it would do, what it is about, and why it cannot
/// be published when it cannot.
fn print_profile_decision(
    action: jit::profile::ProfileTargetAction,
    subject: &str,
    reason: Option<&str>,
) {
    println!("  {}: {subject}", profile_target_action_label(action));
    if let Some(reason) = reason {
        println!("      {reason}");
    }
}

/// Render one profile identity and operation outcome in the family-wide human
/// heading shape.
fn print_profile_heading(
    id: &jit::domain::identity::ProfileId,
    version: &semver::Version,
    origin: &jit::repository_state::RootRelativePath,
    status: &str,
) {
    println!(
        "Profile {id} {version} ({}): {status}",
        profile_origin_label(origin)
    );
}

/// Render file and declaration decisions through the one public action
/// vocabulary.
fn print_profile_decisions(
    targets: &[jit::profile::ProfileTargetChange],
    contributions: &[jit::profile::ProfileContributionChange],
) {
    for target in targets {
        print_profile_decision(target.action, &target.path, target.reason.as_deref());
    }
    for contribution in contributions {
        print_profile_decision(
            contribution.action,
            &contribution.identity,
            contribution.reason.as_deref(),
        );
    }
}

/// Name what a profile decided about one target or declaration, for human
/// output.
fn profile_target_action_label(action: jit::profile::ProfileTargetAction) -> &'static str {
    use jit::profile::ProfileTargetAction;
    match action {
        ProfileTargetAction::Unchanged => "unchanged",
        ProfileTargetAction::Create => "create",
        ProfileTargetAction::Update => "update",
        ProfileTargetAction::Remove => "remove",
        ProfileTargetAction::Conflict => "conflict",
    }
}

/// Render the difference report and terminate on an unpublishable decision.
///
/// The report is the answer either way, a conflicting decision is stated in full before the process exits, and
/// the exit status carries the repository-check contract — `0` when every
/// participating profile can be published, [`ExitCode::ValidationFailed`] when
/// one cannot. Nothing is written in either case.
fn render_profile_difference(
    plans: &jit::profile::ProfileComposedApplyResult,
    json: bool,
) -> Result<()> {
    let conflicted = plans.conflicted().count();
    if json {
        let details = serde_json::to_value(plans)?;
        let message = if conflicted == 0 {
            format!("{} profile(s) can be published", plans.count())
        } else {
            format!(
                "{conflicted} of {} profile(s) decided targets that cannot be published",
                plans.count()
            )
        };
        return render_findings_json(
            details,
            message,
            (conflicted > 0).then_some(ErrorCode::ValidationFailed),
        );
    }

    print_profile_results(plans);
    if conflicted == 0 {
        return Ok(());
    }
    eprintln!(
        "{conflicted} of {} profile(s) decided targets that cannot be published",
        plans.count()
    );
    std::process::exit(jit::ExitCode::ValidationFailed.code());
}

/// Render a package origin for human output, naming the directory a package
/// read from the repository came from.
fn profile_origin_label(origin: &jit::repository_state::RootRelativePath) -> String {
    format!("directory {}", origin.as_path().display())
}

fn profile_result<T>(result: anyhow::Result<T>, json: bool) -> anyhow::Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(error) if json => {
            let json_error = profile_json_error(&error)?;
            println!("{}", json_error.to_json_string()?);
            std::process::exit(json_error.exit_code().code());
        }
        Err(error) => Err(error),
    }
}

/// Wrong-command guess -> canonical-command hint, keyed by (group, guessed verb).
/// Consulted only when clap rejects a subcommand, so a wrong verb prints its
/// hint and clap's diagnostic on stderr and exits 2, outside the failure
/// envelope. See docs/reference/cli-commands.md's "Command and flag aliases"
/// section: these are hints, not aliases.
const COMMAND_HINTS: &[(&str, &str, &str)] = &[
    ("dep", "remove", "jit dep rm"),
    ("dep", "delete", "jit dep rm"),
    ("issue", "rm", "jit issue delete"),
    ("issue", "remove", "jit issue delete"),
    ("issue", "complete", "jit issue update <id> --state done"),
    ("issue", "edit", "jit issue update <id>"),
    ("gate", "rm", "jit gate remove"),
    ("gate", "delete", "jit gate remove"),
    ("doc", "rm", "jit doc remove"),
    ("doc", "delete", "jit doc remove"),
    (
        "label",
        "add",
        "jit issue update <id> --label <namespace:value>",
    ),
    (
        "label",
        "rm",
        "jit issue update <id> --remove-label <namespace:value>",
    ),
    (
        "label",
        "remove",
        "jit issue update <id> --remove-label <namespace:value>",
    ),
    ("invariant", "list", "jit item list --kind invariant"),
];

fn canonical_command_hint(group: &str, verb: &str) -> Option<&'static str> {
    COMMAND_HINTS
        .iter()
        .find_map(|(known_group, known_verb, hint)| {
            (*known_group == group && *known_verb == verb).then_some(*hint)
        })
}

/// Return the canonical command for a parser-level unknown subcommand, if the
/// rejected command is one of the known first-guess residuals.
fn unknown_subcommand_hint(error: &clap::Error, args: &[String]) -> Option<String> {
    if error.kind() != clap::error::ErrorKind::InvalidSubcommand {
        return None;
    }

    let mut positional = args
        .iter()
        .skip(1)
        .filter(|argument| !matches!(argument.as_str(), "-q" | "--quiet" | "--schema"));
    let group = positional.next()?;
    let verb = positional.next()?;
    canonical_command_hint(group, verb).map(|canonical| format!("Hint: use '{canonical}' instead."))
}

/// Read description content for `--description-file` / `--append-description-file`.
///
/// `path == "-"` reads stdin to completion instead of a file. Content is
/// returned verbatim (no trimming), so multi-kilobyte descriptions and any
/// deliberate trailing whitespace survive intact. Only one `-file` flag can
/// be given per invocation (they are mutually exclusive via clap), so stdin
/// is never read more than once.
fn read_description_source(path: &str) -> Result<String> {
    if path == "-" {
        std::io::read_to_string(std::io::stdin()).context("Failed to read description from stdin")
    } else {
        std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read description file: {path}"))
    }
}

/// Resolve the optional gate key from the positional-or-flag pair.
///
/// Unlike [`jit::dispatch::resolve_gate_key`], neither form is required: `Ok(None)` is a valid
/// result (used by the history view, where the gate key is only a filter).
/// Supplying BOTH is still an error, routed `--json`-aware via
/// [`invalid_argument`].
fn resolve_optional_gate_key(
    positional: Option<String>,
    flag: Option<String>,
    json: bool,
) -> Result<Option<GateKey>> {
    match (positional, flag) {
        (None, None) => Ok(None),
        (Some(pos), None) => parse_gate_key(pos, json).map(Some),
        (None, Some(flag_val)) => parse_gate_key(flag_val, json).map(Some),
        (Some(_), Some(_)) => Err(invalid_argument(
            "provide the gate key as a positional argument OR via --gate, not both.".to_string(),
            json,
        )),
    }
}

/// Parse a `--status` filter value into a [`GateRunStatus`].
///
/// Accepts the snake_case status names; an unknown value is an
/// `INVALID_ARGUMENT` error (routed `--json`-aware).
fn parse_run_status(value: &str, json: bool) -> Result<jit::domain::GateRunStatus> {
    use jit::domain::GateRunStatus;
    let value = value.to_ascii_lowercase();
    GateRunStatus::from_str(&value).map_err(|_| {
        invalid_argument(
            format!(
                "unknown --status value '{value}'; expected one of: {}.",
                GateRunStatus::ALL.map(GateRunStatus::as_str).join(", ")
            ),
            json,
        )
    })
}

/// Render a report stream verbatim, keeping only the last `tail` lines when set.
///
/// With no `--tail`, the stored text is returned unchanged. With `--tail N`, only
/// the final N lines are kept (rejoined with `\n`); the content is otherwise
/// undecorated.
fn render_report_stream(text: &str, tail: Option<usize>) -> String {
    match tail {
        Some(n) => {
            let lines: Vec<&str> = text.lines().collect();
            let start = lines.len().saturating_sub(n);
            lines[start..].join("\n")
        }
        None => text.to_string(),
    }
}

/// Render the structured-findings text view of a single gate run (`gate status
/// --findings`).
///
/// One header line carries the gate key, verdict, one-line summary, and finding
/// count; each finding follows on its own line as `<id> [<severity>]` plus any
/// `[<disposition>] [<origin>]` classifications, then `<summary>` and an
/// optional ` (<file>:<line>)` locator and a JSON-encoded `references` array.
/// JSON encoding preserves arbitrary reference strings without delimiter
/// ambiguity. Empty reference lists add no text. The format is stable and
/// greppable (severity via `[high]`, verdict via the header), mirroring the
/// `issue status` one-line convention. A run with no machine-readable block
/// renders a single header line with `verdict: n/a` and `findings: 0`, so a
/// plain-text checker degrades cleanly instead of erroring.
///
/// The returned string always ends with a newline.
fn render_gate_findings_text(result: &GateRunResult) -> String {
    match &result.findings {
        Some(f) => {
            let mut out = format!(
                "{} verdict: {} summary: {} findings: {}\n",
                result.gate_key,
                if f.verdict.is_empty() {
                    "unknown"
                } else {
                    &f.verdict
                },
                if f.summary.is_empty() {
                    "-"
                } else {
                    &f.summary
                },
                f.findings.len(),
            );
            for finding in &f.findings {
                let id = if finding.id.is_empty() {
                    "-"
                } else {
                    &finding.id
                };
                let severity = if finding.severity.is_empty() {
                    "-"
                } else {
                    &finding.severity
                };
                let classifications = [finding.disposition.as_deref(), finding.origin.as_deref()]
                    .into_iter()
                    .flatten()
                    .map(|value| format!(" [{}]", value))
                    .collect::<String>();
                let locator = match (&finding.file, finding.line) {
                    (Some(file), Some(line)) => format!(" ({}:{})", file, line),
                    (Some(file), None) => format!(" ({})", file),
                    _ => String::new(),
                };
                let references = if finding.references.is_empty() {
                    String::new()
                } else {
                    let encoded = serde_json::to_string(&finding.references)
                        .expect("serializing a string array cannot fail");
                    format!(" references: {encoded}")
                };
                out.push_str(&format!(
                    "{} [{}]{} {}{}{}\n",
                    id, severity, classifications, finding.summary, locator, references
                ));
            }
            out
        }
        None => format!(
            "{} verdict: n/a findings: 0 (no machine-readable findings block)\n",
            result.gate_key
        ),
    }
}

fn print_gate_run_details(result: &GateRunResult) {
    println!(
        "Gate '{}' last run: {} (exit code: {})",
        result.gate_key,
        result.status,
        result
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "n/a".to_string())
    );
    if let Some(ms) = result.duration_ms {
        println!("  Duration: {}ms", ms);
    }
    if !result.command.is_empty() {
        println!("  Command: {}", result.command);
    }
    if let Some(branch) = &result.branch {
        println!("  Branch: {}", branch);
    }
    if let Some(commit) = &result.commit {
        println!("  Commit: {}", commit);
    }
    if let jit::domain::GateVerdictOrigin::Reused {
        source_issue,
        source_run,
    } = &result.origin
    {
        println!(
            "  Verdict: taken from issue {source_issue}, run {source_run} over the same declared inputs"
        );
    }
    if result.origin == jit::domain::GateVerdictOrigin::Attested {
        println!(
            "  Attested by {} at {}",
            result
                .by
                .as_ref()
                .map_or_else(|| "an unnamed attestor".to_string(), ToString::to_string),
            result.started_at.to_rfc3339()
        );
    }
    if let Some(f) = &result.findings {
        println!(
            "  Findings: {} (verdict: {})",
            f.findings.len(),
            if f.verdict.is_empty() {
                "unknown"
            } else {
                &f.verdict
            }
        );
    }
    if !result.stdout.is_empty() {
        let lines: Vec<&str> = result.stdout.lines().collect();
        println!("  stdout:\n    {}", lines.join("\n    "));
    }
    if !result.stderr.is_empty() {
        let lines: Vec<&str> = result.stderr.lines().collect();
        println!("  stderr:\n    {}", lines.join("\n    "));
    }
}

/// Build the [`IssueShowResponse`](jit::output::IssueShowResponse) for a single
/// issue id.
///
/// Shared by the multi-id `--json` array path and the multi-id non-JSON loop so
/// each issue renders from the same shape as `issue show --json`.
fn build_issue_show_response<S: IssueStore>(
    executor: &CommandExecutor<S>,
    id: &IssueId,
) -> Result<jit::output::IssueShowResponse> {
    let issue = executor
        .issues()
        .show_issue(id)
        .with_context(|| format!("Failed to load issue {}", id))?;
    executor.issues().show_response(issue)
}

/// Render a single resolved addressable item as a JSON envelope or a short block.
///
/// Used by the `jit issue show <issue>/<self-id>` qualified-id path; the human
/// block is the item response's own rendering.
fn print_item_show(result: &jit::commands::ItemShowResult, json: bool, quiet: bool) -> Result<()> {
    use jit::output::HumanRender;
    if json {
        let output = JsonOutput::success(result);
        println!("{}", output.to_json_string()?);
    } else {
        print!("{}", result.render_human(OutputMode { json, quiet }));
    }
    Ok(())
}

/// Run the `query all` listing, shared by `jit query all` and `jit issue list`.
///
/// Both commands take identical filters/flags and must produce identical output,
/// so they delegate to this single implementation.
fn run_query_all<S: IssueStore>(
    executor: &CommandExecutor<S>,
    quiet: bool,
    filters: jit::cli::IssueFilterArgs,
    full: bool,
    json: bool,
) -> Result<()> {
    let output_ctx = OutputContext::new(quiet, json);
    let filters = filters.parse()?;
    let issues = executor.queries().query_all(
        filters.state,
        filters.assignee,
        filters.priority,
        filters.labels,
    )?;

    if json {
        use jit::domain::MinimalIssue;
        use jit::output::{IssueListFullResponse, IssueListResponse, JsonOutput};

        let msg = format!("Found {} issue(s)", issues.len());
        // `--full` hands back complete stored records (gate list under the
        // storage names `gates_required`/`gates_status`); the default shape emits
        // lean `MinimalIssue` entries. Both go through their typed response so the
        // emitted shape stays in lockstep with the schema derived from the same
        // struct (jit:f40f1b0a).
        let output = if full {
            let response = IssueListFullResponse::new(issues);
            JsonOutput::success(serde_json::to_value(response)?)
        } else {
            let minimal: Vec<MinimalIssue> = issues.iter().map(MinimalIssue::from).collect();
            let response = IssueListResponse::new(minimal);
            JsonOutput::success(serde_json::to_value(response)?)
        }
        .with_message(msg);
        println!("{}", output.to_json_string()?);
    } else {
        print_issue_list(
            &output_ctx,
            "All issues (filtered):",
            plain_issue_rows(&issues),
            &[IssueColumn::State, IssueColumn::Priority],
        )?;
    }
    Ok(())
}

/// Print the human-readable `issue show` view for an already-built response.
///
/// Single source of the `issue show` human field rendering: both the single-id
/// non-JSON branch and the multi-id non-JSON loop delegate here so the field
/// set, order, and document/`[HEAD]` formatting stay identical.
fn print_issue_show_human(response: &jit::output::IssueShowResponse) {
    println!("ID: {}", response.id);
    println!("Title: {}", response.title);
    println!("Description: {}", response.description);
    println!("State: {}", response.state);
    println!("Priority: {}", response.priority);
    println!("Assignee: {:?}", response.assignee);

    if response.dependencies.is_empty() {
        println!("Dependencies: None");
    } else {
        let met_count = response
            .dependencies
            .iter()
            .filter(|d| jit::domain::is_dependency_met(d.state, d.archived_from))
            .count();
        println!(
            "Dependencies ({}/{} met):",
            met_count,
            response.dependencies.len()
        );
        for dep in &response.dependencies {
            println!("  {} - {} [{}]", dep.short_id(), dep.title, dep.state);
        }
    }

    if response.gates.is_empty() {
        println!("Gates: (none)");
    } else {
        println!("Gates:");
        for gate in &response.gates {
            let last_run = gate.last_run_at.as_deref().unwrap_or("never run");
            println!(
                "  - {} [{}] (last run: {})",
                gate.key, gate.status, last_run
            );
        }
    }
    if !response.created_at.is_empty() {
        println!("Created: {}", response.created_at);
    }
    if !response.updated_at.is_empty() {
        println!("Updated: {}", response.updated_at);
    }
    if !response.documents.is_empty() {
        println!("Documents:");
        for doc in &response.documents {
            print!("  - {}", doc.path);
            if let Some(ref label) = doc.label {
                print!(" ({})", label);
            }
            if let Some(ref commit) = doc.commit {
                print!(" [{}]", &commit[..7.min(commit.len())]);
            } else {
                print!(" [HEAD]");
            }
            println!();
        }
    }
}

/// Reject parent-level `jit query` filters when a subcommand is present.
///
/// The bare `jit query` form carries its own filter flags (`--state`,
/// `--assignee`, `--priority`, `--label`, `--full`). When a
/// subcommand such as `available` is also given, those parent-level flags
/// would be silently dropped — a silent-wrong-result footgun for orchestrators
/// consuming `--json`. Detect any that were supplied and return an actionable
/// error pointing the user at the subcommand form or the bare form.
fn reject_parent_query_filters(
    query_cmd: &jit::cli::QueryCommands,
    state: Option<&str>,
    assignee: Option<&str>,
    priority: Option<&str>,
    label: &[String],
    full: bool,
    json: bool,
) -> Result<()> {
    use jit::cli::QueryCommands;

    let offending: Vec<&str> = [
        state.map(|_| "--state"),
        assignee.map(|_| "--assignee"),
        priority.map(|_| "--priority"),
        (!label.is_empty()).then_some("--label"),
        full.then_some("--full"),
    ]
    .into_iter()
    .flatten()
    .collect();

    if offending.is_empty() {
        return Ok(());
    }

    let sub = match query_cmd {
        QueryCommands::All { .. } => "all",
        QueryCommands::Available { .. } => "available",
        QueryCommands::Blocked { .. } => "blocked",
        QueryCommands::Strategic { .. } => "strategic",
        QueryCommands::Closed { .. } => "closed",
        QueryCommands::Count { .. } => "count",
        QueryCommands::Divergence => "divergence",
    };

    // A misplaced pre-subcommand filter is a usage error (exit 2), the same class
    // as clap's own usage errors — routed through the json-aware
    // `invalid_argument` helper so `--json` callers get a machine-readable
    // envelope instead of a bare stderr line. Message text unchanged.
    let flags = offending.join(" ");
    Err(invalid_argument(
        format!(
            "filter(s) {flags} were given before the `{sub}` subcommand, where they \
             are ignored. Put them after the subcommand (e.g. `jit query {sub} {flags}`), \
             or drop the subcommand to use the bare form (e.g. `jit query {flags}`)."
        ),
        json,
    ))
}

/// True when a caught panic payload is std's "failed printing to
/// std{out,err}: Broken pipe" panic (see
/// [`jit::output::is_broken_pipe_write_panic`]) — the panic API hands back an
/// opaque `dyn Any` rather than the typed `io::Error`, so detection happens on
/// the formatted message downcast from either payload shape `panic!` can
/// produce. Takes the unadorned `&dyn Any`: the panic hook's payload is `&dyn
/// Any` and `catch_unwind`'s is `&(dyn Any + Send)`, and a reference to the
/// latter coerces to the former (dropping an auto trait is a valid unsized
/// coercion), so one function serves both call sites.
fn panic_is_broken_pipe(payload: &dyn std::any::Any) -> bool {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .is_some_and(jit::output::is_broken_pipe_write_panic)
}

fn main() {
    // A downstream reader closing the pipe mid-write (`jit ... | head`) makes
    // std's print!/println! machinery panic with a "failed printing to
    // std{out,err}: Broken pipe" message (`library/std/src/io/stdio.rs`).
    // Main.rs's hundreds of direct println!/print! call sites make guarding
    // each individually impractical, so this single top-level guard covers
    // all of them: a panic hook suppresses the panic banner for exactly that
    // message (any other panic still gets the default hook, unwinds out of
    // main, and keeps std's normal exit-101 behavior), and catch_unwind lets
    // main convert the caught broken-pipe panic into a quiet exit using the
    // `ExitCode::BrokenPipe` (128 + SIGPIPE = 141) convention — the exit
    // status a shell reports for a process a signal actually killed,
    // without jit altering its own signal disposition (jit:6f881a85
    // REQ-01/REQ-02).
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !panic_is_broken_pipe(info.payload()) {
            default_hook(info);
        }
    }));

    let outcome = std::panic::catch_unwind(|| match run() {
        (_, Ok(code)) => code,
        (mode, Err(e)) => {
            if mode.json {
                emit_top_level_json_error(&e)
            } else {
                eprintln!("Error: {}", e);
                classify(&e).exit_code()
            }
        }
    });

    let exit_code = match outcome {
        Ok(code) => code,
        Err(payload) => {
            if panic_is_broken_pipe(payload.as_ref()) {
                std::process::exit(ExitCode::BrokenPipe.code());
            }
            // Not a broken-pipe panic: resume the unwind so it escapes main
            // exactly as it would have without this hook (default hook
            // already printed the banner; exit 101).
            std::panic::resume_unwind(payload);
        }
    };

    if exit_code != ExitCode::Success {
        std::process::exit(exit_code.code());
    }
}

/// Emit one structured envelope for a failure propagated to the top level.
/// Handler-owned JSON failures terminate before reaching this boundary.
fn emit_top_level_json_error(error: &anyhow::Error) -> ExitCode {
    let json_error = classify(error);
    let exit_code = json_error.exit_code();

    let rendered = json_error.to_json_string().unwrap_or_else(|_| {
        serde_json::json!({
            "error": {
                "code": json_error.error.code.as_str(),
                "message": json_error.error.message,
            }
        })
        .to_string()
    });
    println!("{}", rendered);
    exit_code
}

/// Parse the command line and dispatch it, handing back the parsed output mode
/// with the result so failure rendering reads the same mode the arms did.
fn run() -> (OutputMode, Result<ExitCode>) {
    let args: Vec<String> = env::args().collect();
    let cli = match Cli::try_parse_from(args.clone()) {
        Ok(cli) => cli,
        Err(error) => {
            if let Some(hint) = unknown_subcommand_hint(&error, &args) {
                eprintln!("{hint}");
            }
            error.exit();
        }
    };
    let mode = OutputMode {
        json: cli.json,
        quiet: cli.quiet,
    };
    (mode, dispatch(cli, mode))
}

fn dispatch(cli: Cli, mode: OutputMode) -> Result<ExitCode> {
    let OutputMode { json, quiet } = mode;

    // Handle --schema flag first
    if cli.schema {
        use jit::CommandSchema;
        let schema = CommandSchema::generate();
        let json = serde_json::to_string_pretty(&schema)?;
        println!("{}", json);
        return Ok(ExitCode::Success);
    }

    // Ensure command is provided
    let command = cli
        .command
        .ok_or_else(|| anyhow::anyhow!("No command provided. Use --help for usage."))?;

    if let Commands::Version = &command {
        return version::dispatch((), mode, &mut io::stdout(), &mut io::stderr());
    }

    // Initialization opens the repository itself: it creates the data root.
    let command = match command {
        Commands::Init(args) => {
            return init::dispatch(args, mode, &mut io::stdout(), &mut io::stderr());
        }
        other => other,
    };

    let current_dir = env::current_dir()?;
    // The per-invocation write stance is read before any repository work, so an
    // unusable token fails the invocation rather than being silently dropped by a
    // command that never consults it.
    let invocation_write_override =
        jit::config_manager::ConfigManager::invocation_write_override()?;
    let jit::commands::OpenedRepository {
        retained_session: mut retained_mutation_session,
        executor,
        data_root: _,
        worktree_paths,
        layout: executor_layout,
        recovered_records: _,
        // Spans the rest of dispatch, so every production mutation context
        // carries a permitted linked-checkout override (jit:f52567ed).
        write_override_annotation: _linked_checkout_write_override,
    } = jit::commands::open_repository(jit::commands::RepositoryInvocation {
        current_dir: current_dir.clone(),
        data_dir_override: env::var("JIT_DATA_DIR").ok().map(PathBuf::from),
        is_init: false,
        opens_mutation_session: command.requires_recovery_dispatch(),
        coordinates_claims_first: command.coordinates_claims_first(),
        is_validation_exempt: matches!(&command, Commands::Profile(ProfileCommands::Show { .. })),
        is_recover: matches!(&command, Commands::Recover),
        write_override: invocation_write_override,
    })?;
    let storage = executor.storage().clone();
    let ctx = CommandContext {
        executor: &executor,
        mode,
    };

    // Normalize top-level first-guess aliases into their canonical noun/verb form
    // so `jit rdeps <id>` and `jit list` resolve to `jit graph rdeps` /
    // `jit issue list` and reuse those handlers verbatim.
    let command = match command {
        Commands::List { filters, full } => Commands::Issue(IssueCommands::List { filters, full }),
        other => other,
    };

    match command {
        Commands::Init(_) => {}
        Commands::Profile(profile_cmd) => match profile_cmd {
            ProfileCommands::Show { profile } => {
                let selectors = jit::commands::parse_profile_selectors(&profile)?;
                match executor.profiles().show_profiles(&selectors) {
                    Ok(result) => {
                        if json {
                            let output = JsonOutput::success(&result);
                            println!("{}", output.to_json_string()?);
                        } else {
                            for result in result.items {
                                let profile = &result.manifest;
                                print_profile_heading(
                                    &result.id,
                                    &result.version,
                                    &result.origin,
                                    "inspected",
                                );
                                println!("Compatible JIT: {}", profile.compatible_jit);
                                println!("Package hash: {}", result.package_hash);
                                println!(
                                    "Files: {} ({} bytes)",
                                    result.file_count, result.byte_size
                                );
                                println!("Targets:");
                                result
                                    .targets
                                    .iter()
                                    .for_each(|target| println!("  {target}"));
                            }
                        }
                    }
                    Err(error) if json => {
                        let json_error = profile_json_error(&error)?;
                        println!("{}", json_error.to_json_string()?);
                        std::process::exit(json_error.exit_code().code());
                    }
                    Err(error) => return Err(error),
                }
            }
            ProfileCommands::Diff {
                profile,
                values_file,
                set,
            } => {
                let selectors = jit::commands::parse_profile_selectors(&profile)?;
                let assignments = jit::commands::parse_profile_variable_assignments(&set)?;
                if selectors.is_empty() {
                    return Err(invalid_argument(
                        "profile diff requires at least one --profile DIR selector".to_string(),
                        json,
                    ));
                }
                let options = ProfileVariableOptions {
                    values_file,
                    assignments,
                };
                let plans = profile_result(
                    executor
                        .profiles()
                        .select_profiles(&selectors, &options)
                        .and_then(|selection| executor.profiles().diff_profiles(&selection)),
                    json,
                )?;
                render_profile_difference(&plans, json)?;
            }
            ProfileCommands::Apply {
                profile,
                values_file,
                set,
                dry_run,
            } => run_profile_apply(
                &executor,
                ProfileApplyInvocation {
                    profile: &profile,
                    values_file,
                    set: &set,
                    dry_run,
                    json,
                },
            )?,
            ProfileCommands::Pack {
                source,
                output,
                dry_run,
            } => {
                let invocation_dir = std::env::current_dir()?;
                let result = if dry_run {
                    executor
                        .profiles()
                        .plan_profile_pack(&invocation_dir, &source, &output)
                } else {
                    executor
                        .profiles()
                        .pack_profile_package(&invocation_dir, &source, &output)
                };
                match result {
                    Ok((result, warnings)) => {
                        for warning in warnings {
                            eprintln!("Warning: {warning}");
                        }
                        if json {
                            let output = JsonOutput::success(&result);
                            println!("{}", output.to_json_string()?);
                        } else {
                            for profile in result.items {
                                let status = profile_status_label(profile.status, "pack", "packed");
                                print_profile_heading(
                                    &profile.id,
                                    &profile.version,
                                    &profile.origin,
                                    &status,
                                );
                                println!("Archive: {}", profile.archive);
                                println!("Package hash: {}", profile.package_hash);
                                println!(
                                    "Files: {} ({} bytes); archive {} bytes",
                                    profile.file_count, profile.byte_size, profile.archive_bytes
                                );
                                print_profile_decisions(&profile.targets, &[]);
                            }
                        }
                    }
                    Err(error) if json => {
                        let json_error = profile_json_error(&error)?;
                        println!("{}", json_error.to_json_string()?);
                        std::process::exit(json_error.exit_code().code());
                    }
                    Err(error) => return Err(error),
                }
            }
            ProfileCommands::Add {
                archive,
                destination,
                dry_run,
            } => {
                let invocation_dir = std::env::current_dir()?;
                let result = if dry_run {
                    executor
                        .profiles()
                        .plan_profile_add(&invocation_dir, &archive, &destination)
                } else {
                    executor
                        .profiles()
                        .add_profile_package(&invocation_dir, &archive, &destination)
                };
                match result {
                    Ok(result) => {
                        if json {
                            let output = JsonOutput::success(&result);
                            println!("{}", output.to_json_string()?);
                        } else {
                            for profile in result.items {
                                let status = profile_status_label(profile.status, "add", "added");
                                print_profile_heading(
                                    &profile.id,
                                    &profile.version,
                                    &profile.origin,
                                    &status,
                                );
                                println!("Archive: {}", profile.archive);
                                println!("Destination: {}", profile.destination);
                                println!("Package hash: {}", profile.package_hash);
                                println!(
                                    "Files: {} ({} bytes)",
                                    profile.file_count, profile.byte_size
                                );
                                print_profile_decisions(&profile.targets, &[]);
                            }
                        }
                    }
                    Err(error) if json => {
                        let json_error = profile_json_error(&error)?;
                        println!("{}", json_error.to_json_string()?);
                        std::process::exit(json_error.exit_code().code());
                    }
                    Err(error) => return Err(error),
                }
            }
        },
        Commands::List { .. } => {
            unreachable!("top-level list is normalized to its canonical command above")
        }
        Commands::Version => unreachable!("version is handled before repository validation"),
        Commands::Issue(issue_cmd) => {
            match issue_cmd {
                IssueCommands::Create {
                    positional_title,
                    title,
                    description,
                    description_file,
                    priority,
                    issue_type,
                    gate,
                    label,
                    content_format,
                    force,
                } => {
                    let gate = parse_gate_keys(gate, json)?;

                    // Clap guarantees exactly one of the two title forms is set.
                    let title = positional_title
                        .or(title)
                        .expect("clap requires title (positional or --title)");

                    let prio = Priority::from_str(&priority)?;
                    let content_format = content_format
                        .map(|s| jit::domain::ContentFormat::from_str(&s))
                        .transpose()?;
                    let description = match (description, description_file) {
                        (Some(text), None) => text,
                        (None, Some(path)) => read_description_source(&path)
                            .map_err(|error| invalid_argument(format!("{error:#}"), json))?,
                        (None, None) => String::new(),
                        (Some(_), Some(_)) => {
                            unreachable!("clap rejects --description with --description-file")
                        }
                    };

                    // The command layer owns `--type` validation and `type:<kind>`
                    // label derivation; the CLI just forwards the typed value.
                    let creation = executor.issues().create_issue(
                        title,
                        description,
                        prio,
                        gate,
                        label,
                        content_format,
                        issue_type,
                        force,
                    )?;
                    if json {
                        let issue = creation.issue;
                        let msg = format!("Created issue {} - {}", issue.short_id(), issue.title);
                        // A new issue has no dependencies and no gate runs.
                        let response = jit::output::IssueCreateResponse {
                            issue: jit::output::IssueShowResponse::from_issue(issue, vec![], &[]),
                            findings: creation.findings,
                        };
                        let output = JsonOutput::success(response).with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        // In quiet mode, output just the ID for scripting
                        if quiet {
                            println!("{}", creation.issue.id);
                        } else {
                            println!("Created issue: {}", creation.issue.id);
                            jit::output::render_validation_findings(&creation.findings, None)
                                .print();
                        }
                    }
                }
                IssueCommands::BatchCreate { from_json, dry_run } => {
                    use jit::commands::BatchIssueDef;

                    // Read + parse the file (both fallible I/O paths carry context).
                    let contents = std::fs::read_to_string(&from_json).with_context(|| {
                        format!("Failed to read batch file {}", from_json.display())
                    })?;
                    let defs: Vec<BatchIssueDef> =
                        serde_json::from_str(&contents).with_context(|| {
                            format!(
                                "Failed to parse batch file {} as a JSON array of issue definitions",
                                from_json.display()
                            )
                        })?;

                    if dry_run {
                        let outcome = executor.issues().validate_batch_from_json(&defs)?;
                        if json {
                            println!("{}", serde_json::to_string_pretty(&outcome)?);
                        } else {
                            println!(
                                "Valid batch: {} issue(s), {} dependency edge(s); no changes made.",
                                outcome.issue_count, outcome.dependency_count
                            );
                        }
                        return Ok(ExitCode::Success);
                    }

                    let outcome = executor.issues().batch_create_from_json(defs)?;

                    if json {
                        // Print EXACTLY the pure `{key: id}` map: every top-level
                        // entry is a symbolic key, no envelope/`message` field.
                        // A BTreeMap gives deterministic (sorted-key) output.
                        let map: std::collections::BTreeMap<&str, &str> = outcome
                            .key_to_id
                            .iter()
                            .map(|(k, v)| (k.as_str(), v.as_str()))
                            .collect();
                        println!("{}", serde_json::to_string_pretty(&map)?);
                    } else {
                        let output_ctx = OutputContext::new(quiet, json);
                        let _ = output_ctx
                            .print_info(format!("Created {} issue(s):", outcome.key_to_id.len()));
                        for (key, id) in &outcome.key_to_id {
                            println!("  {key} -> {id}");
                        }
                    }
                }
                IssueCommands::Search {
                    query,
                    filters,
                    full,
                } => {
                    let output_ctx = OutputContext::new(quiet, json);

                    let filters = filters.parse()?;

                    // The positional query is optional only when at least one
                    // filter narrows the search; otherwise the command would
                    // dump every issue, which is almost never intended.
                    let has_filter = filters.state.is_some()
                        || filters.assignee.is_some()
                        || filters.priority.is_some()
                        || !filters.labels.is_empty();
                    if query.is_none() && !has_filter {
                        return Err(jit::errors::InvalidArgumentError::new(
                            "invalid arguments: provide a search query or at least one filter (--label/--state/--assignee/--priority)",
                        )
                        .into());
                    }

                    let issues = executor.queries().search_issues_with_filters(
                        query.as_deref().unwrap_or(""),
                        filters.priority,
                        filters.state,
                        filters.assignee,
                        filters.labels,
                    )?;

                    if json {
                        use jit::output::{
                            IssueSearchFullResponse, IssueSearchQuery, IssueSearchResponse,
                            JsonOutput,
                        };

                        let count = issues.len();
                        let msg = match &query {
                            Some(q) => format!("Found {count} issue(s) matching '{q}'"),
                            None => format!("Found {count} issue(s)"),
                        };
                        // `--full` returns complete stored records (gate list under
                        // the storage names `gates_required`/`gates_status`); the
                        // default shape returns lean `MinimalIssue` entries. Both go
                        // through their typed response so the emitted shape stays in
                        // lockstep with the schema derived from the same struct
                        // (jit:f40f1b0a).
                        let output_data = if full {
                            serde_json::to_value(IssueSearchFullResponse::with_extras(
                                issues,
                                IssueSearchQuery {
                                    query: query.clone(),
                                },
                            ))?
                        } else {
                            use jit::domain::MinimalIssue;
                            let minimal_issues: Vec<MinimalIssue> =
                                issues.iter().map(MinimalIssue::from).collect();
                            serde_json::to_value(IssueSearchResponse::with_extras(
                                minimal_issues,
                                IssueSearchQuery {
                                    query: query.clone(),
                                },
                            ))?
                        };

                        let output = JsonOutput::success(output_data).with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        print_issue_list(
                            &output_ctx,
                            &format!("Found {} issue(s):", issues.len()),
                            plain_issue_rows(&issues),
                            &[IssueColumn::State, IssueColumn::Priority],
                        )?;
                    }
                }
                IssueCommands::Show {
                    ids,
                    summary,
                    field,
                    fields,
                } => {
                    let projecting = field.is_some() || !fields.is_empty();

                    // Projection targets a single issue: `--field`/`--fields`
                    // with multiple ids is ambiguous, so reject it.
                    if projecting && ids.len() > 1 {
                        return Err(jit::errors::InvalidArgumentError::new(format!(
                            "invalid arguments: --field/--fields require exactly one issue id (got {})",
                            ids.len()
                        ))
                        .into());
                    }

                    // Multi-id: return the uniform list envelope
                    // `{"count": N, "issues": [...]}` with full issue objects in
                    // argument order. Without --json, fall through to printing
                    // each issue's human view in order.
                    if ids.len() > 1 && json {
                        let responses = ids
                            .iter()
                            .map(|id| {
                                build_issue_show_response(&executor, &resolve_issue(&storage, id)?)
                            })
                            .collect::<Result<Vec<_>>>()?;
                        let output = jit::output::JsonOutput::success(serde_json::to_value(
                            jit::output::IssueShowListResponse::new(responses),
                        )?);
                        println!("{}", output.to_json_string()?);
                        return Ok(ExitCode::Success);
                    }

                    if ids.len() > 1 {
                        // Non-JSON multi-id: print each issue's human view.
                        for id in &ids {
                            let response = build_issue_show_response(
                                &executor,
                                &resolve_issue(&storage, id)?,
                            )?;
                            print_issue_show_human(&response);
                        }
                        return Ok(ExitCode::Success);
                    }

                    // Single id (guaranteed by clap `required = true`).
                    let requested = &ids[0];

                    // A `<issue>/<self-id>` argument addresses an item, not an
                    // issue (issue ids never contain '/'). Resolve and render the
                    // addressed item through the same item resolver as
                    // `jit item show`, honoring --json (REQ-04 / concept).
                    if !projecting && !summary && requested.contains('/') {
                        match executor.items().show_item(requested) {
                            Ok(result) => {
                                print_item_show(&result, json, quiet)?;
                                return Ok(ExitCode::Success);
                            }
                            Err(e) => {
                                // `handle_json_error!` either prints the JSON error
                                // and exits, or returns `Err` (non-JSON path).
                                handle_json_error!(
                                    json,
                                    e,
                                    command_json_error(&e, jit::output::ErrorCode::ItemNotFound)
                                );
                            }
                        }
                    }

                    match resolve_issue(&storage, requested)
                        .and_then(|id| executor.issues().show_issue(&id))
                    {
                        Ok(issue) => {
                            // Field projection: print the named field(s) and return.
                            if projecting {
                                let response = executor.issues().show_response(issue)?;
                                let value = serde_json::to_value(&response).with_context(|| {
                                    "Failed to serialize issue for field projection".to_string()
                                })?;

                                if let Some(name) = field {
                                    let rendered = jit::output::project_field(&value, &name)
                                        .ok_or_else(|| {
                                            jit::errors::InvalidArgumentError::new(format!(
                                                "invalid field: unknown field '{}' for issue show",
                                                name
                                            ))
                                        })?;
                                    println!("{}", rendered);
                                } else {
                                    let rendered = jit::output::project_fields(&value, &fields)
                                        .map_err(|unknown| {
                                            jit::errors::InvalidArgumentError::new(format!(
                                                "invalid field: unknown field '{}' for issue show",
                                                unknown.0
                                            ))
                                        })?;
                                    println!("{}", rendered);
                                }
                                return Ok(ExitCode::Success);
                            }

                            if summary && json {
                                let summary_response =
                                    jit::output::IssueShowSummaryResponse::from(&issue);
                                let msg = format!(
                                    "Issue {}: {} [{}]",
                                    summary_response.short_id,
                                    summary_response.title,
                                    summary_response.state
                                );
                                let output = jit::output::JsonOutput::success(summary_response)
                                    .with_message(msg);
                                println!("{}", output.to_json_string()?);
                                return Ok(ExitCode::Success);
                            }
                            let response = executor.issues().show_response(issue)?;

                            let show_msg = format!(
                                "Issue {}: {} [{}]",
                                response.id.short_id(),
                                response.title,
                                response.state
                            );
                            output_data!(quiet, json, response, show_msg, {
                                print_issue_show_human(&response);
                            });
                        }
                        Err(e) => {
                            handle_json_error!(
                                json,
                                e,
                                jit::output::JsonError::issue_not_found(requested)
                            );
                        }
                    }
                }
                IssueCommands::Status { ids } => {
                    // One compact status per id, in argument order. Each status is
                    // projected from the same enriched show response so the unmet
                    // dependency set stays byte-for-byte consistent with
                    // `issue show --json`.
                    //
                    // Resolve each id inline (not via `build_issue_show_response`,
                    // which wraps the lookup error in `.with_context` and would
                    // hide the typed id-resolution error from `refine_id_error`'s
                    // downcast). A bad id is routed through `handle_json_error!`
                    // exactly like `issue show`: under `--json` it prints the
                    // refined error envelope (the id-lookup `ErrorCode` with its
                    // matching exit code) and exits, so a
                    // multi-id run fails fast on the first bad id.
                    let mut statuses = Vec::with_capacity(ids.len());
                    for id in &ids {
                        match resolve_issue(&storage, id)
                            .and_then(|full_id| executor.issues().show_issue(&full_id))
                        {
                            Ok(issue) => {
                                statuses.push(executor.issues().issue_status_response(issue)?);
                            }
                            Err(e) => {
                                handle_json_error!(
                                    json,
                                    e,
                                    jit::output::JsonError::issue_not_found(id)
                                );
                            }
                        }
                    }

                    if json {
                        if statuses.len() == 1 {
                            // Single id stays a bare object, mirroring `issue show`.
                            let output = jit::output::JsonOutput::success(&statuses[0]);
                            println!("{}", output.to_json_string()?);
                        } else {
                            let output = jit::output::JsonOutput::success(serde_json::to_value(
                                jit::output::IssueStatusListResponse::new(statuses),
                            )?);
                            println!("{}", output.to_json_string()?);
                        }
                    } else {
                        for status in &statuses {
                            println!("{}", status.to_line());
                        }
                    }
                }
                IssueCommands::Children { id } => {
                    // Orchestration (child resolution, dangling classification)
                    // lives in `IssueOps::issue_children`; this arm only
                    // dispatches, renders, and routes an id failure through the
                    // JSON error path.
                    match resolve_issue(&storage, &id)
                        .and_then(|full_id| executor.issues().issue_children(&full_id))
                    {
                        Ok(response) => {
                            if json {
                                let output = jit::output::JsonOutput::success(&response);
                                println!("{}", output.to_json_string()?);
                            } else {
                                for child in &response.items {
                                    println!("{}", child.to_line());
                                }
                                if !response.extras.dangling.is_empty() {
                                    println!(
                                        "dangling: {}",
                                        response
                                            .extras
                                            .dangling
                                            .iter()
                                            .map(IssueId::as_str)
                                            .collect::<Vec<_>>()
                                            .join(",")
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            handle_json_error!(
                                json,
                                e,
                                jit::output::JsonError::issue_not_found(&id)
                            );
                        }
                    }
                }
                IssueCommands::Progress { id } => {
                    // Orchestration lives in `IssueOps::issue_progress`;
                    // this arm dispatches, renders, and routes an id failure
                    // through the JSON error path.
                    match resolve_issue(&storage, &id)
                        .and_then(|full_id| executor.issues().issue_progress(&full_id))
                    {
                        Ok(response) => {
                            if json {
                                let output = jit::output::JsonOutput::success(&response);
                                println!("{}", output.to_json_string()?);
                            } else {
                                println!(
                                    "{} [{}] title: {}",
                                    response.container.short_id,
                                    response.container.state.as_str(),
                                    response.container.title
                                );
                                for line in response.rollup.to_lines() {
                                    println!("{}", line);
                                }
                                if !response.dangling.is_empty() {
                                    println!(
                                        "dangling: {}",
                                        response
                                            .dangling
                                            .iter()
                                            .map(IssueId::as_str)
                                            .collect::<Vec<_>>()
                                            .join(",")
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            handle_json_error!(
                                json,
                                e,
                                jit::output::JsonError::issue_not_found(&id)
                            );
                        }
                    }
                }
                IssueCommands::Update {
                    id,
                    filter,
                    title,
                    description,
                    description_file,
                    append_description,
                    append_description_file,
                    priority,
                    state,
                    issue_type,
                    label,
                    remove_label,
                    add_gate,
                    remove_gate,
                    assignee,
                    unassign,
                    content_format,
                    force,
                } => {
                    let add_gate = parse_gate_keys(add_gate, json)?;
                    let remove_gate = parse_gate_keys(remove_gate, json)?;
                    let assignee = assignee
                        .map(|value| Assignee::from_str(&value))
                        .transpose()?;

                    let output_ctx = OutputContext::new(quiet, json);

                    // Batch-mode argument guards are usage errors (exit 2), the
                    // same class as clap's own usage errors — routed through the
                    // json-aware `invalid_argument` helper so `--json` callers get
                    // a machine-readable envelope instead of a bare stderr line.

                    // Validate: exactly one of ID or filter must be provided
                    if id.is_none() && filter.is_none() {
                        return Err(invalid_argument(
                            "Must specify either issue ID or --filter for batch mode".to_string(),
                            json,
                        ));
                    }
                    if id.is_some() && filter.is_some() {
                        return Err(invalid_argument(
                            "Cannot specify both ID and --filter (mutually exclusive)".to_string(),
                            json,
                        ));
                    }
                    // --content-format is a per-issue field; batch mode does not
                    // support it (would set the same format on every match).
                    if filter.is_some() && content_format.is_some() {
                        return Err(invalid_argument(
                            "--content-format is not supported with --filter (batch mode); set it per issue".to_string(),
                            json));
                    }
                    // --type is a per-issue field; batch mode does not support it.
                    if filter.is_some() && issue_type.is_some() {
                        return Err(invalid_argument(
                            "--type is not supported with --filter (batch mode); set it per issue"
                                .to_string(),
                            json,
                        ));
                    }
                    // Description edits are per-issue (a replace/append against
                    // one issue's existing text); batch mode does not apply
                    // them. Reject rather than silently ignore, so the flags are
                    // never a no-op.
                    if filter.is_some()
                        && (description.is_some()
                            || description_file.is_some()
                            || append_description.is_some()
                            || append_description_file.is_some())
                    {
                        return Err(invalid_argument(
                            "description flags (--description/--description-file/--append-description/--append-description-file) are not supported with --filter (batch mode); update descriptions per issue".to_string(),
                            json));
                    }

                    // Single issue mode
                    if let Some(id_str) = id {
                        let prio = priority.map(|p| Priority::from_str(&p)).transpose()?;
                        let st = state.map(|s| State::from_str(&s)).transpose()?;
                        // Resolve the (mutually exclusive, clap-enforced) description
                        // flags into a single operation. Replace forms take TEXT
                        // directly; the `-file` forms read PATH (or stdin for `-`)
                        // verbatim, so raciness/argv-limits/quoting never enter the
                        // picture for large descriptions.
                        let description_update: Option<DescriptionUpdate> =
                            if let Some(text) = description {
                                Some(DescriptionUpdate::Replace(text))
                            } else if let Some(path) = description_file {
                                Some(DescriptionUpdate::Replace(read_description_source(&path)?))
                            } else if let Some(text) = append_description {
                                Some(DescriptionUpdate::Append(text))
                            } else if let Some(path) = append_description_file {
                                Some(DescriptionUpdate::Append(read_description_source(&path)?))
                            } else {
                                None
                            };
                        // Tri-state for the per-issue content_format override:
                        //   flag absent            -> None              (leave unchanged)
                        //   "inherit"/"default"    -> Some(None)        (clear to repo default)
                        //   a ContentFormat token  -> Some(Some(fmt))  (set the override)
                        let content_format: Option<Option<jit::domain::ContentFormat>> =
                            match content_format.as_deref() {
                                None => None,
                                Some("inherit") | Some("default") => Some(None),
                                Some(s) => Some(Some(jit::domain::ContentFormat::from_str(s)?)),
                            };

                        let requested = id_str.parse::<IssueIdPrefix>()?;
                        match executor.issues().update_issue_resolved(
                            &requested,
                            title,
                            description_update,
                            prio,
                            st,
                            label,
                            remove_label,
                            content_format,
                            issue_type,
                            jit::commands::IssueLocalUpdate {
                                add_gates: add_gate,
                                remove_gates: remove_gate,
                                assignee,
                                unassign,
                            },
                            force,
                        ) {
                            Ok(jit::commands::IssueMutation { issue, warnings }) => {
                                // Print warnings to stderr
                                for warning in &warnings {
                                    eprintln!("Warning: {}", warning);
                                }

                                if json {
                                    let msg = format!(
                                        "Updated issue {} to {}",
                                        issue.short_id(),
                                        issue.state
                                    );
                                    let response = jit::output::IssueUpdateResponse::from(&issue);
                                    let output = JsonOutput::success(response).with_message(msg);
                                    println!("{}", output.to_json_string()?);
                                } else {
                                    let _ = output_ctx
                                        .print_success(format!("Updated issue: {}", issue.id));
                                }
                            }
                            Err(e) => {
                                // Branch selection is driven entirely by typed errors
                                // (TransitionBlockedError, IssueNotFoundError, then the
                                // shared classification); the message text never selects a code.
                                let json_error = if let Some(blocked) =
                                    e.downcast_ref::<jit::errors::TransitionBlockedError>()
                                {
                                    jit::output::JsonError::transition_blocked(blocked)
                                } else if e
                                    .downcast_ref::<jit::storage::IssueNotFoundError>()
                                    .is_some()
                                {
                                    jit::output::JsonError::issue_not_found(&id_str)
                                } else {
                                    command_json_error(&e, classify(&e).error.code)
                                };
                                handle_json_error!(json, e, json_error);
                            }
                        }
                    }
                    // Batch mode
                    else if let Some(filter_str) = filter {
                        use jit::commands::bulk_update::UpdateOperations;
                        use jit::query_engine::QueryFilter;

                        // Parse query filter
                        let query_filter = QueryFilter::parse(&filter_str)?;

                        // Build update operations
                        let operations = UpdateOperations {
                            state: state.map(|s| State::from_str(&s)).transpose()?,
                            add_labels: label,
                            remove_labels: remove_label,
                            assignee,
                            unassign,
                            priority: priority.map(|p| Priority::from_str(&p)).transpose()?,
                            add_gates: add_gate,
                            remove_gates: remove_gate,
                        };

                        // Execute bulk update
                        let result = executor.issues().apply_bulk_update(
                            &query_filter,
                            &operations,
                            force,
                        )?;

                        if json {
                            let msg =
                                format!("Modified {} issue(s)", result.summary.total_modified);
                            let output = JsonOutput::success(result).with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            // Human-readable output
                            if result.summary.total_modified > 0 {
                                let _ = output_ctx.print_success(format!(
                                    "Modified {} issue(s)",
                                    result.summary.total_modified
                                ));
                            }

                            if !result.warnings.is_empty() {
                                println!("\nWarnings ({} issue(s)):", result.warnings.len());
                                for (id, warning) in &result.warnings {
                                    println!("  • {}: {}", id.short_id(), warning);
                                }
                            }

                            if !result.skipped.is_empty() {
                                println!("\nSkipped {} issue(s):", result.summary.total_skipped);
                                for (id, reason) in &result.skipped {
                                    println!("  • {}: {}", id.short_id(), reason);
                                }
                            }

                            if !result.errors.is_empty() {
                                println!("\nFailed {} issue(s):", result.summary.total_errors);
                                for (id, error) in &result.errors {
                                    println!("  • {}: {}", id.short_id(), error);
                                }
                            }

                            if result.summary.total_matched > 0 {
                                println!(
                                    "\nSummary: {}/{} succeeded ({:.0}%)",
                                    result.summary.total_modified,
                                    result.summary.total_matched,
                                    (result.summary.total_modified as f64
                                        / result.summary.total_matched as f64)
                                        * 100.0
                                );
                            } else {
                                println!("No issues matched filter");
                            }
                        }
                    }
                }
                IssueCommands::Delete { id } => {
                    // Deletion inside a linked checkout is governed by the
                    // linked-checkout write policy decided at dispatch, like
                    // every other state-mutating invocation (jit:1b6925a9).
                    //
                    // Phase 3 safety check: require JIT_ALLOW_DELETION=1 to discourage
                    // deletion (jit:0daba57d). The env var is read here (dispatch-level
                    // input gathering); the refusal decision itself is
                    // `IssueOps::confirm_deletion_allowed`, so it stays testable
                    // without mutating global process state.
                    //
                    let allow_deletion =
                        std::env::var("JIT_ALLOW_DELETION").unwrap_or_default() == "1";
                    let id = resolve_issue(&storage, &id)?;
                    if let Err(e) = executor
                        .issues()
                        .confirm_deletion_allowed(&id, allow_deletion)
                    {
                        if json {
                            let json_error = jit::output::JsonError::new(
                                jit::output::ErrorCode::DeletionNotConfirmed,
                                e.to_string(),
                            )
                            .with_details(serde_json::json!({ "id": id }))
                            .with_suggestion(format!(
                                "Set JIT_ALLOW_DELETION=1 environment variable to proceed: \
                                 JIT_ALLOW_DELETION=1 jit issue delete {id}"
                            ));
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        }
                        return Err(e.into());
                    }

                    let output_ctx = OutputContext::new(quiet, json);
                    let warnings = executor.issues().delete_issue(&id)?;
                    for warning in warnings {
                        output_ctx.print_warning(&warning)?;
                    }

                    if json {
                        let result = serde_json::json!({
                            "id": id,
                            "deleted": true
                        });
                        let msg = format!("Deleted issue {}", id.short_id());
                        let output = JsonOutput::success(result).with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!("Deleted issue: {}", id));
                    }
                }
                IssueCommands::Assign { id, assignee } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let assignee = Assignee::from_str(&assignee)?;
                    let requested = id.parse::<IssueIdPrefix>()?;
                    let jit::commands::IssueMutation { issue, warnings } =
                        executor.issues().assign_issue(&requested, assignee)?;
                    for warning in warnings {
                        output_ctx.print_warning(&warning)?;
                    }

                    if json {
                        let msg = format!(
                            "Assigned issue {} to {}",
                            issue.short_id(),
                            issue
                                .assignee
                                .as_ref()
                                .map_or_else(|| "unknown".to_string(), |a| a.to_string())
                        );
                        let output = JsonOutput::success(issue).with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!("Assigned issue: {}", issue.id));
                    }
                }
                IssueCommands::Claim {
                    id,
                    assignee,
                    assign_only,
                } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let assignee = Assignee::from_str(&assignee)?;
                    let requested = id.parse::<IssueIdPrefix>()?;

                    if assign_only {
                        // Assign without transitioning state: reuse the same
                        // path as `jit issue assign`.
                        let jit::commands::IssueMutation { issue, warnings } =
                            executor.issues().assign_issue(&requested, assignee)?;
                        for warning in warnings {
                            output_ctx.print_warning(&warning)?;
                        }

                        if json {
                            let msg = format!(
                                "Assigned issue {} to {} (assign-only)",
                                issue.short_id(),
                                issue
                                    .assignee
                                    .as_ref()
                                    .map_or_else(|| "unknown".to_string(), |a| a.to_string())
                            );
                            let output = JsonOutput::success(issue).with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx.print_success(format!(
                                "Assigned issue {} (assign-only)",
                                issue.id
                            ));
                        }
                        return Ok(ExitCode::Success);
                    }

                    let jit::commands::IssueMutation { issue, warnings } =
                        match executor.issues().claim_issue(&requested, assignee) {
                            Ok(result) => result,
                            Err(e) => {
                                if json {
                                    if let Some(blocked) =
                                        e.downcast_ref::<jit::errors::TransitionBlockedError>()
                                    {
                                        let json_error =
                                            jit::output::JsonError::transition_blocked(blocked);
                                        println!("{}", json_error.to_json_string()?);
                                        std::process::exit(json_error.exit_code().code());
                                    }
                                }
                                return Err(e);
                            }
                        };

                    for warning in warnings {
                        output_ctx.print_warning(&warning)?;
                    }

                    if json {
                        let msg = format!("Claimed issue {}", issue.short_id());
                        let output = JsonOutput::success(issue).with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!("Claimed issue: {}", issue.id));
                    }
                }
                IssueCommands::Unassign { id } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let requested = id.parse::<IssueIdPrefix>()?;
                    let jit::commands::IssueMutation { issue, warnings } =
                        executor.issues().unassign_issue(&requested)?;
                    for warning in warnings {
                        output_ctx.print_warning(&warning)?;
                    }

                    if json {
                        let msg = format!("Unassigned issue {}", issue.short_id());
                        let output = JsonOutput::success(issue).with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!("Unassigned issue: {}", issue.id));
                    }
                }
                IssueCommands::Reject { id, reason } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let requested = id.parse::<IssueIdPrefix>()?;

                    let jit::commands::IssueMutation { issue, warnings } = executor
                        .issues()
                        .reject_issue(&requested, reason.as_deref())?;
                    for warning in warnings {
                        output_ctx.print_warning(&warning)?;
                    }

                    if json {
                        let msg = format!("Rejected issue {}", issue.short_id());
                        let output = JsonOutput::success(issue).with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else if let Some(reason_value) = reason {
                        let _ = output_ctx.print_success(format!(
                            "Rejected issue: {} (reason: {})",
                            issue.id, reason_value
                        ));
                    } else {
                        let _ = output_ctx.print_success(format!("Rejected issue: {}", issue.id));
                    }
                }
                IssueCommands::Release { id, reason } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let requested = id.parse::<IssueIdPrefix>()?;
                    let issue = executor.issues().release_issue(&requested, &reason)?;

                    if json {
                        let msg = format!("Released issue {}", issue.short_id());
                        let output = JsonOutput::success(issue).with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!(
                            "Released issue: {} (reason: {})",
                            issue.id, reason
                        ));
                    }
                }
                IssueCommands::ClaimNext { assignee, filter } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let assignee = Assignee::from_str(&assignee)?;
                    let jit::commands::IssueMutation { issue, warnings } =
                        match executor.issues().claim_next(assignee, filter) {
                            Ok(result) => result,
                            Err(e) => {
                                if json {
                                    if let Some(blocked) =
                                        e.downcast_ref::<jit::errors::TransitionBlockedError>()
                                    {
                                        let json_error =
                                            jit::output::JsonError::transition_blocked(blocked);
                                        println!("{}", json_error.to_json_string()?);
                                        std::process::exit(json_error.exit_code().code());
                                    }
                                }
                                return Err(e);
                            }
                        };

                    for warning in warnings {
                        output_ctx.print_warning(&warning)?;
                    }

                    if json {
                        let msg = format!("Claimed issue {}", issue.short_id());
                        let output = JsonOutput::success(issue).with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!("Claimed issue: {}", issue.id));
                    }
                }
                IssueCommands::List { filters, full } => {
                    run_query_all(&executor, quiet, filters, full, json)?;
                }
            }
        }
        Commands::Apply(args) => {
            return apply::dispatch(args, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Dep(command) => {
            return dep::dispatch(command, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Gate(gate_cmd) => {
            let gate_cmd = match jit::dispatch::gate::dispatch(
                gate_cmd,
                &ctx,
                &mut io::stdout(),
                &mut io::stderr(),
            )? {
                jit::dispatch::Dispatched::Handled(status) => return Ok(status),
                jit::dispatch::Dispatched::Unhandled(command) => command,
            };
            match gate_cmd {
                GateCommands::Status {
                    id,
                    gate_key,
                    gate_flag,
                    all,
                    limit,
                    status,
                    stdout,
                    stderr,
                    tail,
                    findings,
                } => {
                    let output_ctx = OutputContext::new(quiet, json);

                    // Four views share one inspection surface (REQ-08, decision D11):
                    //   - history:  list prior runs (`--all` / `--limit`),
                    //   - flat:     verbatim report text (`--stdout` / `--stderr` / `--tail`),
                    //   - findings: structured findings + verdict (`--findings`),
                    //   - default:  the latest run for one gate (unchanged behaviour).
                    let history_mode = all || limit.is_some();
                    let flat_mode = stdout || stderr || tail.is_some();
                    let findings_mode = findings;

                    // These views answer different questions; combining them is
                    // ambiguous, so at most one non-default view may be selected.
                    if [history_mode, flat_mode, findings_mode]
                        .iter()
                        .filter(|on| **on)
                        .count()
                        > 1
                    {
                        return Err(invalid_argument(
                            "history (--all/--limit), flat-output (--stdout/--stderr/--tail), and \
                         findings (--findings) views are mutually exclusive."
                                .to_string(),
                            json,
                        ));
                    }
                    // `--status` filters a history listing; it is meaningless without one.
                    if status.is_some() && !history_mode {
                        return Err(invalid_argument(
                            "--status only applies to the history view; pass --all or --limit."
                                .to_string(),
                            json,
                        ));
                    }

                    if findings_mode {
                        // Findings view selects exactly one gate's latest run and
                        // reports only its structured findings + verdict.
                        let gate_key = jit::dispatch::resolve_gate_key_for(
                            gate_key,
                            gate_flag,
                            "gate status",
                        )?;
                        let id = resolve_issue(&storage, &id)
                            .or_else(|e| report_gate_check_error(e, json))?;
                        match executor.gates().get_last_gate_run(&id, &gate_key) {
                            Ok(Some(result)) => {
                                if json {
                                    use jit::output::{GateFindingsResponse, JsonOutput};
                                    let response = match &result.findings {
                                        Some(f) => GateFindingsResponse {
                                            key: gate_key.clone(),
                                            run_id: result.run_id.clone(),
                                            has_findings: true,
                                            verdict: Some(f.verdict.clone()),
                                            summary: Some(f.summary.clone()),
                                            findings: f.findings.clone(),
                                        },
                                        None => GateFindingsResponse {
                                            key: gate_key.clone(),
                                            run_id: result.run_id.clone(),
                                            has_findings: false,
                                            verdict: None,
                                            summary: None,
                                            findings: vec![],
                                        },
                                    };
                                    let output = JsonOutput::success(response);
                                    println!("{}", output.to_json_string()?);
                                } else {
                                    print!("{}", render_gate_findings_text(&result));
                                }
                            }
                            Ok(None) => {
                                if json {
                                    use jit::output::JsonOutput;
                                    let output = JsonOutput::<Option<()>>::success(None).with_message(
                                    format!(
                                        "Gate '{}' has not been run yet for issue {}. Use 'jit gate evaluate' to run it.",
                                        gate_key, id
                                    ),
                                );
                                    println!("{}", output.to_json_string()?);
                                } else {
                                    println!(
                                        "{}",
                                        jit::output::render_gate_run_status(
                                            jit::output::GateRunLine::NeverRun,
                                            &gate_key,
                                            &id,
                                        )
                                    );
                                }
                            }
                            Err(e) => return report_gate_check_error(e, json),
                        }
                    } else if history_mode {
                        // Gate key is an optional filter here (positional or --gate).
                        let gate_filter = resolve_optional_gate_key(gate_key, gate_flag, json)?;
                        let status_filter = match status {
                            Some(ref s) => Some(parse_run_status(s, json)?),
                            None => None,
                        };

                        let id = resolve_issue(&storage, &id)
                            .or_else(|e| report_gate_check_error(e, json))?;
                        let runs = executor
                            .gates()
                            .list_gate_runs(&id, gate_filter.as_ref())
                            .or_else(|e| report_gate_check_error(e, json))?;
                        // list_gate_runs already returns newest-first; apply the
                        // status filter and the --limit cap on top of that order.
                        let mut runs: Vec<_> = runs
                            .into_iter()
                            .filter(|r| status_filter.is_none_or(|s| r.status == s))
                            .collect();
                        if let Some(n) = limit {
                            runs.truncate(n);
                        }

                        if json {
                            use jit::output::{GateRunHistoryResponse, GateRunSummary, JsonOutput};
                            let summaries: Vec<GateRunSummary> =
                                runs.iter().map(GateRunSummary::full).collect();
                            let count = summaries.len();
                            let response = GateRunHistoryResponse::new(summaries);
                            let msg = format!("{} gate run(s) listed for issue {}", count, id);
                            let output = JsonOutput::success(response).with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else if runs.is_empty() {
                            let _ = output_ctx
                                .print_info(format!("No matching gate runs for issue {}", id));
                        } else {
                            let _ = output_ctx
                                .print_info(format!("Gate run history for issue {}:", id));
                            for run in &runs {
                                print_gate_run_details(run);
                            }
                        }
                    } else if flat_mode {
                        // Flat view selects exactly one gate's latest run.
                        let gate_key = jit::dispatch::resolve_gate_key_for(
                            gate_key,
                            gate_flag,
                            "gate status",
                        )?;
                        // stderr alone shows only stderr; --stdout (or the tail-only
                        // form, where neither stream flag is set) includes stdout.
                        let want_stdout = stdout || !stderr;
                        let want_stderr = stderr;
                        let id = resolve_issue(&storage, &id)
                            .or_else(|e| report_gate_check_error(e, json))?;

                        match executor.gates().get_last_gate_run(&id, &gate_key) {
                            Ok(Some(result)) => {
                                let rendered_stdout =
                                    want_stdout.then(|| render_report_stream(&result.stdout, tail));
                                let rendered_stderr =
                                    want_stderr.then(|| render_report_stream(&result.stderr, tail));
                                if json {
                                    use jit::output::{GateFlatReportResponse, JsonOutput};
                                    let response = GateFlatReportResponse {
                                        key: gate_key.clone(),
                                        run_id: result.run_id.clone(),
                                        stdout: rendered_stdout,
                                        stderr: rendered_stderr,
                                    };
                                    let output = JsonOutput::success(response);
                                    println!("{}", output.to_json_string()?);
                                } else {
                                    // Verbatim: no headers, no decoration, no
                                    // injected trailing newline (print!, not
                                    // println!) so the stored report text is emitted
                                    // byte-for-byte.
                                    if let Some(s) = rendered_stdout {
                                        print!("{}", s);
                                    }
                                    if let Some(s) = rendered_stderr {
                                        print!("{}", s);
                                    }
                                }
                            }
                            Ok(None) => {
                                if json {
                                    use jit::output::JsonOutput;
                                    let output = JsonOutput::<Option<()>>::success(None).with_message(
                                    format!(
                                        "Gate '{}' has not been run yet for issue {}. Use 'jit gate evaluate' to run it.",
                                        gate_key, id
                                    ),
                                );
                                    println!("{}", output.to_json_string()?);
                                } else {
                                    println!(
                                        "{}",
                                        jit::output::render_gate_run_status(
                                            jit::output::GateRunLine::NeverRun,
                                            &gate_key,
                                            &id,
                                        )
                                    );
                                }
                            }
                            Err(e) => return report_gate_check_error(e, json),
                        }
                    } else {
                        let gate_key =
                            jit::dispatch::resolve_gate_key(gate_key, gate_flag, "gate status")?;

                        // Transposed-argument guard. The canonical form is
                        // `jit gate status <issue> <gate-key>`. If <id> is not an issue but
                        // <gate_key> resolves to one and <id> is a registered gate key, the
                        // two positionals are almost certainly swapped, so emit an
                        // actionable did-you-mean rather than misparsing into
                        // "issue not found".
                        let id = match resolve_issue(&storage, &id) {
                            Ok(full_id) => full_id,
                            Err(e) => {
                                let id_is_gate = executor
                                    .gates()
                                    .list_gates()
                                    .map(|gates| gates.iter().any(|g| g.key.as_str() == id))
                                    .unwrap_or(false);
                                if id_is_gate && resolve_issue(&storage, &gate_key).is_ok() {
                                    let transposed =
                                        jit::commands::GateStatusArgumentsTransposedError {
                                            issue: gate_key,
                                            gate_key: id,
                                        };
                                    if json {
                                        let json_error = classify(&transposed.into());
                                        println!("{}", json_error.to_json_string()?);
                                        std::process::exit(json_error.exit_code().code());
                                    } else {
                                        eprintln!("Error: {transposed}");
                                        eprintln!(
                                            "  Did you mean: {}",
                                            transposed.canonical_command()
                                        );
                                        std::process::exit(2);
                                    }
                                }
                                return report_gate_check_error(e, json);
                            }
                        };

                        let gate_key = parse_gate_key(gate_key, json)?;

                        match executor.gates().get_last_gate_run(&id, &gate_key) {
                            Ok(Some(result)) => {
                                if json {
                                    use jit::output::{GateRunSummary, JsonOutput};
                                    let msg = format!("Gate '{}': {}", gate_key, result.status);
                                    let summary = GateRunSummary::full(&result);
                                    let output = JsonOutput::success(summary).with_message(msg);
                                    println!("{}", output.to_json_string()?);
                                } else {
                                    print_gate_run_details(&result);
                                    let _ = output_ctx;
                                }
                            }
                            Ok(None) => {
                                if json {
                                    use jit::output::JsonOutput;
                                    let output = JsonOutput::<Option<()>>::success(None).with_message(
                                    format!(
                                        "Gate '{}' has not been run yet for issue {}. Use 'jit gate evaluate' to run it.",
                                        gate_key, id
                                    ),
                                );
                                    println!("{}", output.to_json_string()?);
                                } else {
                                    println!(
                                        "{}",
                                        jit::output::render_gate_run_status(
                                            jit::output::GateRunLine::NeverRun,
                                            &gate_key,
                                            &id,
                                        )
                                    );
                                }
                            }
                            Err(e) => return report_gate_check_error(e, json),
                        }
                    }
                }
                GateCommands::StatusAll { id, full } => {
                    use jit::domain::GateStatus;
                    let output_ctx = OutputContext::new(quiet, json);
                    let id = resolve_issue(&storage, &id)?;
                    // Latest recorded run of each required gate.
                    let (results, _) = executor.gates().get_last_gate_runs_for_issue(&id)?;
                    // Readiness across EVERY required gate (auto + manual).
                    let gate_statuses =
                        executor.gates().get_required_gate_statuses_for_issue(&id)?;

                    let total = gate_statuses.len();
                    let passed_count = gate_statuses
                        .iter()
                        .filter(|(_, s)| *s == GateStatus::Passed)
                        .count();
                    // Pending keys across all required gates (auto never run, manual
                    // never attested), preserving priority order.
                    let not_run: Vec<GateKey> = gate_statuses
                        .iter()
                        .filter(|(_, s)| *s == GateStatus::Pending)
                        .map(|(key, _)| key.clone())
                        .collect();
                    let all_passed = total == passed_count;

                    if json {
                        use jit::output::{
                            GateCheckAllResponse, GateRunSummary, GateStatusEntry, JsonOutput,
                        };
                        let msg = if all_passed {
                            format!("{}/{} required gates passed", passed_count, total)
                        } else {
                            format!(
                                "{}/{} required gates passed ({} pending, {} failed)",
                                passed_count,
                                total,
                                not_run.len(),
                                total - passed_count - not_run.len()
                            )
                        };
                        let summaries: Vec<GateRunSummary> = results
                            .iter()
                            .map(|r| {
                                if full {
                                    GateRunSummary::full(r)
                                } else {
                                    GateRunSummary::lean(r)
                                }
                            })
                            .collect();
                        let gate_status_entries: Vec<GateStatusEntry> = gate_statuses
                            .iter()
                            .map(|(gate_key, status)| GateStatusEntry {
                                key: gate_key.clone(),
                                status: *status,
                            })
                            .collect();
                        let response = GateCheckAllResponse::with_extras(
                            gate_status_entries,
                            jit::output::GateCheckSummary {
                                results: summaries,
                                passed: passed_count,
                                total,
                                not_run,
                                all_passed,
                            },
                        );
                        let output = JsonOutput::success(response).with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else if total == 0 {
                        let _ = output_ctx
                            .print_info(format!("No required gates to inspect for issue {}", id));
                    } else {
                        let _ = output_ctx.print_info(format!("Gate readiness for issue {}:", id));
                        // Gate keys that have a recorded run to print in detail.
                        let with_runs: std::collections::HashSet<&GateKey> =
                            results.iter().map(|r| &r.gate_key).collect();
                        for result in &results {
                            print_gate_run_details(result);
                        }
                        // Cover every required gate not shown above (gates without a
                        // recorded run) so a strict nonzero exit is explained.
                        for (gate_key, status) in &gate_statuses {
                            if with_runs.contains(gate_key) {
                                continue;
                            }
                            match status {
                                GateStatus::Pending => println!(
                                    "{}",
                                    jit::output::render_gate_run_status(
                                        jit::output::GateRunLine::NeverRun,
                                        gate_key,
                                        &id
                                    )
                                ),
                                GateStatus::Passed => {
                                    println!("Gate '{}' status: passed", gate_key)
                                }
                                GateStatus::Failed => {
                                    println!("Gate '{}' status: failed", gate_key)
                                }
                            }
                        }
                    }

                    if !all_passed {
                        std::process::exit(ExitCode::ValidationFailed.code());
                    }
                }
                GateCommands::Evaluate {
                    id,
                    gate_key,
                    gate_flag,
                    by,
                    force,
                } => {
                    let gate_key =
                        jit::dispatch::resolve_gate_key_for(gate_key, gate_flag, "gate evaluate")?;
                    let output_ctx = OutputContext::new(quiet, json);
                    match resolve_issue(&storage, &id).and_then(|full_id| {
                        executor.gates().pass_gate(&full_id, &gate_key, by, force)
                    }) {
                        Ok(outcome) => {
                            // Print warnings first
                            for warning in &outcome.warnings {
                                output_ctx.print_warning(warning)?;
                            }

                            if json {
                                use jit::output::JsonOutput;
                                let response = serde_json::json!({
                                    "issue_id": id,
                                    "key": gate_key,
                                    "status": "passed",
                                    "verdict": "pass",
                                    "warnings": outcome.warnings,
                                    "message": jit::output::render_gate_run_status(jit::output::GateRunLine::Passed, &gate_key, &id),
                                });
                                let output = JsonOutput::success(response);
                                println!("{}", output.to_json_string()?);
                            } else {
                                let _ =
                                    output_ctx.print_success(jit::output::render_gate_run_status(
                                        jit::output::GateRunLine::Passed,
                                        &gate_key,
                                        &id,
                                    ));
                            }
                        }
                        Err(e) => {
                            render_gate_pass_error(e, &id, &output_ctx, json)?;
                        }
                    }
                }
                GateCommands::EvaluateAll { id, by, force } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    match resolve_issue(&storage, &id)
                        .and_then(|full_id| executor.gates().pass_all_gates(&full_id, by, force))
                    {
                        Ok(outcome) => {
                            // Surface every gate's warnings first.
                            for entry in &outcome.results {
                                for warning in &entry.warnings {
                                    output_ctx.print_warning(warning)?;
                                }
                            }

                            if json {
                                use jit::output::JsonOutput;
                                let gates: Vec<serde_json::Value> = outcome
                                    .results
                                    .iter()
                                    .map(|entry| {
                                        serde_json::json!({
                                            "key": entry.gate_key,
                                            "status": "passed",
                                            "verdict": "pass",
                                            "warnings": entry.warnings,
                                        })
                                    })
                                    .collect();
                                let response = serde_json::json!({
                                    "issue_id": id,
                                    "status": "passed",
                                    "verdict": "pass",
                                    "gates": gates,
                                    "message": format!(
                                        "Passed {} required gate(s) for issue {}",
                                        outcome.results.len(),
                                        id
                                    ),
                                });
                                let output = JsonOutput::success(response);
                                println!("{}", output.to_json_string()?);
                            } else if outcome.results.is_empty() {
                                let _ = output_ctx
                                    .print_success(format!("No required gates for issue {}", id));
                            } else {
                                for entry in &outcome.results {
                                    let _ = output_ctx.print_success(
                                        jit::output::render_gate_run_status(
                                            jit::output::GateRunLine::Passed,
                                            &entry.gate_key,
                                            &id,
                                        ),
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            render_gate_pass_error(e, &id, &output_ctx, json)?;
                        }
                    }
                }
                GateCommands::EvaluateMany {
                    gate_key,
                    ids,
                    by,
                    force,
                } => {
                    let gate_key = parse_gate_key(gate_key, json)?;

                    use jit::output::BatchTarget;

                    let output_ctx = OutputContext::new(quiet, json);
                    // An id that does not resolve fails its own entry, in argument
                    // order, like an evaluation failure.
                    let resolved: Vec<(String, Result<IssueId>)> = ids
                        .into_iter()
                        .map(|requested| {
                            let id = resolve_issue(&storage, &requested);
                            (requested, id)
                        })
                        .collect();
                    let full_ids: Vec<IssueId> = resolved
                        .iter()
                        .filter_map(|(_, id)| id.as_ref().ok().cloned())
                        .collect();
                    let mut evaluated = executor
                        .gates()
                        .pass_gate_many(&full_ids, &gate_key, by, force)
                        .results
                        .into_iter();
                    let entries: Vec<_> = resolved
                        .into_iter()
                        .filter_map(|(requested, id)| match id {
                            Ok(_) => evaluated
                                .next()
                                .map(|entry| (BatchTarget::Resolved(entry.issue_id), entry.result)),
                            Err(error) => Some((BatchTarget::Unresolved(requested), Err(error))),
                        })
                        .collect();
                    let mut exit_code = None;
                    let mut results = Vec::with_capacity(entries.len());

                    for (target, result) in entries {
                        match result {
                            Ok(outcome) => {
                                if !json {
                                    for warning in &outcome.warnings {
                                        output_ctx.print_warning(warning)?;
                                    }
                                    output_ctx.print_success(
                                        jit::output::render_gate_run_status(
                                            jit::output::GateRunLine::Passed,
                                            &gate_key,
                                            target.as_str(),
                                        ),
                                    )?;
                                }
                                results.push(jit::output::GateEvaluateManyResult {
                                    target,
                                    key: gate_key.clone(),
                                    status: "passed".to_string(),
                                    verdict: Some("pass".to_string()),
                                    warnings: outcome.warnings,
                                    error: None,
                                });
                            }
                            Err(error) => {
                                let json_error = gate_pass_json_error(&error, target.as_str());
                                exit_code.get_or_insert(json_error.exit_code().code());
                                let warnings = error
                                    .downcast_ref::<jit::commands::GatePassFailed>()
                                    .map(|failure| failure.warnings.clone())
                                    .unwrap_or_default();
                                if !json {
                                    for warning in &warnings {
                                        output_ctx.print_warning(warning)?;
                                    }
                                    eprintln!(
                                        "Failed gate '{}' for issue {}: {}",
                                        gate_key,
                                        target.as_str(),
                                        error
                                    );
                                }
                                let error_value = serde_json::to_value(&json_error.error)?;
                                let verdict = json_error
                                    .error
                                    .details
                                    .as_ref()
                                    .and_then(|details| details.get("verdict"))
                                    .and_then(serde_json::Value::as_str)
                                    .map(str::to_string);
                                results.push(jit::output::GateEvaluateManyResult {
                                    target,
                                    key: gate_key.clone(),
                                    status: "failed".to_string(),
                                    verdict,
                                    warnings,
                                    error: Some(error_value),
                                });
                            }
                        }
                    }

                    if json {
                        let response = jit::output::GateEvaluateManyResponse::new(results);
                        let passed = response
                            .items
                            .iter()
                            .filter(|result| result.status == "passed")
                            .count();
                        let message = format!(
                            "Evaluated gate '{}' for {} issue(s): {} passed, {} failed",
                            gate_key,
                            response.count(),
                            passed,
                            response.count() - passed
                        );
                        println!(
                            "{}",
                            JsonOutput::success(response)
                                .with_message(message)
                                .to_json_string()?
                        );
                    }

                    if let Some(code) = exit_code {
                        std::process::exit(code);
                    }
                }
                GateCommands::Fail { id, gate_key, by } => {
                    let gate_key = parse_gate_key(gate_key, json)?;

                    let output_ctx = OutputContext::new(quiet, json);
                    match resolve_issue(&storage, &id)
                        .and_then(|full_id| executor.gates().fail_gate(&full_id, &gate_key, by))
                    {
                        Ok(warnings) => {
                            // Print warnings first
                            for warning in warnings {
                                output_ctx.print_warning(&warning)?;
                            }

                            if json {
                                use jit::output::JsonOutput;
                                let response = serde_json::json!({
                                    "issue_id": id,
                                    "key": gate_key,
                                    "status": "failed",
                                    "message": format!("Failed gate '{}' for issue {}", gate_key, id)
                                });
                                let output = JsonOutput::success(response);
                                println!("{}", output.to_json_string()?);
                            } else {
                                let _ = output_ctx.print_success(format!(
                                    "Failed gate '{}' for issue {}",
                                    gate_key, id
                                ));
                            }
                        }
                        Err(e) => {
                            if json {
                                let json_error = gate_pass_json_error(&e, &id);
                                println!("{}", json_error.to_json_string()?);
                                std::process::exit(json_error.exit_code().code());
                            } else {
                                return Err(e);
                            }
                        }
                    }
                }
                _ => unreachable!("gate definition and preset variants were dispatched above"),
            }
        }
        Commands::Graph(graph_cmd) => {
            return graph::dispatch(graph_cmd, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Rdeps { id, depth } => {
            return graph::rdeps::dispatch(
                graph::RdepsArgs { id, depth },
                &ctx,
                &mut io::stdout(),
                &mut io::stderr(),
            );
        }
        Commands::Events(command) => {
            return events::dispatch(command, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Doc(command) => {
            return doc::dispatch(command, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Archive(archive_cmd) => {
            return archive::dispatch(archive_cmd, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Query {
            subcommand,
            filters: bare_filters,
            full: bare_full,
        } => match subcommand {
            None => run_query_all(&executor, quiet, bare_filters, bare_full, json)?,
            Some(query_cmd) => {
                // Parent-level filters only apply to the bare `jit query` form.
                // When a subcommand is present they would be silently ignored
                // (a machine-readable silent-wrong-result footgun), so reject
                // them with an actionable message instead.
                reject_parent_query_filters(
                    &query_cmd,
                    bare_filters.state.as_deref(),
                    bare_filters.assignee.as_deref(),
                    bare_filters.priority.as_deref(),
                    &bare_filters.labels,
                    bare_full,
                    json,
                )?;
                match query_cmd {
                    jit::cli::QueryCommands::All { filters, full } => {
                        run_query_all(&executor, quiet, filters, full, json)?;
                    }
                    jit::cli::QueryCommands::Available { filters, full } => {
                        let output_ctx = OutputContext::new(quiet, json);
                        let filters = filters.parse()?;
                        let issues = executor.queries().query_available(filters)?;

                        if json {
                            use jit::domain::MinimalIssue;
                            use jit::output::JsonOutput;

                            let msg = format!("Found {} issue(s)", issues.len());
                            let output = if full {
                                JsonOutput::success(serde_json::to_value(
                                    jit::output::IssueListFullResponse::new(issues),
                                )?)
                            } else {
                                let minimal: Vec<MinimalIssue> =
                                    issues.iter().map(MinimalIssue::from).collect();
                                JsonOutput::success(serde_json::to_value(
                                    jit::output::IssueListResponse::new(minimal),
                                )?)
                            }
                            .with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            print_issue_list(
                                &output_ctx,
                                "Available issues (unassigned, unblocked):",
                                plain_issue_rows(&issues),
                                &[IssueColumn::Priority],
                            )?;
                        }
                    }
                    jit::cli::QueryCommands::Blocked { filters, full } => {
                        let output_ctx = OutputContext::new(quiet, json);
                        let filters = filters.parse()?;
                        let blocked = executor.queries().query_blocked_filtered(filters)?;

                        if json {
                            use jit::domain::MinimalIssue;
                            use jit::output::JsonOutput;

                            let msg = format!("Found {} issue(s)", blocked.len());
                            let output = if full {
                                use jit::domain::queries::BlockingReason;
                                use jit::output::{BlockedIssue, BlockedReason, BlockedReasonType};
                                let blocked_issues: Vec<BlockedIssue> = blocked
                                    .iter()
                                    .map(|(issue, reasons)| {
                                        let blocked_reasons = reasons
                                            .iter()
                                            .map(|r| match r {
                                                BlockingReason::Dependency { id, title, state } => {
                                                    BlockedReason {
                                                        reason_type: BlockedReasonType::Dependency,
                                                        detail: format!(
                                                            "{} ({}:{})",
                                                            id, title, state
                                                        ),
                                                    }
                                                }
                                                BlockingReason::Gate { key, status } => {
                                                    BlockedReason {
                                                        reason_type: BlockedReasonType::Gate,
                                                        detail: format!("{} ({})", key, status),
                                                    }
                                                }
                                            })
                                            .collect();
                                        BlockedIssue {
                                            issue: MinimalIssue::from(issue),
                                            blocked_reasons,
                                        }
                                    })
                                    .collect();

                                JsonOutput::success(serde_json::to_value(
                                    jit::output::BlockedFullListResponse::new(blocked_issues),
                                )?)
                            } else {
                                use jit::domain::MinimalBlockedIssue;
                                let minimal: Vec<MinimalBlockedIssue> = blocked
                                    .iter()
                                    .map(|(issue, reasons)| {
                                        let reason_strings = reasons
                                            .iter()
                                            .map(jit::output::minimal_blocked_reason)
                                            .collect();
                                        MinimalBlockedIssue::from((issue, reason_strings))
                                    })
                                    .collect();

                                JsonOutput::success(serde_json::to_value(
                                    jit::output::BlockedListResponse::new(minimal),
                                )?)
                            }
                            .with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            print_issue_list(
                                &output_ctx,
                                "Blocked issues:",
                                blocked.iter().map(|(issue, reasons)| {
                                    (issue, reasons.iter().map(ToString::to_string).collect())
                                }),
                                &[IssueColumn::Priority],
                            )?;
                        }
                    }
                    jit::cli::QueryCommands::Strategic { filters, full } => {
                        let output_ctx = OutputContext::new(quiet, json);
                        let filters = filters.parse()?;
                        let issues = executor.queries().query_strategic_filtered(filters)?;

                        if json {
                            use jit::domain::MinimalIssue;
                            use jit::output::JsonOutput;

                            let msg = format!("Found {} issue(s)", issues.len());
                            let output = if full {
                                JsonOutput::success(serde_json::to_value(
                                    jit::output::IssueListFullResponse::new(issues),
                                )?)
                            } else {
                                let minimal: Vec<MinimalIssue> =
                                    issues.iter().map(MinimalIssue::from).collect();
                                JsonOutput::success(serde_json::to_value(
                                    jit::output::IssueListResponse::new(minimal),
                                )?)
                            }
                            .with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            print_issue_list(
                                &output_ctx,
                                "Strategic issues:",
                                plain_issue_rows(&issues),
                                &[IssueColumn::Priority],
                            )?;
                        }
                    }
                    jit::cli::QueryCommands::Closed { filters, full } => {
                        let output_ctx = OutputContext::new(quiet, json);
                        let filters = filters.parse()?;
                        let issues = executor.queries().query_closed_filtered(filters)?;

                        if json {
                            use jit::domain::MinimalIssue;
                            use jit::output::JsonOutput;

                            let msg = format!("Found {} issue(s)", issues.len());
                            let output = if full {
                                JsonOutput::success(serde_json::to_value(
                                    jit::output::IssueListFullResponse::new(issues),
                                )?)
                            } else {
                                let minimal: Vec<MinimalIssue> =
                                    issues.iter().map(MinimalIssue::from).collect();
                                JsonOutput::success(serde_json::to_value(
                                    jit::output::IssueListResponse::new(minimal),
                                )?)
                            }
                            .with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            print_issue_list(
                                &output_ctx,
                                "Closed issues (Done or Rejected):",
                                plain_issue_rows(&issues),
                                &[IssueColumn::State],
                            )?;
                        }
                    }
                    jit::cli::QueryCommands::Count { by, filters } => {
                        let filters = filters.parse()?;
                        let rollup = match by {
                            jit::cli::CountDimension::State => {
                                executor.queries().query_count_by_state(filters)?
                            }
                        };

                        if json {
                            let msg = format!(
                                "{}/{} done ({}%)",
                                rollup.extras.done, rollup.extras.total, rollup.extras.percent
                            );
                            let output =
                                jit::output::JsonOutput::success(&rollup).with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            for line in rollup.to_lines() {
                                println!("{}", line);
                            }
                        }
                    }
                    jit::cli::QueryCommands::Divergence => {
                        let output_ctx = OutputContext::new(quiet, json);
                        let report = executor.graph().detect_divergences()?;

                        if json {
                            let msg = format!("{} divergence(s)", report.count());
                            let output = jit::output::JsonOutput::success(report).with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else if report.items.is_empty() {
                            let _ = output_ctx.print_success("No membership/DAG divergences");
                        } else {
                            let _ = output_ctx.print_info(format!(
                                "{} membership label(s) not backed by the DAG:",
                                report.count()
                            ));
                            for d in &report.items {
                                println!("  {} | {} | {}", d.short_id, d.label, d.title);
                            }
                        }
                    }
                } // end match query_cmd
            } // end Some(query_cmd)
        }, // end match subcommand
        Commands::Label(label_cmd) => {
            return label::dispatch(label_cmd, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Config(command) => {
            return config::dispatch(command, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Hooks(command) => {
            return hooks::dispatch(command, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Item(command) => {
            return item::dispatch(command, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Invariant(command) => {
            return invariant::dispatch(command, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Project(project_cmd) => {
            return project::dispatch(project_cmd, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Search(args) => {
            return search::dispatch(args, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Status => {
            let output_ctx = OutputContext::new(quiet, json);
            let summary = executor.validation().get_status()?;

            if json {
                let msg = format!(
                    "{} backlog, {} ready, {} in_progress, {} done",
                    summary.backlog, summary.ready, summary.in_progress, summary.done
                );
                let output = JsonOutput::success(&summary).with_message(msg);
                println!("{}", output.to_json_string()?);
            } else {
                output_ctx.print_data("Status:")?;
                output_ctx.print_data(format!("  backlog: {}", summary.backlog))?;
                output_ctx.print_data(format!("  ready: {}", summary.ready))?;
                output_ctx.print_data(format!("  in_progress: {}", summary.in_progress))?;
                output_ctx.print_data(format!("  done: {}", summary.done))?;
                output_ctx.print_data(format!("  rejected: {}", summary.rejected))?;
                output_ctx.print_data(format!("  blocked: {}", summary.blocked))?;
            }
        }
        Commands::Validate(args) => {
            return validate::dispatch(args, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Recover => {
            return claim::recover::dispatch((), &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Serve(args) => {
            // Release this process's startup mutation session before the
            // server starts: the server child runs its own recovery under the
            // ordinary bootstrap, so holding the guards across the wait
            // deadlocks the child against the parent.
            if let Some(guard) = retained_mutation_session.take() {
                guard.release()?;
            }
            return serve::dispatch(args, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Claim(command) => {
            return claim::dispatch(command, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Worktree(worktree_cmd) => match worktree_cmd {
            jit::cli::WorktreeCommands::Info => {
                use jit::commands::worktree::execute_worktree_info;
                use jit::output::{JsonOutput, OutputContext};

                match execute_worktree_info(&worktree_paths) {
                    Ok((info, warnings)) => {
                        if json {
                            let response = serde_json::json!({
                                "worktree_id": info.worktree_id,
                                "branch": info.branch,
                                "root_path": info.root_path,
                                "is_main_worktree": info.is_main_worktree,
                                "common_dir": info.common_dir,
                                "warnings": warnings,
                            });
                            let output = JsonOutput::success(response).with_message(format!(
                                "Worktree {} on branch {}",
                                info.worktree_id, info.branch
                            ));
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("Worktree Information:");
                            println!("  ID:         {}", info.worktree_id);
                            println!("  Branch:     {}", info.branch);
                            println!("  Root:       {}", info.root_path);
                            println!(
                                "  Type:       {}",
                                if info.is_main_worktree {
                                    "main worktree"
                                } else {
                                    "secondary worktree"
                                }
                            );
                            println!("  Common dir: {}", info.common_dir);
                            let output_ctx = OutputContext::new(quiet, json);
                            for warning in &warnings {
                                output_ctx.print_warning(warning)?;
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error = command_json_error(&e, ErrorCode::WorktreeInfoError);
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            jit::cli::WorktreeCommands::List => {
                use jit::commands::worktree::execute_worktree_list;
                use jit::output::{JsonOutput, OutputContext, WorktreeListResponse};

                match execute_worktree_list(&worktree_paths) {
                    Ok((worktrees, warnings)) => {
                        if json {
                            let count = worktrees.len();
                            let response = WorktreeListResponse::new(worktrees);
                            let mut value = serde_json::to_value(&response)?;
                            if let serde_json::Value::Object(map) = &mut value {
                                map.insert(
                                    "warnings".to_string(),
                                    serde_json::to_value(&warnings)?,
                                );
                            }
                            let output = JsonOutput::success(value)
                                .with_message(format!("{} worktree(s)", count));
                            println!("{}", output.to_json_string()?);
                        } else {
                            // Human-readable table format
                            println!(
                                "{:<16} {:<25} {:<50} {:>6}",
                                "WORKTREE ID", "BRANCH", "PATH", "CLAIMS"
                            );
                            println!("{}", "-".repeat(100));

                            for entry in worktrees {
                                println!(
                                    "{:<16} {:<25} {:<50} {:>6}",
                                    entry.worktree_id,
                                    entry.branch,
                                    entry.path,
                                    entry.active_claims
                                );
                            }
                            let output_ctx = OutputContext::new(quiet, json);
                            for warning in &warnings {
                                output_ctx.print_warning(warning)?;
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error = command_json_error(&e, ErrorCode::WorktreeListError);
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            jit::cli::WorktreeCommands::Exec { command } => {
                let (program, args) = command.split_first().context("no command to run")?;
                let status = jit::commands::worktree::execute_worktree_exec(
                    &storage,
                    &executor_layout,
                    program,
                    args,
                )?;
                std::process::exit(status);
            }
            jit::cli::WorktreeCommands::StoreDivergence => {
                use jit::commands::worktree::execute_worktree_store_divergence;
                use jit::output::{JsonOutput, WorktreeStoreDivergenceResponse};

                match execute_worktree_store_divergence(&worktree_paths) {
                    Ok(report) => {
                        let count = report.divergences.len();
                        if json {
                            let response = WorktreeStoreDivergenceResponse::with_extras(
                                report.divergences,
                                jit::output::StoreComparison {
                                    checkout_store: report.checkout_store,
                                    reference_store: report.reference_store,
                                },
                            );
                            let output = JsonOutput::success(serde_json::to_value(&response)?)
                                .with_message(format!("{} divergence(s)", count));
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("Checkout store:  {}", report.checkout_store);
                            match &report.reference_store {
                                Some(reference) => println!("Reference store: {}", reference),
                                None => println!(
                                    "Reference store: none (this checkout has no other store to compare against)"
                                ),
                            }
                            println!();
                            if report.divergences.is_empty() {
                                println!("No divergent records.");
                            } else {
                                println!("{:<8} {:<16} ID", "RECORD", "CLASS");
                                println!("{}", "-".repeat(72));
                                for divergence in &report.divergences {
                                    println!(
                                        "{:<8} {:<16} {}",
                                        divergence.record.as_str(),
                                        divergence.class.as_str(),
                                        divergence.id
                                    );
                                }
                                println!();
                                println!("{} divergence(s)", count);
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error =
                                command_json_error(&e, ErrorCode::WorktreeStoreDivergenceError);
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
        },
        Commands::Snapshot(snapshot_cmd) => match snapshot_cmd {
            jit::cli::SnapshotCommands::Export {
                out,
                format,
                scope,
                at,
                working_tree,
                committed_only,
                force,
            } => {
                use jit::snapshot::{SnapshotFormat, SnapshotScope};

                // Parse scope
                let snapshot_scope =
                    SnapshotScope::parse(&scope, |id| resolve_issue(&storage, id))?;

                // Parse format
                let snapshot_format = SnapshotFormat::parse(&format)
                    .with_context(|| format!("Invalid format: {}", format))?;

                // Determine source mode
                let source_mode = executor.archive().determine_snapshot_source_mode(
                    at.as_deref(),
                    working_tree,
                    committed_only,
                )?;

                if !force {
                    executor.validation().validate_silent()?;
                }

                let (result, warnings) = executor.archive().export_snapshot(
                    &current_dir,
                    &snapshot_scope,
                    &source_mode,
                    &snapshot_format,
                    out.as_deref().map(std::path::Path::new),
                )?;

                for warning in warnings {
                    eprintln!("Warning: {}", warning);
                }

                if json {
                    use jit::output::JsonOutput;

                    let output = JsonOutput::success(&result).with_message(format!(
                        "Exported {} issues to {}",
                        result.issue_count, result.path
                    ));
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Snapshot exported to: {}", result.path);
                    println!("  {} issues", result.issue_count);
                    println!("  {} documents", result.document_count);
                    if let Some(size) = result.size_bytes {
                        println!("  Archive: {} bytes", size);
                    }
                }
            }
        },
    }

    Ok(ExitCode::Success)
}

#[cfg(test)]
mod gate_findings_text_tests {
    use super::render_gate_findings_text;
    use chrono::Utc;
    use jit::declarations::GateStage;
    use jit::domain::gate_findings::{GateFinding, GateFindings};
    use jit::domain::{GateRunResult, GateRunStatus, GATE_RUN_SCHEMA_VERSION};

    fn run_with(references: Vec<String>) -> GateRunResult {
        GateRunResult {
            schema_version: GATE_RUN_SCHEMA_VERSION,
            run_id: jit::test_utils::gate_run_id(1),
            gate_key: "review".parse().unwrap(),
            stage: GateStage::Postcheck,
            issue_id: jit::test_utils::issue_id(7),
            commit: None,
            branch: None,
            status: GateRunStatus::Failed,
            started_at: Utc::now(),
            completed_at: None,
            duration_ms: None,
            exit_code: Some(1),
            stdout: String::new(),
            stderr: String::new(),
            command: "review".to_string(),
            by: None,
            message: None,
            findings: Some(GateFindings {
                verdict: "fail".to_string(),
                summary: "one defect".to_string(),
                findings: vec![GateFinding {
                    id: "F1".to_string(),
                    severity: "high".to_string(),
                    disposition: Some("blocking".to_string()),
                    origin: Some("issue-impact".to_string()),
                    summary: "unsafe write".to_string(),
                    file: Some("src/storage.rs".to_string()),
                    line: Some(12),
                    references,
                }],
            }),
            inputs_digest: None,
            origin: jit::domain::GateVerdictOrigin::Executed,
            evaluation_stability: jit::domain::GateEvaluationStability::Stable,
        }
    }

    #[test]
    fn test_render_gate_findings_text_exposes_references_exactly() {
        let rendered = render_gate_findings_text(&run_with(vec![
            "@/inv/atomic-writes".to_string(),
            "checker:opaque value".to_string(),
        ]));

        assert!(rendered.contains(r#"references: ["@/inv/atomic-writes","checker:opaque value"]"#));
    }

    #[test]
    fn test_render_gate_findings_text_json_encoding_distinguishes_collisions() {
        let comma_value = render_gate_findings_text(&run_with(vec!["a, b".to_string()]));
        let two_values =
            render_gate_findings_text(&run_with(vec!["a".to_string(), "b".to_string()]));
        let empty_value = render_gate_findings_text(&run_with(vec![String::new()]));

        assert!(comma_value.contains(r#"references: ["a, b"]"#));
        assert!(two_values.contains(r#"references: ["a","b"]"#));
        assert!(empty_value.contains(r#"references: [""]"#));
        assert_ne!(comma_value, two_values);
        assert_ne!(
            empty_value,
            render_gate_findings_text(&run_with(Vec::new()))
        );
    }

    #[test]
    fn test_render_gate_findings_text_omits_empty_references() {
        let rendered = render_gate_findings_text(&run_with(Vec::new()));

        assert!(!rendered.contains("references:"));
    }
}
