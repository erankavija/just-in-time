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

use anyhow::{Context, Result};
use clap::Parser;
use jit::cli::{Cli, Commands, IssueCommands, ProfileCommands};
use jit::dispatch::{
    apply, archive, claim, config, dep, doc, events, gate, graph, hooks, init, invariant, issue,
    item, label, profile, project, query, search, serve, snapshot, validate, version,
    CommandContext,
};
use jit::output::failure::classify;
use jit::output::{ExitCode, OutputMode};
use std::env;
use std::io;
use std::path::PathBuf;

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
        Commands::Issue(command) => {
            return issue::dispatch(command, &ctx, &mut io::stdout(), &mut io::stderr());
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
