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

#![deny(unsafe_code)]

// Binary-specific module (not in library)
mod output_macros;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use jit::cli::{
    ClaimCommands, Cli, Commands, DepCommands, DocCommands, EventCommands, GateCommands,
    GraphCommands, InvariantCommands, IssueCommands, ItemCommands, RegistryCommands,
};
use jit::commands::CommandExecutor;
use jit::domain::{GateRunResult, Priority, State};
use jit::output::{ExitCode, JsonOutput, OutputContext};
use jit::storage::{IssueStore, JsonFileStorage};
use std::env;
use std::str::FromStr;

/// Helper to determine exit code from error message
fn error_to_exit_code(error: &anyhow::Error) -> ExitCode {
    // A `gate pass` checker that ran but did not pass: split checker-failure
    // (verdict `fail`, validation error) from runner/infra error (verdict
    // `error`, external error). `Passed` never produces this error.
    if let Some(gate_failure) = error.downcast_ref::<jit::commands::GatePassFailed>() {
        return match gate_failure.status {
            jit::domain::GateRunStatus::Error => ExitCode::ExternalError,
            _ => ExitCode::ValidationFailed,
        };
    }
    // Targeting a gate the issue does not require is an argument/lookup error,
    // classified before the run path, never reported as a runner error.
    if error
        .downcast_ref::<jit::commands::GateNotRequiredError>()
        .is_some()
    {
        return ExitCode::InvalidArgument;
    }
    if error
        .downcast_ref::<jit::errors::TransitionBlockedError>()
        .is_some()
        || error
            .downcast_ref::<jit::errors::ValidationFailedError>()
            .is_some()
    {
        return ExitCode::ValidationFailed;
    }
    // A failed batch-create pre-validation is an argument error (exit 2): no
    // writes happened, the file is malformed.
    if error
        .downcast_ref::<jit::commands::BatchValidationError>()
        .is_some()
    {
        return ExitCode::InvalidArgument;
    }
    // A mid-write batch-create failure is an infra/external error (exit 10): some
    // issues were created, then a write failed.
    if error
        .downcast_ref::<jit::commands::BatchWriteError>()
        .is_some()
    {
        return ExitCode::ExternalError;
    }

    // Claim/lease commands require git; running them outside a git repository is
    // an external-dependency failure (exit 10).
    if error
        .downcast_ref::<jit::errors::ClaimRequiresGitError>()
        .is_some()
    {
        return ExitCode::ExternalError;
    }

    // Check root cause for IO errors
    if let Some(io_error) = error.downcast_ref::<std::io::Error>() {
        return match io_error.kind() {
            std::io::ErrorKind::NotFound => ExitCode::NotFound,
            std::io::ErrorKind::PermissionDenied => ExitCode::PermissionDenied,
            _ => ExitCode::ExternalError,
        };
    }

    // Graph errors are typed: a cycle is a validation failure, a missing node is
    // a not-found condition. Classified by downcast, not by message text.
    if let Some(graph_error) = error.downcast_ref::<jit::GraphError>() {
        return match graph_error {
            jit::GraphError::CycleDetected => ExitCode::ValidationFailed,
            jit::GraphError::NodeNotFound { .. } => ExitCode::NotFound,
        };
    }

    // A missing storage-backed resource is a typed not-found condition (exit 3):
    // an issue or gate key (either backend), a gate-run record, a gate preset, the
    // repository itself, or a lease. Classified by downcast, not message text.
    if error
        .downcast_ref::<jit::storage::IssueNotFoundError>()
        .is_some()
        || error
            .downcast_ref::<jit::storage::GateNotFoundError>()
            .is_some()
        || error
            .downcast_ref::<jit::storage::GateRunNotFoundError>()
            .is_some()
        || error
            .downcast_ref::<jit::storage::PresetNotFoundError>()
            .is_some()
        || error
            .downcast_ref::<jit::storage::RepositoryNotFoundError>()
            .is_some()
        || error
            .downcast_ref::<jit::errors::LeaseNotFoundError>()
            .is_some()
        || error.downcast_ref::<jit::errors::NotFoundError>().is_some()
    {
        return ExitCode::NotFound;
    }

    // An already-exists condition is typed: the gate-registry case plus the shared
    // AlreadyExistsError carrier (e.g. an occupied snapshot output path).
    if error
        .downcast_ref::<jit::storage::GateAlreadyExistsError>()
        .is_some()
        || error
            .downcast_ref::<jit::errors::AlreadyExistsError>()
            .is_some()
    {
        return ExitCode::AlreadyExists;
    }

    // Invalid-argument conditions are typed: the shared InvalidArgumentError, the
    // enum parse errors (gate stage/mode), and a UTF-8 decode failure of
    // subprocess/git output (`String::from_utf8` / `str::from_utf8`, possibly
    // behind a `.context(...)` describing which output) all map to exit code 2 by
    // downcast against their concrete types.
    if error
        .downcast_ref::<jit::errors::InvalidArgumentError>()
        .is_some()
        || error
            .downcast_ref::<jit::domain::GateStageParseError>()
            .is_some()
        || error
            .downcast_ref::<jit::domain::GateModeParseError>()
            .is_some()
        || error
            .downcast_ref::<jit::document::DocumentScopeParseError>()
            .is_some()
        || error.downcast_ref::<std::string::FromUtf8Error>().is_some()
        || error.downcast_ref::<std::str::Utf8Error>().is_some()
    {
        return ExitCode::InvalidArgument;
    }

    // Path-based storage errors are typed via PathReadError; classify by variant.
    // A wrapped (`Other`) cause is re-classified by recursing on its inner error,
    // so an io::Error or an InvalidArgumentError nested in PathReadError still
    // reaches the right code.
    if let Some(path_error) = error.downcast_ref::<jit::storage::PathReadError>() {
        return match path_error {
            jit::storage::PathReadError::NotFound(_)
            | jit::storage::PathReadError::CommitNotFound(_) => ExitCode::NotFound,
            jit::storage::PathReadError::InvalidPath(_) => ExitCode::InvalidArgument,
            jit::storage::PathReadError::OutsideRepoRoot(_) => ExitCode::GenericError,
            jit::storage::PathReadError::Other(inner) => error_to_exit_code(inner),
        };
    }

    // Archive command errors are typed: a missing source document is a not-found
    // condition (3); an occupied destination is already-exists (6).
    if let Some(archive_error) = error.downcast_ref::<jit::commands::ArchiveError>() {
        return match archive_error {
            jit::commands::ArchiveError::SourceMissing { .. } => ExitCode::NotFound,
            jit::commands::ArchiveError::DestinationOccupied { .. } => ExitCode::AlreadyExists,
        };
    }

    // A missing/unreadable plan document is a not-found condition (3); a missing
    // content-parser cargo feature is a generic failure.
    if let Some(plan_error) = error.downcast_ref::<jit::commands::plan_doc::PlanDocError>() {
        return match plan_error {
            jit::commands::plan_doc::PlanDocError::Read { .. } => ExitCode::NotFound,
            jit::commands::plan_doc::PlanDocError::ContentParser(_) => ExitCode::GenericError,
        };
    }

    // A template whose internal depends_on edges form a cycle is a validation
    // failure (4); other template-config errors are generic.
    if let Some(template_error) = error.downcast_ref::<jit::templates::TemplateConfigError>() {
        return match template_error {
            jit::templates::TemplateConfigError::CyclicDependsOn { .. } => {
                ExitCode::ValidationFailed
            }
            _ => ExitCode::GenericError,
        };
    }

    // No typed classifier matched: a genuinely-unknown error keeps the historical
    // default exit code. Every condition the CLI deliberately distinguishes is
    // classified by a typed downcast above; classification is never driven by
    // matching against a human-readable error string.
    ExitCode::GenericError
}

/// Build the JSON error envelope for a failed claim/lease command.
///
/// The git-missing condition ([`jit::errors::ClaimRequiresGitError`]) is mapped
/// to the `CLAIM_REQUIRES_GIT` code so the `--json` path resolves to exit code
/// 10, matching the human path and the documented contract. The actionable
/// message (which names the git requirement) is preserved on both paths. Any
/// other failure keeps the command-specific `fallback_code`.
fn claim_json_error(
    error: &anyhow::Error,
    fallback_code: &str,
    command: &'static str,
) -> jit::output::JsonError {
    use jit::output::{ErrorCode, JsonError};
    let code = if error
        .downcast_ref::<jit::errors::ClaimRequiresGitError>()
        .is_some()
    {
        ErrorCode::CLAIM_REQUIRES_GIT
    } else {
        fallback_code
    };
    JsonError::new(code, error.to_string(), command)
}

/// Render a failed `gate pass` / `gate pass-all` outcome and terminate appropriately.
///
/// Shared by the `gate pass` and `gate pass-all` handlers so both classify the
/// same error the same way. In `--json` mode it prints a structured
/// [`JsonError`](jit::output::JsonError) and exits with its mapped code:
/// `GatePassFailed` becomes a checker failure (`GATE_FAILED`, exit 4, verdict
/// `fail`) or a runner error (`IO_ERROR`, exit 10, verdict `error`) per the
/// carried [`GateRunStatus`](jit::domain::GateRunStatus); `GateNotRequiredError`
/// becomes `INVALID_ARGUMENT` (exit 2); an unresolved id becomes
/// `ISSUE_NOT_FOUND` (exit 3); anything else `GATE_ERROR`. In non-JSON mode it
/// surfaces any gate-failure warnings and returns `Err(e)` so the top-level
/// handler maps the exit code via [`error_to_exit_code`].
fn render_gate_pass_error(
    e: anyhow::Error,
    id: &str,
    output_ctx: &OutputContext,
    json: bool,
    command: &str,
) -> Result<()> {
    if !json {
        if let Some(gate_failure) = e.downcast_ref::<jit::commands::GatePassFailed>() {
            for warning in &gate_failure.warnings {
                output_ctx.print_warning(warning)?;
            }
        }
        return Err(e);
    }

    use jit::output::JsonError;
    let json_error = if let Some(gate_failure) = e.downcast_ref::<jit::commands::GatePassFailed>() {
        // Distinguish a checker failure (verdict `fail`, exit 4) from a
        // runner/infra error (verdict `error`, exit 10). The error code drives
        // the exit code, so the JSON and non-JSON paths agree.
        let (error_code, verdict) = match gate_failure.status {
            jit::domain::GateRunStatus::Error => ("IO_ERROR", "error"),
            _ => ("GATE_FAILED", "fail"),
        };
        JsonError::new(error_code, e.to_string(), command)
            .with_details(serde_json::json!({
                "issue_id": gate_failure.issue_id,
                "gate_key": gate_failure.gate_key,
                "status": "failed",
                "verdict": verdict,
                "checker_result": gate_failure.result,
                "warnings": gate_failure.warnings,
            }))
            .with_suggestion(format!(
                "Inspect the checker result with: jit gate check {} {}",
                gate_failure.issue_id, gate_failure.gate_key
            ))
            .with_suggestion(format!(
                "Fix the failing gate and rerun: jit gate pass {} {}",
                gate_failure.issue_id, gate_failure.gate_key
            ))
    } else if let Some(not_required) = e.downcast_ref::<jit::commands::GateNotRequiredError>() {
        // Pre-verdict argument error: not a gate verdict, so it carries no
        // `verdict` field.
        JsonError::new("INVALID_ARGUMENT", e.to_string(), command)
            .with_details(serde_json::json!({
                "issue_id": not_required.issue_id,
                "gate_key": not_required.gate_key,
            }))
            .with_suggestion(format!(
                "Add the gate first: jit gate add {} {}",
                not_required.issue_id, not_required.gate_key
            ))
    } else if e
        .downcast_ref::<jit::storage::IssueNotFoundError>()
        .is_some()
    {
        // Pre-verdict lookup error: issue id did not resolve.
        JsonError::issue_not_found(id, command)
    } else {
        JsonError::new("GATE_ERROR", e.to_string(), command)
    };
    println!("{}", json_error.to_json_string()?);
    std::process::exit(json_error.exit_code().code());
}

/// Print the outcome of a graph-template apply (`jit apply <template> <container>`).
///
/// In `--json` mode emits the structured
/// [`TemplateApplyResult`](jit::commands::TemplateApplyResult) (template name,
/// resolved anchor bindings, created/refreshed node ids by role, and the
/// pre-apply anchor dependency snapshots) plus the freshly-loaded created
/// issues, keyed by role. In quiet mode prints just the created node ids (one per
/// line, role-ordered) for scripting; otherwise a short human summary.
fn print_apply_result(
    storage: &JsonFileStorage,
    result: &jit::commands::TemplateApplyResult,
    container: &str,
    quiet: bool,
    json: bool,
) -> Result<()> {
    if json {
        // Load each created node so the JSON consumer gets the full issues
        // alongside the role→id map.
        let created_issues: serde_json::Map<String, serde_json::Value> = result
            .created_node_ids_by_role
            .iter()
            .map(|(role, id)| {
                let issue = storage.load_issue(id)?;
                Ok((role.clone(), serde_json::to_value(issue)?))
            })
            .collect::<Result<_>>()?;
        let data = serde_json::json!({
            "template": result.template,
            "container": container,
            "anchor_bindings": result.anchor_bindings,
            "created_node_ids_by_role": result.created_node_ids_by_role,
            "anchor_dependency_snapshots": result.anchor_dependency_snapshots,
            "created_issues": created_issues,
        });
        let msg = format!("Applied template '{}' to {}", result.template, container);
        let output = JsonOutput::success(data, "apply").with_message(msg);
        println!("{}", output.to_json_string()?);
    } else if quiet {
        for id in result.created_node_ids_by_role.values() {
            println!("{}", id);
        }
    } else {
        let roles = result
            .created_node_ids_by_role
            .iter()
            .map(|(role, id)| format!("{role}={id}"))
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "Applied template '{}' to {}: {}",
            result.template, container, roles
        );
    }
    Ok(())
}

/// Set up .gitattributes with merge drivers for jit files.
/// Only runs if we're in a git repository.
fn setup_gitattributes() -> Result<()> {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    // Check if we're in a git repository
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();

    let repo_root = match output {
        Ok(o) if o.status.success() => String::from_utf8(o.stdout)?.trim().to_string(),
        _ => return Ok(()), // Not in a git repo, skip
    };

    let gitattributes_path = Path::new(&repo_root).join(".gitattributes");
    let jit_marker = "# JIT merge drivers";
    let jit_config = format!(
        "{}\n.jit/events.jsonl merge=union\n.jit/claims.jsonl merge=union\n",
        jit_marker
    );

    if gitattributes_path.exists() {
        let content = fs::read_to_string(&gitattributes_path)?;

        // Check if already configured (idempotent)
        if content.contains(jit_marker) {
            return Ok(());
        }

        // Append to existing file
        let new_content = if content.ends_with('\n') {
            format!("{}\n{}", content, jit_config)
        } else {
            format!("{}\n\n{}", content, jit_config)
        };
        fs::write(&gitattributes_path, new_content)?;
    } else {
        // Create new file
        fs::write(&gitattributes_path, jit_config)?;
    }

    Ok(())
}

/// Resolve the gate key from the CLI's positional-or-flag pair (REQ-03).
///
/// Exactly one of `positional` and `flag` must be `Some`. Supplying both or
/// neither returns a typed [`InvalidArgumentError`](jit::errors::InvalidArgumentError)
/// (exit code 2) carrying an actionable message — routed through the normal error
/// path so `error_to_exit_code` classifies it and `--json` callers can render it
/// as a machine-readable error (see [`resolve_gate_key_for`]).
fn resolve_gate_key(
    positional: Option<String>,
    flag: Option<String>,
    command: &str,
) -> Result<String> {
    match (positional, flag) {
        (Some(pos), None) => Ok(pos),
        (None, Some(flag_val)) => Ok(flag_val),
        (Some(_), Some(_)) => Err(jit::errors::InvalidArgumentError::new(format!(
            "provide the gate key as a positional argument OR via --gate, not both.\n\
             Usage: jit {command} <ISSUE_ID> <GATE_KEY>\n\
             Usage: jit {command} <ISSUE_ID> --gate <GATE_KEY>"
        ))
        .into()),
        (None, None) => Err(jit::errors::InvalidArgumentError::new(format!(
            "a gate key is required; provide it as a positional argument or via --gate.\n\
             Usage: jit {command} <ISSUE_ID> <GATE_KEY>\n\
             Usage: jit {command} <ISSUE_ID> --gate <GATE_KEY>"
        ))
        .into()),
    }
}

/// [`resolve_gate_key`] with `--json`-aware error rendering. On the both/neither
/// error, a `--json` caller emits a machine-readable `INVALID_ARGUMENT` JSON
/// error (exit 2) instead of letting the plain-text error bubble to the top-level
/// handler; a non-`--json` caller propagates the typed error unchanged.
fn resolve_gate_key_for(
    positional: Option<String>,
    flag: Option<String>,
    command: &str,
    json: bool,
) -> Result<String> {
    match resolve_gate_key(positional, flag, command) {
        Ok(key) => Ok(key),
        Err(e) => {
            if json {
                let json_error = jit::output::JsonError::new(
                    jit::output::ErrorCode::INVALID_ARGUMENT,
                    e.to_string(),
                    command,
                );
                println!("{}", json_error.to_json_string()?);
                std::process::exit(json_error.exit_code().code());
            }
            Err(e)
        }
    }
}

fn print_gate_run_details(result: &GateRunResult) {
    let status_str = match result.status {
        jit::domain::GateRunStatus::Passed => "passed",
        jit::domain::GateRunStatus::Failed => "failed",
        jit::domain::GateRunStatus::Error => "error",
        _ => "unknown",
    };

    println!(
        "Gate '{}' last run: {} (exit code: {})",
        result.gate_key,
        status_str,
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
    if !result.stdout.is_empty() {
        let lines: Vec<&str> = result.stdout.lines().collect();
        println!("  stdout:\n    {}", lines.join("\n    "));
    }
    if !result.stderr.is_empty() {
        let lines: Vec<&str> = result.stderr.lines().collect();
        println!("  stderr:\n    {}", lines.join("\n    "));
    }
}

/// Print a dependency tree with tree symbols (├─, └─, │)
fn print_dependency_tree(nodes: &[jit::output::DependencyTreeNode], prefix: &str, is_root: bool) {
    let count = nodes.len();
    for (i, node) in nodes.iter().enumerate() {
        let is_last = i == count - 1;

        // Determine tree symbols
        let (connector, child_prefix) = if is_root {
            ("  ", "  ")
        } else if is_last {
            ("└─ ", "   ")
        } else {
            ("├─ ", "│  ")
        };

        // State symbol
        let state_symbol = node.state_symbol();

        // Shared indicator
        let shared_marker = if node.shared.unwrap_or(false) {
            " (shared)"
        } else {
            ""
        };

        // Print the node
        println!(
            "{}{}{} {} - {}{}",
            prefix, connector, state_symbol, node.short_id, node.title, shared_marker
        );

        // Recursively print children
        if !node.children.is_empty() {
            let new_prefix = format!("{}{}", prefix, child_prefix);
            print_dependency_tree(&node.children, &new_prefix, false);
        }
    }
}

/// Build the [`IssueShowResponse`](jit::output::IssueShowResponse) for a single
/// issue id, loading its enriched dependencies and gate runs.
///
/// Shared by the multi-id `--json` array path and the multi-id non-JSON loop so
/// each issue renders from the same shape as `issue show --json`.
fn build_issue_show_response<S: IssueStore>(
    executor: &CommandExecutor<S>,
    id: &str,
) -> Result<jit::output::IssueShowResponse> {
    let issue = executor
        .show_issue(id)
        .with_context(|| format!("Failed to load issue {}", id))?;
    let enriched_deps = executor.get_dependencies_enriched(&issue);
    let gate_runs = executor
        .list_gate_runs(&issue.id, None)
        .with_context(|| format!("Failed to load gate runs for issue {}", issue.id))?;
    Ok(jit::output::IssueShowResponse::from_issue(
        issue,
        enriched_deps,
        &gate_runs,
    ))
}

/// Render a single resolved addressable item as a JSON envelope or a short block.
///
/// Shared by `jit item show/resolve` and the `jit issue show <issue>/<self-id>`
/// qualified-id path so both render an addressed item identically.
fn print_item_show(result: &jit::commands::ItemShowResult, json: bool, quiet: bool) -> Result<()> {
    let output_ctx = OutputContext::new(quiet, json);
    if json {
        let output = JsonOutput::success(result, "item show");
        println!("{}", output.to_json_string()?);
    } else {
        output_ctx.print_data(format!("Qualified id: {}", result.item.qualified_id))?;
        output_ctx.print_data(format!("Kind:         {}", result.item.kind))?;
        output_ctx.print_data(format!("Self id:      {}", result.item.self_id))?;
        match (&result.issue_full_id, &result.issue_title) {
            (Some(full_id), Some(title)) => {
                output_ctx.print_data(format!("Issue:        {full_id} | {title}"))?;
            }
            // A project-scoped item (`@/<self-id>`) has no owning issue.
            _ => {
                output_ctx.print_data("Scope:        @ (project)".to_string())?;
            }
        }
        output_ctx.print_data(format!("Text:         {}", result.item.text))?;
    }
    Ok(())
}

/// Dispatch the `jit item` subcommands.
///
/// A thin delegation over the [`CommandExecutor`] item methods: it selects the
/// list/search/show executor call, then renders the result as JSON or as
/// human-readable lines.
fn run_item<S: IssueStore>(
    executor: &CommandExecutor<S>,
    command: ItemCommands,
    quiet: bool,
) -> Result<()> {
    // The `--json` flag is per-subcommand; extract it up front so a FAILURE can
    // also honor the machine-readable contract (finding 4): when --json is set,
    // an error is rendered as a JSON object and the process exits with the
    // error's code, rather than the top-level plain `Error: ...`.
    let json = match &command {
        ItemCommands::List { json, .. }
        | ItemCommands::Search { json, .. }
        | ItemCommands::Show { json, .. }
        | ItemCommands::Resolve { json, .. } => *json,
    };

    let result = run_item_inner(executor, command, quiet);
    if let Err(e) = result {
        handle_json_error!(
            json,
            e,
            jit::output::JsonError::new("ITEM_COMMAND_FAILED", e.to_string(), "item")
        );
    }
    Ok(())
}

/// Inner dispatch for `jit item`; errors are converted to JSON by
/// [`run_item`] when `--json` is set.
fn run_item_inner<S: IssueStore>(
    executor: &CommandExecutor<S>,
    command: ItemCommands,
    quiet: bool,
) -> Result<()> {
    use jit::commands::ItemListResult;

    // Render an item list either as a JSON envelope or one line per item.
    fn print_list(result: &ItemListResult, json: bool, quiet: bool) -> Result<()> {
        let output_ctx = OutputContext::new(quiet, json);
        if json {
            let msg = format!("Found {} item(s)", result.count);
            let output = JsonOutput::success(result, "item list").with_message(msg);
            println!("{}", output.to_json_string()?);
        } else if result.items.is_empty() {
            let _ = output_ctx.print_info("No addressable items found");
        } else {
            for item in &result.items {
                output_ctx.print_data(format!(
                    "{}  [{}]  {}",
                    item.qualified_id, item.kind, item.text
                ))?;
            }
        }
        Ok(())
    }

    match command {
        ItemCommands::List { kind, json } => {
            let result = executor.list_items(kind.as_deref())?;
            print_list(&result, json, quiet)?;
        }
        ItemCommands::Search { query, kind, json } => {
            let result = executor.search_items(&query, kind.as_deref())?;
            print_list(&result, json, quiet)?;
        }
        ItemCommands::Show { qualified_id, json }
        | ItemCommands::Resolve { qualified_id, json } => {
            let result = executor.show_item(&qualified_id)?;
            print_item_show(&result, json, quiet)?;
        }
    }
    Ok(())
}

/// Run `jit invariant <subcommand>`.
///
/// A thin delegation over the [`CommandExecutor`] invariant methods: `render`
/// projects the registry into its configured documentation target and reports the
/// written target; `check` runs the enforcement-drift check (the sole
/// declared-but-unenforced direction) and exits non-zero (via
/// [`run_invariant_inner`]) when any drift is present. On
/// `--json` a failure is rendered as a JSON error object (honoring the
/// machine-readable contract) rather than the top-level plain `Error: ...`.
fn run_invariant<S: IssueStore>(
    executor: &CommandExecutor<S>,
    command: InvariantCommands,
    quiet: bool,
) -> Result<()> {
    let json = match &command {
        InvariantCommands::Render { json } => *json,
        InvariantCommands::Check { json } => *json,
    };

    let result = run_invariant_inner(executor, command, quiet);
    if let Err(e) = result {
        handle_json_error!(
            json,
            e,
            jit::output::JsonError::new("INVARIANT_COMMAND_FAILED", e.to_string(), "invariant")
        );
    }
    Ok(())
}

/// Inner dispatch for `jit invariant`; errors are converted to JSON by
/// [`run_invariant`] when `--json` is set.
fn run_invariant_inner<S: IssueStore>(
    executor: &CommandExecutor<S>,
    command: InvariantCommands,
    quiet: bool,
) -> Result<()> {
    match command {
        InvariantCommands::Render { json } => {
            let result = executor.render_invariants()?;
            let output_ctx = OutputContext::new(quiet, json);
            if json {
                let msg = format!(
                    "Rendered {} invariant(s) to {}",
                    result.count, result.target
                );
                let output = JsonOutput::success(&result, "invariant render").with_message(msg);
                println!("{}", output.to_json_string()?);
            } else {
                output_ctx.print_data(format!(
                    "Rendered {} invariant(s) to {} ({} mode)",
                    result.count, result.target, result.mode
                ))?;
            }
        }
        InvariantCommands::Check { json } => {
            let result = executor.check_invariants()?;
            let exit_nonzero = result.has_drift();
            let output_ctx = OutputContext::new(quiet, json);
            if json {
                let msg = if exit_nonzero {
                    format!("Enforcement drift: {} finding(s)", result.count)
                } else {
                    "No enforcement drift".to_string()
                };
                let output = JsonOutput::success(&result, "invariant check").with_message(msg);
                println!("{}", output.to_json_string()?);
            } else if result.findings.is_empty() {
                output_ctx.print_data("✓ No enforcement drift".to_string())?;
            } else {
                for finding in &result.findings {
                    // Every finding is declared-but-unenforced (the sole drift
                    // direction); the message names that direction inline.
                    println!("❌ {}", finding.message());
                }
                eprintln!("Enforcement drift: {} finding(s)", result.count);
            }
            // Exit non-zero (4) when any drift is present, matching the project's
            // validation-failed convention. Done AFTER emitting output so `--json`
            // still prints a valid payload.
            if exit_nonzero {
                std::process::exit(jit::ExitCode::ValidationFailed.code());
            }
        }
    }
    Ok(())
}

/// Run the `query all` listing, shared by `jit query all` and `jit issue list`.
///
/// Both commands take identical filters/flags and must produce identical output,
/// so they delegate to this single implementation.
#[allow(clippy::too_many_arguments)]
fn run_query_all<S: IssueStore>(
    executor: &CommandExecutor<S>,
    quiet: bool,
    state: Option<String>,
    assignee: Option<String>,
    priority: Option<String>,
    label: Option<String>,
    full: bool,
    json: bool,
) -> Result<()> {
    let output_ctx = OutputContext::new(quiet, json);
    let state_filter = state.as_ref().map(|s| State::from_str(s)).transpose()?;
    let priority_filter = priority
        .as_ref()
        .map(|p| Priority::from_str(p))
        .transpose()?;
    let issues = executor.query_all(
        state_filter,
        assignee.as_deref(),
        priority_filter,
        label.as_deref(),
    )?;

    if json {
        use jit::domain::MinimalIssue;
        use jit::output::JsonOutput;
        use serde_json::json;

        let msg = format!("Found {} issue(s)", issues.len());
        let output = if full {
            JsonOutput::success(
                json!({
                    "count": issues.len(),
                    "issues": issues,
                }),
                "query all",
            )
        } else {
            let minimal: Vec<MinimalIssue> = issues.iter().map(MinimalIssue::from).collect();
            JsonOutput::success(
                json!({
                    "count": minimal.len(),
                    "issues": minimal,
                }),
                "query all",
            )
        }
        .with_message(msg);
        println!("{}", output.to_json_string()?);
    } else {
        let _ = output_ctx.print_info("All issues (filtered):");
        for issue in &issues {
            println!(
                "  {} | {} | {:?} | {:?}",
                issue.id, issue.title, issue.state, issue.priority
            );
        }
        let _ = output_ctx.print_info(format!("\nTotal: {}", issues.len()));
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
    println!("State: {:?}", response.state);
    println!("Priority: {:?}", response.priority);
    println!("Assignee: {:?}", response.assignee);

    if response.dependencies.is_empty() {
        println!("Dependencies: None");
    } else {
        let done_count = response
            .dependencies
            .iter()
            .filter(|d| d.state.is_terminal())
            .count();
        println!(
            "Dependencies ({}/{} complete):",
            done_count,
            response.dependencies.len()
        );
        for dep in &response.dependencies {
            println!(
                "  {} {} - {} [{}]",
                dep.state_symbol(),
                dep.short_id(),
                dep.title,
                format!("{:?}", dep.state).to_lowercase()
            );
        }
    }

    if response.gates.is_empty() {
        println!("Gates: (none)");
    } else {
        println!("Gates:");
        for gate in &response.gates {
            let last_run = gate.last_run_at.as_deref().unwrap_or("never run");
            println!(
                "  - {} [{:?}] (last run: {})",
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
/// `--assignee`, `--priority`, `--label`, `--full`, `--json`). When a
/// subcommand such as `available` is also given, those parent-level flags
/// would be silently dropped — a silent-wrong-result footgun for orchestrators
/// consuming `--json`. Detect any that were supplied and return an actionable
/// error pointing the user at the subcommand form or the bare form.
fn reject_parent_query_filters(
    query_cmd: &jit::cli::QueryCommands,
    state: Option<&str>,
    assignee: Option<&str>,
    priority: Option<&str>,
    label: Option<&str>,
    full: bool,
    json: bool,
) -> Result<()> {
    use jit::cli::QueryCommands;

    let offending: Vec<&str> = [
        state.map(|_| "--state"),
        assignee.map(|_| "--assignee"),
        priority.map(|_| "--priority"),
        label.map(|_| "--label"),
        full.then_some("--full"),
        json.then_some("--json"),
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
    };

    Err(anyhow!(
        "filter(s) {flags} were given before the `{sub}` subcommand, where they \
         are ignored. Put them after the subcommand (e.g. `jit query {sub} {flags}`), \
         or drop the subcommand to use the bare form (e.g. `jit query {flags}`).",
        flags = offending.join(" "),
        sub = sub,
    ))
}

fn main() {
    let exit_code = match run() {
        Ok(()) => ExitCode::Success,
        Err(e) => {
            eprintln!("Error: {}", e);
            error_to_exit_code(&e)
        }
    };

    if exit_code != ExitCode::Success {
        std::process::exit(exit_code.code());
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let quiet = cli.quiet;

    // Handle --schema flag first
    if cli.schema {
        use jit::CommandSchema;
        let schema = CommandSchema::generate();
        let json = serde_json::to_string_pretty(&schema)?;
        println!("{}", json);
        return Ok(());
    }

    // Ensure command is provided
    let command = cli
        .command
        .ok_or_else(|| anyhow::anyhow!("No command provided. Use --help for usage."))?;

    if let Commands::Version { json } = &command {
        let info = jit::build_info::version_info();
        if *json {
            let output = JsonOutput::success(&info, "version");
            println!("{}", output.to_json_string()?);
        } else {
            println!("Version: {}", info.version);
            println!("Commit: {} ({})", info.git_short_commit, info.git_commit);
            let dirty = info
                .git_dirty
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            println!("Dirty: {}", dirty);
            println!("Profile: {}", info.build_profile);
            println!("Built: {}", info.build_timestamp);
            println!("Target: {}", info.target);
        }
        return Ok(());
    }

    let current_dir = env::current_dir()?;

    // Determine jit data directory: JIT_DATA_DIR env var or default to .jit/
    let jit_dir = if let Ok(custom_dir) = env::var("JIT_DATA_DIR") {
        current_dir.join(custom_dir)
    } else {
        current_dir.join(".jit")
    };

    let storage = JsonFileStorage::new(&jit_dir);
    let mut executor = CommandExecutor::new(storage.clone());

    match &command {
        Commands::Init { hierarchy_template } => {
            let output_ctx = OutputContext::new(quiet, false);

            // Resolve the template before init so we can error early on bad names
            let template = if let Some(template_name) = hierarchy_template {
                Some(
                    jit::hierarchy_templates::HierarchyTemplate::get(template_name)
                        .ok_or_else(|| anyhow!("Unknown hierarchy template: {}", template_name))?,
                )
            } else {
                None
            };

            // Note whether config.toml already exists before we touch anything
            let config_path = jit_dir.join("config.toml");
            let config_already_existed = config_path.exists();

            let (worktree_identity, init_warnings) = executor.init()?;
            for warning in &init_warnings {
                output_ctx.print_warning(warning)?;
            }

            // Set up .gitattributes for merge drivers (if in git repo)
            if let Err(e) = setup_gitattributes() {
                eprintln!("Warning: Could not set up .gitattributes: {}", e);
            }

            // The chosen template defines the on-disk config.toml (namespace
            // registry + type hierarchy) from which the fixed default rules.toml
            // is derived.
            let chosen = template
                .as_ref()
                .cloned()
                .unwrap_or_else(jit::hierarchy_templates::HierarchyTemplate::default);

            // Write config.toml only when it did not already exist (idempotent).
            // Atomic temp-file + rename per the project's file-write invariant.
            if !config_already_existed {
                jit::storage::atomic_write::write_file_atomic(
                    &config_path,
                    &chosen.generate_config_toml(),
                )?;
            }

            // Scaffold .jit/rules.toml (the operative single source of truth) with
            // the FIXED default ruleset derived from the repo's namespace registry
            // + type hierarchy. A no-op when rules.toml already exists (re-init
            // never clobbers user edits).
            let scaffolded = executor.scaffold_default_rules()?;
            if scaffolded {
                let _ = output_ctx.print_success("Scaffolded .jit/rules.toml");
            }

            if let Some(ref t) = template {
                let _ = output_ctx
                    .print_success(format!("Initialized with '{}' hierarchy template", t.name));
            } else if let Some(identity) = worktree_identity {
                let _ = output_ctx.print_success(format!(
                    "Initialized jit repository (worktree: {})",
                    identity.worktree_id
                ));
            } else {
                let _ = output_ctx.print_success("Initialized jit repository");
            }
        }
        _ => {
            // Validate repository exists for all commands except init
            storage.validate()?;
        }
    }

    // Normalize top-level first-guess aliases into their canonical noun/verb form
    // so `jit rdeps <id>` and `jit list` resolve to `jit graph rdeps` /
    // `jit issue list` and reuse those handlers verbatim.
    let command = match command {
        Commands::Rdeps { id, depth, json } => {
            Commands::Graph(GraphCommands::Rdeps { id, depth, json })
        }
        Commands::List {
            state,
            assignee,
            priority,
            label,
            full,
            json,
        } => Commands::Issue(IssueCommands::List {
            state,
            assignee,
            priority,
            label,
            full,
            json,
        }),
        other => other,
    };

    match command {
        Commands::Init { .. } => {
            // Already handled above
        }
        Commands::Rdeps { .. } | Commands::List { .. } => {
            unreachable!("top-level rdeps/list are normalized to canonical commands above")
        }
        Commands::Version { .. } => unreachable!("version is handled before repository validation"),
        Commands::Issue(issue_cmd) => {
            match issue_cmd {
                IssueCommands::Create {
                    positional_title,
                    title,
                    description,
                    priority,
                    issue_type,
                    gate,
                    label,
                    content_format,
                    force,
                    orphan,
                    json,
                } => {
                    // Clap guarantees exactly one of the two title forms is set.
                    let title = positional_title
                        .or(title)
                        .expect("clap requires title (positional or --title)");

                    let prio = Priority::from_str(&priority)?;
                    let content_format = content_format
                        .map(|s| jit::domain::ContentFormat::from_str(&s))
                        .transpose()?;

                    // The command layer owns `--type` validation and `type:<kind>`
                    // label derivation; the CLI just forwards the typed value.
                    let (id, warnings) = executor.create_issue(
                        title,
                        description,
                        prio,
                        gate,
                        label,
                        content_format,
                        issue_type,
                        force,
                    )?;

                    // Print warnings to stderr
                    for warning in &warnings {
                        eprintln!("⚠️  Warning: {}", warning);
                    }

                    let output_ctx = OutputContext::new(quiet, json);

                    if json {
                        let issue = storage.load_issue(&id)?;
                        let msg = format!("Created issue {} - {}", issue.short_id(), issue.title);
                        let enriched_deps = executor.get_dependencies_enriched(&issue);
                        let gate_runs =
                            executor.list_gate_runs(&issue.id, None).with_context(|| {
                                format!("Failed to load gate runs for issue {}", issue.id)
                            })?;
                        let response = jit::output::IssueShowResponse::from_issue(
                            issue,
                            enriched_deps,
                            &gate_runs,
                        );
                        let output =
                            JsonOutput::success(response, "issue create").with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        // In quiet mode, output just the ID for scripting
                        if quiet {
                            println!("{}", id);
                        } else {
                            println!("Created issue: {}", id);
                        }

                        // Surface the built-in type-hierarchy warnings
                        // (orphan-leaf / strategic-consistency) for the new issue
                        // unless --force or --quiet. These are now GRAPH rule
                        // findings (`default:orphan-leaf` /
                        // `default:strategic-consistency`) produced by the rule
                        // engine, not a hard-coded check. `--orphan` suppresses the
                        // orphan-leaf hint (acknowledged intentional orphan).
                        if !force && !quiet {
                            let issues = storage.list_issues()?;
                            let graph_findings = executor.evaluate_graph_rules(&issues)?;
                            for gf in graph_findings.iter().filter(|gf| {
                                gf.issue_id.as_deref() == Some(id.as_str())
                                    && !(orphan && gf.finding.rule == "default:orphan-leaf")
                            }) {
                                let _ =
                                    output_ctx.print_warning(format!("\n⚠ {}", gf.finding.message));
                            }
                        }
                    }
                }
                IssueCommands::BatchCreate { from_json, json } => {
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

                    // The method does FULL pre-validation before any write and
                    // returns a typed error (validation list or partial-write map)
                    // on failure, which the top-level handler maps to an exit code.
                    let outcome = executor.batch_create_from_json(defs)?;

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
                    state,
                    assignee,
                    priority,
                    labels,
                    full,
                    json,
                } => {
                    let output_ctx = OutputContext::new(quiet, json);

                    // The positional query is optional only when at least one
                    // filter narrows the search; otherwise the command would
                    // dump every issue, which is almost never intended.
                    let has_filter = state.is_some()
                        || assignee.is_some()
                        || priority.is_some()
                        || !labels.is_empty();
                    if query.is_none() && !has_filter {
                        return Err(jit::errors::InvalidArgumentError::new(
                            "invalid arguments: provide a search query or at least one filter (--label/--state/--assignee/--priority)",
                        )
                        .into());
                    }

                    // Reject malformed label filters up front with a clear,
                    // argument-classified error rather than silently matching
                    // nothing.
                    for label in &labels {
                        jit::labels::validate_label(label).map_err(|_| {
                            jit::errors::InvalidArgumentError::new(format!(
                                "invalid --label filter '{label}'"
                            ))
                        })?;
                    }

                    let state_filter = state.map(|s| State::from_str(&s)).transpose()?;
                    let priority_filter = priority.map(|p| Priority::from_str(&p)).transpose()?;
                    let issues = executor.search_issues_with_filters(
                        query.as_deref().unwrap_or(""),
                        priority_filter,
                        state_filter,
                        assignee,
                        &labels,
                    )?;

                    if json {
                        use jit::output::JsonOutput;
                        use serde_json::json;

                        // Use MinimalIssue unless --full flag is provided
                        let output_data = if full {
                            json!({
                                "query": query,
                                "issues": issues,
                                "count": issues.len(),
                            })
                        } else {
                            use jit::domain::MinimalIssue;
                            let minimal_issues: Vec<MinimalIssue> =
                                issues.iter().map(MinimalIssue::from).collect();
                            json!({
                                "query": query,
                                "issues": minimal_issues,
                                "count": minimal_issues.len(),
                            })
                        };

                        let msg = match &query {
                            Some(q) => format!("Found {} issue(s) matching '{}'", issues.len(), q),
                            None => format!("Found {} issue(s)", issues.len()),
                        };
                        let output =
                            JsonOutput::success(output_data, "issue search").with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_info(format!("Found {} issue(s):", issues.len()));
                        for issue in issues {
                            println!(
                                "{} | {} | {:?} | {:?}",
                                issue.id, issue.title, issue.state, issue.priority
                            );
                        }
                    }
                }
                IssueCommands::Show {
                    ids,
                    summary,
                    field,
                    fields,
                    json,
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

                    // Multi-id: return a JSON array of full issue objects in
                    // argument order. Without --json, fall through to printing
                    // each issue's human view in order.
                    if ids.len() > 1 && json {
                        let responses = ids
                            .iter()
                            .map(|id| build_issue_show_response(&executor, id))
                            .collect::<Result<Vec<_>>>()?;
                        let output = jit::output::JsonOutput::success(responses, "issue show");
                        println!("{}", output.to_json_string()?);
                        return Ok(());
                    }

                    if ids.len() > 1 {
                        // Non-JSON multi-id: print each issue's human view.
                        for id in &ids {
                            let response = build_issue_show_response(&executor, id)?;
                            print_issue_show_human(&response);
                        }
                        return Ok(());
                    }

                    // Single id (guaranteed by clap `required = true`).
                    let id = ids[0].clone();

                    // A `<issue>/<self-id>` argument addresses an item, not an
                    // issue (issue ids never contain '/'). Resolve and render the
                    // addressed item through the same item resolver as
                    // `jit item show`, honoring --json (REQ-04 / concept).
                    if !projecting && !summary && id.contains('/') {
                        match executor.show_item(&id) {
                            Ok(result) => {
                                print_item_show(&result, json, quiet)?;
                                return Ok(());
                            }
                            Err(e) => {
                                // `handle_json_error!` either prints the JSON error
                                // and exits, or returns `Err` (non-JSON path).
                                handle_json_error!(
                                    json,
                                    e,
                                    jit::output::JsonError::new(
                                        "ITEM_NOT_FOUND",
                                        e.to_string(),
                                        "issue show",
                                    )
                                );
                            }
                        }
                    }

                    match executor.show_issue(&id) {
                        Ok(issue) => {
                            // Field projection: print the named field(s) and return.
                            if projecting {
                                let enriched_deps = executor.get_dependencies_enriched(&issue);
                                let gate_runs = executor
                                    .list_gate_runs(&issue.id, None)
                                    .with_context(|| {
                                        format!("Failed to load gate runs for issue {}", issue.id)
                                    })?;
                                let response = jit::output::IssueShowResponse::from_issue(
                                    issue,
                                    enriched_deps,
                                    &gate_runs,
                                );
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
                                return Ok(());
                            }

                            if summary && json {
                                let summary_response =
                                    jit::output::IssueShowSummaryResponse::from(&issue);
                                let msg = format!(
                                    "Issue {}: {} [{:?}]",
                                    summary_response.short_id,
                                    summary_response.title,
                                    summary_response.state
                                );
                                let output = jit::output::JsonOutput::success(
                                    summary_response,
                                    "issue show",
                                )
                                .with_message(msg);
                                println!("{}", output.to_json_string()?);
                                return Ok(());
                            }
                            let enriched_deps = executor.get_dependencies_enriched(&issue);
                            let gate_runs =
                                executor.list_gate_runs(&issue.id, None).with_context(|| {
                                    format!("Failed to load gate runs for issue {}", issue.id)
                                })?;
                            let response = jit::output::IssueShowResponse::from_issue(
                                issue,
                                enriched_deps,
                                &gate_runs,
                            );

                            let show_msg = format!(
                                "Issue {}: {} [{:?}]",
                                &response.id[..8],
                                response.title,
                                response.state
                            );
                            output_data!(quiet, json, "issue show", response, show_msg, {
                                print_issue_show_human(&response);
                            });
                        }
                        Err(e) => {
                            handle_json_error!(
                                json,
                                e,
                                jit::output::JsonError::issue_not_found(&id, "issue show")
                            );
                        }
                    }
                }
                IssueCommands::Update {
                    id,
                    filter,
                    title,
                    description,
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
                    json,
                } => {
                    let output_ctx = OutputContext::new(quiet, json);

                    // Validate: exactly one of ID or filter must be provided
                    if id.is_none() && filter.is_none() {
                        return Err(anyhow!(
                            "Must specify either issue ID or --filter for batch mode"
                        ));
                    }
                    if id.is_some() && filter.is_some() {
                        return Err(anyhow!(
                            "Cannot specify both ID and --filter (mutually exclusive)"
                        ));
                    }
                    // --content-format is a per-issue field; batch mode does not
                    // support it (would set the same format on every match).
                    if filter.is_some() && content_format.is_some() {
                        return Err(anyhow!(
                            "--content-format is not supported with --filter (batch mode); set it per issue"
                        ));
                    }
                    // --type is a per-issue field; batch mode does not support it.
                    if filter.is_some() && issue_type.is_some() {
                        return Err(anyhow!(
                            "--type is not supported with --filter (batch mode); set it per issue"
                        ));
                    }

                    // Single issue mode
                    if let Some(id_str) = id {
                        // Resolve short hash to full UUID first
                        let full_id = storage.resolve_issue_id(&id_str)?;

                        let prio = priority.map(|p| Priority::from_str(&p)).transpose()?;
                        let st = state.map(|s| State::from_str(&s)).transpose()?;
                        // Tri-state for the per-issue content_format override:
                        //   flag absent            -> None              (leave unchanged)
                        //   "inherit"/"default"    -> Some(None)        (clear to repo default)
                        //   "markdown"/"html"/"xml" -> Some(Some(fmt))  (set the override)
                        let content_format: Option<Option<jit::domain::ContentFormat>> =
                            match content_format.as_deref() {
                                None => None,
                                Some("inherit") | Some("default") => Some(None),
                                Some(s) => Some(Some(jit::domain::ContentFormat::from_str(s)?)),
                            };

                        // Handle gate modifications first (before other updates)
                        if !add_gate.is_empty() {
                            let (_result, warnings) = executor.add_gates(&full_id, &add_gate)?;
                            for warning in warnings {
                                output_ctx.print_warning(&warning)?;
                            }
                        }

                        if !remove_gate.is_empty() {
                            let (_result, warnings) =
                                executor.remove_gates(&full_id, &remove_gate)?;
                            for warning in warnings {
                                output_ctx.print_warning(&warning)?;
                            }
                        }

                        // Handle assignee changes
                        if unassign {
                            let warnings = executor.unassign_issue(&full_id)?;
                            for warning in warnings {
                                output_ctx.print_warning(&warning)?;
                            }
                        } else if let Some(assignee_str) = assignee {
                            let warnings = executor.assign_issue(&full_id, assignee_str)?;
                            for warning in warnings {
                                output_ctx.print_warning(&warning)?;
                            }
                        }

                        match executor.update_issue(
                            &full_id,
                            title,
                            description,
                            prio,
                            st,
                            label,
                            remove_label,
                            content_format,
                            issue_type,
                            force,
                        ) {
                            Ok(warnings) => {
                                // Print warnings to stderr
                                for warning in &warnings {
                                    eprintln!("⚠️  Warning: {}", warning);
                                }

                                if json {
                                    let issue = storage.load_issue(&full_id)?;
                                    let msg = format!(
                                        "Updated issue {} to {:?}",
                                        issue.short_id(),
                                        issue.state
                                    );
                                    let response = jit::output::IssueUpdateResponse::from(&issue);
                                    let output = JsonOutput::success(response, "issue update")
                                        .with_message(msg);
                                    println!("{}", output.to_json_string()?);
                                } else {
                                    let _ = output_ctx
                                        .print_success(format!("Updated issue: {}", full_id));
                                }
                            }
                            Err(e) => {
                                // `error_msg` is display-payload only: the branch
                                // selection below is driven entirely by `downcast_ref`
                                // (typed TransitionBlockedError / IssueNotFoundError),
                                // and the string is used solely as the GENERIC_ERROR
                                // message body. No control flow branches on it.
                                let error_msg = e.to_string();
                                let json_error = if let Some(blocked) =
                                    e.downcast_ref::<jit::errors::TransitionBlockedError>()
                                {
                                    jit::output::JsonError::transition_blocked(
                                        blocked,
                                        "issue update",
                                    )
                                } else if e
                                    .downcast_ref::<jit::storage::IssueNotFoundError>()
                                    .is_some()
                                {
                                    jit::output::JsonError::issue_not_found(
                                        &full_id,
                                        "issue update",
                                    )
                                } else {
                                    // Generic error - use the JsonError::new directly
                                    jit::output::JsonError::new(
                                        "GENERIC_ERROR",
                                        &error_msg,
                                        "issue update",
                                    )
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
                        let result =
                            executor.apply_bulk_update(&query_filter, &operations, force)?;

                        if json {
                            let msg =
                                format!("Modified {} issue(s)", result.summary.total_modified);
                            let output =
                                JsonOutput::success(result, "bulk update").with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            // Human-readable output
                            if result.summary.total_modified > 0 {
                                let _ = output_ctx.print_success(format!(
                                    "✓ Modified {} issue(s)",
                                    result.summary.total_modified
                                ));
                            }

                            if !result.warnings.is_empty() {
                                println!("\n⚠ Warnings ({} issue(s)):", result.warnings.len());
                                for (id, warning) in &result.warnings {
                                    println!("  • {}: {}", &id[..8.min(id.len())], warning);
                                }
                            }

                            if !result.skipped.is_empty() {
                                println!("\nℹ Skipped {} issue(s):", result.summary.total_skipped);
                                for (id, reason) in &result.skipped {
                                    println!("  • {}: {}", &id[..8.min(id.len())], reason);
                                }
                            }

                            if !result.errors.is_empty() {
                                println!("\n✗ Failed {} issue(s):", result.summary.total_errors);
                                for (id, error) in &result.errors {
                                    println!("  • {}: {}", &id[..8.min(id.len())], error);
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
                IssueCommands::Delete { id, json } => {
                    // Phase 3 safety check: Block deletion in secondary worktrees
                    if storage.is_secondary_worktree() {
                        anyhow::bail!("Deletion is not allowed in secondary worktrees. Deletions must be performed from the main worktree to maintain consistency across all worktrees.");
                    }

                    // Phase 3 safety check: Require JIT_ALLOW_DELETION=1 to discourage deletion
                    if std::env::var("JIT_ALLOW_DELETION").unwrap_or_default() != "1" {
                        anyhow::bail!(
                            "Issue deletion is discouraged and requires explicit confirmation.\n\
                             Set JIT_ALLOW_DELETION=1 environment variable to proceed.\n\
                             Example: JIT_ALLOW_DELETION=1 jit issue delete {}\n\
                             \n\
                             Note: Deletion is a destructive operation. Consider closing issues instead of deleting them.",
                            id
                        );
                    }

                    let output_ctx = OutputContext::new(quiet, json);
                    let warnings = executor.delete_issue(&id)?;
                    for warning in warnings {
                        output_ctx.print_warning(&warning)?;
                    }

                    if json {
                        let short = if id.len() >= 8 { &id[..8] } else { &id };
                        let result = serde_json::json!({
                            "id": id,
                            "deleted": true
                        });
                        let msg = format!("Deleted issue {}", short);
                        let output = JsonOutput::success(result, "issue delete").with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!("Deleted issue: {}", id));
                    }
                }
                IssueCommands::Breakdown {
                    parent_id,
                    child_type,
                    subtask_titles,
                    subtask_descriptions,
                    gate_preset,
                    inherit_gates,
                    json,
                } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    // Pad descriptions with empty strings if not enough provided
                    let mut descs = subtask_descriptions.clone();
                    while descs.len() < subtask_titles.len() {
                        descs.push(String::new());
                    }

                    let subtasks: Vec<(String, String)> = subtask_titles
                        .iter()
                        .zip(descs.iter())
                        .map(|(t, d)| (t.clone(), d.clone()))
                        .collect();

                    let subtask_ids = if inherit_gates {
                        executor.breakdown_issue_with_inherit(
                            &parent_id,
                            &child_type,
                            subtasks,
                            true,
                        )?
                    } else {
                        executor.breakdown_issue(
                            &parent_id,
                            &child_type,
                            subtasks,
                            gate_preset.clone(),
                        )?
                    };

                    if json {
                        use jit::output::JsonOutput;
                        let response = serde_json::json!({
                            "parent_id": parent_id,
                            "child_type": child_type,
                            "subtask_ids": subtask_ids,
                            "count": subtask_ids.len(),
                            "gate_preset": gate_preset,
                            "inherit_gates": inherit_gates,
                            "message": format!("Broke down {} into {} subtasks of type {}", parent_id, subtask_ids.len(), child_type)
                        });
                        let output = JsonOutput::success(response, "issue breakdown");
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_info(format!(
                            "Broke down {} into {} subtasks of type {}:",
                            parent_id,
                            subtask_ids.len(),
                            child_type
                        ));
                        for (i, id) in subtask_ids.iter().enumerate() {
                            println!("  {}. {}", i + 1, id);
                        }
                    }
                }
                IssueCommands::Assign { id, assignee, json } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let full_id = storage.resolve_issue_id(&id)?;
                    let warnings = executor.assign_issue(&full_id, assignee)?;
                    for warning in warnings {
                        output_ctx.print_warning(&warning)?;
                    }

                    if json {
                        let issue = storage.load_issue(&full_id)?;
                        let msg = format!(
                            "Assigned issue {} to {}",
                            issue.short_id(),
                            issue
                                .assignee
                                .as_ref()
                                .map_or_else(|| "unknown".to_string(), |a| a.to_string())
                        );
                        let output = JsonOutput::success(issue, "issue assign").with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!("Assigned issue: {}", full_id));
                    }
                }
                IssueCommands::Claim {
                    id,
                    assignee,
                    assign_only,
                    json,
                } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let full_id = storage.resolve_issue_id(&id)?;

                    if assign_only {
                        // Assign without transitioning state: reuse the same
                        // path as `jit issue assign`.
                        let warnings = executor.assign_issue(&full_id, assignee)?;
                        for warning in warnings {
                            output_ctx.print_warning(&warning)?;
                        }

                        if json {
                            let issue = storage.load_issue(&full_id)?;
                            let msg = format!(
                                "Assigned issue {} to {} (assign-only)",
                                issue.short_id(),
                                issue
                                    .assignee
                                    .as_ref()
                                    .map_or_else(|| "unknown".to_string(), |a| a.to_string())
                            );
                            let output =
                                JsonOutput::success(issue, "issue claim").with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx
                                .print_success(format!("Assigned issue {} (assign-only)", full_id));
                        }
                        return Ok(());
                    }

                    let claim_warnings = match executor.claim_issue(&full_id, assignee) {
                        Ok(warnings) => warnings,
                        Err(e) => {
                            if json {
                                if let Some(blocked) =
                                    e.downcast_ref::<jit::errors::TransitionBlockedError>()
                                {
                                    let json_error = jit::output::JsonError::transition_blocked(
                                        blocked,
                                        "issue claim",
                                    );
                                    println!("{}", json_error.to_json_string()?);
                                    std::process::exit(json_error.exit_code().code());
                                }
                            }
                            return Err(e);
                        }
                    };

                    if json {
                        let issue = storage.load_issue(&full_id)?;
                        let msg = format!("Claimed issue {}", issue.short_id());
                        let mut value = serde_json::to_value(&issue)?;
                        if let serde_json::Value::Object(map) = &mut value {
                            map.insert(
                                "warnings".to_string(),
                                serde_json::to_value(&claim_warnings)?,
                            );
                        }
                        let output = JsonOutput::success(value, "issue claim").with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!("Claimed issue: {}", full_id));
                        for warning in &claim_warnings {
                            output_ctx.print_warning(warning)?;
                        }
                    }
                }
                IssueCommands::Unassign { id, json } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let full_id = storage.resolve_issue_id(&id)?;
                    let warnings = executor.unassign_issue(&full_id)?;
                    for warning in warnings {
                        output_ctx.print_warning(&warning)?;
                    }

                    if json {
                        let issue = storage.load_issue(&full_id)?;
                        let msg = format!("Unassigned issue {}", issue.short_id());
                        let output = JsonOutput::success(issue, "issue unassign").with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!("Unassigned issue: {}", full_id));
                    }
                }
                IssueCommands::Reject { id, reason, json } => {
                    use jit::domain::State;

                    let output_ctx = OutputContext::new(quiet, json);
                    let full_id = storage.resolve_issue_id(&id)?;

                    // Update state to rejected
                    let mut all_warnings =
                        executor.update_issue_state(&full_id, State::Rejected)?;

                    // Add resolution label if reason provided
                    if let Some(ref reason_value) = reason {
                        let label = format!("resolution:{}", reason_value);
                        let warnings = executor.add_label(&full_id, &label)?;
                        all_warnings.extend(warnings);
                    }

                    // Print all warnings
                    for warning in all_warnings {
                        output_ctx.print_warning(&warning)?;
                    }

                    if json {
                        let issue = storage.load_issue(&full_id)?;
                        let msg = format!("Rejected issue {}", issue.short_id());
                        let output = JsonOutput::success(issue, "issue reject").with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else if let Some(reason_value) = reason {
                        let _ = output_ctx.print_success(format!(
                            "Rejected issue: {} (reason: {})",
                            full_id, reason_value
                        ));
                    } else {
                        let _ = output_ctx.print_success(format!("Rejected issue: {}", full_id));
                    }
                }
                IssueCommands::Release { id, reason, json } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let full_id = storage.resolve_issue_id(&id)?;
                    executor.release_issue(&full_id, &reason)?;

                    if json {
                        let issue = storage.load_issue(&full_id)?;
                        let msg = format!("Released issue {}", issue.short_id());
                        let output = JsonOutput::success(issue, "issue release").with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!(
                            "Released issue: {} (reason: {})",
                            full_id, reason
                        ));
                    }
                }
                IssueCommands::ClaimNext {
                    assignee,
                    filter,
                    json,
                } => {
                    let output_ctx = OutputContext::new(quiet, json);
                    let (id, claim_warnings) = match executor.claim_next(assignee, filter) {
                        Ok(result) => result,
                        Err(e) => {
                            if json {
                                if let Some(blocked) =
                                    e.downcast_ref::<jit::errors::TransitionBlockedError>()
                                {
                                    let json_error = jit::output::JsonError::transition_blocked(
                                        blocked,
                                        "issue claim-next",
                                    );
                                    println!("{}", json_error.to_json_string()?);
                                    std::process::exit(json_error.exit_code().code());
                                }
                            }
                            return Err(e);
                        }
                    };

                    if json {
                        let issue = storage.load_issue(&id)?;
                        let msg = format!("Claimed issue {}", issue.short_id());
                        let mut value = serde_json::to_value(&issue)?;
                        if let serde_json::Value::Object(map) = &mut value {
                            map.insert(
                                "warnings".to_string(),
                                serde_json::to_value(&claim_warnings)?,
                            );
                        }
                        let output =
                            JsonOutput::success(value, "issue claim-next").with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        let _ = output_ctx.print_success(format!("Claimed issue: {}", id));
                        for warning in &claim_warnings {
                            output_ctx.print_warning(warning)?;
                        }
                    }
                }
                IssueCommands::List {
                    state,
                    assignee,
                    priority,
                    label,
                    full,
                    json,
                } => {
                    run_query_all(
                        &executor, quiet, state, assignee, priority, label, full, json,
                    )?;
                }
            }
        }
        Commands::Apply {
            template,
            container,
            anchor,
            force,
            json,
        } => {
            // Parse the repeatable `--anchor role=id` pairs. The `container`
            // anchor (the `plan` template's only anchor) is auto-bound to the
            // positional `<container>`; an explicit `--anchor container=…`
            // overrides it because it is applied after the default.
            let mut bindings: std::collections::BTreeMap<String, String> =
                std::collections::BTreeMap::new();
            bindings.insert("container".to_string(), container.clone());
            for pair in &anchor {
                let (role, id) = pair.split_once('=').ok_or_else(|| {
                    anyhow!("malformed --anchor '{pair}'; expected role=id (with an '=')")
                })?;
                if role.is_empty() {
                    return Err(anyhow!(
                        "malformed --anchor '{pair}'; the role (left of '=') must not be empty"
                    ));
                }
                bindings.insert(role.to_string(), id.to_string());
            }

            let (result, warnings) =
                executor.apply_template(&template, &container, &bindings, force)?;
            for warning in &warnings {
                eprintln!("⚠️  Warning: {}", warning);
            }
            print_apply_result(&storage, &result, &container, quiet, json)?;
        }
        Commands::Dep(dep_cmd) => match dep_cmd {
            DepCommands::Add {
                from_id,
                to_ids,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.add_dependencies(&from_id, &to_ids) {
                    Ok(mut result) => {
                        // If all dependencies failed, return error
                        if result.added.is_empty() && !result.errors.is_empty() {
                            // Classify the first failure by its TYPED error, not by
                            // scanning the message. `typed_errors` mirrors `errors`
                            // 1:1, so it is guaranteed non-empty here.
                            let (dep_id, typed) = result
                                .typed_errors
                                .drain(..)
                                .next()
                                .expect("typed_errors mirrors the non-empty errors list");
                            if json {
                                use jit::output::JsonError;
                                use jit::GraphError;
                                let json_error = if let Some(GraphError::CycleDetected) =
                                    typed.downcast_ref::<GraphError>()
                                {
                                    JsonError::cycle_detected(&from_id, &dep_id, "dep add")
                                } else if typed
                                    .downcast_ref::<jit::storage::IssueNotFoundError>()
                                    .is_some()
                                    || matches!(
                                        typed.downcast_ref::<GraphError>(),
                                        Some(GraphError::NodeNotFound { .. })
                                    )
                                {
                                    JsonError::issue_not_found(&dep_id, "dep add")
                                } else {
                                    JsonError::new("DEPENDENCY_ERROR", typed.to_string(), "dep add")
                                };
                                println!("{}", json_error.to_json_string()?);
                                std::process::exit(json_error.exit_code().code());
                            } else {
                                // Propagate the typed error so `error_to_exit_code`
                                // can classify it by downcast (preserving the
                                // cycle -> exit 4 / not-found -> exit 3 mapping).
                                return Err(typed);
                            }
                        }

                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "from_id": from_id,
                                "added": result.added,
                                "already_exist": result.already_exist,
                                "skipped": result.skipped,
                                "errors": result.errors,
                                "message": format!("Added {} dependencies to issue {}", result.added.len(), from_id)
                            });
                            let output = JsonOutput::success(response, "dep add");
                            println!("{}", output.to_json_string()?);
                        } else {
                            if !result.added.is_empty() {
                                let _ = output_ctx.print_success(format!(
                                    "Added {} dependenc{}:",
                                    result.added.len(),
                                    if result.added.len() == 1 { "y" } else { "ies" }
                                ));
                                for dep in &result.added {
                                    println!("  • {} → {}", from_id, dep);
                                }
                            }
                            if !result.already_exist.is_empty() {
                                println!("ℹ Already exist ({}):", result.already_exist.len());
                                for dep in &result.already_exist {
                                    println!("  • {}", dep);
                                }
                            }
                            if !result.skipped.is_empty() {
                                println!("ℹ Skipped ({}):", result.skipped.len());
                                for (dep, reason) in &result.skipped {
                                    println!("  • {}: {}", dep, reason);
                                }
                            }
                            if !result.errors.is_empty() {
                                println!("✗ Errors ({}):", result.errors.len());
                                for (dep, error) in &result.errors {
                                    println!("  • {}: {}", dep, error);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let error_str = e.to_string();
                            let json_error =
                                JsonError::new("DEPENDENCY_ERROR", error_str, "dep add");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            DepCommands::Rm {
                from_id,
                to_ids,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.remove_dependencies(&from_id, &to_ids) {
                    Ok(result) => {
                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "from_id": from_id,
                                "removed": result.removed,
                                "not_found": result.not_found,
                                "message": format!("Removed {} dependencies from issue {}", result.removed.len(), from_id)
                            });
                            let output = JsonOutput::success(response, "dep rm");
                            println!("{}", output.to_json_string()?);
                        } else {
                            if !result.removed.is_empty() {
                                let _ = output_ctx.print_success(format!(
                                    "Removed {} dependenc{}:",
                                    result.removed.len(),
                                    if result.removed.len() == 1 {
                                        "y"
                                    } else {
                                        "ies"
                                    }
                                ));
                                for dep in &result.removed {
                                    println!("  • {}", dep);
                                }
                            }
                            if !result.not_found.is_empty() {
                                println!("ℹ Not found ({}):", result.not_found.len());
                                for dep in &result.not_found {
                                    println!("  • {}", dep);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let error_str = e.to_string();
                            let json_error =
                                JsonError::new("DEPENDENCY_ERROR", error_str, "dep rm");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
        },
        Commands::Gate(gate_cmd) => match gate_cmd {
            GateCommands::Define {
                key,
                title,
                description,
                stage,
                mode,
                auto,
                checker_command,
                timeout,
                working_dir,
                pass_context,
                prompt,
                prompt_file,
                env,
                priority,
                json,
            } => {
                use jit::domain::GateChecker;

                // `--auto` is a convenience spelling of `--mode auto`; it wins
                // over `--mode` when both are supplied.
                let mode = if auto {
                    jit::domain::GateMode::Auto
                } else {
                    mode
                };

                let output_ctx = OutputContext::new(quiet, json);

                // Parse --env KEY=VALUE pairs into a HashMap
                let env_map: std::collections::HashMap<String, String> = env
                    .into_iter()
                    .map(|pair| {
                        pair.split_once('=')
                            .map(|(k, v)| (k.to_string(), v.to_string()))
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "Invalid --env format '{}': expected KEY=VALUE",
                                    pair
                                )
                            })
                    })
                    .collect::<Result<_, _>>()
                    .unwrap_or_else(|e| {
                        eprintln!("Error: {}", e);
                        std::process::exit(2);
                    });

                // Build checker if command provided
                let checker = checker_command.map(|cmd| GateChecker::Exec {
                    command: cmd,
                    timeout_seconds: timeout,
                    working_dir: working_dir.clone(),
                    env: env_map,
                    pass_context,
                    prompt,
                    prompt_file,
                });

                match executor.define_gate(
                    key.clone(),
                    title.clone(),
                    description.clone(),
                    stage,
                    mode,
                    checker,
                    priority,
                ) {
                    Ok(_) => {
                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "gate_key": key,
                                "message": format!("Defined gate '{}'", key)
                            });
                            let output = JsonOutput::success(response, "gate define");
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx.print_success(format!("Defined gate '{}'", key));
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error = JsonError::new("GATE_ERROR", e.to_string(), "gate");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            GateCommands::List { json } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.list_gates() {
                    Ok(gates) => {
                        if json {
                            use jit::output::{GateDefinition, GateListResponse, JsonOutput};
                            let gate_defs: Vec<GateDefinition> =
                                gates.into_iter().map(GateDefinition::from).collect();
                            let count = gate_defs.len();
                            let response = GateListResponse {
                                count,
                                gates: gate_defs,
                            };
                            let msg = format!("{} gate definition(s)", count);
                            let output =
                                JsonOutput::success(response, "gate list").with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else if gates.is_empty() {
                            let _ = output_ctx.print_info("No gates defined");
                        } else {
                            let _ = output_ctx.print_info("Gates:");
                            for gate in gates {
                                println!(
                                    "  {} - {} ({:?}, {:?})",
                                    gate.key, gate.title, gate.stage, gate.mode
                                );
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error = JsonError::new("GATE_ERROR", e.to_string(), "gate");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            GateCommands::Show { key, json } => match executor.show_gate_definition(&key) {
                Ok(gate) => {
                    if json {
                        use jit::output::JsonOutput;
                        let msg = format!("Gate {}: {}", gate.key, gate.title);
                        let output = JsonOutput::success(gate, "gate show").with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        println!("Gate: {}", gate.key);
                        println!("  Title: {}", gate.title);
                        println!("  Description: {}", gate.description);
                        println!("  Stage: {:?}", gate.stage);
                        println!("  Mode: {:?}", gate.mode);
                        if let Some(checker) = gate.checker {
                            match checker {
                                jit::domain::GateChecker::Exec {
                                    command,
                                    timeout_seconds,
                                    working_dir,
                                    ..
                                } => {
                                    println!("  Checker:");
                                    println!("    Command: {}", command);
                                    println!("    Timeout: {}s", timeout_seconds);
                                    if let Some(wd) = working_dir {
                                        println!("    Working dir: {}", wd);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    if json {
                        use jit::output::JsonError;
                        let json_error = JsonError::gate_not_found(&key, "gate show");
                        println!("{}", json_error.to_json_string()?);
                        std::process::exit(json_error.exit_code().code());
                    } else {
                        return Err(e);
                    }
                }
            },
            GateCommands::Remove { key, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.remove_gate_definition(&key) {
                    Ok(_) => {
                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "gate_key": key,
                                "message": format!("Removed gate '{}'", key)
                            });
                            let output = JsonOutput::success(response, "gate remove");
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx.print_success(format!("Removed gate '{}'", key));
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error = JsonError::gate_not_found(&key, "gate show");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            GateCommands::Check {
                id,
                gate_key,
                gate_flag,
                json,
            } => {
                let gate_key = resolve_gate_key_for(gate_key, gate_flag, "gate check", json)?;
                let output_ctx = OutputContext::new(quiet, json);

                // Transposed-argument guard. The canonical form is
                // `jit gate check <issue> <gate-key>`. If <id> is not an issue but
                // <gate_key> resolves to one and <id> is a registered gate key, the
                // two positionals are almost certainly swapped, so emit an
                // actionable did-you-mean rather than misparsing into
                // "issue not found".
                if executor.storage().resolve_issue_id(&id).is_err() {
                    let gate_key_is_issue = executor.storage().resolve_issue_id(&gate_key).is_ok();
                    let id_is_gate = executor
                        .list_gates()
                        .map(|gates| gates.iter().any(|g| g.key == id))
                        .unwrap_or(false);
                    if gate_key_is_issue && id_is_gate {
                        let canonical = format!("jit gate check {} {}", gate_key, id);
                        let message = format!(
                            "'{id}' is a gate key and '{gate_key}' is an issue; the issue id and gate key look transposed."
                        );
                        if json {
                            use jit::output::JsonError;
                            let json_error =
                                JsonError::new("INVALID_ARGUMENT", message, "gate check")
                                    .with_details(serde_json::json!({
                                        "issue_id": gate_key,
                                        "gate_key": id,
                                        "transposed": true,
                                    }))
                                    .with_suggestion(format!("Did you mean: {canonical}"));
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            eprintln!("Error: {message}");
                            eprintln!("  Did you mean: {canonical}");
                            std::process::exit(2);
                        }
                    }
                }

                match executor.get_last_gate_run(&id, &gate_key) {
                    Ok(Some(result)) => {
                        if json {
                            use jit::output::JsonOutput;
                            let msg = format!("Gate '{}': {:?}", gate_key, result.status);
                            let output =
                                JsonOutput::success(result, "gate check").with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            print_gate_run_details(&result);
                            let _ = output_ctx;
                        }
                    }
                    Ok(None) => {
                        let msg = format!(
                            "Gate '{}' has not been run yet for issue {}. Use 'jit gate pass' to run it.",
                            gate_key, id
                        );
                        if json {
                            use jit::output::JsonOutput;
                            let output = JsonOutput::<Option<()>>::success(None, "gate check")
                                .with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("{}", msg);
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error =
                                JsonError::new("GATE_CHECK_ERROR", e.to_string(), "gate check");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            GateCommands::CheckAll { id, full, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                let (results, not_run) = executor.get_last_gate_runs_for_issue(&id)?;

                if json {
                    use jit::output::{GateCheckAllResponse, GateRunSummary, JsonOutput};
                    let passed_count = results
                        .iter()
                        .filter(|r| r.status == jit::domain::GateRunStatus::Passed)
                        .count();
                    let total = results.len() + not_run.len();
                    let msg = if not_run.is_empty() {
                        format!("{}/{} recorded gate runs passed", passed_count, total)
                    } else {
                        format!(
                            "Showing last run results for {}/{} automated gates ({} not run yet)",
                            results.len(),
                            total,
                            not_run.len()
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
                    let response = GateCheckAllResponse {
                        results: summaries,
                        passed: passed_count,
                        total,
                        not_run,
                    };
                    let output = JsonOutput::success(response, "gate check-all").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else if results.is_empty() && not_run.is_empty() {
                    let _ = output_ctx
                        .print_info(format!("No automated gates to inspect for issue {}", id));
                } else {
                    let _ = output_ctx.print_info(format!("Gate run results for issue {}:", id));
                    for result in &results {
                        print_gate_run_details(result);
                    }
                    for gate_key in not_run {
                        println!(
                            "Gate '{}' has not been run yet for issue {}. Use 'jit gate pass' to run it.",
                            gate_key, id
                        );
                    }
                }
            }
            GateCommands::Add {
                id,
                gate_keys,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.add_gates(&id, &gate_keys) {
                    Ok((result, warnings)) => {
                        // Print warnings first
                        for warning in warnings {
                            output_ctx.print_warning(&warning)?;
                        }

                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "issue_id": id,
                                "added": result.added,
                                "already_exist": result.already_exist,
                                "message": format!("Added {} gate(s) to issue {}", result.added.len(), id)
                            });
                            let output = JsonOutput::success(response, "gate add");
                            println!("{}", output.to_json_string()?);
                        } else {
                            if !result.added.is_empty() {
                                let _ = output_ctx.print_success(format!(
                                    "Added {} gate(s) to issue {}:",
                                    result.added.len(),
                                    id
                                ));
                                for gate in &result.added {
                                    println!("  • {}", gate);
                                }
                            }
                            if !result.already_exist.is_empty() {
                                println!(
                                    "ℹ Already required ({} gate(s)):",
                                    result.already_exist.len()
                                );
                                for gate in &result.already_exist {
                                    println!("  • {}", gate);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let error_str = e.to_string();
                            let json_error = if e
                                .downcast_ref::<jit::storage::IssueNotFoundError>()
                                .is_some()
                            {
                                JsonError::issue_not_found(&id, "gate add")
                            } else if e
                                .downcast_ref::<jit::storage::GateNotFoundError>()
                                .is_some()
                            {
                                JsonError::new("GATE_NOT_FOUND", error_str, "gate add")
                            } else {
                                JsonError::new("GATE_ERROR", error_str, "gate add")
                            };
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            GateCommands::Pass {
                id,
                gate_key,
                gate_flag,
                by,
                force,
                json,
            } => {
                let gate_key = resolve_gate_key_for(gate_key, gate_flag, "gate pass", json)?;
                let output_ctx = OutputContext::new(quiet, json);
                match executor.pass_gate(&id, gate_key.clone(), by, force) {
                    Ok(outcome) => {
                        // Print warnings first
                        for warning in &outcome.warnings {
                            output_ctx.print_warning(warning)?;
                        }

                        let already_passed = outcome.already_passed;
                        if json {
                            use jit::output::JsonOutput;
                            let message = if already_passed {
                                format!(
                                    "Gate '{}' already passed at HEAD for issue {}; skipped",
                                    gate_key, id
                                )
                            } else {
                                format!("Passed gate '{}' for issue {}", gate_key, id)
                            };
                            let response = serde_json::json!({
                                "issue_id": id,
                                "gate_key": gate_key,
                                "status": "passed",
                                "verdict": "pass",
                                "already_passed": already_passed,
                                "message": message,
                            });
                            let output = JsonOutput::success(response, "gate pass");
                            println!("{}", output.to_json_string()?);
                        } else if already_passed {
                            let _ = output_ctx.print_success(format!(
                                "Gate '{}' already passed at HEAD for issue {}, skipping (use --force to re-run)",
                                gate_key, id
                            ));
                        } else {
                            let _ = output_ctx.print_success(format!(
                                "Passed gate '{}' for issue {}",
                                gate_key, id
                            ));
                        }
                    }
                    Err(e) => {
                        render_gate_pass_error(e, &id, &output_ctx, json, "gate pass")?;
                    }
                }
            }
            GateCommands::PassAll {
                id,
                by,
                force,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.pass_all_gates(&id, by, force) {
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
                                        "gate_key": entry.gate_key,
                                        "status": "passed",
                                        "verdict": "pass",
                                        "already_passed": entry.already_passed,
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
                            let output = JsonOutput::success(response, "gate pass-all");
                            println!("{}", output.to_json_string()?);
                        } else if outcome.results.is_empty() {
                            let _ = output_ctx
                                .print_success(format!("No required gates for issue {}", id));
                        } else {
                            for entry in &outcome.results {
                                let suffix = if entry.already_passed {
                                    " (already passed at HEAD)"
                                } else {
                                    ""
                                };
                                let _ = output_ctx.print_success(format!(
                                    "Passed gate '{}' for issue {}{}",
                                    entry.gate_key, id, suffix
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        render_gate_pass_error(e, &id, &output_ctx, json, "gate pass-all")?;
                    }
                }
            }
            GateCommands::Fail {
                id,
                gate_key,
                by,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                match executor.fail_gate(&id, gate_key.clone(), by) {
                    Ok(warnings) => {
                        // Print warnings first
                        for warning in warnings {
                            output_ctx.print_warning(&warning)?;
                        }

                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "issue_id": id,
                                "gate_key": gate_key,
                                "status": "failed",
                                "message": format!("Failed gate '{}' for issue {}", gate_key, id)
                            });
                            let output = JsonOutput::success(response, "gate fail");
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
                            use jit::output::JsonError;
                            let json_error =
                                JsonError::new("GATE_ERROR", e.to_string(), "gate fail");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            GateCommands::Preset(preset_cmd) => match preset_cmd {
                jit::cli::PresetCommands::List { json } => {
                    use jit::output::JsonOutput;
                    match executor.list_gate_presets() {
                        Ok(presets) => {
                            if json {
                                let msg = format!("{} preset(s)", presets.len());
                                let output = JsonOutput::success(
                                    serde_json::json!({ "presets": presets }),
                                    "gate preset list",
                                )
                                .with_message(msg);
                                println!("{}", output.to_json_string()?);
                            } else if presets.is_empty() {
                                println!("No gate presets available");
                            } else {
                                for preset in presets {
                                    let source = if preset.builtin {
                                        "[builtin]"
                                    } else {
                                        "[custom]"
                                    };
                                    let gate_word = if preset.gate_count == 1 {
                                        "gate"
                                    } else {
                                        "gates"
                                    };
                                    println!(
                                        "{} {} - {} ({} {})",
                                        source,
                                        preset.name,
                                        preset.description,
                                        preset.gate_count,
                                        gate_word
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            if json {
                                use jit::output::JsonError;
                                let json_error = JsonError::new(
                                    "PRESET_ERROR",
                                    e.to_string(),
                                    "gate preset list",
                                );
                                println!("{}", json_error.to_json_string()?);
                                std::process::exit(json_error.exit_code().code());
                            } else {
                                return Err(e);
                            }
                        }
                    }
                }
                jit::cli::PresetCommands::Show { name, json } => {
                    use jit::output::JsonOutput;
                    match executor.show_gate_preset(&name) {
                        Ok(preset) => {
                            if json {
                                let msg = format!("Preset {}: {}", preset.name, preset.description);
                                let output = JsonOutput::success(preset, "gate preset show")
                                    .with_message(msg);
                                println!("{}", output.to_json_string()?);
                            } else {
                                println!("Preset: {}", preset.name);
                                println!("Description: {}", preset.description);
                                println!("\nGates:");
                                for gate in &preset.gates {
                                    println!(
                                        "  {} - {} ({}:{})",
                                        gate.key,
                                        gate.title,
                                        gate.stage.as_str(),
                                        gate.mode.as_str()
                                    );
                                    if let Some(checker) = &gate.checker {
                                        match checker {
                                            jit::domain::GateChecker::Exec {
                                                command,
                                                timeout_seconds,
                                                ..
                                            } => {
                                                println!("    Command: {}", command);
                                                println!("    Timeout: {}s", timeout_seconds);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            if json {
                                use jit::output::JsonError;
                                let json_error = JsonError::new(
                                    "PRESET_ERROR",
                                    e.to_string(),
                                    "gate preset show",
                                );
                                println!("{}", json_error.to_json_string()?);
                                std::process::exit(json_error.exit_code().code());
                            } else {
                                return Err(e);
                            }
                        }
                    }
                }
                jit::cli::PresetCommands::Apply {
                    name,
                    ids,
                    timeout,
                    no_precheck,
                    no_postcheck,
                    except,
                    json,
                } => {
                    use jit::output::JsonOutput;

                    let mut results = Vec::new();
                    let mut errors = Vec::new();

                    for id in &ids {
                        match executor.apply_gate_preset(
                            id,
                            &name,
                            timeout,
                            no_precheck,
                            no_postcheck,
                            &except,
                        ) {
                            Ok((result, warnings)) => {
                                // Store warnings with result
                                results.push((id.clone(), result, warnings));
                            }
                            Err(e) => {
                                errors.push((id.clone(), e.to_string()));
                            }
                        }
                    }

                    if json {
                        let msg =
                            format!("Applied preset '{}' to {} issue(s)", name, results.len());
                        let output = JsonOutput::success(
                            serde_json::json!({
                                "preset": name,
                                "success": results.iter().map(|(id, r, _)| {
                                    serde_json::json!({
                                        "issue_id": id,
                                        "gates_added": r.added,
                                        "already_existed": r.already_exist
                                    })
                                }).collect::<Vec<_>>(),
                                "errors": errors.iter().map(|(id, e)| {
                                    serde_json::json!({
                                        "issue_id": id,
                                        "error": e
                                    })
                                }).collect::<Vec<_>>()
                            }),
                            "gate preset apply",
                        )
                        .with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        if !results.is_empty() {
                            println!("Applied preset '{}' to {} issue(s):", name, results.len());
                            for (id, result, warnings) in &results {
                                // Print warnings for each issue
                                for warning in warnings {
                                    eprintln!("⚠️  Warning ({}): {}", id, warning);
                                }
                                println!("  {} - gates added: {}", id, result.added.join(", "));
                            }
                        }
                        if !errors.is_empty() {
                            eprintln!("\nErrors ({}):", errors.len());
                            for (id, error) in &errors {
                                eprintln!("  {} - {}", id, error);
                            }
                            std::process::exit(1);
                        }
                    }
                }
                jit::cli::PresetCommands::Create {
                    from_issue,
                    name,
                    json,
                } => {
                    use jit::output::JsonOutput;
                    match executor.create_gate_preset(&name, &from_issue) {
                        Ok(path) => {
                            if json {
                                let msg = format!("Created preset '{}'", name);
                                let output = JsonOutput::success(
                                    serde_json::json!({ "name": name, "path": path.display().to_string() }),
                                    "gate preset create",
                                )
                                .with_message(msg);
                                println!("{}", output.to_json_string()?);
                            } else {
                                println!("Created preset '{}' at {}", name, path.display());
                            }
                        }
                        Err(e) => {
                            if json {
                                use jit::output::JsonError;
                                let json_error = JsonError::new(
                                    "PRESET_ERROR",
                                    e.to_string(),
                                    "gate preset create",
                                );
                                println!("{}", json_error.to_json_string()?);
                                std::process::exit(json_error.exit_code().code());
                            } else {
                                return Err(e);
                            }
                        }
                    }
                }
            },
        },
        Commands::Graph(graph_cmd) => match graph_cmd {
            GraphCommands::Deps { id, depth, json } => {
                let output_ctx = OutputContext::new(quiet, json);

                if json {
                    // For JSON output, use tree structure
                    use jit::output::{GraphDepsTreeResponse, JsonOutput};

                    let tree = executor.build_dependency_tree(&id, depth)?;
                    let summary = jit::commands::graph::compute_dependency_summary(&tree);

                    let response = GraphDepsTreeResponse {
                        issue_id: id.clone(),
                        depth,
                        tree,
                        summary,
                    };
                    let msg = format!("{} dependencies", response.summary.total);
                    let output = JsonOutput::success(response, "graph deps").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    // For human output, use tree structure
                    let tree = executor.build_dependency_tree(&id, depth)?;
                    let depth_str = match depth {
                        0 => "all transitive".to_string(),
                        1 => "immediate".to_string(),
                        n => format!("depth {}", n),
                    };

                    let _ =
                        output_ctx.print_info(format!("Dependencies of {} ({}):", id, depth_str));

                    if tree.is_empty() {
                        println!("  (none)");
                    } else {
                        // Print summary first
                        let summary = jit::commands::graph::compute_dependency_summary(&tree);
                        if summary.total > 0 {
                            let done_count = summary.by_state.get(&State::Done).unwrap_or(&0);
                            println!("  Summary: {}/{} complete", done_count, summary.total);
                            println!();
                        }

                        // Print tree with indentation
                        print_dependency_tree(&tree, "", true);
                    }
                }
            }
            GraphCommands::Rdeps { id, depth, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                let issues = executor.show_rdeps_with_depth(&id, depth)?;
                if json {
                    use jit::domain::MinimalIssue;
                    use jit::output::{GraphDownstreamResponse, JsonOutput};

                    let minimal_issues: Vec<MinimalIssue> =
                        issues.iter().map(MinimalIssue::from).collect();
                    let response = GraphDownstreamResponse {
                        issue_id: id.clone(),
                        dependents: minimal_issues,
                        count: issues.len(),
                    };
                    let msg = format!("{} dependents", issues.len());
                    let output = JsonOutput::success(response, "graph rdeps").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ = output_ctx.print_info(format!("Reverse dependencies of {}:", id));
                    for issue in issues {
                        println!("  {} | {}", issue.id, issue.title);
                    }
                }
            }
            GraphCommands::Roots { json } => {
                let output_ctx = OutputContext::new(quiet, json);
                let issues = executor.show_roots()?;
                if json {
                    use jit::domain::MinimalIssue;
                    use jit::output::{GraphRootsResponse, JsonOutput};

                    let minimal_issues: Vec<MinimalIssue> =
                        issues.iter().map(MinimalIssue::from).collect();
                    let response = GraphRootsResponse {
                        roots: minimal_issues,
                        count: issues.len(),
                    };
                    let msg = format!("{} root issues", issues.len());
                    let output = JsonOutput::success(response, "graph roots").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ = output_ctx.print_info("Root issues (no dependencies):");
                    for issue in issues {
                        println!("  {} | {}", issue.id, issue.title);
                    }
                }
            }
            GraphCommands::Export { format, output } => {
                let output_ctx = OutputContext::new(quiet, false);
                let graph_output = executor.export_graph(format)?;

                if let Some(path) = output {
                    std::fs::write(&path, graph_output)?;
                    let _ = output_ctx.print_success(format!("Graph exported to: {}", path));
                } else {
                    println!("{}", graph_output);
                }
            }
        },
        Commands::Registry(registry_cmd) => match registry_cmd {
            RegistryCommands::List { json } => {
                let gates = executor.list_gates()?;
                if json {
                    use jit::output::{GateDefinition, JsonOutput, RegistryListResponse};

                    let gate_defs: Vec<GateDefinition> =
                        gates.into_iter().map(GateDefinition::from).collect();

                    let response = RegistryListResponse {
                        count: gate_defs.len(),
                        gates: gate_defs,
                    };
                    let msg = format!("{} gate definition(s)", response.count);
                    let output = JsonOutput::success(response, "registry list").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    for gate in gates {
                        println!("{} | {} | auto:{}", gate.key, gate.title, gate.auto);
                    }
                }
            }
            RegistryCommands::Add {
                key,
                title,
                description,
                auto,
                example,
                stage,
            } => {
                let output_ctx = OutputContext::new(quiet, false);
                executor.add_gate_definition(
                    key.clone(),
                    title,
                    description,
                    auto,
                    example,
                    stage,
                )?;
                let _ = output_ctx.print_success(format!("Added gate definition: {}", key));
            }
            RegistryCommands::Remove { key } => {
                let output_ctx = OutputContext::new(quiet, false);
                executor.remove_gate_definition(&key)?;
                let _ = output_ctx.print_success(format!("Removed gate definition: {}", key));
            }
            RegistryCommands::Show { key, json } => {
                let gate = executor.show_gate_definition(&key)?;
                if json {
                    use jit::output::{GateDefinition, JsonOutput};

                    let msg = format!("Gate {}: {}", gate.key, gate.title);
                    let gate_def = GateDefinition::from(gate);
                    let output = JsonOutput::success(gate_def, "registry show").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Key: {}", gate.key);
                    println!("Title: {}", gate.title);
                    println!("Description: {}", gate.description);
                    println!("Auto: {}", gate.auto);
                    println!("Example Integration: {:?}", gate.example_integration);
                    println!("Stage: {:?}", gate.stage);
                    println!("Mode: {:?}", gate.mode);
                }
            }
        },
        Commands::Events(event_cmd) => match event_cmd {
            EventCommands::Tail { n, json } => {
                let events = executor.tail_events(n)?;
                if json {
                    use jit::output::JsonOutput;
                    let output = JsonOutput::success(
                        serde_json::json!({
                            "count": events.len(),
                            "events": events,
                        }),
                        "events tail",
                    )
                    .with_message(format!("{} event(s)", events.len()));
                    println!("{}", output.to_json_string()?);
                } else {
                    for event in events {
                        println!("{}", serde_json::to_string(&event)?);
                    }
                }
            }
            EventCommands::Query {
                event_type,
                issue_id,
                limit,
                json,
            } => {
                let events = executor.query_events(event_type, issue_id, limit)?;
                if json {
                    use jit::output::JsonOutput;
                    let output = JsonOutput::success(
                        serde_json::json!({
                            "count": events.len(),
                            "events": events,
                        }),
                        "events query",
                    )
                    .with_message(format!("{} event(s)", events.len()));
                    println!("{}", output.to_json_string()?);
                } else {
                    for event in events {
                        println!("{}", serde_json::to_string(&event)?);
                    }
                }
            }
        },
        Commands::Doc(doc_cmd) => match doc_cmd {
            DocCommands::Add {
                id,
                path,
                commit,
                label,
                doc_type,
                skip_scan,
                json,
            } => {
                let output_ctx = OutputContext::new(quiet, json);
                let (result, warnings) = executor.add_document_reference(
                    &id,
                    &path,
                    commit.as_deref(),
                    label.as_deref(),
                    doc_type.as_deref(),
                    skip_scan,
                )?;
                for warning in warnings {
                    output_ctx.print_warning(&warning)?;
                }

                if json {
                    use jit::output::JsonOutput;
                    let msg = format!("Added document reference to issue {}", result.issue_id);
                    let output = JsonOutput::success(&result, "doc add").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Added document reference to issue {}", result.issue_id);
                    println!("  Path: {}", result.document.path);
                    if let Some(ref c) = result.document.commit {
                        println!("  Commit: {}", c);
                    }
                    if let Some(ref l) = result.document.label {
                        println!("  Label: {}", l);
                    }
                    if let Some(ref t) = result.document.doc_type {
                        println!("  Type: {}", t);
                    }
                    if let Some(ref f) = result.document.format {
                        println!("  Format: {}", f);
                    }
                    if !result.document.assets.is_empty() {
                        println!("  Assets: {} discovered", result.document.assets.len());
                    }
                }
            }
            DocCommands::List { id, json } => {
                use jit::output::JsonOutput;

                let output_ctx = OutputContext::new(quiet, json);
                let result = executor.list_document_references(&id)?;

                if json {
                    let msg = format!("{} document(s) attached", result.count);
                    let output = JsonOutput::success(&result, "doc list").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else if result.documents.is_empty() {
                    output_ctx.print_data(format!(
                        "No document references for issue {}",
                        result.issue_id
                    ))?;
                } else {
                    output_ctx.print_data(format!(
                        "Document references for issue {}:",
                        result.issue_id
                    ))?;
                    for doc in &result.documents {
                        let mut line = format!("  - {}", doc.path);
                        if let Some(ref label) = doc.label {
                            line.push_str(&format!(" ({})", label));
                        }
                        if let Some(ref commit) = doc.commit {
                            line.push_str(&format!(" [{}]", &commit[..7.min(commit.len())]));
                        } else {
                            line.push_str(" [HEAD]");
                        }
                        if let Some(ref doc_type) = doc.doc_type {
                            line.push_str(&format!(" <{}>", doc_type));
                        }
                        output_ctx.print_data(line)?;
                    }
                    output_ctx.print_data(format!("\nTotal: {}", result.count))?;
                }
            }
            DocCommands::Remove { id, path, json } => {
                let result = executor.remove_document_reference(&id, &path)?;

                if json {
                    use jit::output::JsonOutput;
                    let msg = format!(
                        "Removed document reference {} from issue {}",
                        result.path, result.issue_id
                    );
                    let output = JsonOutput::success(&result, "doc remove").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    println!(
                        "Removed document reference {} from issue {}",
                        result.path, result.issue_id
                    );
                }
            }
            DocCommands::Show { id, path, at, json } => {
                let result = executor.show_document_content(&id, &path, at.as_deref())?;

                if json {
                    use jit::output::JsonOutput;
                    let msg = "Document content retrieved".to_string();
                    let output = JsonOutput::success(&result, "doc show").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Document: {}", result.path);
                    if let Some(ref label) = result.label {
                        println!("Label: {}", label);
                    }
                    println!("Commit: {}", result.commit);
                    if let Some(ref doc_type) = result.doc_type {
                        println!("Type: {}", doc_type);
                    }
                    println!("\n---\n");
                    println!("{}", result.content);
                }
            }
            DocCommands::History { id, path, json } => {
                use jit::output::JsonOutput;

                let output_ctx = OutputContext::new(quiet, json);
                let result = executor.document_history(&id, &path)?;

                if json {
                    let msg = format!("{} commits in history", result.commits.len());
                    let output = JsonOutput::success(&result, "doc history").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    output_ctx.print_data(format!("History for {}:\n", result.path))?;
                    for commit in &result.commits {
                        output_ctx.print_data(format!("commit {}", commit.sha))?;
                        output_ctx.print_data(format!("Author: {}", commit.author))?;
                        output_ctx.print_data(format!("Date:   {}", commit.date))?;
                        output_ctx.print_data("")?;
                        output_ctx.print_data(format!("    {}", commit.message))?;
                        output_ctx.print_data("")?;
                    }
                }
            }
            DocCommands::Diff {
                id,
                path,
                from,
                to,
                json,
            } => {
                let result = executor.document_diff(&id, &path, &from, to.as_deref())?;

                if json {
                    use jit::output::JsonOutput;
                    let msg = "Document diff retrieved".to_string();
                    let output = JsonOutput::success(&result, "doc diff").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    print!("{}", result.diff);
                }
            }
            DocCommands::Assets { command } => match command {
                jit::cli::AssetCommands::List {
                    id,
                    path,
                    rescan,
                    json,
                } => {
                    use jit::document::AssetType;
                    use jit::output::JsonOutput;

                    let output_ctx = OutputContext::new(quiet, json);
                    let result = executor.list_document_assets(&id, &path, rescan)?;

                    // Print warnings first if any
                    for warning in &result.warnings {
                        output_ctx.print_warning(warning)?;
                    }

                    if json {
                        let msg = format!("{} assets found", result.summary.total);
                        let output = JsonOutput::success(&result, "doc assets").with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        // Get repository root to check if assets exist
                        let repo_root = executor.storage().root().parent().ok_or_else(|| {
                            jit::errors::InvalidArgumentError::new("Invalid storage path")
                        })?;

                        output_ctx.print_data(format!(
                            "Assets for document {} (issue {}):",
                            result.document_path,
                            &result.issue_id[..8.min(result.issue_id.len())]
                        ))?;

                        if result.assets.is_empty() {
                            output_ctx.print_data("  No assets found for this document")?;
                        } else {
                            // Categorize and check existence
                            let per_doc: Vec<_> = result
                                .assets
                                .iter()
                                .filter(|a| !a.is_shared && a.asset_type == AssetType::Local)
                                .map(|a| {
                                    let exists = a
                                        .resolved_path
                                        .as_ref()
                                        .map(|p| repo_root.join(p).exists())
                                        .unwrap_or(false);
                                    (a, exists)
                                })
                                .collect();
                            let shared: Vec<_> = result
                                .assets
                                .iter()
                                .filter(|a| a.is_shared && a.asset_type == AssetType::Local)
                                .map(|a| {
                                    let exists = a
                                        .resolved_path
                                        .as_ref()
                                        .map(|p| repo_root.join(p).exists())
                                        .unwrap_or(false);
                                    (a, exists)
                                })
                                .collect();
                            let external: Vec<_> = result
                                .assets
                                .iter()
                                .filter(|a| a.asset_type == AssetType::External)
                                .collect();
                            let missing: Vec<_> = result
                                .assets
                                .iter()
                                .filter(|a| a.asset_type == AssetType::Missing)
                                .collect();

                            if !per_doc.is_empty() {
                                output_ctx.print_data("\nPer-document assets:")?;
                                for (asset, exists) in &per_doc {
                                    let status = if *exists { "✓" } else { "✗" };
                                    output_ctx.print_data(format!(
                                        "  {} {}",
                                        status, asset.original_path
                                    ))?;
                                    if let Some(ref resolved) = asset.resolved_path {
                                        output_ctx
                                            .print_data(format!("     → {}", resolved.display()))?;
                                    }
                                    if let Some(ref mime) = asset.mime_type {
                                        output_ctx.print_data(format!("     MIME: {}", mime))?;
                                    }
                                }
                            }

                            if !shared.is_empty() {
                                output_ctx.print_data("\nShared assets:")?;
                                for (asset, exists) in &shared {
                                    let status = if *exists { "✓" } else { "✗" };
                                    output_ctx.print_data(format!(
                                        "  {} {}",
                                        status, asset.original_path
                                    ))?;
                                    if let Some(ref resolved) = asset.resolved_path {
                                        output_ctx
                                            .print_data(format!("     → {}", resolved.display()))?;
                                    }
                                }
                            }

                            if !external.is_empty() {
                                output_ctx.print_data("\nExternal URLs:")?;
                                for asset in &external {
                                    output_ctx
                                        .print_data(format!("  🌐 {}", asset.original_path))?;
                                }
                            }

                            if !missing.is_empty() {
                                output_ctx.print_data("\n⚠ Missing assets:")?;
                                for asset in &missing {
                                    output_ctx
                                        .print_data(format!("  ✗ {}", asset.original_path))?;
                                    if let Some(ref resolved) = asset.resolved_path {
                                        output_ctx.print_data(format!(
                                            "     Expected at: {}",
                                            resolved.display()
                                        ))?;
                                    }
                                }
                            }

                            output_ctx.print_data(format!(
                                "\nSummary: {} total ({} per-doc, {} shared, {} external, {} missing)",
                                result.summary.total,
                                result.summary.per_doc,
                                result.summary.shared,
                                result.summary.external,
                                result.summary.missing
                            ))?;
                        }
                    }
                }
            },
            DocCommands::CheckLinks { scope, json } => {
                use jit::document::DocumentScope;
                use jit::output::JsonOutput;
                use std::str::FromStr;

                let output_ctx = OutputContext::new(quiet, json);
                let scope = DocumentScope::from_str(&scope)?;
                let result = executor.check_document_links(&scope)?;

                if json {
                    let msg = if result.summary.errors == 0 && result.summary.warnings == 0 {
                        "All links valid".to_string()
                    } else {
                        format!(
                            "{} error(s), {} warning(s)",
                            result.summary.errors, result.summary.warnings
                        )
                    };
                    let output = JsonOutput::success(&result, "doc check-links").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    output_ctx.print_data(format!(
                        "Checking {} document(s) in scope '{}'...\n",
                        result.summary.total_documents, result.scope
                    ))?;

                    if !result.errors.is_empty() {
                        output_ctx
                            .print_data(format!("❌ Errors found ({}):", result.errors.len()))?;
                        for error in &result.errors {
                            output_ctx.print_data(format!(
                                "  {} ({}): {}",
                                error["document"].as_str().unwrap_or(""),
                                error["type"].as_str().unwrap_or(""),
                                error["message"].as_str().unwrap_or("")
                            ))?;
                        }
                        output_ctx.print_data("")?;
                    }

                    if !result.warnings.is_empty() {
                        output_ctx
                            .print_data(format!("⚠️  Warnings ({}):", result.warnings.len()))?;
                        for warning in &result.warnings {
                            output_ctx.print_data(format!(
                                "  {} ({}): {}",
                                warning["document"].as_str().unwrap_or(""),
                                warning["type"].as_str().unwrap_or(""),
                                warning["message"].as_str().unwrap_or("")
                            ))?;
                        }
                        output_ctx.print_data("")?;
                    }

                    if result.errors.is_empty() && result.warnings.is_empty() {
                        output_ctx.print_data("✅ All documents valid!")?;
                    }

                    output_ctx.print_data(format!(
                        "Summary: {} document(s) checked, {} error(s), {} warning(s)",
                        result.summary.total_documents,
                        result.summary.errors,
                        result.summary.warnings
                    ))?;
                }

                std::process::exit(result.exit_code);
            }
            DocCommands::Archive {
                path,
                category,
                dry_run,
                force,
                json,
            } => {
                use jit::output::JsonOutput;

                let output_ctx = OutputContext::new(quiet, json);
                let (result, warnings) =
                    executor.archive_document(&path, &category, dry_run, force)?;

                // Print warnings
                for warning in warnings {
                    output_ctx.print_warning(&warning)?;
                }

                if json {
                    let msg = if result.dry_run {
                        "Archival plan (dry run)".to_string()
                    } else {
                        format!("Archived {} to {}", result.source_path, result.dest_path)
                    };
                    let output = JsonOutput::success(result, "doc archive").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else if result.dry_run {
                    println!("✓ Archival plan (--dry-run)\n");
                    println!("  Document:");
                    println!("    📄 {}", result.source_path);
                    println!("       → {}", result.dest_path);
                    println!("\n  Category: {}", result.category);

                    if result.assets_moved > 0 {
                        println!("\n  Assets to move: {}", result.assets_moved);
                    } else {
                        println!("\n  No per-doc assets found");
                    }

                    println!("\n  Run without --dry-run to execute.");
                } else {
                    println!("✓ Archived successfully");
                    println!("  {} → {}", result.source_path, result.dest_path);
                    if result.assets_moved > 0 {
                        println!("  Moved {} asset(s)", result.assets_moved);
                    }
                    if !result.updated_issues.is_empty() {
                        println!("  Updated {} issue(s)", result.updated_issues.len());
                    }
                }
            }
        },
        Commands::Query {
            subcommand,
            state: bare_state,
            assignee: bare_assignee,
            priority: bare_priority,
            label: bare_label,
            full: bare_full,
            json: bare_json,
        } => match subcommand {
            None => run_query_all(
                &executor,
                quiet,
                bare_state,
                bare_assignee,
                bare_priority,
                bare_label,
                bare_full,
                bare_json,
            )?,
            Some(query_cmd) => {
                // Parent-level filters only apply to the bare `jit query` form.
                // When a subcommand is present they would be silently ignored
                // (a machine-readable silent-wrong-result footgun), so reject
                // them with an actionable message instead.
                reject_parent_query_filters(
                    &query_cmd,
                    bare_state.as_deref(),
                    bare_assignee.as_deref(),
                    bare_priority.as_deref(),
                    bare_label.as_deref(),
                    bare_full,
                    bare_json,
                )?;
                match query_cmd {
                    jit::cli::QueryCommands::All {
                        state,
                        assignee,
                        priority,
                        label,
                        full,
                        json,
                    } => {
                        run_query_all(
                            &executor, quiet, state, assignee, priority, label, full, json,
                        )?;
                    }
                    jit::cli::QueryCommands::Available {
                        priority,
                        label,
                        full,
                        json,
                    } => {
                        let output_ctx = OutputContext::new(quiet, json);
                        let priority_filter = priority
                            .as_ref()
                            .map(|p| Priority::from_str(p))
                            .transpose()?;
                        let issues = executor.query_available(priority_filter, label.as_deref())?;

                        if json {
                            use jit::domain::MinimalIssue;
                            use jit::output::JsonOutput;
                            use serde_json::json;

                            let msg = format!("Found {} issue(s)", issues.len());
                            let output = if full {
                                JsonOutput::success(
                                    json!({
                                        "count": issues.len(),
                                        "issues": issues,
                                    }),
                                    "query available",
                                )
                            } else {
                                let minimal: Vec<MinimalIssue> =
                                    issues.iter().map(MinimalIssue::from).collect();
                                JsonOutput::success(
                                    json!({
                                        "count": minimal.len(),
                                        "issues": minimal,
                                    }),
                                    "query available",
                                )
                            }
                            .with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ =
                                output_ctx.print_info("Available issues (unassigned, unblocked):");
                            for issue in &issues {
                                println!("  {} | {} | {:?}", issue.id, issue.title, issue.priority);
                            }
                            let _ = output_ctx.print_info(format!("\nTotal: {}", issues.len()));
                        }
                    }
                    jit::cli::QueryCommands::Blocked {
                        priority,
                        label,
                        full,
                        json,
                    } => {
                        let output_ctx = OutputContext::new(quiet, json);
                        let priority_filter = priority
                            .as_ref()
                            .map(|p| Priority::from_str(p))
                            .transpose()?;
                        let blocked =
                            executor.query_blocked_filtered(priority_filter, label.as_deref())?;

                        if json {
                            use jit::domain::MinimalIssue;
                            use jit::output::JsonOutput;
                            use serde_json::json;

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
                                                            "{} ({}:{:?})",
                                                            id, title, state
                                                        ),
                                                    }
                                                }
                                                BlockingReason::Gate { key, status } => {
                                                    BlockedReason {
                                                        reason_type: BlockedReasonType::Gate,
                                                        detail: format!("{} ({:?})", key, status),
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

                                JsonOutput::success(
                                    json!({
                                        "count": blocked_issues.len(),
                                        "issues": blocked_issues,
                                    }),
                                    "query blocked",
                                )
                            } else {
                                use jit::domain::MinimalBlockedIssue;
                                let minimal: Vec<MinimalBlockedIssue> = blocked
                                    .iter()
                                    .map(|(issue, reasons)| {
                                        let reason_strings =
                                            reasons.iter().map(ToString::to_string).collect();
                                        MinimalBlockedIssue::from((issue, reason_strings))
                                    })
                                    .collect();

                                JsonOutput::success(
                                    json!({
                                        "count": minimal.len(),
                                        "issues": minimal,
                                    }),
                                    "query blocked",
                                )
                            }
                            .with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx.print_info("Blocked issues:");
                            for (issue, reasons) in &blocked {
                                println!("  {} | {} | {:?}", issue.id, issue.title, issue.priority);
                                for reason in reasons {
                                    println!("    - {}", reason);
                                }
                            }
                            let _ = output_ctx.print_info(format!("\nTotal: {}", blocked.len()));
                        }
                    }
                    jit::cli::QueryCommands::Strategic {
                        priority,
                        label,
                        full,
                        json,
                    } => {
                        let output_ctx = OutputContext::new(quiet, json);
                        let priority_filter = priority
                            .as_ref()
                            .map(|p| Priority::from_str(p))
                            .transpose()?;
                        let issues =
                            executor.query_strategic_filtered(priority_filter, label.as_deref())?;

                        if json {
                            use jit::domain::MinimalIssue;
                            use jit::output::JsonOutput;
                            use serde_json::json;

                            let msg = format!("Found {} issue(s)", issues.len());
                            let output = if full {
                                JsonOutput::success(
                                    json!({
                                        "count": issues.len(),
                                        "issues": issues,
                                    }),
                                    "query strategic",
                                )
                            } else {
                                let minimal: Vec<MinimalIssue> =
                                    issues.iter().map(MinimalIssue::from).collect();
                                JsonOutput::success(
                                    json!({
                                        "count": minimal.len(),
                                        "issues": minimal,
                                    }),
                                    "query strategic",
                                )
                            }
                            .with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx.print_info("Strategic issues:");
                            for issue in &issues {
                                println!("  {} | {} | {:?}", issue.id, issue.title, issue.priority);
                            }
                            let _ = output_ctx.print_info(format!("\nTotal: {}", issues.len()));
                        }
                    }
                    jit::cli::QueryCommands::Closed {
                        priority,
                        label,
                        full,
                        json,
                    } => {
                        let output_ctx = OutputContext::new(quiet, json);
                        let priority_filter = priority
                            .as_ref()
                            .map(|p| Priority::from_str(p))
                            .transpose()?;
                        let issues =
                            executor.query_closed_filtered(priority_filter, label.as_deref())?;

                        if json {
                            use jit::domain::MinimalIssue;
                            use jit::output::JsonOutput;
                            use serde_json::json;

                            let msg = format!("Found {} issue(s)", issues.len());
                            let output = if full {
                                JsonOutput::success(
                                    json!({
                                        "count": issues.len(),
                                        "issues": issues,
                                    }),
                                    "query closed",
                                )
                            } else {
                                let minimal: Vec<MinimalIssue> =
                                    issues.iter().map(MinimalIssue::from).collect();
                                JsonOutput::success(
                                    json!({
                                        "count": minimal.len(),
                                        "issues": minimal,
                                    }),
                                    "query closed",
                                )
                            }
                            .with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx.print_info("Closed issues (Done or Rejected):");
                            for issue in &issues {
                                println!("  {} | {} | {:?}", issue.id, issue.title, issue.state);
                            }
                            let _ = output_ctx.print_info(format!("\nTotal: {}", issues.len()));
                        }
                    }
                } // end match query_cmd
            } // end Some(query_cmd)
        }, // end match subcommand
        Commands::Label(label_cmd) => match label_cmd {
            jit::cli::LabelCommands::Namespaces { json } => {
                let output_ctx = OutputContext::new(quiet, json);
                use jit::config_manager::ConfigManager;
                let config_mgr = ConfigManager::new(&jit_dir);
                let namespaces = config_mgr.get_namespaces()?;
                if json {
                    use jit::output::{JsonOutput, NamespacesResponse};
                    let namespace_names: Vec<String> =
                        namespaces.namespaces.keys().cloned().collect();
                    let response = NamespacesResponse {
                        count: namespace_names.len(),
                        namespaces: namespace_names,
                    };
                    let msg = format!("{} namespace(s)", response.count);
                    let output =
                        JsonOutput::success(response, "label namespaces").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ = output_ctx.print_info("Label Namespaces:\n");
                    for (name, ns) in &namespaces.namespaces {
                        println!("  {}", name);
                        println!("    Description: {}", ns.description);
                        println!("    Unique: {}", ns.unique);
                        println!();
                    }
                }
            }
            jit::cli::LabelCommands::Values { namespace, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                let values = executor.list_label_values(&namespace)?;
                if json {
                    use jit::output::JsonOutput;
                    let msg = format!("{} value(s)", values.len());
                    let output = JsonOutput::success(
                        serde_json::json!({
                            "namespace": namespace,
                            "values": values,
                            "count": values.len()
                        }),
                        "label values",
                    )
                    .with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    let _ =
                        output_ctx.print_info(format!("Values in namespace '{}':\n", namespace));
                    for value in &values {
                        println!("  {}", value);
                    }
                    let _ = output_ctx.print_info(format!("\nTotal: {}", values.len()));
                }
            }
        },
        Commands::Config(config_cmd) => match config_cmd {
            jit::cli::ConfigCommands::Show { json } => {
                use jit::config::ConfigLoader;
                use jit::config_manager::ConfigManager;
                use jit::output::JsonOutput;
                use serde_json::json;

                // Build effective config from all sources
                let mut loader = ConfigLoader::new();

                // Try to load system config
                let system_path = std::path::Path::new("/etc/jit");
                if system_path.exists() {
                    loader = loader.with_system_config(system_path)?;
                }

                // Try to load user config
                if let Some(home) = dirs::home_dir() {
                    let user_path = home.join(".config/jit");
                    if user_path.exists() {
                        loader = loader.with_user_config(&user_path)?;
                    }
                }

                // Load repo config
                loader = loader.with_repo_config(&jit_dir)?;

                let config = loader.build();

                if json {
                    // Namespace registry is sourced from repo-level config.toml
                    // (same path the server's /config/namespaces endpoint uses,
                    // so MCP/web consumers see a single canonical shape).
                    // Propagate load errors rather than silently hiding the registry.
                    let label_namespaces = ConfigManager::new(&jit_dir).get_namespaces()?;
                    let mut namespaces_map = serde_json::Map::new();
                    for (name, cfg) in label_namespaces.namespaces {
                        namespaces_map.insert(name, serde_json::to_value(cfg)?);
                    }
                    let namespaces_json = serde_json::Value::Object(namespaces_map);

                    let output = json!({
                        "worktree": {
                            "mode": format!("{:?}", config.worktree_mode().unwrap_or(jit::config::WorktreeMode::Auto)).to_lowercase(),
                            "enforce_leases": format!("{:?}", config.enforcement_mode().unwrap_or(jit::config::EnforcementMode::Strict)).to_lowercase(),
                        },
                        "coordination": {
                            "default_ttl_secs": config.coordination().default_ttl_secs(),
                            "heartbeat_interval_secs": config.coordination().heartbeat_interval_secs(),
                            "lease_renewal_threshold_pct": config.coordination().lease_renewal_threshold_pct(),
                            "stale_threshold_secs": config.coordination().stale_threshold_secs(),
                            "max_indefinite_leases_per_agent": config.coordination().max_indefinite_leases_per_agent(),
                            "max_indefinite_leases_per_repo": config.coordination().max_indefinite_leases_per_repo(),
                            "auto_renew_leases": config.coordination().auto_renew_leases(),
                        },
                        "global_operations": {
                            "require_main_history": config.global_operations().require_main_history(),
                            "allowed_branches": config.global_operations().allowed_branches(),
                        },
                        "locks": {
                            "max_age_secs": config.locks().max_age_secs(),
                            "enable_metadata": config.locks().enable_metadata(),
                        },
                        "events": {
                            "enable_sequences": config.events().enable_sequences(),
                            "use_unified_envelope": config.events().use_unified_envelope(),
                        },
                        "namespaces": namespaces_json,
                    });
                    println!(
                        "{}",
                        JsonOutput::success(output, "config show")
                            .with_message("Effective configuration")
                            .to_json_string()?
                    );
                } else {
                    println!("Effective Configuration:");
                    println!();
                    println!("[worktree]");
                    println!(
                        "  mode = {:?}",
                        config
                            .worktree_mode()
                            .unwrap_or(jit::config::WorktreeMode::Auto)
                    );
                    println!(
                        "  enforce_leases = {:?}",
                        config
                            .enforcement_mode()
                            .unwrap_or(jit::config::EnforcementMode::Strict)
                    );
                    println!();
                    println!("[coordination]");
                    println!(
                        "  default_ttl_secs = {}",
                        config.coordination().default_ttl_secs()
                    );
                    println!(
                        "  heartbeat_interval_secs = {}",
                        config.coordination().heartbeat_interval_secs()
                    );
                    println!(
                        "  lease_renewal_threshold_pct = {}",
                        config.coordination().lease_renewal_threshold_pct()
                    );
                    println!(
                        "  stale_threshold_secs = {}",
                        config.coordination().stale_threshold_secs()
                    );
                    println!(
                        "  max_indefinite_leases_per_agent = {}",
                        config.coordination().max_indefinite_leases_per_agent()
                    );
                    println!(
                        "  max_indefinite_leases_per_repo = {}",
                        config.coordination().max_indefinite_leases_per_repo()
                    );
                    println!(
                        "  auto_renew_leases = {}",
                        config.coordination().auto_renew_leases()
                    );
                    println!();
                    println!("[global_operations]");
                    println!(
                        "  require_main_history = {}",
                        config.global_operations().require_main_history()
                    );
                    println!(
                        "  allowed_branches = {:?}",
                        config.global_operations().allowed_branches()
                    );
                    println!();
                    println!("[locks]");
                    println!("  max_age_secs = {}", config.locks().max_age_secs());
                    println!("  enable_metadata = {}", config.locks().enable_metadata());
                    println!();
                    println!("[events]");
                    println!(
                        "  enable_sequences = {}",
                        config.events().enable_sequences()
                    );
                    println!(
                        "  use_unified_envelope = {}",
                        config.events().use_unified_envelope()
                    );
                }
            }
            jit::cli::ConfigCommands::Get { key, json } => {
                use jit::config::ConfigLoader;
                use jit::output::JsonOutput;
                use serde_json::json;

                // Build effective config
                let mut loader = ConfigLoader::new();
                let system_path = std::path::Path::new("/etc/jit");
                if system_path.exists() {
                    loader = loader.with_system_config(system_path)?;
                }
                if let Some(home) = dirs::home_dir() {
                    let user_path = home.join(".config/jit");
                    if user_path.exists() {
                        loader = loader.with_user_config(&user_path)?;
                    }
                }
                loader = loader.with_repo_config(&jit_dir)?;
                let config = loader.build();

                // Parse key and get value
                let value: Option<serde_json::Value> = match key.as_str() {
                    "worktree.mode" => Some(json!(format!(
                        "{:?}",
                        config
                            .worktree_mode()
                            .unwrap_or(jit::config::WorktreeMode::Auto)
                    )
                    .to_lowercase())),
                    "worktree.enforce_leases" => Some(json!(format!(
                        "{:?}",
                        config
                            .enforcement_mode()
                            .unwrap_or(jit::config::EnforcementMode::Strict)
                    )
                    .to_lowercase())),
                    "coordination.default_ttl_secs" => {
                        Some(json!(config.coordination().default_ttl_secs()))
                    }
                    "coordination.heartbeat_interval_secs" => {
                        Some(json!(config.coordination().heartbeat_interval_secs()))
                    }
                    "coordination.lease_renewal_threshold_pct" => {
                        Some(json!(config.coordination().lease_renewal_threshold_pct()))
                    }
                    "coordination.stale_threshold_secs" => {
                        Some(json!(config.coordination().stale_threshold_secs()))
                    }
                    "coordination.max_indefinite_leases_per_agent" => Some(json!(config
                        .coordination()
                        .max_indefinite_leases_per_agent())),
                    "coordination.max_indefinite_leases_per_repo" => Some(json!(config
                        .coordination()
                        .max_indefinite_leases_per_repo())),
                    "coordination.auto_renew_leases" => {
                        Some(json!(config.coordination().auto_renew_leases()))
                    }
                    "global_operations.require_main_history" => {
                        Some(json!(config.global_operations().require_main_history()))
                    }
                    "global_operations.allowed_branches" => {
                        Some(json!(config.global_operations().allowed_branches()))
                    }
                    "locks.max_age_secs" => Some(json!(config.locks().max_age_secs())),
                    "locks.enable_metadata" => Some(json!(config.locks().enable_metadata())),
                    "events.enable_sequences" => Some(json!(config.events().enable_sequences())),
                    "events.use_unified_envelope" => {
                        Some(json!(config.events().use_unified_envelope()))
                    }
                    _ => None,
                };

                match value {
                    Some(v) => {
                        if json {
                            println!(
                                "{}",
                                JsonOutput::success(json!({"key": key, "value": v}), "config get")
                                    .with_message(format!("{} = {}", key, v))
                                    .to_json_string()?
                            );
                        } else {
                            println!("{}", v);
                        }
                    }
                    None => {
                        anyhow::bail!(
                            "Unknown config key: {}. Use 'jit config show' to see available keys.",
                            key
                        );
                    }
                }
            }
            jit::cli::ConfigCommands::Set {
                key,
                value,
                global,
                json,
            } => {
                use jit::output::JsonOutput;
                use serde_json::json;
                use std::fs;

                // Determine target config file
                let config_path = if global {
                    let home = dirs::home_dir()
                        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
                    let config_dir = home.join(".config/jit");
                    fs::create_dir_all(&config_dir)?;
                    config_dir.join("config.toml")
                } else {
                    jit_dir.join("config.toml")
                };

                // Load existing config or create empty
                let mut doc = if config_path.exists() {
                    let content = fs::read_to_string(&config_path)?;
                    content
                        .parse::<toml_edit::DocumentMut>()
                        .map_err(|e| anyhow::anyhow!("Failed to parse config: {}", e))?
                } else {
                    toml_edit::DocumentMut::new()
                };

                // Parse key into section.field
                let parts: Vec<&str> = key.split('.').collect();
                if parts.len() != 2 {
                    anyhow::bail!("Config key must be in format 'section.field' (e.g., coordination.default_ttl_secs)");
                }
                let section = parts[0];
                let field = parts[1];

                // Ensure section exists
                if doc.get(section).is_none() {
                    doc[section] = toml_edit::Item::Table(toml_edit::Table::new());
                }

                // Parse and set value based on expected type
                let parsed_value: toml_edit::Item = match key.as_str() {
                    k if k.ends_with("_secs") || k.ends_with("_pct") || k.contains("max_") => {
                        let num: i64 = value
                            .parse()
                            .map_err(|_| anyhow::anyhow!("Expected numeric value for {}", key))?;
                        toml_edit::value(num)
                    }
                    k if k.contains("enable_") || k.contains("require_") || k.contains("auto_") => {
                        let b: bool = value.parse().map_err(|_| {
                            anyhow::anyhow!("Expected boolean (true/false) for {}", key)
                        })?;
                        toml_edit::value(b)
                    }
                    _ => toml_edit::value(&value),
                };

                doc[section][field] = parsed_value;

                // Write back
                fs::write(&config_path, doc.to_string())?;

                if json {
                    println!(
                        "{}",
                        JsonOutput::success(
                            json!({
                                "key": key,
                                "value": value,
                                "file": config_path.display().to_string(),
                                "scope": if global { "user" } else { "repo" }
                            }),
                            "config set"
                        )
                        .to_json_string()?
                    );
                } else {
                    println!("Set {} = {} in {}", key, value, config_path.display());
                }
            }
            jit::cli::ConfigCommands::Validate { json } => {
                use jit::config::{ConfigLoader, JitConfig};
                use jit::output::JsonOutput;
                use serde_json::json;

                #[derive(Default)]
                struct ValidationResult {
                    errors: Vec<String>,
                    warnings: Vec<String>,
                }

                let mut result = ValidationResult::default();

                // Check repo config — invalid worktree/enforcement tokens are now
                // caught at TOML parse time, so a successful load implies valid values.
                let repo_config_path = jit_dir.join("config.toml");
                if repo_config_path.exists() {
                    if let Err(e) = JitConfig::load(&jit_dir) {
                        result.errors.push(format!("repo config: {}", e));
                    }
                }

                // Check user config.
                if let Some(home) = dirs::home_dir() {
                    let user_config_path = home.join(".config/jit/config.toml");
                    if user_config_path.exists() {
                        let user_dir = home.join(".config/jit");
                        if let Err(e) = JitConfig::load(&user_dir) {
                            result.errors.push(format!("user config: {}", e));
                        }
                    }
                }

                // Check env vars — use the same FromStr as TOML parsing so
                // case-handling is identical across both sources.
                if let Ok(val) = std::env::var("JIT_WORKTREE_MODE") {
                    if let Err(e) = val.parse::<jit::config::WorktreeMode>() {
                        result.errors.push(format!("JIT_WORKTREE_MODE: {e}"));
                    }
                }
                if let Ok(val) = std::env::var("JIT_ENFORCE_LEASES") {
                    if let Err(e) = val.parse::<jit::config::EnforcementMode>() {
                        result.errors.push(format!("JIT_ENFORCE_LEASES: {e}"));
                    }
                }

                // Try to build effective config to catch merge issues
                let loader = ConfigLoader::new();
                let _ = loader.with_repo_config(&jit_dir);

                let has_errors = !result.errors.is_empty();
                let has_warnings = !result.warnings.is_empty();

                if json {
                    let output = json!({
                        "valid": !has_errors,
                        "errors": result.errors,
                        "warnings": result.warnings,
                    });
                    println!(
                        "{}",
                        JsonOutput::success(output, "config validate")
                            .with_message(if has_errors {
                                format!("Validation failed: {} error(s)", result.errors.len())
                            } else if has_warnings {
                                format!(
                                    "Validation passed with {} warning(s)",
                                    result.warnings.len()
                                )
                            } else {
                                "Configuration is valid".to_string()
                            })
                            .to_json_string()?
                    );
                } else if result.errors.is_empty() && result.warnings.is_empty() {
                    println!("✓ Configuration is valid");
                } else {
                    if !result.errors.is_empty() {
                        println!("Errors:");
                        for err in &result.errors {
                            println!("  ✗ {}", err);
                        }
                    }
                    if !result.warnings.is_empty() {
                        println!("Warnings:");
                        for warn in &result.warnings {
                            println!("  ⚠ {}", warn);
                        }
                    }
                }

                // Exit with appropriate code
                if has_errors {
                    std::process::exit(1);
                } else if has_warnings {
                    std::process::exit(2);
                }
            }
            jit::cli::ConfigCommands::ShowHierarchy { json } => {
                let output_ctx = OutputContext::new(quiet, json);
                use jit::config_manager::ConfigManager;
                let config_mgr = ConfigManager::new(&jit_dir);
                let namespaces = config_mgr.get_namespaces()?;
                let hierarchy = namespaces.get_type_hierarchy();

                if json {
                    use jit::output::JsonOutput;
                    println!(
                        "{}",
                        JsonOutput::success(hierarchy, "config show-hierarchy")
                            .with_message("Type hierarchy")
                            .to_json_string()?
                    );
                } else {
                    let _ = output_ctx.print_info("Type Hierarchy:\n");
                    let mut sorted: Vec<_> = hierarchy.iter().collect();
                    sorted.sort_by_key(|(_, level)| *level);
                    for (type_name, level) in sorted {
                        println!("  {} → Level {}", type_name, level);
                    }
                }
            }
            jit::cli::ConfigCommands::ListTemplates { json } => {
                let output_ctx = OutputContext::new(quiet, json);
                let templates = jit::hierarchy_templates::HierarchyTemplate::all();

                if json {
                    use jit::output::JsonOutput;
                    use serde_json::json;
                    let template_data: Vec<_> = templates
                        .iter()
                        .map(|t| {
                            json!({
                                "name": t.name,
                                "description": t.description,
                                "hierarchy": t.hierarchy
                            })
                        })
                        .collect();
                    let count = template_data.len();
                    println!(
                        "{}",
                        JsonOutput::success(
                            serde_json::json!({"templates": template_data, "count": count}),
                            "config list-templates",
                        )
                        .with_message(format!("{} template(s)", count))
                        .to_json_string()?
                    );
                } else {
                    let _ = output_ctx.print_info("Available Hierarchy Templates:\n");
                    for template in templates {
                        println!("  {}", template.name);
                        println!("    {}", template.description);
                        println!();
                    }
                }
            }
        },
        Commands::Hooks(hooks_cmd) => match hooks_cmd {
            jit::cli::HooksCommands::Install { json } => {
                use jit::commands::hooks::install_hooks;

                match install_hooks(None) {
                    Ok(result) => {
                        if json {
                            let output = jit::output::JsonOutput::success(
                                serde_json::json!({
                                    "hooks_dir": result.hooks_dir,
                                    "installed": result.installed,
                                    "skipped": result.skipped,
                                }),
                                "hooks install",
                            )
                            .with_message(format!(
                                "Installed {} hook(s) to {}",
                                result.installed.len(),
                                result.hooks_dir
                            ));
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("Installed hooks to: {}", result.hooks_dir);
                            if !result.installed.is_empty() {
                                println!("\nInstalled:");
                                for hook in &result.installed {
                                    println!("  ✓ {}", hook);
                                }
                            }
                            if !result.skipped.is_empty() {
                                println!("\nSkipped (already exist):");
                                for hook in &result.skipped {
                                    println!("  - {}", hook);
                                }
                            }
                            println!("\nHooks are now active. Configure enforcement in .jit/config.toml:");
                            println!("  [worktree]");
                            println!("  enforce_leases = \"strict\"");
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error = jit::output::JsonError::new(
                                "HOOKS_INSTALL_ERROR",
                                e.to_string(),
                                "hooks install",
                            );
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
        },
        Commands::Item(item_cmd) => {
            run_item(&executor, item_cmd, quiet)?;
        }
        Commands::Invariant(invariant_cmd) => {
            run_invariant(&executor, invariant_cmd, quiet)?;
        }
        Commands::Search {
            query,
            regex,
            case_sensitive,
            context,
            limit,
            glob,
            json,
        } => {
            let output_ctx = OutputContext::new(quiet, json);
            use jit::search::{search, SearchOptions};

            let options = SearchOptions {
                case_sensitive,
                regex,
                context_lines: context,
                max_results: limit,
                file_pattern: glob.clone(),
                file_patterns: Vec::new(),
            };

            match search(&jit_dir, &query, options) {
                Ok(results) => {
                    if json {
                        use jit::output::{JsonOutput, SearchResponse};

                        let msg = format!("Found {} result(s)", results.len());
                        let response = SearchResponse {
                            query,
                            count: results.len(),
                            results,
                        };
                        let output = JsonOutput::success(response, "search").with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else if results.is_empty() {
                        let _ =
                            output_ctx.print_info(format!("No matches found for \"{}\"", query));
                    } else {
                        let _ = output_ctx.print_info(format!(
                            "Search results for \"{}\" ({} matches):\n",
                            query,
                            results.len()
                        ));

                        let mut current_file = String::new();
                        for result in &results {
                            if result.path != current_file {
                                current_file = result.path.clone();

                                if let Some(issue_id) = &result.issue_id {
                                    // Try to load issue for title
                                    if let Ok(issue) = storage.load_issue(issue_id) {
                                        println!("Issue {} | {}", issue_id, issue.title);
                                    } else {
                                        println!("Issue {}", issue_id);
                                    }
                                } else {
                                    println!("Document {}", result.path);
                                }
                            }

                            println!("  Line {}: {}", result.line_number, result.line_text.trim());
                        }
                        println!();
                    }
                }
                Err(e) => {
                    if json {
                        use jit::output::JsonError;

                        // Classify by downcast against the typed SearchError, not by
                        // scanning the message text. RipgrepNotInstalled -> the
                        // not-found code; every other failure (rg ran and failed, an
                        // io/spawn error, a parse error) -> the generic search code.
                        let error_code = if matches!(
                            e.downcast_ref::<jit::search::SearchError>(),
                            Some(jit::search::SearchError::RipgrepNotInstalled)
                        ) {
                            "RIPGREP_NOT_FOUND"
                        } else {
                            "SEARCH_FAILED"
                        };

                        let suggestion = if error_code == "RIPGREP_NOT_FOUND" {
                            Some(
                                "Install ripgrep from https://github.com/BurntSushi/ripgrep"
                                    .to_string(),
                            )
                        } else {
                            None
                        };

                        let mut json_error = JsonError::new(error_code, e.to_string(), "validate");
                        if let Some(sug) = suggestion {
                            json_error = json_error.with_suggestion(sug);
                        }
                        println!("{}", json_error.to_json_string()?);
                        std::process::exit(10); // External dependency failed
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        Commands::Status { json } => {
            let output_ctx = OutputContext::new(quiet, json);
            let summary = executor.get_status()?;

            if json {
                let msg = format!(
                    "{} open, {} ready, {} in progress, {} done",
                    summary.open, summary.ready, summary.in_progress, summary.done
                );
                let output = JsonOutput::success(&summary, "status").with_message(msg);
                println!("{}", output.to_json_string()?);
            } else {
                output_ctx.print_data("Status:")?;
                output_ctx.print_data(format!("  Open: {}", summary.open))?;
                output_ctx.print_data(format!("  Ready: {}", summary.ready))?;
                output_ctx.print_data(format!("  In Progress: {}", summary.in_progress))?;
                output_ctx.print_data(format!("  Done: {}", summary.done))?;
                output_ctx.print_data(format!("  Rejected: {}", summary.rejected))?;
                output_ctx.print_data(format!("  Blocked: {}", summary.blocked))?;
            }
        }
        Commands::Validate {
            id,
            json,
            explain,
            fix,
            dry_run,
            divergence,
            leases,
            scope,
        } => {
            // Validate dry_run requires fix
            if dry_run && !fix {
                return Err(anyhow!("--dry-run requires --fix to be specified"));
            }

            // `--scope <C>` is a self-contained gate checker over a container's
            // bracket subtree. It owns its own exit code (4 on any
            // error-severity finding) and is mutually exclusive with the other
            // validate modes, so it is dispatched FIRST after the combo checks
            // below reject conflicting flags.
            if scope.is_some() && (id.is_some() || fix || divergence || leases || explain) {
                return Err(anyhow!(
                    "`--scope` cannot be combined with a positional id or with \
                     `--fix`/`--divergence`/`--leases`/`--explain`"
                ));
            }

            // `--fix`, `--divergence`, and `--leases` are repo-wide operations and
            // are NOT scoped to a single issue. Combining any of them with a
            // positional issue id is rejected explicitly: previously the id was
            // silently ignored and the command ran repo-wide, which is dangerous
            // for `--fix` (it could mutate the entire repository when the user
            // believed they had scoped it to one issue).
            if id.is_some() && (fix || divergence || leases) {
                return Err(anyhow!(
                    "`--fix`/`--divergence`/`--leases` cannot be combined with a \
                     positional issue id (they are repo-wide)"
                ));
            }

            // --scope path: evaluate the container's bracket subtree as a
            // deterministic gate checker. Exit 4 (ValidationFailed) on any
            // error-severity finding, 0 when clean.
            if let Some(container) = scope.as_deref() {
                let report = executor.validate_scope(container)?;
                let exit_nonzero = report.has_errors();
                if json {
                    use jit::output::JsonOutput;
                    let value = serde_json::to_value(&report)?;
                    let output =
                        JsonOutput::success(value, "validate").with_message(if exit_nonzero {
                            format!(
                                "Scope validation failed with {} error(s)",
                                report.error_count()
                            )
                        } else {
                            "Scope validation passed".to_string()
                        });
                    println!("{}", output.to_json_string()?);
                } else if report.findings.is_empty() {
                    println!("✓ Scope validation passed");
                } else {
                    for finding in &report.findings {
                        println!(
                            "{} [{}] {}",
                            if finding.is_error() { "❌" } else { "⚠" },
                            finding.rule,
                            finding.message
                        );
                    }
                    if exit_nonzero {
                        eprintln!(
                            "Scope validation failed with {} error(s)",
                            report.error_count()
                        );
                    } else {
                        println!("✓ Scope validation passed");
                    }
                }
                if exit_nonzero {
                    std::process::exit(jit::ExitCode::ValidationFailed.code());
                }
                return Ok(());
            }

            // --explain requires an issue id (it is a per-issue debugging view).
            if explain && id.is_none() {
                return Err(anyhow!("--explain requires an issue id"));
            }

            // --explain path: report matched selectors -> rule names -> outcomes.
            if explain {
                let issue_id = id.as_deref().expect("checked above");
                let report = executor.explain_rules(issue_id)?;
                let exit_nonzero = report.has_errors();
                if json {
                    use jit::output::JsonOutput;
                    let value = serde_json::to_value(&report)?;
                    let output =
                        JsonOutput::success(value, "validate").with_message(if exit_nonzero {
                            "Validation found error-severity rule failures".to_string()
                        } else {
                            "Validation passed".to_string()
                        });
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Rule explanation for issue {}", report.issue_id);
                    if report.outcomes.is_empty() {
                        println!("  (no rules defined)");
                    }
                    // Matched rules first, with their PASS/FAIL result.
                    let mut any_matched = false;
                    for outcome in report.outcomes.iter().filter(|o| o.matched) {
                        any_matched = true;
                        let status = if outcome.passed { "PASS" } else { "FAIL" };
                        println!(
                            "  [{}] {} ({}, {}) selector: {}",
                            status,
                            outcome.rule,
                            outcome.scope.token(),
                            outcome.severity.token(),
                            outcome.selector
                        );
                        for message in &outcome.messages {
                            println!("      - {}", message);
                        }
                    }
                    if !any_matched && !report.outcomes.is_empty() {
                        println!("  (no rules match this issue)");
                    }
                    // Non-matching rules after, each with the reason its selector
                    // did not apply (e.g. the state predicate did not match).
                    for outcome in report.outcomes.iter().filter(|o| !o.matched) {
                        let reason = outcome
                            .skip_reason
                            .as_deref()
                            .unwrap_or("selector did not match");
                        println!(
                            "  [SKIP] {} ({}, {}) selector: {} — {}",
                            outcome.rule,
                            outcome.scope.token(),
                            outcome.severity.token(),
                            outcome.selector,
                            reason
                        );
                    }
                }
                if exit_nonzero {
                    std::process::exit(1);
                }
                return Ok(());
            }

            // Per-issue rule run: `jit validate <id>`. Incompatible flag combos
            // (`--fix`/`--divergence`/`--leases` + id) were already rejected above,
            // so a present id here is always a pure per-issue rule run.
            if id.is_some() {
                let report = executor.run_rules(id.as_deref())?;
                let exit_nonzero = report.has_errors();
                if json {
                    use jit::output::JsonOutput;
                    let value = serde_json::to_value(&report)?;
                    let output =
                        JsonOutput::success(value, "validate").with_message(if exit_nonzero {
                            format!("Validation failed with {} error(s)", report.error_count())
                        } else {
                            "Validation passed".to_string()
                        });
                    println!("{}", output.to_json_string()?);
                } else if report.findings.is_empty() {
                    println!("✓ Issue validation passed");
                } else {
                    for finding in &report.findings {
                        println!(
                            "{} [{}] {}",
                            if finding.is_error() { "❌" } else { "⚠" },
                            finding.rule,
                            finding.message
                        );
                    }
                    if exit_nonzero {
                        eprintln!("Validation failed with {} error(s)", report.error_count());
                    }
                }
                if exit_nonzero {
                    std::process::exit(1);
                }
                return Ok(());
            }

            // Handle specific validations if requested
            if divergence || leases {
                let mut validation_results = Vec::new();

                if divergence {
                    match executor.validate_divergence() {
                        Ok(()) => {
                            if !json {
                                println!("✓ Branch is up-to-date with origin/main");
                            }
                            validation_results.push(("divergence", true, String::new()));
                        }
                        Err(e) => {
                            if json {
                                validation_results.push(("divergence", false, e.to_string()));
                            } else {
                                eprintln!("❌ Divergence validation failed:\n{}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                }

                if leases {
                    match executor.validate_leases() {
                        Ok(invalid_leases) => {
                            if invalid_leases.is_empty() {
                                if !json {
                                    println!("✓ All active leases are valid");
                                }
                                validation_results.push(("leases", true, String::new()));
                            } else {
                                let message = format!(
                                    "Found {} invalid lease(s):\n{}",
                                    invalid_leases.len(),
                                    invalid_leases.join("\n\n")
                                );
                                if json {
                                    validation_results.push(("leases", false, message.clone()));
                                } else {
                                    eprintln!("❌ Lease validation failed:\n{}", message);
                                    std::process::exit(1);
                                }
                            }
                        }
                        Err(e) => {
                            if json {
                                validation_results.push(("leases", false, format!("Error: {}", e)));
                            } else {
                                eprintln!("❌ Lease validation error: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                }

                if json {
                    use jit::output::JsonOutput;
                    use serde_json::json;

                    let all_valid = validation_results.iter().all(|(_, valid, _)| *valid);
                    let results_json: Vec<_> = validation_results
                        .iter()
                        .map(|(name, valid, message)| {
                            json!({
                                "validation": name,
                                "valid": valid,
                                "message": message
                            })
                        })
                        .collect();

                    let msg = if all_valid {
                        "Validation passed".to_string()
                    } else {
                        "Validation failed".to_string()
                    };
                    let output = JsonOutput::success(
                        json!({
                            "valid": all_valid,
                            "validations": results_json
                        }),
                        "validate",
                    )
                    .with_message(msg);
                    println!("{}", output.to_json_string()?);

                    if !all_valid {
                        std::process::exit(1);
                    }
                }

                return Ok(());
            }

            // Standard repository validation (existing code)
            if fix {
                // Use auto-fix mode
                let (fixes_applied, messages) = executor.validate_with_fix(true, dry_run)?;

                // Print messages unless in JSON mode
                if !json {
                    for message in &messages {
                        println!("{}", message);
                    }
                }

                if json {
                    use jit::output::JsonOutput;
                    use serde_json::json;

                    let output = JsonOutput::success(
                        json!({
                            "valid": true,
                            "fixes_applied": fixes_applied,
                            "dry_run": dry_run,
                            "message": if dry_run {
                                format!("{} fixes would be applied", fixes_applied)
                            } else if fixes_applied > 0 {
                                format!("Applied {} fixes, repository is now valid", fixes_applied)
                            } else {
                                "Repository is valid".to_string()
                            }
                        }),
                        "validate",
                    );
                    println!("{}", output.to_json_string()?);
                }
            } else {
                // Standard whole-repo validation. Run the repo-integrity checks
                // (broken deps, gates, labels, DAG, transitive reduction, claims
                // index) SEPARATELY from the declarative rule report, and capture
                // (do NOT `?`-propagate) any integrity error: surfacing it through
                // the generic error path would lose the structured rule report
                // (which includes graph-rule findings). The exit status is decided
                // AFTER rendering, below.
                // Wrap any integrity violation in the typed ValidationFailedError
                // (message preserved verbatim) so the top-level handler classifies
                // it as a validation failure by downcast rather than by message text.
                let integrity_error = executor.validate_integrity_silent().err().map(|e| {
                    anyhow::Error::new(jit::errors::ValidationFailedError::new(e.to_string()))
                });
                let integrity_message = integrity_error.as_ref().map(|e| e.to_string());

                // Run the declarative rules for every issue AND the cross-issue
                // graph rules so a whole-repo `jit validate [--json]` surfaces (and
                // fails on) local AND graph rule findings, not just integrity
                // checks. `run_rules(None)` already folds in graph-rule findings,
                // INCLUDING the built-in type-hierarchy warnings (orphan-leaf,
                // strategic-consistency) that were formerly surfaced by the
                // hard-coded `collect_all_warnings` path.
                let rule_report = executor.run_rules(None)?;
                let rules_failed = rule_report.has_errors();
                let validation_failed = rules_failed || integrity_error.is_some();

                // Warn-severity findings are reported separately as "warnings" for
                // output-shape stability (the prior orphan/strategic warning list).
                let warning_findings: Vec<&jit::validation::report::ReportedFinding> = rule_report
                    .findings
                    .iter()
                    .filter(|f| !f.is_error())
                    .collect();

                if json {
                    use jit::output::JsonOutput;
                    use serde_json::json;

                    let warnings_json: Vec<_> = warning_findings
                        .iter()
                        .map(|f| {
                            json!({
                                "type": "rule_warning",
                                "issue_id": f.issue_id,
                                "rule": f.rule,
                                "message": f.message,
                            })
                        })
                        .collect();

                    let findings_json = serde_json::to_value(&rule_report.findings)?;
                    let message = if let Some(err) = &integrity_message {
                        if rules_failed {
                            format!(
                                "Repository validation failed: {} rule error(s) and a \
                                 repository-integrity error: {}",
                                rule_report.error_count(),
                                err
                            )
                        } else {
                            format!("Repository integrity validation failed: {}", err)
                        }
                    } else if rules_failed {
                        format!(
                            "Repository validation failed with {} rule error(s)",
                            rule_report.error_count()
                        )
                    } else {
                        "Repository validation passed".to_string()
                    };
                    let output = JsonOutput::success(
                        json!({
                            "valid": !validation_failed,
                            "integrity_error": integrity_message,
                            "warnings": warnings_json,
                            "warning_count": warnings_json.len(),
                            "rule_findings": findings_json,
                            "error_count": rule_report.error_count(),
                            "message": message
                        }),
                        "validate",
                    );
                    println!("{}", output.to_json_string()?);
                } else {
                    if validation_failed {
                        if rules_failed {
                            println!(
                                "❌ Repository validation failed with {} rule error(s)",
                                rule_report.error_count()
                            );
                        }
                        if let Some(err) = &integrity_message {
                            println!("❌ Repository integrity error: {}", err);
                        }
                    } else {
                        println!("✓ Repository validation passed");
                    }

                    // Every finding (errors AND warnings — including the built-in
                    // type-hierarchy warnings) is rendered through the rule report.
                    for finding in &rule_report.findings {
                        println!(
                            "{} [{}] {}",
                            if finding.is_error() { "❌" } else { "⚠" },
                            finding.rule,
                            finding.message
                        );
                    }

                    if !warning_findings.is_empty() {
                        println!("\nWarnings: {}", warning_findings.len());
                    }
                }

                // Decide the exit status AFTER rendering. The structured rule
                // report (including graph-rule findings) has already been printed
                // above, so finding #1 is fixed regardless of how we exit.
                //
                // A repository-integrity error is propagated as an `Err` so it
                // keeps its specific exit code (e.g. a broken dependency maps to
                // `ExitCode::ValidationFailed`) and is surfaced on stderr by the
                // top-level handler — it is never lost. Otherwise, an
                // error-severity rule finding (local OR graph) exits non-zero.
                if let Some(err) = integrity_error {
                    return Err(err);
                }
                if rules_failed {
                    std::process::exit(1);
                }
            }
        }
        Commands::Recover { json } => {
            use jit::commands::claim::execute_recover;
            use jit::output::{JsonOutput, OutputContext};
            use serde_json::json;

            match execute_recover(&storage) {
                Ok(report) => {
                    if json {
                        let msg = format!(
                            "Recovery: {} locks cleaned, {} leases evicted",
                            report.stale_locks_cleaned, report.expired_leases_evicted
                        );
                        let output = JsonOutput::success(
                            json!({
                                "success": true,
                                "stale_locks_cleaned": report.stale_locks_cleaned,
                                "index_rebuilt": report.index_rebuilt,
                                "expired_leases_evicted": report.expired_leases_evicted,
                                "temp_files_removed": report.temp_files_removed,
                                "warnings": report.warnings,
                            }),
                            "recover",
                        )
                        .with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else {
                        println!("Recovery complete:");
                        println!("  • Stale locks cleaned: {}", report.stale_locks_cleaned);
                        println!("  • Index rebuilt: {}", report.index_rebuilt);
                        println!(
                            "  • Expired leases evicted: {}",
                            report.expired_leases_evicted
                        );
                        println!("  • Temp files removed: {}", report.temp_files_removed);
                        let output_ctx = OutputContext::new(quiet, json);
                        for warning in &report.warnings {
                            output_ctx.print_warning(warning)?;
                        }
                    }
                }
                Err(e) => {
                    if json {
                        let output = jit::output::JsonError::new(
                            "recovery_failed",
                            e.to_string(),
                            "recover",
                        );
                        eprintln!("{}", serde_json::to_string(&output)?);
                        std::process::exit(1);
                    } else {
                        eprintln!("Recovery failed: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        }
        Commands::Serve {
            port,
            stop,
            status,
            fg,
            log,
            web_dir,
            json,
        } => {
            use jit::commands::serve::{
                find_web_dir, server_status, start_server, stop_server, ServeOptions, ServeOutcome,
                StopOutcome,
            };
            use serde_json::json;

            let log_file = log.map(|l| jit_dir.join(l));

            if stop {
                match stop_server(&jit_dir) {
                    Ok(StopOutcome::Stopped { pid, port: p }) => {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&json!({
                                    "status": "stopped",
                                    "pid": pid,
                                    "port": p
                                }))?
                            );
                        } else {
                            println!("Server stopped (was PID {pid} on port {p})");
                        }
                    }
                    Ok(StopOutcome::NotRunning) => {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&json!({"status": "not_running"}))?
                            );
                        } else {
                            println!("Server is not running.");
                        }
                    }
                    Err(e) => {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&json!({
                                    "status": "error",
                                    "error": e.to_string()
                                }))?
                            );
                        } else {
                            eprintln!("Error stopping server: {e}");
                        }
                        std::process::exit(1);
                    }
                }
            } else if status {
                match server_status(&jit_dir) {
                    Ok(Some(pf)) => {
                        let url = format!("http://localhost:{}", pf.port);
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&json!({
                                    "status": "running",
                                    "pid": pf.pid,
                                    "port": pf.port,
                                    "url": url,
                                    "log_file": pf.log_file,
                                    "started_at": pf.started_at
                                }))?
                            );
                        } else {
                            println!(
                                "Server is running: {} (PID {}, started {})",
                                url, pf.pid, pf.started_at
                            );
                        }
                    }
                    Ok(None) => {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&json!({"status": "not_running"}))?
                            );
                        } else {
                            println!("Server is not running.");
                        }
                    }
                    Err(e) => {
                        if json {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&json!({
                                    "status": "error",
                                    "error": e.to_string()
                                }))?
                            );
                        } else {
                            eprintln!("Error checking server status: {e}");
                        }
                        std::process::exit(1);
                    }
                }
            } else {
                // Foreground mode: run inline so we can print the URL before blocking.
                if fg {
                    use jit::commands::serve::{
                        find_available_port, find_server_binary, find_web_dir, is_process_alive,
                        read_pid_file,
                    };

                    // Honour existing running server.
                    if let Some(pf) = read_pid_file(&jit_dir)? {
                        if is_process_alive(pf.pid) {
                            let url = format!("http://localhost:{}", pf.port);
                            if json {
                                println!(
                                    "{}",
                                    serde_json::to_string_pretty(&json!({
                                        "status": "running",
                                        "pid": pf.pid,
                                        "port": pf.port,
                                        "url": url
                                    }))?
                                );
                            } else {
                                println!("Server is already running on {url} (PID {})", pf.pid);
                            }
                            return Ok(());
                        }
                    }

                    let p = find_available_port(port)?;
                    let url = format!("http://localhost:{p}");
                    let server_bin = find_server_binary()?;
                    let data_dir_str = jit_dir
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("data_dir is not valid UTF-8"))?;

                    let resolved_web_dir = web_dir
                        .map(|d| jit_dir.parent().unwrap_or(&jit_dir).join(d))
                        .or_else(find_web_dir);

                    if !json {
                        println!("Starting server on {url} (foreground, Ctrl+C to stop)");
                        println!("  API: {url}/api");
                        if let Some(web) = resolved_web_dir.as_deref().filter(|d| d.is_dir()) {
                            println!("  Web: {url}/ (from {})", web.display());
                        } else {
                            println!("  Web: {url}/ (embedded assets)");
                        }
                    }

                    let mut cmd = std::process::Command::new(&server_bin);
                    cmd.arg("--data-dir")
                        .arg(data_dir_str)
                        .arg("--bind")
                        .arg(format!("0.0.0.0:{p}"));
                    if let Some(web) = &resolved_web_dir {
                        if web.is_dir() {
                            cmd.arg("--web-dir").arg(web);
                        }
                    }
                    let status = cmd.status().context("Failed to run jit-server")?;
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&json!({
                                "status": "exited",
                                "port": p,
                                "exit_code": status.code()
                            }))?
                        );
                    }
                    if !status.success() {
                        std::process::exit(status.code().unwrap_or(1));
                    }
                } else {
                    // Daemonize via start_server.
                    let resolved_web_dir = web_dir
                        .map(|d| jit_dir.parent().unwrap_or(&jit_dir).join(d))
                        .or_else(find_web_dir);
                    let web_dir_display = resolved_web_dir
                        .as_deref()
                        .filter(|d| d.is_dir())
                        .map(|d| d.display().to_string());
                    let opts = ServeOptions {
                        data_dir: jit_dir.clone(),
                        preferred_port: port,
                        log_file,
                        web_dir: resolved_web_dir,
                        server_binary: None,
                    };
                    match start_server(opts) {
                        Ok(ServeOutcome::Started {
                            pid,
                            port: p,
                            log_file: lf,
                        }) => {
                            let url = format!("http://localhost:{p}");
                            if json {
                                println!(
                                    "{}",
                                    serde_json::to_string_pretty(&json!({
                                        "status": "started",
                                        "pid": pid,
                                        "port": p,
                                        "url": url,
                                        "log_file": lf,
                                        "web_ui": true,
                                        "web_ui_source": if web_dir_display.is_some() {
                                            "filesystem"
                                        } else {
                                            "embedded"
                                        }
                                    }))?
                                );
                            } else {
                                println!("Server started on {url} (PID {pid})");
                                println!("  API: {url}/api");
                                if let Some(ref dir) = web_dir_display {
                                    println!("  Web: {url}/ (from {dir})");
                                } else {
                                    println!("  Web: {url}/ (embedded assets)");
                                }
                                println!("  Log: {}", lf.display());
                            }
                        }
                        Ok(ServeOutcome::AlreadyRunning { pid, port: p }) => {
                            let url = format!("http://localhost:{p}");
                            if json {
                                println!(
                                    "{}",
                                    serde_json::to_string_pretty(&json!({
                                        "status": "running",
                                        "pid": pid,
                                        "port": p,
                                        "url": url
                                    }))?
                                );
                            } else {
                                println!("Server is already running on {url} (PID {pid})");
                            }
                        }
                        Err(e) => {
                            if json {
                                println!(
                                    "{}",
                                    serde_json::to_string_pretty(&json!({
                                        "status": "error",
                                        "error": e.to_string()
                                    }))?
                                );
                            } else {
                                eprintln!("Error starting server: {e}");
                            }
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        Commands::Claim(claim_cmd) => match claim_cmd {
            ClaimCommands::Acquire {
                issue_id,
                ttl,
                agent_id,
                reason,
                json,
            } => {
                use jit::commands::claim::execute_claim_acquire;
                use jit::output::{JsonOutput, OutputContext};

                match execute_claim_acquire(
                    &storage,
                    &issue_id,
                    ttl,
                    agent_id.as_deref(),
                    reason.as_deref(),
                ) {
                    Ok((lease_id, warnings)) => {
                        if json {
                            let response = serde_json::json!({
                                "lease_id": lease_id,
                                "issue_id": issue_id,
                                "ttl_secs": ttl,
                                "warnings": warnings,
                                "message": format!("Acquired lease {} on issue {}", lease_id, issue_id),
                            });
                            let output = JsonOutput::success(response, "claim acquire");
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("✓ Acquired lease: {}", lease_id);
                            println!("  Issue: {}", issue_id);
                            println!("  TTL: {} seconds", ttl);
                            let output_ctx = OutputContext::new(quiet, json);
                            for warning in &warnings {
                                output_ctx.print_warning(warning)?;
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error =
                                claim_json_error(&e, "CLAIM_ACQUIRE_ERROR", "claim acquire");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            ClaimCommands::Release { issue_id, json } => {
                use jit::commands::claim::execute_claim_release_by_issue;
                use jit::output::{JsonOutput, OutputContext};

                match execute_claim_release_by_issue(&storage, &issue_id) {
                    Ok((released, warnings)) => {
                        if json {
                            let response = serde_json::json!({
                                "lease_id": released.lease_id,
                                "issue_id": released.issue_id,
                                "previous_owner": released.previous_owner,
                                "actor": released.actor,
                                "warnings": warnings,
                                "message": format!(
                                    "Released lease {} on issue {} (was held by {}) by {}",
                                    released.lease_id,
                                    released.issue_id,
                                    released.previous_owner,
                                    released.actor
                                ),
                            });
                            let output = JsonOutput::success(response, "claim release");
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("✓ Released lease: {}", released.lease_id);
                            println!("  Issue: {}", released.issue_id);
                            println!("  Previous owner: {}", released.previous_owner);
                            println!("  Released by: {}", released.actor);
                            let output_ctx = OutputContext::new(quiet, json);
                            for warning in &warnings {
                                output_ctx.print_warning(warning)?;
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error =
                                claim_json_error(&e, "CLAIM_RELEASE_ERROR", "claim release");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            ClaimCommands::Renew {
                lease_id,
                extension,
                json,
            } => {
                use jit::commands::claim::execute_claim_renew;
                use jit::output::{JsonOutput, OutputContext};

                match execute_claim_renew::<jit::JsonFileStorage>(&lease_id, extension) {
                    Ok((renewed_lease, warnings)) => {
                        if json {
                            let response = serde_json::json!({
                                "lease": renewed_lease,
                                "warnings": warnings,
                                "message": format!("Renewed lease {} by {} seconds", lease_id, extension),
                            });
                            let output = JsonOutput::success(response, "claim renew");
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("✓ Renewed lease: {}", lease_id);
                            println!("  Issue: {}", renewed_lease.issue_id);
                            println!("  Extended by: {} seconds", extension);
                            if let Some(expires_at) = renewed_lease.expires_at {
                                println!("  New expiry: {}", expires_at.to_rfc3339());
                            }
                            let output_ctx = OutputContext::new(quiet, json);
                            for warning in &warnings {
                                output_ctx.print_warning(warning)?;
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error =
                                claim_json_error(&e, "CLAIM_RENEW_ERROR", "claim renew");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            ClaimCommands::Heartbeat { lease_id, json } => {
                use jit::commands::claim::execute_claim_heartbeat;
                use jit::output::{JsonOutput, OutputContext};

                match execute_claim_heartbeat(&lease_id) {
                    Ok(warnings) => {
                        if json {
                            let response = serde_json::json!({
                                "lease_id": lease_id,
                                "warnings": warnings,
                                "message": format!("Heartbeat sent for lease {}", lease_id),
                            });
                            let output = JsonOutput::success(response, "claim heartbeat");
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("✓ Heartbeat sent: {}", lease_id);
                            let output_ctx = OutputContext::new(quiet, json);
                            for warning in &warnings {
                                output_ctx.print_warning(warning)?;
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error =
                                claim_json_error(&e, "CLAIM_HEARTBEAT_ERROR", "claim heartbeat");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            ClaimCommands::Status { issue, agent, json } => {
                use jit::commands::claim::execute_claim_status;
                use jit::output::{JsonOutput, OutputContext};

                match execute_claim_status::<jit::JsonFileStorage>(
                    issue.as_deref(),
                    agent.as_deref(),
                ) {
                    Ok((leases, warnings)) => {
                        if json {
                            let msg = format!("{} active lease(s)", leases.len());
                            let response = serde_json::json!({
                                "leases": leases,
                                "count": leases.len(),
                                "warnings": warnings,
                            });
                            let output =
                                JsonOutput::success(response, "claim status").with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else if leases.is_empty() {
                            println!("No active leases found.");
                        } else {
                            use chrono::Utc;
                            println!("Active leases ({}):\n", leases.len());
                            for lease in &leases {
                                println!("Lease: {}", lease.lease_id);
                                println!("  Issue:    {}", lease.issue_id);
                                println!("  Agent:    {}", lease.agent_id);
                                println!("  Worktree: {}", lease.worktree_id);
                                if let Some(branch) = &lease.branch {
                                    println!("  Branch:   {}", branch);
                                }
                                println!("  Acquired: {}", lease.acquired_at);

                                if lease.ttl_secs > 0 {
                                    // Finite lease - show expiry and remaining time
                                    if let Some(expires_at) = lease.expires_at {
                                        let now = Utc::now();
                                        let remaining = expires_at.signed_duration_since(now);
                                        println!(
                                            "  Expires:  {} ({} seconds remaining)",
                                            expires_at,
                                            remaining.num_seconds().max(0)
                                        );
                                    }
                                } else {
                                    // Indefinite lease - show last beat and time since
                                    let now = Utc::now();
                                    let since_beat = now.signed_duration_since(lease.last_beat);
                                    println!("  TTL:      indefinite");
                                    println!(
                                        "  Last beat: {} ({} seconds ago)",
                                        lease.last_beat,
                                        since_beat.num_seconds()
                                    );

                                    // Show stale status
                                    if lease.stale {
                                        println!(
                                            "  ⚠️  STALE: Lease marked stale (no heartbeat for {} minutes)",
                                            since_beat.num_minutes()
                                        );
                                        println!(
                                            "     Use 'jit claim heartbeat {}' to refresh",
                                            lease.lease_id
                                        );
                                    }
                                }
                                println!();
                            }
                        }
                        if !json {
                            let output_ctx = OutputContext::new(quiet, json);
                            for warning in &warnings {
                                output_ctx.print_warning(warning)?;
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error =
                                claim_json_error(&e, "CLAIM_STATUS_ERROR", "claim status");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            ClaimCommands::List { json } => {
                use jit::commands::claim::execute_claim_list;
                use jit::output::{JsonOutput, OutputContext};

                match execute_claim_list() {
                    Ok((leases, warnings)) => {
                        if json {
                            let msg = format!("{} lease(s) found", leases.len());
                            let response = serde_json::json!({
                                "leases": leases,
                                "count": leases.len(),
                                "warnings": warnings,
                            });
                            let output =
                                JsonOutput::success(response, "claim list").with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else if leases.is_empty() {
                            println!("No active leases.");
                        } else {
                            use chrono::Utc;
                            println!("All active leases ({}):\n", leases.len());
                            for lease in &leases {
                                println!("Lease: {}", lease.lease_id);
                                println!("  Issue:    {}", lease.issue_id);
                                println!("  Agent:    {}", lease.agent_id);
                                println!("  Worktree: {}", lease.worktree_id);
                                if let Some(branch) = &lease.branch {
                                    println!("  Branch:   {}", branch);
                                }
                                println!("  Acquired: {}", lease.acquired_at);

                                if lease.ttl_secs > 0 {
                                    // Finite lease
                                    if let Some(expires_at) = lease.expires_at {
                                        let now = Utc::now();
                                        let remaining = expires_at.signed_duration_since(now);
                                        println!(
                                            "  Expires:  {} ({} seconds remaining)",
                                            expires_at,
                                            remaining.num_seconds().max(0)
                                        );
                                    }
                                } else {
                                    // Indefinite lease
                                    let now = Utc::now();
                                    let since_beat = now.signed_duration_since(lease.last_beat);
                                    println!("  TTL:      indefinite");
                                    println!(
                                        "  Last beat: {} ({} seconds ago)",
                                        lease.last_beat,
                                        since_beat.num_seconds()
                                    );
                                }
                                println!();
                            }
                        }
                        if !json {
                            let output_ctx = OutputContext::new(quiet, json);
                            for warning in &warnings {
                                output_ctx.print_warning(warning)?;
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error = claim_json_error(&e, "CLAIM_LIST_ERROR", "claim list");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            ClaimCommands::ForceEvict {
                lease_id,
                reason,
                json,
            } => {
                use jit::commands::claim::execute_claim_force_evict;
                use jit::output::{JsonOutput, OutputContext};

                match execute_claim_force_evict::<jit::JsonFileStorage>(&lease_id, &reason) {
                    Ok(warnings) => {
                        if json {
                            let response = serde_json::json!({
                                "lease_id": lease_id,
                                "reason": reason,
                                "warnings": warnings,
                                "message": format!("Force-evicted lease {}", lease_id),
                            });
                            let output = JsonOutput::success(response, "claim force-evict");
                            println!("{}", output.to_json_string()?);
                        } else {
                            println!("✓ Force-evicted lease: {}", lease_id);
                            println!("  Reason: {}", reason);
                            let output_ctx = OutputContext::new(quiet, json);
                            for warning in &warnings {
                                output_ctx.print_warning(warning)?;
                            }
                        }
                    }
                    Err(e) => {
                        if json {
                            let json_error = claim_json_error(
                                &e,
                                "CLAIM_FORCE_EVICT_ERROR",
                                "claim force-evict",
                            );
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
        },
        Commands::Worktree(worktree_cmd) => match worktree_cmd {
            jit::cli::WorktreeCommands::Info { json } => {
                use jit::commands::worktree::execute_worktree_info;
                use jit::output::{JsonError, JsonOutput, OutputContext};

                match execute_worktree_info() {
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
                            let output = JsonOutput::success(response, "worktree info")
                                .with_message(format!(
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
                            let json_error = JsonError::new(
                                "WORKTREE_INFO_ERROR",
                                e.to_string(),
                                "worktree info",
                            );
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            jit::cli::WorktreeCommands::List { json } => {
                use jit::commands::worktree::execute_worktree_list;
                use jit::output::{JsonError, JsonOutput, OutputContext, WorktreeListResponse};

                match execute_worktree_list() {
                    Ok((worktrees, warnings)) => {
                        if json {
                            let count = worktrees.len();
                            let response = WorktreeListResponse { count, worktrees };
                            let mut value = serde_json::to_value(&response)?;
                            if let serde_json::Value::Object(map) = &mut value {
                                map.insert(
                                    "warnings".to_string(),
                                    serde_json::to_value(&warnings)?,
                                );
                            }
                            let output = JsonOutput::success(value, "worktree list")
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
                            let json_error = JsonError::new(
                                "WORKTREE_LIST_ERROR",
                                e.to_string(),
                                "worktree list",
                            );
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
                json,
            } => {
                use jit::commands::snapshot::SnapshotExporter;
                use jit::snapshot::{SnapshotFormat, SnapshotScope};

                // Parse scope
                let snapshot_scope = SnapshotScope::parse(&scope)
                    .with_context(|| format!("Invalid scope: {}", scope))?;

                // Parse format
                let snapshot_format = SnapshotFormat::parse(&format)
                    .with_context(|| format!("Invalid format: {}", format))?;

                // Determine source mode
                let source_mode = SnapshotExporter::<jit::JsonFileStorage>::determine_source_mode(
                    at.as_deref(),
                    working_tree,
                    committed_only,
                )?;

                // TODO: Add validation unless --force
                if !force {
                    executor.validate_silent()?;
                }

                // Create exporter and export
                let exporter = SnapshotExporter::new(storage);
                let (result, warnings) = exporter.export(
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

                    let output = JsonOutput::success(&result, "snapshot export").with_message(
                        format!("Exported {} issues to {}", result.issue_count, result.path),
                    );
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("✓ Snapshot exported to: {}", result.path);
                    println!("  {} issues", result.issue_count);
                    println!("  {} documents", result.document_count);
                    if let Some(size) = result.size_bytes {
                        println!("  Archive: {} bytes", size);
                    }
                }
            }
        },
    }

    Ok(())
}
