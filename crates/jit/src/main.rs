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
use jit::cli::{Cli, Commands, IssueCommands, ProfileCommands};
use jit::commands::DescriptionUpdate;
use jit::dispatch::{
    apply, archive, claim, config, dep, doc, events, gate, graph, hooks, init, invariant, issue,
    item, label, profile, project, query, resolve_issue, search, serve, snapshot, validate,
    version, CommandContext, Dispatched,
};
use jit::domain::identity::{GateKey, IssueIdPrefix};
use jit::domain::{Assignee, Priority, State};
use jit::output::failure::{classify, command_json_error};
use jit::output::{ExitCode, JsonOutput, OutputContext, OutputMode};
use std::env;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;

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
                let failure = classify(&e);
                for warning in failure.warnings.iter().filter(|_| !mode.quiet) {
                    eprintln!("Warning: {warning}");
                }
                eprintln!("Error: {}", e);
                failure.exit_code()
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
        worktree_paths: _,
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
        Commands::Profile(command) => {
            return profile::dispatch(command, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::List { .. } => {
            unreachable!("top-level list is normalized to its canonical command above")
        }
        Commands::Version => unreachable!("version is handled before repository validation"),
        Commands::Issue(issue_cmd) => {
            let issue_cmd =
                match issue::dispatch(issue_cmd, &ctx, &mut io::stdout(), &mut io::stderr())? {
                    Dispatched::Handled(code) => return Ok(code),
                    Dispatched::Unhandled(rest) => rest,
                };
            match issue_cmd {
                IssueCommands::Show { .. }
                | IssueCommands::List { .. }
                | IssueCommands::Search { .. }
                | IssueCommands::Status { .. }
                | IssueCommands::Children { .. }
                | IssueCommands::Progress { .. } => {
                    unreachable!("the issue read commands are served by dispatch::issue")
                }
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
            }
        }
        Commands::Apply(args) => {
            return apply::dispatch(args, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Dep(command) => {
            return dep::dispatch(command, &ctx, &mut io::stdout(), &mut io::stderr());
        }
        Commands::Gate(command) => {
            return gate::dispatch(command, &ctx, &mut io::stdout(), &mut io::stderr());
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
        Commands::Query(args) => {
            return query::run(args, &ctx, &mut io::stdout(), &mut io::stderr());
        }
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
            return query::status::dispatch((), &ctx, &mut io::stdout(), &mut io::stderr());
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
        Commands::Worktree(command) => {
            match jit::dispatch::worktree::dispatch(
                command,
                &ctx,
                &mut io::stdout(),
                &mut io::stderr(),
            )? {
                jit::dispatch::Dispatched::Handled(code) => return Ok(code),
                jit::dispatch::Dispatched::Unhandled(command) => match command {
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
                    _ => unreachable!("registered worktree leaves are handled"),
                },
            }
        }
        Commands::Snapshot(command) => {
            return snapshot::dispatch(command, &ctx, &mut io::stdout(), &mut io::stderr());
        }
    }

    Ok(ExitCode::Success)
}
