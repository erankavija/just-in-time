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
    ArchiveCommands, ClaimCommands, Cli, Commands, DepCommands, DocCommands, EventCommands,
    GateCommands, GraphCommands, InvariantCommands, IssueCommands, ItemCommands, MigrateCommands,
    ProfileCommands, ProjectCommands,
};
use jit::commands::{CommandExecutor, DescriptionUpdate};
use jit::domain::{GateRunResult, Priority, State};
use jit::output::{ExitCode, JsonOutput, OutputContext};
use jit::storage::{IssueStore, JsonFileStorage};
use std::env;
use std::str::FromStr;

/// Helper to determine exit code from error message
fn error_to_exit_code(error: &anyhow::Error) -> ExitCode {
    // A rejected `jit dep add` batch (jit:c8518f2a) wraps every rejected edge's
    // own typed error; classify by the batch's dominant edge rather than this
    // wrapper itself, so a mixed batch still maps to the right exit code.
    if let Some(batch) = error.downcast_ref::<jit::errors::DependencyBatchRejectedError>() {
        return dependency_batch_exit_code(batch);
    }
    // A `gate evaluate` checker that ran but did not pass: split checker-failure
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
    // A manual gate evaluated without --by (jit:1d59070d REQ-03): a usage
    // error, raised before any write, in the same family as gate define's
    // manual+checker-command conflict.
    if error
        .downcast_ref::<jit::commands::ManualGateAttestationRequiredError>()
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
        || error
            .downcast_ref::<jit::errors::RedundantDependencyError>()
            .is_some()
        || error
            .downcast_ref::<jit::validation::projection::ProjectionError>()
            .is_some()
        || error
            .downcast_ref::<jit::profile::ProfilePlanError>()
            .is_some()
        || error
            .downcast_ref::<jit::commands::ProfileApplyError>()
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

    // A repository whose on-disk format is newer than this binary supports is an
    // external-dependency failure (exit 10): the binary, not the repository, is
    // out of date, and the fix is upgrading jit — the same family as the git and
    // filesystem dependency failures, and deliberately NOT NotFound (3), which
    // would misread as "repository/resource missing".
    if error
        .downcast_ref::<jit::storage::RepositoryFormatTooNewError>()
        .is_some()
    {
        return ExitCode::ExternalError;
    }

    // A gate checker refused to run because the binary predates the tree
    // under review (jit:7446af34): the same "the binary, not the repository,
    // needs attention" family as the two checks above — external-dependency
    // failure (exit 10).
    if error
        .downcast_ref::<jit::errors::StaleBinaryError>()
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
    // An ambiguous or too-short id prefix is an argument error (exit 2): the
    // caller gave an id that does not uniquely (or legally) resolve. Classified
    // by downcast against the typed storage errors, never by message text.
    if error
        .downcast_ref::<jit::storage::AmbiguousIdError>()
        .is_some()
        || error
            .downcast_ref::<jit::storage::InvalidIdPrefixError>()
            .is_some()
    {
        return ExitCode::InvalidArgument;
    }
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

    // A `jit issue delete` refused for missing operator confirmation
    // (jit:0daba57d) is an argument/usage error (exit 2), not the generic
    // fallback: the caller omitted the required `JIT_ALLOW_DELETION=1`
    // confirmation, the same family as a malformed or missing required value.
    if error
        .downcast_ref::<jit::errors::DeletionNotConfirmedError>()
        .is_some()
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

/// Pick the dominant exit code across every rejected edge of a `jit dep add`
/// batch (jit:c8518f2a). Resolution failures (bad/ambiguous id prefix, not
/// found — exit 2/3) always win over graph-validation failures (cycle, or a
/// rejected redundant edge — exit 4): resolution runs before graph
/// validation, so a resolution failure blocks that edge from ever reaching
/// graph validation and takes precedence when a batch mixes both classes.
/// Ties within the winning class are broken by request order (the first
/// offending edge wins). Each rejected edge is classified by recursing
/// through [`error_to_exit_code`], so this stays in lockstep with every other
/// typed classification above.
fn dependency_batch_exit_code(batch: &jit::errors::DependencyBatchRejectedError) -> ExitCode {
    batch
        .rejected()
        .iter()
        .map(|(_, e)| error_to_exit_code(e))
        .find(|code| *code != ExitCode::ValidationFailed)
        .unwrap_or(ExitCode::ValidationFailed)
}

/// Build the `--json` error envelope for a rejected `jit dep add` batch
/// (jit:c8518f2a). Every rejected edge is named under `details.rejected`
/// (REQ-02), each carrying its own classified `code` — the same per-edge
/// classification the single-edge path used before the batch was made atomic
/// (jit:a05b87ae). The top-level `code`/`message` mirror the batch's dominant
/// edge, matching [`dependency_batch_exit_code`]'s tiering exactly (both walk
/// the same rejected list and stop at the first non-`ValidationFailed`
/// classification), so the JSON and non-JSON paths always agree on exit code.
fn dep_add_batch_json_error(
    batch: &jit::errors::DependencyBatchRejectedError,
) -> jit::output::JsonError {
    use jit::output::{ErrorCode, JsonError};
    use jit::GraphError;

    let from_id = batch.from_id();

    // Classify every rejected edge (for the `details.rejected` array): the
    // SAME per-edge classification the single-edge path used before the
    // batch was made atomic (jit:a05b87ae).
    let classify = |err: &anyhow::Error| -> &'static str {
        if matches!(
            err.downcast_ref::<GraphError>(),
            Some(GraphError::CycleDetected)
        ) {
            ErrorCode::CYCLE_DETECTED
        } else if err
            .downcast_ref::<jit::errors::RedundantDependencyError>()
            .is_some()
        {
            ErrorCode::VALIDATION_FAILED
        } else if err
            .downcast_ref::<jit::storage::IssueNotFoundError>()
            .is_some()
            || matches!(
                err.downcast_ref::<GraphError>(),
                Some(GraphError::NodeNotFound { .. })
            )
        {
            ErrorCode::ISSUE_NOT_FOUND
        } else if err
            .downcast_ref::<jit::storage::InvalidIdPrefixError>()
            .is_some()
        {
            ErrorCode::INVALID_ID_PREFIX
        } else if err
            .downcast_ref::<jit::storage::AmbiguousIdError>()
            .is_some()
        {
            ErrorCode::AMBIGUOUS_ID
        } else {
            "DEPENDENCY_ERROR"
        }
    };
    let edge_message = |to: &str, err: &anyhow::Error| -> String {
        // The nicer templated cycle message names both ends of the edge;
        // every other kind's `Display` already names what's needed.
        if matches!(
            err.downcast_ref::<GraphError>(),
            Some(GraphError::CycleDetected)
        ) {
            format!("Adding dependency would create a cycle: {from_id} -> {to}")
        } else {
            err.to_string()
        }
    };

    let rejected_details: Vec<serde_json::Value> = batch
        .rejected()
        .iter()
        .map(|(to, err)| {
            serde_json::json!({
                "from": from_id,
                "to": to,
                "code": classify(err),
                "message": edge_message(to, err),
            })
        })
        .collect();

    // The dominant edge — the same tiering `dependency_batch_exit_code` uses
    // (a resolution failure always wins over a graph-validation failure) —
    // drives the top-level `code`/`message`/`suggestions`. Built via the SAME
    // per-kind constructor the single-edge path used (`cycle_detected`,
    // `issue_not_found`, `refine_id_error`), so this batch wrapper preserves
    // their suggestions and kind-specific detail fields; the batch-wide
    // per-edge listing is then merged in alongside them, not in place of them.
    let dominant_index = batch
        .rejected()
        .iter()
        .position(|(_, err)| error_to_exit_code(err) != ExitCode::ValidationFailed)
        .unwrap_or(0);
    let (to, err) = &batch.rejected()[dominant_index];

    let mut json_error = if matches!(
        err.downcast_ref::<GraphError>(),
        Some(GraphError::CycleDetected)
    ) {
        JsonError::cycle_detected(from_id, to, "dep add")
    } else if err
        .downcast_ref::<jit::errors::RedundantDependencyError>()
        .is_some()
    {
        JsonError::new(ErrorCode::VALIDATION_FAILED, err.to_string(), "dep add")
    } else if err
        .downcast_ref::<jit::storage::IssueNotFoundError>()
        .is_some()
        || matches!(
            err.downcast_ref::<GraphError>(),
            Some(GraphError::NodeNotFound { .. })
        )
    {
        JsonError::issue_not_found(to, "dep add")
    } else {
        jit::output::refine_id_error(
            err,
            JsonError::new("DEPENDENCY_ERROR", err.to_string(), "dep add"),
        )
    };

    let mut details = json_error
        .error
        .details
        .take()
        .unwrap_or_else(|| serde_json::json!({}));
    if let serde_json::Value::Object(ref mut map) = details {
        map.insert("from_id".to_string(), serde_json::json!(from_id));
        map.insert(
            "rejected".to_string(),
            serde_json::Value::Array(rejected_details),
        );
    }
    json_error.error.details = Some(details);
    json_error
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

/// Render a failed `gate evaluate` / `gate evaluate-all` outcome and terminate appropriately.
///
/// Shared by the `gate evaluate` and `gate evaluate-all` handlers so both classify the
/// same error the same way. In `--json` mode it prints a structured
/// [`JsonError`](jit::output::JsonError) and exits with its mapped code:
/// `GatePassFailed` becomes a checker failure (`GATE_FAILED`, exit 4, verdict
/// `fail`) or a runner error (`IO_ERROR`, exit 10, verdict `error`) per the
/// carried [`GateRunStatus`](jit::domain::GateRunStatus); `GateNotRequiredError`
/// becomes `INVALID_ARGUMENT` (exit 2); an unresolved id becomes
/// `ISSUE_NOT_FOUND` (exit 3); [`StaleBinaryError`](jit::errors::StaleBinaryError)
/// becomes `STALE_BINARY` (exit 10) — a PRE-verdict refusal, so, like
/// `GateNotRequiredError`, it carries no `verdict` field and no gate run was
/// ever recorded; anything else `GATE_ERROR`. In non-JSON mode it surfaces any
/// gate-failure warnings and returns `Err(e)` so the top-level handler maps the
/// exit code via [`error_to_exit_code`] — which classifies `StaleBinaryError`
/// to the same `ExitCode::ExternalError`, so both output modes agree.
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

    use jit::output::{GateRunSummary, JsonError};
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
                "key": gate_failure.gate_key,
                "status": "failed",
                "verdict": verdict,
                "checker_result": GateRunSummary::full(&gate_failure.result),
                "warnings": gate_failure.warnings,
            }))
            .with_suggestion(format!(
                "Inspect the checker result with: jit gate status {} {}",
                gate_failure.issue_id, gate_failure.gate_key
            ))
            .with_suggestion(format!(
                "View run history with: jit gate status {} {} --all",
                gate_failure.issue_id, gate_failure.gate_key
            ))
            .with_suggestion(format!(
                "Fix the failing gate and rerun: jit gate evaluate {} {}",
                gate_failure.issue_id, gate_failure.gate_key
            ))
    } else if let Some(not_required) = e.downcast_ref::<jit::commands::GateNotRequiredError>() {
        // Pre-verdict argument error: not a gate verdict, so it carries no
        // `verdict` field.
        JsonError::new("INVALID_ARGUMENT", e.to_string(), command)
            .with_details(serde_json::json!({
                "issue_id": not_required.issue_id,
                "key": not_required.gate_key,
            }))
            .with_suggestion(format!(
                "Add the gate first: jit gate add {} {}",
                not_required.issue_id, not_required.gate_key
            ))
    } else if let Some(needs_attestor) =
        e.downcast_ref::<jit::commands::ManualGateAttestationRequiredError>()
    {
        // Pre-verdict argument error (jit:1d59070d REQ-03): a manual gate has
        // no checker to run, so a bare evaluate would silently record an
        // unattributed pass. No write happened, so — like `GateNotRequiredError`
        // above — this carries no `verdict` field.
        JsonError::new("INVALID_ARGUMENT", e.to_string(), command)
            .with_details(serde_json::json!({
                "issue_id": needs_attestor.issue_id,
                "key": needs_attestor.gate_key,
            }))
            .with_suggestion(format!(
                "Record the pass with: jit gate evaluate {} {} --by <attestor>",
                needs_attestor.issue_id, needs_attestor.gate_key
            ))
    } else if e
        .downcast_ref::<jit::storage::IssueNotFoundError>()
        .is_some()
    {
        // Pre-verdict lookup error: issue id did not resolve.
        JsonError::issue_not_found(id, command)
    } else if let Some(stale) = e.downcast_ref::<jit::errors::StaleBinaryError>() {
        // Pre-verdict refusal (jit:7446af34): the checker never spawned, so —
        // like `GateNotRequiredError` above, and unlike `GatePassFailed` — this
        // carries no `verdict` field. `STALE_BINARY` maps to exit code 10
        // (`ErrorCode::to_exit_code`), matching the non-JSON path's
        // `ExitCode::ExternalError` classification of the same typed error in
        // `error_to_exit_code`.
        stale_binary_json_error(stale, command)
    } else {
        JsonError::new("GATE_ERROR", e.to_string(), command)
    };
    println!("{}", json_error.to_json_string()?);
    std::process::exit(json_error.exit_code().code());
}

/// Build the `--json` error envelope for a stale-binary refusal
/// ([`StaleBinaryError`](jit::errors::StaleBinaryError), jit:7446af34).
///
/// Shared by [`render_gate_pass_error`] (the evaluator's own refusal, REQ-01)
/// and [`emit_startup_json_error`] (a checker-spawned `jit` child's own
/// self-refusal, REQ-02), so both carry the identical `STALE_BINARY` code,
/// `details` (issue id, gate key, reason, build commit), and reinstall
/// suggestion — one envelope shape regardless of which process in the gate
/// run detected the staleness.
fn stale_binary_json_error(
    stale: &jit::errors::StaleBinaryError,
    command: &str,
) -> jit::output::JsonError {
    use jit::domain::build_provenance::StaleBinaryReason;
    use jit::output::{ErrorCode, JsonError};

    let (reason_code, built_from) = match stale.reason() {
        StaleBinaryReason::CommitMismatch { built_from, .. } => {
            ("commit_mismatch", built_from.clone())
        }
        StaleBinaryReason::DirtyBuild { built_from } => ("dirty_build", built_from.clone()),
    };
    JsonError::new(ErrorCode::STALE_BINARY, stale.to_string(), command)
        .with_details(serde_json::json!({
            "issue_id": stale.issue_id(),
            "key": stale.gate_key(),
            "reason": reason_code,
            "built_from": built_from,
        }))
        .with_suggestion(
            "Rebuild and reinstall with build provenance: scripts/install-jit.sh \
             (wraps cargo install --path crates/jit)",
        )
        .with_suggestion(format!(
            "Verify with: jit --version (should show commit {built_from})"
        ))
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
        // Load each created node so the JSON consumer gets the full stored
        // records (gate list under the storage names) alongside the role→id map.
        // Routed through the typed response so the schema derives from the same
        // `Issue` the emission serializes (jit:f40f1b0a).
        let created_issues: std::collections::BTreeMap<String, jit::domain::Issue> = result
            .created_node_ids_by_role
            .iter()
            .map(|(role, id)| Ok((role.clone(), storage.load_issue(id)?)))
            .collect::<Result<_>>()?;
        let response = jit::output::TemplateApplyResponse {
            template: result.template.clone(),
            container: container.to_string(),
            anchor_bindings: result.anchor_bindings.clone(),
            created_node_ids_by_role: result.created_node_ids_by_role.clone(),
            anchor_dependency_snapshots: result.anchor_dependency_snapshots.clone(),
            created_issues,
        };
        let msg = format!("Applied template '{}' to {}", result.template, container);
        let output = JsonOutput::success(response, "apply").with_message(msg);
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

/// Build an `INVALID_ARGUMENT` error, rendering it `--json`-aware.
///
/// Mirrors [`resolve_gate_key_for`]: a `--json` caller emits a machine-readable
/// `INVALID_ARGUMENT` JSON error (exit 2) and the process exits; a non-`--json`
/// caller receives the typed [`InvalidArgumentError`](jit::errors::InvalidArgumentError)
/// so the top-level handler classifies it (exit 2) and prints the plain message.
/// Never returns when `json` is true.
fn invalid_argument(message: String, command: &str, json: bool) -> anyhow::Error {
    if json {
        let json_error =
            jit::output::JsonError::new(jit::output::ErrorCode::INVALID_ARGUMENT, message, command);
        if let Ok(s) = json_error.to_json_string() {
            println!("{}", s);
        }
        std::process::exit(json_error.exit_code().code());
    }
    jit::errors::InvalidArgumentError::new(message).into()
}

fn profile_json_error(error: &anyhow::Error, command: &str) -> jit::output::JsonError {
    use jit::output::{ErrorCode, JsonError};

    if error.downcast_ref::<jit::errors::NotFoundError>().is_some() {
        JsonError::new(ErrorCode::PROFILE_NOT_FOUND, error.to_string(), command)
            .with_suggestion("Run 'jit profile list --json' to see embedded profiles")
    } else if error
        .downcast_ref::<jit::profile::ProfilePlanError>()
        .is_some()
        || error
            .downcast_ref::<jit::commands::ProfileApplyError>()
            .is_some()
    {
        JsonError::new(ErrorCode::PROFILE_CONFLICT, error.to_string(), command)
    } else {
        JsonError::new("PROFILE_ERROR", error.to_string(), command)
    }
}

fn profile_result<T>(result: anyhow::Result<T>, command: &str, json: bool) -> anyhow::Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(error) if json => {
            let json_error = profile_json_error(&error, command);
            println!("{}", json_error.to_json_string()?);
            std::process::exit(json_error.exit_code().code());
        }
        Err(error) => Err(error),
    }
}

/// Wrong-verb guess -> canonical-command hint, keyed by (group, guessed verb).
/// Backs the hidden stub subcommands in `cli.rs` (`IssueCommands::Rm`,
/// `DepCommands::Remove`, ...): each stub always fails through
/// [`verb_hint_error`], which looks itself up here. Adding a newly observed
/// wrong guess is a one-line addition: wire a hidden stub variant in
/// `cli.rs`, add its match arm below, and add the mapping here. See
/// docs/reference/cli-commands.md's "Command and flag aliases" section for
/// the user-facing writeup — these are hints, not aliases: the wrong verb
/// still fails, it just names the right one.
const VERB_HINTS: &[(&str, &str, &str)] = &[
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
];

/// Fail fast on an observed wrong-verb guess (`jit <group> <verb>`) with a
/// hint naming the canonical command, instead of executing (there is nothing
/// to execute — these are hidden stubs) or falling through to clap's generic
/// "unrecognized subcommand" error. `args` is the wrong subcommand's raw
/// trailing argv (positionals and flags alike, captured verbatim by the
/// stub); it is inspected only for a literal `--json` so the hint renders
/// through the same JSON error envelope the real command would have used.
/// Always returns an error; never used as a fallible operation.
fn verb_hint_error(group: &str, verb: &str, args: &[String]) -> anyhow::Error {
    let json = args.iter().any(|a| a == "--json");
    let canonical = VERB_HINTS
        .iter()
        .find_map(|(g, v, hint)| (*g == group && *v == verb).then_some(*hint))
        .unwrap_or_else(|| panic!("no verb hint registered for 'jit {group} {verb}'"));
    let command = format!("{group} {verb}");
    invalid_argument(
        format!("'jit {command}' is not a jit command. Use '{canonical}' instead."),
        &command,
        json,
    )
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
/// Unlike [`resolve_gate_key`], neither form is required: `Ok(None)` is a valid
/// result (used by the history view, where the gate key is only a filter).
/// Supplying BOTH is still an error, routed `--json`-aware via
/// [`invalid_argument`].
fn resolve_optional_gate_key(
    positional: Option<String>,
    flag: Option<String>,
    command: &str,
    json: bool,
) -> Result<Option<String>> {
    match (positional, flag) {
        (None, None) => Ok(None),
        (Some(pos), None) => Ok(Some(pos)),
        (None, Some(flag_val)) => Ok(Some(flag_val)),
        (Some(_), Some(_)) => Err(invalid_argument(
            "provide the gate key as a positional argument OR via --gate, not both.".to_string(),
            command,
            json,
        )),
    }
}

/// Parse a `--status` filter value into a [`GateRunStatus`].
///
/// Accepts the snake_case status names; an unknown value is an
/// `INVALID_ARGUMENT` error (routed `--json`-aware).
fn parse_run_status(value: &str, command: &str, json: bool) -> Result<jit::domain::GateRunStatus> {
    use jit::domain::GateRunStatus;
    match value.to_ascii_lowercase().as_str() {
        "passed" => Ok(GateRunStatus::Passed),
        "failed" => Ok(GateRunStatus::Failed),
        "error" => Ok(GateRunStatus::Error),
        "pending" => Ok(GateRunStatus::Pending),
        "skipped" => Ok(GateRunStatus::Skipped),
        other => Err(invalid_argument(
            format!(
                "unknown --status value '{other}'; expected one of: \
                 passed, failed, error, pending, skipped."
            ),
            command,
            json,
        )),
    }
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

#[cfg(test)]
mod gate_findings_text_tests {
    use super::render_gate_findings_text;
    use chrono::Utc;
    use jit::domain::{
        GateFinding, GateFindings, GateRunResult, GateRunStatus, GateStage, GATE_RUN_SCHEMA_VERSION,
    };

    fn run_with(references: Vec<String>) -> GateRunResult {
        GateRunResult {
            schema_version: GATE_RUN_SCHEMA_VERSION,
            run_id: "run-1".to_string(),
            gate_key: "review".to_string(),
            stage: GateStage::Postcheck,
            issue_id: "issue-1".to_string(),
            commit: None,
            branch: None,
            tree_dirty: None,
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
    if let Some(dirty) = result.tree_dirty {
        println!("  Tree: {}", if dirty { "dirty" } else { "clean" });
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
            // A project-scoped item (`@/<kind>/<self-id>`) has no owning issue.
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
    let InvariantCommands::Check { json } = command;

    let result = run_invariant_inner(executor, InvariantCommands::Check { json }, quiet);
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

/// Run `jit project <subcommand>`.
///
/// A thin delegation over [`CommandExecutor::project_render`]: `render` writes
/// every declared `[projection.*]` (or a single `--name`d one) into its configured
/// documentation target and reports what was written. On `--json` a failure is
/// rendered as a JSON error object (honoring the machine-readable contract) rather
/// than the top-level plain `Error: ...`.
fn run_project<S: IssueStore>(
    executor: &CommandExecutor<S>,
    command: ProjectCommands,
    quiet: bool,
) -> Result<()> {
    let json = match &command {
        ProjectCommands::Render { json, .. } => *json,
    };

    let result = run_project_inner(executor, command, quiet);
    if let Err(e) = result {
        handle_json_error!(
            json,
            e,
            jit::output::JsonError::new("PROJECT_COMMAND_FAILED", e.to_string(), "project")
        );
    }
    Ok(())
}

/// Inner dispatch for `jit project`; errors are converted to JSON by
/// [`run_project`] when `--json` is set.
fn run_project_inner<S: IssueStore>(
    executor: &CommandExecutor<S>,
    command: ProjectCommands,
    quiet: bool,
) -> Result<()> {
    match command {
        ProjectCommands::Render { name, json } => {
            let result = executor.project_render(name.as_deref())?;
            let output_ctx = OutputContext::new(quiet, json);
            if json {
                let msg = format!("Rendered {} projection(s)", result.count);
                let output = JsonOutput::success(&result, "project render").with_message(msg);
                println!("{}", output.to_json_string()?);
            } else if result.projections.is_empty() {
                output_ctx.print_data("No projections declared".to_string())?;
            } else {
                for projection in &result.projections {
                    output_ctx.print_data(format!(
                        "Rendered projection '{}' to {} ({} mode, {} style)",
                        projection.name, projection.target, projection.mode, projection.style
                    ))?;
                }
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
    label: Vec<String>,
    full: bool,
    json: bool,
) -> Result<()> {
    let output_ctx = OutputContext::new(quiet, json);
    let state_filter = state.as_ref().map(|s| State::from_str(s)).transpose()?;
    let priority_filter = priority
        .as_ref()
        .map(|p| Priority::from_str(p))
        .transpose()?;
    let issues = executor.query_all(state_filter, assignee.as_deref(), priority_filter, &label)?;

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
            let response = IssueListFullResponse {
                count: issues.len(),
                issues,
            };
            JsonOutput::success(serde_json::to_value(response)?, "query all")
        } else {
            let minimal: Vec<MinimalIssue> = issues.iter().map(MinimalIssue::from).collect();
            let response = IssueListResponse {
                count: minimal.len(),
                issues: minimal,
            };
            JsonOutput::success(serde_json::to_value(response)?, "query all")
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
        QueryCommands::Count { .. } => "count",
        QueryCommands::Divergence { .. } => "divergence",
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
        "query",
        json,
    ))
}

fn main() {
    let exit_code = match run() {
        Ok(()) => ExitCode::Success,
        Err(e) => {
            eprintln!("Error: {}", e);
            emit_startup_json_error(&e);
            error_to_exit_code(&e)
        }
    };

    if exit_code != ExitCode::Success {
        std::process::exit(exit_code.code());
    }
}

/// Under `--json`, emit a structured error envelope on stdout for the startup
/// failures that abort before any command handler runs: repository-not-found
/// (exit 3), repository-format-too-new (exit 10), and — when this process is
/// itself a child spawned inside a gate checker's process tree (jit:7446af34
/// REQ-02) — a stale-binary self-refusal (exit 10).
///
/// The human-readable line always goes to stderr (via `main`) and the exit code
/// is unchanged; this only ADDS the machine-readable object so `--json` callers
/// can branch on the error class instead of parsing an empty stdout. The `--json`
/// flag is read from argv because these failures occur during repository
/// discovery/validation, before a parsed command-level `json` field exists. A
/// no-op unless the error is one of these startup conditions AND `--json` was
/// requested, so command handlers (which already render their own JSON) never
/// double-print.
fn emit_startup_json_error(error: &anyhow::Error) {
    if !std::env::args().any(|arg| arg == "--json") {
        return;
    }

    use jit::output::{ErrorCode, JsonError};
    let json_error = if error
        .downcast_ref::<jit::storage::RepositoryNotFoundError>()
        .is_some()
    {
        JsonError::new(ErrorCode::REPOSITORY_NOT_FOUND, error.to_string(), "")
    } else if error
        .downcast_ref::<jit::storage::RepositoryFormatTooNewError>()
        .is_some()
    {
        JsonError::new(ErrorCode::REPOSITORY_FORMAT_TOO_NEW, error.to_string(), "")
    } else if let Some(stale) = error.downcast_ref::<jit::errors::StaleBinaryError>() {
        stale_binary_json_error(stale, "")
    } else {
        return;
    };

    if let Ok(rendered) = json_error.to_json_string() {
        println!("{}", rendered);
    }
}

/// REQ-02 (jit:7446af34): self-check this process's own build provenance
/// before running ANY command, when it is itself running inside a gate
/// checker's process tree.
///
/// `JIT_GATE_RUN` is set on every gate checker's environment
/// ([`gate_execution::execute_gate_checker_with_context`](jit::gate_execution::execute_gate_checker_with_context))
/// and inherited by anything the checker spawns — including a checker SCRIPT
/// that itself shells out to `jit` (e.g. `scripts/jit-validate.sh`'s `exec
/// jit validate "$@"`), which resolves `jit` from `PATH` independently of the
/// evaluator process. The evaluator's own guard
/// ([`check_gate`](jit::commands::CommandExecutor::check_gate), REQ-01) only
/// covers the evaluator's own binary, so without this, a stale PATH `jit`
/// inside the checker's process tree could still silently produce the
/// checker's exit code and output — the incident that motivated this feature
/// (jit:7446af34) — and the evaluator would faithfully persist it as a gate
/// run. This makes any such staleness visible in the run record instead: the
/// child refuses (exit `10`) rather than running its command, so the
/// checker's own exit code and stderr — captured into the persisted
/// [`GateRunResult`](jit::domain::GateRunResult) — carry the refusal.
///
/// A no-op when `JIT_GATE_RUN` is absent (an ordinary, non-gate-context
/// invocation is completely unaffected), and silent under the identical
/// REQ-03 identity predicate the evaluator-side check uses (outside git, an
/// unrelated repository, or an unknown build commit never refuses). Reads
/// `JIT_ISSUE_ID`/`JIT_GATE_KEY` (set alongside `JIT_GATE_RUN`) to label the
/// refusal the same way the evaluator's own guard does.
fn refuse_if_stale_gate_child(executor: &CommandExecutor<JsonFileStorage>) -> Result<()> {
    if env::var_os("JIT_GATE_RUN").is_none() {
        return Ok(());
    }
    if let Some(reason) = executor.stale_binary_reason() {
        let issue_id = env::var("JIT_ISSUE_ID").unwrap_or_default();
        let gate_key = env::var("JIT_GATE_KEY").unwrap_or_default();
        return Err(jit::errors::StaleBinaryError::new(&issue_id, &gate_key, &reason).into());
    }
    Ok(())
}

/// [`refuse_if_stale_gate_child`] for the pre-dispatch paths: runs before any
/// early return (`--schema`, `version`), so even those outputs are never
/// served from a stale binary inside a gate checker's process tree — a
/// checker script can consume them to inform its verdict just like any other
/// command's output (REQ-02, jit:7446af34). Constructs a discovery-backed
/// executor only when `JIT_GATE_RUN` is present; the ordinary invocation
/// path pays nothing.
fn stale_gate_child_precheck() -> Result<()> {
    if env::var_os("JIT_GATE_RUN").is_none() {
        return Ok(());
    }
    let current_dir = env::current_dir()?;
    let jit_dir = if let Ok(custom_dir) = env::var("JIT_DATA_DIR") {
        current_dir.join(custom_dir)
    } else {
        jit::storage::discovery::discover_jit_dir(&current_dir)
            .unwrap_or_else(|| current_dir.join(".jit"))
    };
    let executor = CommandExecutor::new(JsonFileStorage::new(&jit_dir));
    refuse_if_stale_gate_child(&executor)
}

fn run() -> Result<()> {
    // REQ-02 (jit:7446af34): refuse before ANY output — before Clap even
    // parses (its `--help`/`-V` auto-exits print and terminate inside
    // `Cli::parse`), and ahead of the `--schema` and `version` early returns
    // below — when this process is itself stale and running inside a gate
    // checker's process tree. Nothing observable precedes this guard.
    stale_gate_child_precheck()?;

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
    let requires_recovery_dispatch = command.requires_recovery_dispatch();

    // Determine the jit data directory.
    //
    // `JIT_DATA_DIR` always wins (highest precedence, checked first). Absent an
    // override, `jit init` always targets the current directory — like `git
    // init`, it never searches upward, so initializing inside an existing
    // repository creates a nested `.jit/` rather than reinitializing an
    // ancestor. Every other command discovers the repository root by walking
    // up from the current directory the way git discovers `.git`: stopping at
    // the first ancestor containing `.jit/`, and never crossing a `.git`
    // boundary or the filesystem root. When no `.jit/` is found within that
    // boundary, fall back to `<cwd>/.jit` so the "repository not found" error
    // from `storage.validate()` below names a sensible path and still
    // suggests `jit init`.
    let jit_dir = if let Ok(custom_dir) = env::var("JIT_DATA_DIR") {
        current_dir.join(custom_dir)
    } else if matches!(command, Commands::Init { .. }) {
        current_dir.join(".jit")
    } else if requires_recovery_dispatch {
        jit::storage::discovery::discover_recovery_jit_dir(&current_dir)
            .unwrap_or_else(|| current_dir.join(".jit"))
    } else {
        jit::storage::discovery::discover_jit_dir(&current_dir)
            .unwrap_or_else(|| current_dir.join(".jit"))
    };

    let storage = JsonFileStorage::new(&jit_dir);
    // Recovery is deliberately ahead of repository validation and
    // CommandExecutor construction: both can load state a pending journal is
    // responsible for repairing. Keep the session alive through dispatch so
    // mutating CLI commands retain bootstrap → repository serialization until
    // their last write.
    let recovery_session = requires_recovery_dispatch
        .then(|| jit::storage::RecoveryCoordinator::recover_before_services(&storage))
        .transpose()?;
    let transactions_recovered = recovery_session
        .as_ref()
        .map(|session| session.report().recovered_count())
        .unwrap_or(0);
    if let Some(session) = recovery_session {
        storage.retain_recovery_session(session)?;
    }
    let mut executor = CommandExecutor::new(storage.clone());

    match &command {
        Commands::Init {
            hierarchy_template,
            profile,
            json,
        } => {
            let output_ctx = OutputContext::new(quiet, *json);

            // Resolve the template before init so we can error early on bad names.
            // Routed through the json-aware `invalid_argument` helper (rather than
            // a bare `anyhow!`) so `--json` callers get a machine-readable envelope
            // instead of a silently-empty stdout.
            let template = if let Some(template_name) = hierarchy_template {
                match jit::hierarchy_templates::HierarchyTemplate::get(template_name) {
                    Some(t) => Some(t),
                    None => {
                        return Err(invalid_argument(
                            format!("Unknown hierarchy template: {}", template_name),
                            "init",
                            *json,
                        ));
                    }
                }
            } else {
                None
            };

            let chosen = template
                .as_ref()
                .cloned()
                .unwrap_or_else(jit::hierarchy_templates::HierarchyTemplate::default);
            if let Some(id) = profile.as_deref() {
                profile_result(executor.validate_profile_id(id), "init", *json)?;
            }

            // Snapshot which core repository files already exist so the `--json`
            // envelope can report exactly what THIS run created, rather than the
            // full idempotent set `executor.init()` always ensures.
            let index_existed = jit_dir.join("index.json").exists();
            let gates_existed = jit_dir.join("gates.toml").exists();
            let events_existed = jit_dir.join("events.jsonl").exists();
            let config_existed = jit_dir.join("config.toml").exists();
            let rules_existed = jit_dir.join("rules.toml").exists();
            let fresh = !jit_dir.exists();
            let fresh_result = profile_result(
                if let Some(id) = profile.as_deref() {
                    Some(executor.initialize_profiled_repository(&current_dir, &chosen, id))
                } else {
                    fresh.then(|| executor.initialize_fresh_repository(&current_dir, &chosen, None))
                }
                .transpose(),
                "init",
                *json,
            )?;
            let (worktree_identity, init_warnings) = if fresh || profile.is_some() {
                executor.initialize_worktree_identity()?
            } else {
                executor.init()?
            };
            for warning in &init_warnings {
                output_ctx.print_warning(warning)?;
            }
            if let Some(result) = &fresh_result {
                for warning in &result.warnings {
                    eprintln!("Warning: {warning}");
                }
            }

            // Set up .gitattributes for merge drivers (if in git repo). The
            // git-subprocess detection and file read/append/create live in
            // the storage layer (`jit::storage::gitattributes`); this call
            // site only handles the outcome. A failure here is non-fatal
            // (warning only); `None` means "nothing to report" for the
            // `--json` created/modified path lists below.
            let gitattributes_outcome = match jit::storage::gitattributes::setup_gitattributes() {
                Ok(outcome) => Some(outcome),
                Err(e) => {
                    eprintln!("Warning: Could not set up .gitattributes: {}", e);
                    None
                }
            };

            // Seed the `[project]` identity (REQ-01). The command layer owns the
            // orchestration — existence check, default-name computation, and the
            // store write — and is idempotent, so a re-init leaves an existing
            // `[project]` table untouched.
            let project_name = if let Some(result) = &fresh_result {
                (!config_existed).then(|| result.project_name.clone())
            } else {
                executor.seed_project_config(&current_dir, &chosen.generate_config_toml())?
            };

            // Scaffold .jit/rules.toml (the operative ruleset) with the FIXED
            // default ruleset derived from the repo's namespace registry + type
            // hierarchy. A no-op when rules.toml already exists (re-init
            // never clobbers user edits).
            let scaffolded = if fresh_result.is_some() {
                !rules_existed
            } else {
                executor.scaffold_default_rules()?
            };
            if scaffolded {
                let _ = output_ctx.print_success("Scaffolded .jit/rules.toml");
            }
            let profile_result = if let Some(result) = fresh_result {
                result.profile
            } else {
                None
            };

            let message = if let Some(ref t) = template {
                format!("Initialized with '{}' hierarchy template", t.name)
            } else if let Some(ref identity) = worktree_identity {
                format!(
                    "Initialized jit repository (worktree: {})",
                    identity.worktree_id
                )
            } else {
                "Initialized jit repository".to_string()
            };
            let _ = output_ctx.print_success(&message);

            if *json {
                let mut created_paths = Vec::new();
                if !index_existed {
                    created_paths.push(".jit/index.json".to_string());
                }
                if !gates_existed {
                    created_paths.push(".jit/gates.toml".to_string());
                }
                if !events_existed {
                    created_paths.push(".jit/events.jsonl".to_string());
                }
                if project_name.is_some() {
                    created_paths.push(".jit/config.toml".to_string());
                }
                if scaffolded {
                    created_paths.push(".jit/rules.toml".to_string());
                }
                use jit::storage::gitattributes::GitattributesOutcome;
                if gitattributes_outcome == Some(GitattributesOutcome::Created) {
                    created_paths.push(".gitattributes".to_string());
                }

                let mut modified_paths = Vec::new();
                if gitattributes_outcome == Some(GitattributesOutcome::Modified) {
                    modified_paths.push(".gitattributes".to_string());
                }

                let payload = serde_json::json!({
                    "repository_root": current_dir.display().to_string(),
                    "data_dir": jit_dir.display().to_string(),
                    "repository_id": worktree_identity.map(|identity| identity.worktree_id),
                    "hierarchy_template": chosen.name,
                    "created_paths": created_paths,
                    "modified_paths": modified_paths,
                    "profile": profile_result,
                });
                let output = JsonOutput::success(payload, "init").with_message(message);
                println!("{}", output.to_json_string()?);
            }
        }
        Commands::Recover { .. } if !storage.root().exists() && transactions_recovered > 0 => {
            // A prepared fresh-root transaction may legitimately recover to
            // "no repository". The explicit recovery command still succeeds:
            // its requested work completed before validation became relevant.
        }
        Commands::Profile(ProfileCommands::List { .. } | ProfileCommands::Show { .. }) => {}
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
        Commands::Profile(profile_cmd) => match profile_cmd {
            ProfileCommands::List { json } => match executor.list_embedded_profiles() {
                Ok(result) => {
                    if json {
                        let output = JsonOutput::success(&result, "profile list");
                        println!("{}", output.to_json_string()?);
                    } else {
                        for profile in result.profiles {
                            println!(
                                "{} {} embedded{}",
                                profile.id,
                                profile.version,
                                if profile.applied { " (applied)" } else { "" }
                            );
                        }
                    }
                }
                Err(error) if json => {
                    let json_error = profile_json_error(&error, "profile list");
                    println!("{}", json_error.to_json_string()?);
                    std::process::exit(json_error.exit_code().code());
                }
                Err(error) => return Err(error),
            },
            ProfileCommands::Show { id, json } => match executor.show_embedded_profile(&id) {
                Ok(result) => {
                    if json {
                        let output = JsonOutput::success(&result, "profile show");
                        println!("{}", output.to_json_string()?);
                    } else {
                        let profile = &result.manifest.profile;
                        println!("Profile: {}", profile.id);
                        println!("Version: {}", profile.version);
                        println!("Origin: embedded");
                        println!("Compatible JIT: {}", profile.jit);
                        println!("Package hash: {}", result.package_hash);
                        println!("Files: {} ({} bytes)", result.file_count, result.byte_size);
                        println!("Targets: {}", result.target_hashes.len());
                        println!(
                            "Applied: {}",
                            if result.applied.is_some() {
                                "yes"
                            } else {
                                "no"
                            }
                        );
                    }
                }
                Err(error) if json => {
                    let json_error = profile_json_error(&error, "profile show");
                    println!("{}", json_error.to_json_string()?);
                    std::process::exit(json_error.exit_code().code());
                }
                Err(error) => return Err(error),
            },
            ProfileCommands::Apply { id, dry_run, json } => {
                if dry_run {
                    match executor.plan_embedded_profile(&id) {
                        Ok(plan) => {
                            if json {
                                let output = JsonOutput::success(&plan, "profile apply");
                                println!("{}", output.to_json_string()?);
                            } else {
                                let status = match plan.status {
                                    jit::profile::ProfilePlanStatus::Unchanged => "unchanged",
                                    jit::profile::ProfilePlanStatus::WouldApply => "would apply",
                                };
                                println!("Profile {} {}: {}", plan.id, plan.version, status);
                                for target in plan.targets {
                                    let action = match target.action {
                                        jit::profile::ProfileTargetAction::Unchanged => "unchanged",
                                        jit::profile::ProfileTargetAction::Create => "create",
                                        jit::profile::ProfileTargetAction::Update => "update",
                                    };
                                    println!("  {action}: {}", target.path);
                                }
                            }
                        }
                        Err(error) if json => {
                            let json_error = profile_json_error(&error, "profile apply");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    match executor.apply_profile(&id) {
                        Ok(applied) => {
                            if json {
                                let output = JsonOutput::success(&applied, "profile apply");
                                println!("{}", output.to_json_string()?);
                            } else {
                                let status = match applied.status {
                                    jit::profile::ProfileApplicationStatus::Unchanged => {
                                        "unchanged"
                                    }
                                    jit::profile::ProfileApplicationStatus::Applied => "applied",
                                };
                                println!("Profile {} {}: {}", applied.id, applied.version, status);
                                for warning in applied.warnings {
                                    eprintln!("Warning: {:?}", warning);
                                }
                            }
                        }
                        Err(error) if json => {
                            let json_error = profile_json_error(&error, "profile apply");
                            println!("{}", json_error.to_json_string()?);
                            std::process::exit(json_error.exit_code().code());
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
        },
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
                        // findings (`orphan-leaf` / `strategic-consistency`,
                        // origin = "default") produced by the rule engine, not a
                        // hard-coded check. `--orphan` suppresses the orphan-leaf
                        // hint (acknowledged intentional orphan).
                        if !force && !quiet {
                            let issues = storage.list_issues()?;
                            let graph_findings = executor.evaluate_graph_rules(&issues)?;
                            for gf in graph_findings.iter().filter(|gf| {
                                gf.issue_id.as_deref() == Some(id.as_str())
                                    && !(orphan && gf.finding.rule == "orphan-leaf")
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
                        use jit::output::{
                            IssueSearchFullResponse, IssueSearchResponse, JsonOutput,
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
                            serde_json::to_value(IssueSearchFullResponse {
                                query: query.clone(),
                                issues,
                                count,
                            })?
                        } else {
                            use jit::domain::MinimalIssue;
                            let minimal_issues: Vec<MinimalIssue> =
                                issues.iter().map(MinimalIssue::from).collect();
                            serde_json::to_value(IssueSearchResponse {
                                query: query.clone(),
                                issues: minimal_issues,
                                count,
                            })?
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

                    // Multi-id: return the uniform list envelope
                    // `{"count": N, "issues": [...]}` with full issue objects in
                    // argument order. Without --json, fall through to printing
                    // each issue's human view in order.
                    if ids.len() > 1 && json {
                        let responses = ids
                            .iter()
                            .map(|id| build_issue_show_response(&executor, id))
                            .collect::<Result<Vec<_>>>()?;
                        let output = jit::output::JsonOutput::success(
                            serde_json::to_value(jit::output::IssueShowListResponse {
                                count: responses.len(),
                                issues: responses,
                            })?,
                            "issue show",
                        );
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
                IssueCommands::Status { ids, json } => {
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
                    // refined error envelope (ISSUE_NOT_FOUND / INVALID_ID_PREFIX /
                    // AMBIGUOUS_ID with the matching exit code) and exits, so a
                    // multi-id run fails fast on the first bad id.
                    let mut statuses = Vec::with_capacity(ids.len());
                    for id in &ids {
                        match executor.show_issue(id) {
                            Ok(issue) => {
                                statuses.push(executor.issue_status_response(issue)?);
                            }
                            Err(e) => {
                                handle_json_error!(
                                    json,
                                    e,
                                    jit::output::JsonError::issue_not_found(id, "issue status")
                                );
                            }
                        }
                    }

                    if json {
                        if statuses.len() == 1 {
                            // Single id stays a bare object, mirroring `issue show`.
                            let output =
                                jit::output::JsonOutput::success(&statuses[0], "issue status");
                            println!("{}", output.to_json_string()?);
                        } else {
                            let output = jit::output::JsonOutput::success(
                                serde_json::to_value(jit::output::IssueStatusListResponse {
                                    count: statuses.len(),
                                    issues: statuses,
                                })?,
                                "issue status",
                            );
                            println!("{}", output.to_json_string()?);
                        }
                    } else {
                        for status in &statuses {
                            println!("{}", status.to_line());
                        }
                    }
                }
                IssueCommands::Children { id, json } => {
                    // Orchestration (child resolution, dangling classification)
                    // lives in `CommandExecutor::issue_children`; this arm only
                    // dispatches, renders, and routes an id failure through the
                    // JSON error path.
                    match executor.issue_children(&id) {
                        Ok(response) => {
                            if json {
                                let output =
                                    jit::output::JsonOutput::success(&response, "issue children");
                                println!("{}", output.to_json_string()?);
                            } else {
                                for child in &response.issues {
                                    println!("{}", child.to_line());
                                }
                                if !response.dangling.is_empty() {
                                    println!("dangling: {}", response.dangling.join(","));
                                }
                            }
                        }
                        Err(e) => {
                            handle_json_error!(
                                json,
                                e,
                                jit::output::JsonError::issue_not_found(&id, "issue children")
                            );
                        }
                    }
                }
                IssueCommands::Progress { id, json } => {
                    // Orchestration lives in `CommandExecutor::issue_progress`;
                    // this arm dispatches, renders, and routes an id failure
                    // through the JSON error path.
                    match executor.issue_progress(&id) {
                        Ok(response) => {
                            if json {
                                let output =
                                    jit::output::JsonOutput::success(&response, "issue progress");
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
                                    println!("dangling: {}", response.dangling.join(","));
                                }
                            }
                        }
                        Err(e) => {
                            handle_json_error!(
                                json,
                                e,
                                jit::output::JsonError::issue_not_found(&id, "issue progress")
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
                    json,
                } => {
                    let output_ctx = OutputContext::new(quiet, json);

                    // Batch-mode argument guards are usage errors (exit 2), the
                    // same class as clap's own usage errors — routed through the
                    // json-aware `invalid_argument` helper so `--json` callers get
                    // a machine-readable envelope instead of a bare stderr line.

                    // Validate: exactly one of ID or filter must be provided
                    if id.is_none() && filter.is_none() {
                        return Err(invalid_argument(
                            "Must specify either issue ID or --filter for batch mode".to_string(),
                            "issue update",
                            json,
                        ));
                    }
                    if id.is_some() && filter.is_some() {
                        return Err(invalid_argument(
                            "Cannot specify both ID and --filter (mutually exclusive)".to_string(),
                            "issue update",
                            json,
                        ));
                    }
                    // --content-format is a per-issue field; batch mode does not
                    // support it (would set the same format on every match).
                    if filter.is_some() && content_format.is_some() {
                        return Err(invalid_argument(
                            "--content-format is not supported with --filter (batch mode); set it per issue".to_string(),
                            "issue update",
                            json,
                        ));
                    }
                    // --type is a per-issue field; batch mode does not support it.
                    if filter.is_some() && issue_type.is_some() {
                        return Err(invalid_argument(
                            "--type is not supported with --filter (batch mode); set it per issue"
                                .to_string(),
                            "issue update",
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
                            "issue update",
                            json,
                        ));
                    }

                    // Single issue mode
                    if let Some(id_str) = id {
                        // Resolve short hash to full UUID first
                        let full_id = storage.resolve_issue_id(&id_str)?;

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
                            description_update,
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

                    // Phase 3 safety check: require JIT_ALLOW_DELETION=1 to discourage
                    // deletion (jit:0daba57d). The env var is read here (dispatch-level
                    // input gathering); the refusal decision itself is
                    // `CommandExecutor::confirm_deletion_allowed`, so it stays testable
                    // without mutating global process state.
                    let allow_deletion =
                        std::env::var("JIT_ALLOW_DELETION").unwrap_or_default() == "1";
                    if let Err(e) = executor.confirm_deletion_allowed(&id, allow_deletion) {
                        if json {
                            let json_error = jit::output::JsonError::new(
                                jit::output::ErrorCode::DELETION_NOT_CONFIRMED,
                                e.to_string(),
                                "issue delete",
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
                IssueCommands::Rm { args } => return Err(verb_hint_error("issue", "rm", &args)),
                IssueCommands::Remove { args } => {
                    return Err(verb_hint_error("issue", "remove", &args))
                }
                IssueCommands::Complete { args } => {
                    return Err(verb_hint_error("issue", "complete", &args))
                }
                IssueCommands::Edit { args } => {
                    return Err(verb_hint_error("issue", "edit", &args))
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
                        // Record echo: the raw stored issue (gate list under the
                        // storage names) plus the advisory warnings, routed
                        // through the typed response so the schema stays in
                        // lockstep with the emission (jit:f40f1b0a).
                        let response = jit::output::ClaimResponse {
                            issue,
                            warnings: claim_warnings,
                        };
                        let output = JsonOutput::success(response, "issue claim").with_message(msg);
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
                        // Same record-echo shape as `issue claim` (jit:f40f1b0a).
                        let response = jit::output::ClaimResponse {
                            issue,
                            warnings: claim_warnings,
                        };
                        let output =
                            JsonOutput::success(response, "issue claim-next").with_message(msg);
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
            // Parse the repeatable `--anchor role=id` pairs. The repository's
            // container anchor (`.jit/templates.toml`'s `[anchors] container`) is
            // auto-bound to the positional `<container>`; an explicit
            // `--anchor <that-anchor>=…` overrides it, being applied after the
            // default.
            let mut bindings: std::collections::BTreeMap<String, String> =
                std::collections::BTreeMap::new();
            bindings.insert(executor.container_anchor()?.to_string(), container.clone());
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
                reduce,
                json,
            } => {
                use jit::commands::RedundancyPolicy;
                let policy = if reduce {
                    RedundancyPolicy::Reduce
                } else {
                    RedundancyPolicy::Reject
                };
                let output_ctx = OutputContext::new(quiet, json);
                match executor.add_dependencies_with_policy(&from_id, &to_ids, policy) {
                    Ok(result) => {
                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "from_id": from_id,
                                "added": result.added,
                                "already_exist": result.already_exist,
                                "skipped": result.skipped,
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
                        }
                    }
                    Err(e) => {
                        // A rejected-batch failure (jit:c8518f2a) names every
                        // rejected edge; anything else (bad `<from>`, empty
                        // `dep_ids`) falls through to the generic fallback below.
                        if let Some(batch) =
                            e.downcast_ref::<jit::errors::DependencyBatchRejectedError>()
                        {
                            if json {
                                let json_error = dep_add_batch_json_error(batch);
                                let code = json_error.exit_code().code();
                                println!("{}", json_error.to_json_string()?);
                                std::process::exit(code);
                            }
                            return Err(e);
                        }
                        // `handle_json_error!` refines the fallback when the
                        // failure is a typed id-resolution error (ambiguous /
                        // too-short prefix), consistent with `dep rm`.
                        handle_json_error!(
                            json,
                            e,
                            jit::output::JsonError::new(
                                "DEPENDENCY_ERROR",
                                e.to_string(),
                                "dep add",
                            )
                        );
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
                        // `handle_json_error!` refines the fallback when the
                        // failure is a typed id-resolution error (ambiguous /
                        // too-short prefix), so those carry their distinguishing
                        // code and exit 2 instead of the generic DEPENDENCY_ERROR.
                        handle_json_error!(
                            json,
                            e,
                            jit::output::JsonError::new(
                                "DEPENDENCY_ERROR",
                                e.to_string(),
                                "dep rm",
                            )
                        );
                    }
                }
            }
            DepCommands::Remove { args } => return Err(verb_hint_error("dep", "remove", &args)),
            DepCommands::Delete { args } => return Err(verb_hint_error("dep", "delete", &args)),
        },
        Commands::Gate(gate_cmd) => match gate_cmd {
            GateCommands::Define {
                key,
                title,
                description,
                stage,
                mode,
                auto,
                example,
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
                // over `--mode` when both are supplied. Otherwise resolve the
                // omitted-`--mode` case: a checker command with no explicit
                // mode infers `auto` (REQ-01), so the checker is never
                // silently discarded by a defaulted-to-manual gate. An
                // EXPLICIT `--mode manual` combined with `--checker-command`
                // is a usage error (REQ-02) rather than a silent drop — a
                // manual gate cannot carry a checker.
                let has_checker_command = checker_command.is_some();
                let mode = if auto {
                    jit::domain::GateMode::Auto
                } else {
                    match mode {
                        Some(jit::domain::GateMode::Manual) if has_checker_command => {
                            return Err(invalid_argument(
                                format!(
                                    "--mode manual conflicts with --checker-command for gate '{}': a manual gate cannot have a checker. Drop --checker-command, or omit --mode to define an automated gate.",
                                    key
                                ),
                                "gate define",
                                json,
                            ));
                        }
                        Some(explicit) => explicit,
                        None if has_checker_command => jit::domain::GateMode::Auto,
                        None => jit::domain::GateMode::Manual,
                    }
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
                    example,
                ) {
                    Ok(_) => {
                        if json {
                            use jit::output::JsonOutput;
                            let response = serde_json::json!({
                                "key": key,
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
            GateCommands::Update {
                key,
                title,
                description,
                stage,
                mode,
                auto,
                checker_command,
                timeout,
                working_dir,
                clear_working_dir,
                pass_context,
                prompt,
                clear_prompt,
                prompt_file,
                clear_prompt_file,
                env,
                clear_env,
                priority,
                json,
            } => {
                use jit::commands::{FieldEdit, GateUpdate};

                let output_ctx = OutputContext::new(quiet, json);

                // `--auto` is a convenience spelling of `--mode auto`; it wins
                // over `--mode` when both are supplied.
                let mode = if auto {
                    Some(jit::domain::GateMode::Auto)
                } else {
                    mode
                };

                // Resolve a set/clear flag pair into a tri-state `FieldEdit`. A
                // set value AND its --clear-* twin together is an actionable,
                // JSON-aware INVALID_ARGUMENT error (no eprintln + exit).
                fn resolve_field_edit<T>(
                    set: Option<T>,
                    clear: bool,
                    flag: &str,
                    json: bool,
                ) -> std::result::Result<FieldEdit<T>, anyhow::Error> {
                    match (set, clear) {
                        (Some(_), true) => Err(invalid_argument(
                            format!(
                                "--{flag} and --clear-{flag} are mutually exclusive; provide only one."
                            ),
                            "gate update",
                            json,
                        )),
                        (Some(v), false) => Ok(FieldEdit::Set(v)),
                        (None, true) => Ok(FieldEdit::Clear),
                        (None, false) => Ok(FieldEdit::Keep),
                    }
                }

                let working_dir_edit =
                    resolve_field_edit(working_dir, clear_working_dir, "working-dir", json)?;
                let prompt_edit = resolve_field_edit(prompt, clear_prompt, "prompt", json)?;
                let prompt_file_edit =
                    resolve_field_edit(prompt_file, clear_prompt_file, "prompt-file", json)?;

                // `--env` (non-empty) sets the whole set; `--clear-env` empties
                // it; the two together are mutually exclusive. Parse KEY=VALUE
                // pairs through the typed/JSON arg-error path.
                let env_set = if env.is_empty() {
                    None
                } else {
                    let parsed: std::result::Result<
                        std::collections::HashMap<String, String>,
                        String,
                    > = env
                        .into_iter()
                        .map(|pair| {
                            pair.split_once('=')
                                .map(|(k, v)| (k.to_string(), v.to_string()))
                                .ok_or_else(|| {
                                    format!("Invalid --env format '{}': expected KEY=VALUE", pair)
                                })
                        })
                        .collect();
                    match parsed {
                        Ok(map) => Some(map),
                        Err(msg) => return Err(invalid_argument(msg, "gate update", json)),
                    }
                };
                let env_edit = resolve_field_edit(env_set, clear_env, "env", json)?;

                let update = GateUpdate {
                    title,
                    description,
                    stage,
                    mode,
                    priority,
                    checker_command,
                    timeout,
                    working_dir: working_dir_edit,
                    // `--pass-context true|false` sets the flag; absence leaves
                    // it unchanged.
                    pass_context,
                    prompt: prompt_edit,
                    prompt_file: prompt_file_edit,
                    env: env_edit,
                };

                if update.is_empty() {
                    return Err(invalid_argument(
                        "no fields to update; provide at least one field to change \
                         (e.g. --title, --description, --mode, --checker-command)"
                            .to_string(),
                        "gate update",
                        json,
                    ));
                }

                match executor.update_gate(&key, update) {
                    Ok(gate) => {
                        if json {
                            use jit::output::JsonOutput;
                            let msg = format!("Updated gate '{}'", gate.key);
                            let output = JsonOutput::success(gate, "gate update").with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            let _ = output_ctx.print_success(format!("Updated gate '{}'", key));
                        }
                    }
                    Err(e) => {
                        if json {
                            use jit::output::JsonError;
                            let json_error = if e
                                .downcast_ref::<jit::storage::GateNotFoundError>()
                                .is_some()
                            {
                                JsonError::gate_not_found(&key, "gate update")
                            } else {
                                JsonError::new("GATE_ERROR", e.to_string(), "gate update")
                            };
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
                        if let Some(example) = &gate.example_integration {
                            println!("  Example Integration: {}", example);
                        }
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
                                jit::domain::GateChecker::RepositoryValidation => {
                                    println!("  Checker: repository validation (built-in)");
                                }
                                jit::domain::GateChecker::IssueValidation => {
                                    println!("  Checker: issue validation (built-in)");
                                }
                                jit::domain::GateChecker::LabelTargetValidation {
                                    label_namespace,
                                } => {
                                    println!(
                                        "  Checker: label-target validation (built-in, namespace: {})",
                                        label_namespace
                                    );
                                }
                                jit::domain::GateChecker::ReviewPlaceholder => {
                                    println!("  Checker: WARNING — external review placeholder");
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
                                "key": key,
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
            GateCommands::Rm { args } => return Err(verb_hint_error("gate", "rm", &args)),
            GateCommands::Delete { args } => return Err(verb_hint_error("gate", "delete", &args)),
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
                json,
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
                        "gate status",
                        json,
                    ));
                }
                // `--status` filters a history listing; it is meaningless without one.
                if status.is_some() && !history_mode {
                    return Err(invalid_argument(
                        "--status only applies to the history view; pass --all or --limit."
                            .to_string(),
                        "gate status",
                        json,
                    ));
                }

                if findings_mode {
                    // Findings view selects exactly one gate's latest run and
                    // reports only its structured findings + verdict.
                    let gate_key = resolve_gate_key_for(gate_key, gate_flag, "gate status", json)?;
                    match executor.get_last_gate_run(&id, &gate_key) {
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
                                let output = JsonOutput::success(response, "gate status");
                                println!("{}", output.to_json_string()?);
                            } else {
                                print!("{}", render_gate_findings_text(&result));
                            }
                        }
                        Ok(None) => {
                            let msg = format!(
                                "Gate '{}' has not been run yet for issue {}. Use 'jit gate evaluate' to run it.",
                                gate_key, id
                            );
                            if json {
                                use jit::output::JsonOutput;
                                let output = JsonOutput::<Option<()>>::success(None, "gate status")
                                    .with_message(msg);
                                println!("{}", output.to_json_string()?);
                            } else {
                                println!("{}", msg);
                            }
                        }
                        Err(e) => {
                            if json {
                                use jit::output::JsonError;
                                let json_error = JsonError::new(
                                    "GATE_CHECK_ERROR",
                                    e.to_string(),
                                    "gate status",
                                );
                                println!("{}", json_error.to_json_string()?);
                                std::process::exit(json_error.exit_code().code());
                            }
                            return Err(e);
                        }
                    }
                } else if history_mode {
                    // Gate key is an optional filter here (positional or --gate).
                    let gate_filter =
                        resolve_optional_gate_key(gate_key, gate_flag, "gate status", json)?;
                    let status_filter = match status {
                        Some(ref s) => Some(parse_run_status(s, "gate status", json)?),
                        None => None,
                    };

                    let runs = match executor.list_gate_runs(&id, gate_filter.as_deref()) {
                        Ok(runs) => runs,
                        Err(e) => {
                            if json {
                                use jit::output::JsonError;
                                let json_error = JsonError::new(
                                    "GATE_CHECK_ERROR",
                                    e.to_string(),
                                    "gate status",
                                );
                                println!("{}", json_error.to_json_string()?);
                                std::process::exit(json_error.exit_code().code());
                            }
                            return Err(e);
                        }
                    };
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
                        let response = GateRunHistoryResponse {
                            results: summaries,
                            count,
                        };
                        let msg = format!("{} gate run(s) listed for issue {}", count, id);
                        let output = JsonOutput::success(response, "gate status").with_message(msg);
                        println!("{}", output.to_json_string()?);
                    } else if runs.is_empty() {
                        let _ = output_ctx
                            .print_info(format!("No matching gate runs for issue {}", id));
                    } else {
                        let _ =
                            output_ctx.print_info(format!("Gate run history for issue {}:", id));
                        for run in &runs {
                            print_gate_run_details(run);
                        }
                    }
                } else if flat_mode {
                    // Flat view selects exactly one gate's latest run.
                    let gate_key = resolve_gate_key_for(gate_key, gate_flag, "gate status", json)?;
                    // stderr alone shows only stderr; --stdout (or the tail-only
                    // form, where neither stream flag is set) includes stdout.
                    let want_stdout = stdout || !stderr;
                    let want_stderr = stderr;

                    match executor.get_last_gate_run(&id, &gate_key) {
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
                                let output = JsonOutput::success(response, "gate status");
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
                            let msg = format!(
                                "Gate '{}' has not been run yet for issue {}. Use 'jit gate evaluate' to run it.",
                                gate_key, id
                            );
                            if json {
                                use jit::output::JsonOutput;
                                let output = JsonOutput::<Option<()>>::success(None, "gate status")
                                    .with_message(msg);
                                println!("{}", output.to_json_string()?);
                            } else {
                                println!("{}", msg);
                            }
                        }
                        Err(e) => {
                            if json {
                                use jit::output::JsonError;
                                let json_error = JsonError::new(
                                    "GATE_CHECK_ERROR",
                                    e.to_string(),
                                    "gate status",
                                );
                                println!("{}", json_error.to_json_string()?);
                                std::process::exit(json_error.exit_code().code());
                            }
                            return Err(e);
                        }
                    }
                } else {
                    let gate_key = resolve_gate_key_for(gate_key, gate_flag, "gate status", json)?;

                    // Transposed-argument guard. The canonical form is
                    // `jit gate status <issue> <gate-key>`. If <id> is not an issue but
                    // <gate_key> resolves to one and <id> is a registered gate key, the
                    // two positionals are almost certainly swapped, so emit an
                    // actionable did-you-mean rather than misparsing into
                    // "issue not found".
                    if executor.storage().resolve_issue_id(&id).is_err() {
                        let gate_key_is_issue =
                            executor.storage().resolve_issue_id(&gate_key).is_ok();
                        let id_is_gate = executor
                            .list_gates()
                            .map(|gates| gates.iter().any(|g| g.key == id))
                            .unwrap_or(false);
                        if gate_key_is_issue && id_is_gate {
                            let canonical = format!("jit gate status {} {}", gate_key, id);
                            let message = format!(
                                "'{id}' is a gate key and '{gate_key}' is an issue; the issue id and gate key look transposed."
                            );
                            if json {
                                use jit::output::JsonError;
                                let json_error =
                                    JsonError::new("INVALID_ARGUMENT", message, "gate status")
                                        .with_details(serde_json::json!({
                                            "issue_id": gate_key,
                                            "key": id,
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
                                use jit::output::{GateRunSummary, JsonOutput};
                                let msg = format!("Gate '{}': {:?}", gate_key, result.status);
                                let summary = GateRunSummary::full(&result);
                                let output =
                                    JsonOutput::success(summary, "gate status").with_message(msg);
                                println!("{}", output.to_json_string()?);
                            } else {
                                print_gate_run_details(&result);
                                let _ = output_ctx;
                            }
                        }
                        Ok(None) => {
                            let msg = format!(
                                "Gate '{}' has not been run yet for issue {}. Use 'jit gate evaluate' to run it.",
                                gate_key, id
                            );
                            if json {
                                use jit::output::JsonOutput;
                                let output = JsonOutput::<Option<()>>::success(None, "gate status")
                                    .with_message(msg);
                                println!("{}", output.to_json_string()?);
                            } else {
                                println!("{}", msg);
                            }
                        }
                        Err(e) => {
                            if json {
                                use jit::output::JsonError;
                                let json_error = JsonError::new(
                                    "GATE_CHECK_ERROR",
                                    e.to_string(),
                                    "gate status",
                                );
                                println!("{}", json_error.to_json_string()?);
                                std::process::exit(json_error.exit_code().code());
                            } else {
                                return Err(e);
                            }
                        }
                    }
                }
            }
            GateCommands::StatusAll { id, full, json } => {
                use jit::domain::GateStatus;
                let output_ctx = OutputContext::new(quiet, json);
                // Automated gate run detail (unchanged display source).
                let (results, _) = executor.get_last_gate_runs_for_issue(&id)?;
                // Readiness across EVERY required gate (auto + manual).
                let gate_statuses = executor.get_required_gate_statuses_for_issue(&id)?;

                let total = gate_statuses.len();
                let passed_count = gate_statuses
                    .iter()
                    .filter(|(_, s)| *s == GateStatus::Passed)
                    .count();
                // Pending keys across all required gates (auto never run, manual
                // never attested), preserving priority order.
                let not_run: Vec<String> = gate_statuses
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
                    let response = GateCheckAllResponse {
                        count: gate_status_entries.len(),
                        results: summaries,
                        passed: passed_count,
                        total,
                        not_run,
                        gates: gate_status_entries,
                        all_passed,
                    };
                    let output = JsonOutput::success(response, "gate status-all").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else if total == 0 {
                    let _ = output_ctx
                        .print_info(format!("No required gates to inspect for issue {}", id));
                } else {
                    let _ = output_ctx.print_info(format!("Gate readiness for issue {}:", id));
                    // Gate keys that have a recorded automated run to print in detail.
                    let with_runs: std::collections::HashSet<&str> =
                        results.iter().map(|r| r.gate_key.as_str()).collect();
                    for result in &results {
                        print_gate_run_details(result);
                    }
                    // Cover every required gate not shown above (manual gates and
                    // pending auto gates) so a strict nonzero exit is explained.
                    for (gate_key, status) in &gate_statuses {
                        if with_runs.contains(gate_key.as_str()) {
                            continue;
                        }
                        match status {
                            GateStatus::Pending => println!(
                                "Gate '{}' has not been run yet for issue {}. Use 'jit gate evaluate' to run it.",
                                gate_key, id
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
            GateCommands::Evaluate {
                id,
                gate_key,
                gate_flag,
                by,
                force,
                json,
            } => {
                let gate_key = resolve_gate_key_for(gate_key, gate_flag, "gate evaluate", json)?;
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
                                "key": gate_key,
                                "status": "passed",
                                "verdict": "pass",
                                "already_passed": already_passed,
                                "warnings": outcome.warnings,
                                "message": message,
                            });
                            let output = JsonOutput::success(response, "gate evaluate");
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
                        render_gate_pass_error(e, &id, &output_ctx, json, "gate evaluate")?;
                    }
                }
            }
            GateCommands::EvaluateAll {
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
                                        "key": entry.gate_key,
                                        "status": "passed",
                                        "verdict": "pass",
                                        "already_passed": entry.already_passed,
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
                            let output = JsonOutput::success(response, "gate evaluate-all");
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
                        render_gate_pass_error(e, &id, &output_ctx, json, "gate evaluate-all")?;
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
                                "key": gate_key,
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
                                    serde_json::json!({
                                        "count": presets.len(),
                                        "presets": presets,
                                    }),
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
                                            jit::domain::GateChecker::RepositoryValidation => {
                                                println!("    Built-in: repository validation");
                                            }
                                            jit::domain::GateChecker::IssueValidation => {
                                                println!("    Built-in: issue validation");
                                            }
                                            jit::domain::GateChecker::LabelTargetValidation {
                                                label_namespace,
                                            } => {
                                                println!(
                                                    "    Built-in: label-target validation ({label_namespace}:)"
                                                );
                                            }
                                            jit::domain::GateChecker::ReviewPlaceholder => {
                                                println!(
                                                    "    Built-in: WARNING — external review placeholder"
                                                );
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
                        count: tree.len(),
                        nodes: tree,
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
            GraphCommands::Tree { root, json } => {
                let output_ctx = OutputContext::new(quiet, json);
                let response = executor.resolve_hierarchy_tree(root.as_deref())?;

                if json {
                    use jit::output::JsonOutput;
                    let msg = format!("{} nodes", response.count);
                    let output = JsonOutput::success(response, "graph tree").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    let scope = match &response.root {
                        Some(id) => format!("subtree of {}", &id[..8.min(id.len())]),
                        None => "repository".to_string(),
                    };
                    let _ = output_ctx.print_info(format!("Resolved hierarchy ({}):", scope));
                    if response.nodes.is_empty() {
                        println!("  (none)");
                    } else {
                        for node in &response.nodes {
                            let parent = node
                                .hierarchy
                                .parent
                                .as_deref()
                                .map(|p| &p[..8.min(p.len())])
                                .unwrap_or("-");
                            println!(
                                "  {} | parent={} children={} rank={} | {}",
                                node.short_id,
                                parent,
                                node.hierarchy.children.len(),
                                node.hierarchy.rank,
                                node.title
                            );
                        }
                    }
                }
            }
            GraphCommands::Export {
                format,
                json,
                full,
                scope,
                output,
            } => {
                use jit::commands::GraphExportFormat;

                // `--json` is sugar for `--format json`; combining it with an
                // explicit conflicting `--format` (dot/mermaid/batch) is a usage
                // error (exit 2), classified by the typed InvalidArgumentError.
                if json && matches!(format, Some(f) if f != GraphExportFormat::Json) {
                    return Err(jit::errors::InvalidArgumentError::new(
                        "--json conflicts with --format dot/mermaid/batch; use one or the other",
                    )
                    .into());
                }
                let format = if json {
                    GraphExportFormat::Json
                } else {
                    format.unwrap_or(GraphExportFormat::Dot)
                };

                // `--full` selects the complete-record JSON node shape and applies
                // only to `--format json` (or `--json`); pairing it with
                // dot/mermaid/batch is a usage error (exit 2), classified by the
                // typed InvalidArgumentError.
                if full && format != GraphExportFormat::Json {
                    return Err(jit::errors::InvalidArgumentError::new(
                        "--full is only valid with --format json",
                    )
                    .into());
                }
                let output_ctx = OutputContext::new(quiet, false);

                // `batch` runs a distinct projection (issue definitions plus
                // reported boundary edges); every other format renders the graph.
                let graph_output = if format == GraphExportFormat::Batch {
                    let export = executor.export_graph_batch(scope.as_deref())?;
                    // Boundary edges never join the array (it must stay a clean
                    // batch-create payload); they are reported to stderr so no
                    // crossing edge is dropped silently (REQ-06).
                    if !export.boundary_edges.is_empty() {
                        eprintln!(
                            "Excluded {} edge(s) crossing the scope boundary:",
                            export.boundary_edges.len()
                        );
                        for edge in &export.boundary_edges {
                            eprintln!("  {} -> {}", edge.from, edge.to);
                        }
                    }
                    serde_json::to_string_pretty(&export.defs)?
                } else {
                    executor.export_graph(format, full, scope.as_deref())?
                };

                if let Some(path) = output {
                    // Write through the shared atomic primitive (temp file +
                    // rename) so a reader never observes a partially written
                    // export file (@/inv/atomic-writes), matching every storage write.
                    jit::storage::atomic_write::write_file_atomic(
                        std::path::Path::new(&path),
                        &graph_output,
                    )?;
                    let _ = output_ctx.print_success(format!("Graph exported to: {}", path));
                } else {
                    println!("{}", graph_output);
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

                let verb = if result.updated { "Updated" } else { "Added" };
                if json {
                    use jit::output::JsonOutput;
                    let msg = format!("{} document reference on issue {}", verb, result.issue_id);
                    let output = JsonOutput::success(&result, "doc add").with_message(msg);
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("{} document reference on issue {}", verb, result.issue_id);
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
            DocCommands::Rm { args } => return Err(verb_hint_error("doc", "rm", &args)),
            DocCommands::Delete { args } => return Err(verb_hint_error("doc", "delete", &args)),
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
        },
        Commands::Archive(archive_cmd) => match archive_cmd {
            ArchiveCommands::Candidates { json } => {
                let report = executor.archive_candidates()?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    print!("{}", jit::output::render_archive_candidates(&report));
                }
            }
            ArchiveCommands::Document {
                path,
                execute,
                json,
            } => {
                if execute {
                    let result = executor.execute_archive_document(&path)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else {
                        println!(
                                "Archive execution complete: {} publication(s), {} reference change(s), {} source deletion(s)",
                                result.publications.len(),
                                result.reference_changes.len(),
                                result.deleted_sources.len()
                            );
                        for warning in result.warnings {
                            println!(
                                "warning: {}{}",
                                warning.code.as_str(),
                                warning
                                    .path
                                    .map(|path| format!(" ({path})"))
                                    .unwrap_or_default()
                            );
                        }
                    }
                } else {
                    let plan = executor.preview_archive_document(&path)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&plan)?);
                    } else {
                        print!("{}", jit::output::render_archive_plan(&plan));
                    }
                }
            }
            ArchiveCommands::Container { id, execute, json } => {
                if execute {
                    let result = executor.execute_archive_container(&id)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else {
                        println!(
                                "Archive execution complete: {} publication(s), {} reference change(s), {} source deletion(s)",
                                result.publications.len(),
                                result.reference_changes.len(),
                                result.deleted_sources.len()
                            );
                        for warning in result.warnings {
                            println!(
                                "warning: {}{}",
                                warning.code.as_str(),
                                warning
                                    .path
                                    .map(|path| format!(" ({path})"))
                                    .unwrap_or_default()
                            );
                        }
                    }
                } else {
                    let plan = executor.preview_archive_container(&id)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&plan)?);
                    } else {
                        print!("{}", jit::output::render_archive_plan(&plan));
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
                    &bare_label,
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
                        let issues = executor.query_available(priority_filter, &label)?;

                        if json {
                            use jit::domain::MinimalIssue;
                            use jit::output::JsonOutput;

                            let msg = format!("Found {} issue(s)", issues.len());
                            let output = if full {
                                JsonOutput::success(
                                    serde_json::to_value(jit::output::IssueListFullResponse {
                                        count: issues.len(),
                                        issues,
                                    })?,
                                    "query available",
                                )
                            } else {
                                let minimal: Vec<MinimalIssue> =
                                    issues.iter().map(MinimalIssue::from).collect();
                                JsonOutput::success(
                                    serde_json::to_value(jit::output::IssueListResponse {
                                        count: minimal.len(),
                                        issues: minimal,
                                    })?,
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
                        let blocked = executor.query_blocked_filtered(priority_filter, &label)?;

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
                                    serde_json::to_value(jit::output::BlockedFullListResponse {
                                        count: blocked_issues.len(),
                                        issues: blocked_issues,
                                    })?,
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
                                    serde_json::to_value(jit::output::BlockedListResponse {
                                        count: minimal.len(),
                                        issues: minimal,
                                    })?,
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
                        let issues = executor.query_strategic_filtered(priority_filter, &label)?;

                        if json {
                            use jit::domain::MinimalIssue;
                            use jit::output::JsonOutput;

                            let msg = format!("Found {} issue(s)", issues.len());
                            let output = if full {
                                JsonOutput::success(
                                    serde_json::to_value(jit::output::IssueListFullResponse {
                                        count: issues.len(),
                                        issues,
                                    })?,
                                    "query strategic",
                                )
                            } else {
                                let minimal: Vec<MinimalIssue> =
                                    issues.iter().map(MinimalIssue::from).collect();
                                JsonOutput::success(
                                    serde_json::to_value(jit::output::IssueListResponse {
                                        count: minimal.len(),
                                        issues: minimal,
                                    })?,
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
                        let issues = executor.query_closed_filtered(priority_filter, &label)?;

                        if json {
                            use jit::domain::MinimalIssue;
                            use jit::output::JsonOutput;

                            let msg = format!("Found {} issue(s)", issues.len());
                            let output = if full {
                                JsonOutput::success(
                                    serde_json::to_value(jit::output::IssueListFullResponse {
                                        count: issues.len(),
                                        issues,
                                    })?,
                                    "query closed",
                                )
                            } else {
                                let minimal: Vec<MinimalIssue> =
                                    issues.iter().map(MinimalIssue::from).collect();
                                JsonOutput::success(
                                    serde_json::to_value(jit::output::IssueListResponse {
                                        count: minimal.len(),
                                        issues: minimal,
                                    })?,
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
                    jit::cli::QueryCommands::Count { by, label, json } => {
                        // Aggregation lives in the executor; this arm dispatches
                        // by dimension and renders. The bucket is every issue
                        // matching all --label patterns (ANDed; none = whole repo).
                        let rollup = match by {
                            jit::cli::CountDimension::State => {
                                executor.query_count_by_state(&label)?
                            }
                        };

                        if json {
                            let msg = format!(
                                "{}/{} done ({}%)",
                                rollup.done, rollup.total, rollup.percent
                            );
                            let output = jit::output::JsonOutput::success(&rollup, "query count")
                                .with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else {
                            for line in rollup.to_lines() {
                                println!("{}", line);
                            }
                        }
                    }
                    jit::cli::QueryCommands::Divergence { json } => {
                        let output_ctx = OutputContext::new(quiet, json);
                        let report = executor.detect_divergences()?;

                        if json {
                            let msg = format!("{} divergence(s)", report.count);
                            let output =
                                jit::output::JsonOutput::success(report, "query divergence")
                                    .with_message(msg);
                            println!("{}", output.to_json_string()?);
                        } else if report.divergences.is_empty() {
                            let _ = output_ctx.print_success("No membership/DAG divergences");
                        } else {
                            let _ = output_ctx.print_info(format!(
                                "{} membership label(s) not backed by the DAG:",
                                report.count
                            ));
                            for d in &report.divergences {
                                println!("  {} | {} | {}", d.short_id, d.label, d.title);
                            }
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
            jit::cli::LabelCommands::Add { args } => {
                return Err(verb_hint_error("label", "add", &args))
            }
            jit::cli::LabelCommands::Rm { args } => {
                return Err(verb_hint_error("label", "rm", &args))
            }
            jit::cli::LabelCommands::Remove { args } => {
                return Err(verb_hint_error("label", "remove", &args))
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
                            "lease_renewal_threshold_pct": config.coordination().lease_renewal_threshold_pct(),
                            "stale_threshold_secs": config.coordination().stale_threshold_secs(),
                            "max_indefinite_leases_per_agent": config.coordination().max_indefinite_leases_per_agent(),
                            "max_indefinite_leases_per_repo": config.coordination().max_indefinite_leases_per_repo(),
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
                use jit::output::{render_config_get_value, ErrorCode, JsonError, JsonOutput};
                use serde_json::json;

                match executor.get_config(&key) {
                    Ok(outcome) => {
                        if json {
                            println!(
                                "{}",
                                JsonOutput::success(
                                    json!({"key": outcome.key, "value": outcome.value}),
                                    "config get"
                                )
                                .to_json_string()?
                            );
                        } else {
                            println!("{}", render_config_get_value(&outcome.value));
                        }
                    }
                    Err(e) => {
                        // Only an unknown/missing dotted key (the typed,
                        // shared `InvalidArgumentError` `get_config` converts
                        // `ConfigKeyError` into) is an argument error. A
                        // load/parse/filesystem failure (e.g. a malformed
                        // `config.toml`) is NOT downcastable to it, so it
                        // falls through to the normal error path and is
                        // classified the same way every other config-load
                        // failure in this codebase is — never misreported as
                        // a bad CLI argument.
                        if e.downcast_ref::<jit::errors::InvalidArgumentError>()
                            .is_some()
                        {
                            handle_json_error!(
                                json,
                                e,
                                JsonError::new(
                                    ErrorCode::INVALID_ARGUMENT,
                                    e.to_string(),
                                    "config get"
                                )
                            );
                        } else {
                            return Err(e);
                        }
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

                let outcome = executor.set_config(&key, &value, global)?;

                if json {
                    println!(
                        "{}",
                        JsonOutput::success(
                            json!({
                                "key": outcome.key,
                                "value": outcome.value,
                                "file": outcome.file.display().to_string(),
                                "scope": outcome.scope
                            }),
                            "config set"
                        )
                        .to_json_string()?
                    );
                } else {
                    println!(
                        "Set {} = {} in {}",
                        outcome.key,
                        outcome.value,
                        outcome.file.display()
                    );
                }
            }
            jit::cli::ConfigCommands::Validate { json } => {
                use jit::config::{ConfigLoader, JitConfig};
                use jit::output::JsonOutput;
                use serde_json::json;

                #[derive(Default)]
                struct ValidationResult {
                    errors: Vec<String>,
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

                if json {
                    let output = json!({
                        "valid": !has_errors,
                        "errors": result.errors,
                    });
                    println!(
                        "{}",
                        JsonOutput::success(output, "config validate")
                            .with_message(if has_errors {
                                format!("Validation failed: {} error(s)", result.errors.len())
                            } else {
                                "Configuration is valid".to_string()
                            })
                            .to_json_string()?
                    );
                } else if has_errors {
                    println!("Errors:");
                    for err in &result.errors {
                        println!("  ✗ {}", err);
                    }
                } else {
                    println!("✓ Configuration is valid");
                }

                // A source that failed to load or carried an invalid value exits 1;
                // a valid configuration exits 0. There is no warning outcome.
                if has_errors {
                    std::process::exit(1);
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
        Commands::Project(project_cmd) => {
            run_project(&executor, project_cmd, quiet)?;
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
            branch_drift,
            divergence,
            leases,
            scope,
        } => {
            // `--divergence` is a hidden stub (cli.rs): the word names the
            // membership-vs-DAG report of `jit query divergence`, so the git
            // check owns `--branch-drift` and this spelling only hints.
            if divergence {
                return Err(invalid_argument(
                    "`--divergence` is not a `jit validate` flag. Use \
                     `jit validate --branch-drift` for git branch drift, or \
                     `jit query divergence` for membership labels the DAG does \
                     not back."
                        .to_string(),
                    "validate",
                    json,
                ));
            }

            // Validate dry_run requires fix
            if dry_run && !fix {
                return Err(anyhow!("--dry-run requires --fix to be specified"));
            }

            // `--scope <C>` is a self-contained gate checker over a container's
            // bracket subtree. It owns its own exit code (4 on any
            // error-severity finding) and is mutually exclusive with the other
            // validate modes, so it is dispatched FIRST after the combo checks
            // below reject conflicting flags.
            if scope.is_some() && (id.is_some() || fix || branch_drift || leases || explain) {
                return Err(anyhow!(
                    "`--scope` cannot be combined with a positional id or with \
                     `--fix`/`--branch-drift`/`--leases`/`--explain`"
                ));
            }

            // `--fix`, `--branch-drift`, and `--leases` are repo-wide operations and
            // are NOT scoped to a single issue. Combining any of them with a
            // positional issue id is rejected explicitly: previously the id was
            // silently ignored and the command ran repo-wide, which is dangerous
            // for `--fix` (it could mutate the entire repository when the user
            // believed they had scoped it to one issue).
            if id.is_some() && (fix || branch_drift || leases) {
                return Err(anyhow!(
                    "`--fix`/`--branch-drift`/`--leases` cannot be combined with a \
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
            // (`--fix`/`--branch-drift`/`--leases` + id) were already rejected above,
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
            if branch_drift || leases {
                let mut validation_results = Vec::new();

                if branch_drift {
                    match executor.validate_branch_drift() {
                        Ok(()) => {
                            if !json {
                                println!("✓ Branch is up-to-date with origin/main");
                            }
                            validation_results.push(("branch_drift", true, String::new()));
                        }
                        Err(e) => {
                            if json {
                                validation_results.push(("branch_drift", false, e.to_string()));
                            } else {
                                eprintln!("❌ Branch-drift validation failed:\n{}", e);
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
                // Standard whole-repo validation. Load every repository-dependent
                // input through the one read-only filesystem view. The same
                // boundary accepts an overlay for a future profile planner, so
                // no parser needs to reopen live `.jit` bytes while judging a
                // proposed final state. Capture (do NOT `?`-propagate) any
                // integrity error so it can be rendered before the exit status is
                // decided below.
                // Wrap any integrity violation in the typed ValidationFailedError
                // (message preserved verbatim) so the top-level handler classifies
                // it as a validation failure by downcast rather than by message text.
                let repository_view =
                    jit::validation::repository::FilesystemRepositoryView::from_jit_root(&jit_dir)?;
                let (integrity_error, rule_report) =
                    match jit::validation::repository::validate_repository(&repository_view) {
                        Ok(report) => (None, report.rule_report),
                        Err(failure) => {
                            let (error, report) = failure.into_parts();
                            (
                                Some(anyhow::Error::new(jit::errors::ValidationFailedError::new(
                                    format!("Invalid repository: {error:#}"),
                                ))),
                                report.rule_report,
                            )
                        }
                    };
                let integrity_message = integrity_error.as_ref().map(|e| e.to_string());

                // The view-derived report contains the declarative local/graph
                // findings and built-in semantic findings. Do not reopen the
                // storage root through `CommandExecutor` after judging the view.
                let rules_failed = rule_report.has_errors();
                let validation_failed = rules_failed || integrity_error.is_some();

                // Warn-severity findings are reported separately as "warnings" for
                // output-shape stability (the prior orphan/strategic warning list).
                let warning_findings: Vec<&jit::validation::report::ReportedFinding> = rule_report
                    .findings
                    .iter()
                    .filter(|f| !f.is_error())
                    .collect();

                // Membership-vs-DAG divergences are advisory: they surface here as
                // a warning-severity count but never change the exit status (a repo
                // with real labels must not start failing `jit validate`). A
                // resolution error degrades to an empty report rather than failing
                // the whole validate run.
                let divergence_report = executor.detect_divergences().unwrap_or_else(|_| {
                    jit::output::DivergenceResponse {
                        count: 0,
                        divergences: Vec::new(),
                    }
                });

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
                    let divergences_json = serde_json::to_value(&divergence_report.divergences)?;
                    let output = JsonOutput::success(
                        json!({
                            "valid": !validation_failed,
                            "integrity_error": integrity_message,
                            "warnings": warnings_json,
                            "warning_count": warnings_json.len(),
                            "membership_divergences": divergences_json,
                            "divergence_count": divergence_report.count,
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

                    // Advisory only — does not affect the exit status.
                    if divergence_report.count > 0 {
                        println!(
                            "⚠ {} membership label(s) not backed by the DAG \
                             (run `jit query divergence` for details)",
                            divergence_report.count
                        );
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

            if !storage.root().exists() && transactions_recovered > 0 {
                if json {
                    let output = JsonOutput::success(
                        json!({
                            "success": true,
                            "transactions_recovered": transactions_recovered,
                            "stale_locks_cleaned": 0,
                            "index_rebuilt": false,
                            "expired_leases_evicted": 0,
                            "temp_files_removed": 0,
                            "warnings": [],
                        }),
                        "recover",
                    )
                    .with_message(format!(
                        "Recovery: {transactions_recovered} transaction(s) recovered; repository absence restored"
                    ));
                    println!("{}", output.to_json_string()?);
                } else {
                    println!("Recovery complete:");
                    println!("  • Transactions recovered: {transactions_recovered}");
                    println!("  • Repository state: not initialized (restored)");
                }
                return Ok(());
            }

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
                                "transactions_recovered": transactions_recovered,
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
                        println!("  • Transactions recovered: {}", transactions_recovered);
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
        Commands::Migrate(migrate_cmd) => match migrate_cmd {
            MigrateCommands::LifecycleTimestamps { json } => {
                let result = executor.backfill_lifecycle_timestamps()?;
                if json {
                    use jit::output::JsonOutput;
                    let output = JsonOutput::success(
                        serde_json::json!({
                            "issues_scanned": result.issues_scanned,
                            "issues_updated": result.issues_updated,
                        }),
                        "migrate lifecycle-timestamps",
                    )
                    .with_message(format!(
                        "Backfilled lifecycle timestamps on {} of {} issue(s)",
                        result.issues_updated, result.issues_scanned
                    ));
                    println!("{}", output.to_json_string()?);
                } else {
                    let output_ctx = OutputContext::new(quiet, false);
                    let _ = output_ctx.print_success(format!(
                        "Backfilled lifecycle timestamps on {} of {} issue(s)",
                        result.issues_updated, result.issues_scanned
                    ));
                }
            }
        },
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
                        read_pid_file, spawn_with_listener,
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

                    let listener = find_available_port(port)?;
                    let p = listener
                        .local_addr()
                        .context("Failed to read bound port")?
                        .port();
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
                    // Hand the bound socket to the child (inherited fd on
                    // Unix); it adopts this exact socket instead of re-binding.
                    let mut child = spawn_with_listener(&mut cmd, listener)
                        .context("Failed to run jit-server")?;
                    let status = child.wait().context("Failed to wait on jit-server")?;
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
                        std::process::exit(jit::commands::serve::foreground_exit_code(
                            status.code(),
                        ));
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

#[cfg(test)]
mod exit_code_projection_tests {
    //! REQ-02: verify the projected command exit-code mappings against the
    //! runtime classifier they document. Each representative typed error is run
    //! through [`error_to_exit_code`] (the exact classifier the CLI dispatch
    //! uses) and the resulting code is confirmed to (a) equal the expected code
    //! and (b) be a code the schema's `command_exit_codes` projection documents.
    //! If the classifier ever reclassifies one of these conditions, or the
    //! projection stops documenting a code the classifier still emits, this test
    //! fails — so the projection cannot silently drift from runtime behavior.

    use super::error_to_exit_code;
    use jit::domain::{GateRunResult, GateRunStatus, GateStage};
    use jit::schema::CommandSchema;

    /// Build a `gate evaluate` checker failure carrying `status`, so the
    /// `gate evaluate` / `gate evaluate-all` exception rows (checker verdict `4`,
    /// runner error `10`) are pinned to the real classifier: `error_to_exit_code`
    /// splits `GatePassFailed` on this status.
    fn gate_pass_failed(status: GateRunStatus) -> anyhow::Error {
        let result = GateRunResult {
            schema_version: 1,
            run_id: "run-1".to_string(),
            gate_key: "tests".to_string(),
            stage: GateStage::Precheck,
            issue_id: "abc123".to_string(),
            commit: None,
            branch: None,
            tree_dirty: None,
            status,
            started_at: chrono::Utc::now(),
            completed_at: None,
            duration_ms: None,
            exit_code: Some(1),
            stdout: String::new(),
            stderr: String::new(),
            command: "false".to_string(),
            by: None,
            message: None,
            findings: None,
        };
        jit::commands::GatePassFailed {
            issue_id: "abc123".to_string(),
            gate_key: "tests".to_string(),
            status,
            exit_code: Some(1),
            result,
            warnings: Vec::new(),
        }
        .into()
    }

    /// Representative typed errors, each paired with the exit code the classifier
    /// must produce for it and the `command_exit_codes` **row** that pair pins
    /// (`"*"` for the universal rows, which any command reaches through the shared
    /// classifier).
    ///
    /// Naming the row — not just the code — is what binds each documented
    /// command-family mapping to runtime behavior: a row whose condition is raised
    /// by a typed error is verified here against the classifier that command's
    /// dispatch actually runs. The rows whose conditions are raised by
    /// crate-private errors (`TransitionBlockedError`) or by direct
    /// `std::process::exit` sites are bound end-to-end instead, by the subprocess
    /// tests in `tests/command_exit_code_projection_tests.rs`; the guard there
    /// asserts every row is covered by one binding or the other.
    fn classifier_cases() -> Vec<(anyhow::Error, i32, &'static str)> {
        use std::io::{Error as IoError, ErrorKind};
        vec![
            // Gate-evaluation exception rows route through `error_to_exit_code`:
            // a checker that failed (`Failed`) is `4`; one that could not run
            // (`Error`) is `10`.
            (
                gate_pass_failed(GateRunStatus::Failed),
                4,
                "gate evaluate, gate evaluate-all",
            ),
            (
                gate_pass_failed(GateRunStatus::Error),
                10,
                "gate evaluate, gate evaluate-all",
            ),
            // Universal rows: raised from shared paths, reachable from any command.
            (anyhow::anyhow!("untyped failure"), 1, "*"),
            (
                jit::errors::InvalidArgumentError::new("bad arg").into(),
                2,
                "*",
            ),
            (
                jit::storage::AmbiguousIdError::issue(
                    "aaaa",
                    ["aaaa1111".to_string(), "aaaa2222".to_string()],
                )
                .into(),
                2,
                "*",
            ),
            (jit::storage::InvalidIdPrefixError::new("ab").into(), 2, "*"),
            (
                jit::commands::GateNotRequiredError {
                    issue_id: "abc123".to_string(),
                    gate_key: "tests".to_string(),
                }
                .into(),
                2,
                "*",
            ),
            (
                jit::storage::IssueNotFoundError::new("abc123").into(),
                3,
                "*",
            ),
            (IoError::new(ErrorKind::NotFound, "missing").into(), 3, "*"),
            // `validate_for_write` is the shared write-validation path, so every
            // command that writes an issue — and only those — can reject a
            // blocking rule finding with this type.
            (
                jit::errors::ValidationFailedError::new("bad").into(),
                4,
                "any command that writes an issue",
            ),
            (
                IoError::new(ErrorKind::PermissionDenied, "denied").into(),
                5,
                "*",
            ),
            (
                jit::storage::RepositoryFormatTooNewError::new(9999, 1).into(),
                10,
                "*",
            ),
            (
                jit::errors::StaleBinaryError::new(
                    "abc123",
                    "tests",
                    &jit::domain::build_provenance::StaleBinaryReason::CommitMismatch {
                        built_from: "a".repeat(40),
                        head: "b".repeat(40),
                    },
                )
                .into(),
                10,
                "*",
            ),
            // Command-family rows, each pinned by the error that command raises.
            (jit::GraphError::CycleDetected.into(), 4, "dep add"),
            (
                jit::errors::RedundantDependencyError::new(
                    ("aaaa1111".to_string(), "bbbb2222".to_string()),
                    vec![("aaaa1111".to_string(), "cccc3333".to_string())],
                )
                .into(),
                4,
                "dep add",
            ),
            (
                jit::storage::GateAlreadyExistsError::new("tests").into(),
                6,
                "gate define",
            ),
            (
                jit::errors::AlreadyExistsError::new("Output path already exists: taken").into(),
                6,
                "snapshot export",
            ),
            (
                jit::commands::BatchValidationError {
                    problems: vec![jit::commands::BatchValidationProblem::UnknownDependency {
                        key: "a".to_string(),
                        missing: "ghost".to_string(),
                    }],
                }
                .into(),
                2,
                "issue batch-create",
            ),
            (
                jit::commands::BatchWriteError {
                    created: vec![("a".to_string(), "aaaa1111".to_string())],
                    failed_key: "b".to_string(),
                    stage: "create".to_string(),
                    reason: "disk full".to_string(),
                }
                .into(),
                10,
                "issue batch-create",
            ),
            (
                jit::errors::ClaimRequiresGitError::new(
                    jit::errors::GitRequirementGap::NoRepository,
                )
                .into(),
                10,
                "claim",
            ),
        ]
    }

    #[test]
    fn test_error_to_exit_code_produces_documented_codes() {
        for (error, expected, _row) in classifier_cases() {
            assert_eq!(
                error_to_exit_code(&error).code(),
                expected,
                "classifier produced the wrong code for `{error}`"
            );
        }
    }

    /// Every classifier case must land on the exact projected row it documents —
    /// same command family, same code. Code-level membership is not enough: a row
    /// that names the wrong command family, or a command family whose error is
    /// reclassified, must fail here.
    #[test]
    fn test_command_exit_codes_documents_every_classified_row() {
        let schema = CommandSchema::generate();
        for (error, _expected, row) in classifier_cases() {
            let actual = error_to_exit_code(&error).code();
            assert!(
                schema
                    .command_exit_codes
                    .iter()
                    .any(|c| c.command == row && c.code == Some(actual)),
                "classifier emits code {actual} for `{error}`, but the \
                 command_exit_codes projection has no `{row}` row documenting it"
            );
        }
    }
}
