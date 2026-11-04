//! Library-side command handlers, one module per command group.
//!
//! A handler takes a [`CommandContext`] and its parsed arguments and returns
//! [`HandlerResult<R>`] for its registered response type `R`. Each
//! group declares its leaves once with `command_group!`, which generates the
//! group's `dispatch` and its `output_schema` lookup; a group joins the schema
//! by adding that lookup to [`REGISTERED_GROUPS`].
//!
//! `main.rs` opens the repository with [`bootstrap`] and hands it to
//! [`dispatch`], which routes the command to its group; the returned
//! [`RunStatus`] becomes the process status in `main()`. Handlers build
//! successes with `Outcome::from` (or `.into()`) and
//! [`Outcome::success_with`]; a `message` becomes the `--json` envelope's
//! top-level `message`, and [`Outcome::with_warnings`] attaches warnings the
//! emitter renders (human) or lists (`--json`). Handlers resolve issue-id arguments with
//! [`resolve_issue`].

use crate::cli::{Cli, Commands, IssueCommands, ProfileCommands};
use crate::commands::{open_repository, CommandExecutor, OpenedRepository, RepositoryInvocation};
use crate::config_manager::ConfigManager;
use crate::domain::identity::{IssueId, IssueIdPrefix};
use crate::output::{status_after, ErrorCode, ExitCode, OutputMode, Report, RunStatus};
use crate::schema::OutputSchema;
use crate::storage::{IssueNotFoundError, IssueStore, RepositoryStateStore};
use anyhow::{anyhow, Context};
use std::io::Write;
use std::path::PathBuf;

/// The return type of every handler.
pub type HandlerResult<R> = anyhow::Result<Outcome<R>>;

/// The store bound handlers share when they read or write repository state.
pub trait HandlerStore: IssueStore + RepositoryStateStore {}
impl<S: IssueStore + RepositoryStateStore> HandlerStore for S {}

/// What every handler receives besides its parsed arguments.
pub struct CommandContext<'a, S: IssueStore> {
    pub executor: &'a CommandExecutor<S>,
    pub mode: OutputMode,
}

/// A handler's result, rendered by [`crate::output::emit_outcome`].
pub enum Outcome<R> {
    /// `message`, when present, is the `--json` success envelope's top-level
    /// `message`; the human rendering ignores it. `warnings` print as
    /// `Warning:` lines on stderr in human, non-quiet mode; under `--json`,
    /// non-empty `warnings` are the envelope's top-level `warnings` array.
    Success {
        response: R,
        message: Option<String>,
        warnings: Vec<String>,
    },
    /// Findings that fail the command: `code` is the envelope code and exit
    /// status, and `report` is the envelope's `error.details`. `warnings`
    /// render as on a success, beside `error` under `--json`.
    Findings {
        code: ErrorCode,
        message: String,
        report: Box<dyn Report>,
        warnings: Vec<String>,
    },
    /// A child process ran on this process's streams: nothing renders, and its
    /// exit status is the run's.
    Exited(i32),
}

/// A success with no `--json` message.
impl<R> From<R> for Outcome<R> {
    fn from(response: R) -> Self {
        Self::Success {
            response,
            message: None,
            warnings: Vec::new(),
        }
    }
}

impl<R> Outcome<R> {
    /// A success whose `--json` envelope carries `message`.
    pub fn success_with(response: R, message: impl Into<String>) -> Self {
        Self::Success {
            response,
            message: Some(message.into()),
            warnings: Vec::new(),
        }
    }

    /// Findings that fail the command, with no warnings.
    pub fn findings(
        code: ErrorCode,
        message: impl Into<String>,
        report: impl Report + 'static,
    ) -> Self {
        Self::Findings {
            code,
            message: message.into(),
            report: Box::new(report),
            warnings: Vec::new(),
        }
    }

    /// This outcome with `warnings` appended; an `Exited` outcome renders none.
    #[must_use]
    pub fn with_warnings<I>(mut self, warnings: I) -> Self
    where
        I: IntoIterator,
        I::Item: std::fmt::Display,
    {
        if let Self::Success { warnings: own, .. } | Self::Findings { warnings: own, .. } =
            &mut self
        {
            own.extend(warnings.into_iter().map(|warning| warning.to_string()));
        }
        self
    }
}

/// Open the repository `cli`'s command runs over; `--schema`, a missing
/// command, `version` and `init` run without one.
///
/// # Errors
///
/// An unusable linked-checkout write token, or [`open_repository`]'s failure.
pub fn bootstrap(cli: &Cli) -> anyhow::Result<Option<OpenedRepository>> {
    let command = match &cli.command {
        Some(Commands::Version | Commands::Init(_)) | None => return Ok(None),
        Some(_) if cli.schema => return Ok(None),
        Some(command) => command,
    };
    open_repository(RepositoryInvocation {
        current_dir: std::env::current_dir()?,
        data_dir_override: std::env::var("JIT_DATA_DIR").ok().map(PathBuf::from),
        is_init: false,
        opens_mutation_session: command.requires_recovery_dispatch(),
        coordinates_claims_first: command.coordinates_claims_first(),
        is_validation_exempt: matches!(command, Commands::Profile(ProfileCommands::Show { .. })),
        is_recover: matches!(command, Commands::Recover),
        // Read before any repository work, so an unusable token fails the
        // invocation even for a command that never writes.
        write_override: ConfigManager::invocation_write_override()?,
    })
    .map(Some)
}

/// Run `cli`'s command over the `repository` [`bootstrap`] opened for it and
/// render its outcome; the repository's guards drop before this returns.
///
/// # Errors
///
/// The handler's failure, for the caller to render.
pub fn dispatch(
    cli: Cli,
    mut repository: Option<OpenedRepository>,
    mode: OutputMode,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> anyhow::Result<RunStatus> {
    if cli.schema {
        let schema = serde_json::to_string_pretty(&crate::CommandSchema::generate())?;
        return Ok(status_after(
            writeln!(out, "{schema}"),
            ExitCode::Success.into(),
        )?);
    }
    let command = cli
        .command
        .ok_or_else(|| anyhow!("No command provided. Use --help for usage."))?;
    // The server child runs its own startup recovery, so holding this session
    // across the wait deadlocks the child against the parent.
    if let Some(guard) = repository
        .as_mut()
        .filter(|_| matches!(command, Commands::Serve(_)))
        .and_then(|repository| repository.retained_session.take())
    {
        guard.release()?;
    }
    let opened = repository.as_ref().map(|repository| CommandContext {
        executor: &repository.executor,
        mode,
    });
    let ctx = || opened.as_ref().context("bootstrap opened no repository");
    match command {
        Commands::Version => version::dispatch((), mode, out, err),
        Commands::Init(args) => init::dispatch(args, mode, out, err),
        Commands::Profile(command) => profile::dispatch(command, ctx()?, out, err),
        Commands::List { filters, full } => {
            issue::dispatch(IssueCommands::List { filters, full }, ctx()?, out, err)
        }
        Commands::Issue(command) => issue::dispatch(command, ctx()?, out, err),
        Commands::Apply(args) => apply::dispatch(args, ctx()?, out, err),
        Commands::Dep(command) => dep::dispatch(command, ctx()?, out, err),
        Commands::Gate(command) => gate::dispatch(command, ctx()?, out, err),
        Commands::Graph(command) => graph::dispatch(command, ctx()?, out, err),
        Commands::Rdeps { id, depth } => {
            graph::rdeps::dispatch(graph::RdepsArgs { id, depth }, ctx()?, out, err)
        }
        Commands::Events(command) => events::dispatch(command, ctx()?, out, err),
        Commands::Doc(command) => doc::dispatch(command, ctx()?, out, err),
        Commands::Archive(command) => archive::dispatch(command, ctx()?, out, err),
        Commands::Query(args) => query::run(args, ctx()?, out, err),
        Commands::Label(command) => label::dispatch(command, ctx()?, out, err),
        Commands::Config(command) => config::dispatch(command, ctx()?, out, err),
        Commands::Hooks(command) => hooks::dispatch(command, ctx()?, out, err),
        Commands::Item(command) => item::dispatch(command, ctx()?, out, err),
        Commands::Invariant(command) => invariant::dispatch(command, ctx()?, out, err),
        Commands::Project(command) => project::dispatch(command, ctx()?, out, err),
        Commands::Search(args) => search::dispatch(args, ctx()?, out, err),
        Commands::Status => query::status::dispatch((), ctx()?, out, err),
        Commands::Validate(args) => validate::dispatch(args, ctx()?, out, err),
        Commands::Recover => claim::recover::dispatch((), ctx()?, out, err),
        Commands::Serve(args) => serve::dispatch(args, ctx()?, out, err),
        Commands::Claim(command) => claim::dispatch(command, ctx()?, out, err),
        Commands::Worktree(command) => worktree::dispatch(command, ctx()?, out, err),
        Commands::Snapshot(command) => snapshot::dispatch(command, ctx()?, out, err),
    }
}

/// Parse and resolve one user-supplied issue-id argument: the CLI's single
/// prefix boundary, applied before any executor call.
///
/// # Errors
///
/// [`InvalidIdPrefixError`](crate::domain::identity::InvalidIdPrefixError)
/// before any read, then the resolver's not-found or ambiguous-id error.
pub fn resolve_issue<S: IssueStore>(storage: &S, requested: &str) -> anyhow::Result<IssueId> {
    storage.resolve_issue_id(&requested.parse::<IssueIdPrefix>()?)
}

/// Parse an issue-id filter into the prefix that matches stored ids: the
/// canonical id it resolves to or, when it names no live issue, the parsed
/// input itself, so records of a deleted issue still match.
///
/// # Errors
///
/// [`InvalidIdPrefixError`](crate::domain::identity::InvalidIdPrefixError)
/// before any read, or [`AmbiguousIdError`](crate::storage::AmbiguousIdError).
pub fn resolve_issue_filter<S: IssueStore>(
    storage: &S,
    requested: &str,
) -> anyhow::Result<IssueIdPrefix> {
    let prefix = requested.parse::<IssueIdPrefix>()?;
    match storage.resolve_issue_id(&prefix) {
        Ok(id) => Ok(IssueIdPrefix::from(&id)),
        Err(error) if error.is::<IssueNotFoundError>() => Ok(prefix),
        Err(error) => Err(error),
    }
}

/// Select the positional-or-flag gate argument before contextual validation.
///
/// Returns an invalid-argument error when both or neither are provided.
pub fn resolve_gate_key(
    positional: Option<String>,
    flag: Option<String>,
    command: &str,
) -> anyhow::Result<String> {
    let usage = format!(
        "Usage: jit {command} <ISSUE_ID> <GATE_KEY>\n\
         Usage: jit {command} <ISSUE_ID> --gate <GATE_KEY>"
    );
    let message = match (positional, flag) {
        (Some(value), None) | (None, Some(value)) => return Ok(value),
        (Some(_), Some(_)) => {
            "provide the gate key as a positional argument OR via --gate, not both."
        }
        (None, None) => {
            "a gate key is required; provide it as a positional argument or via --gate."
        }
    };
    Err(crate::errors::InvalidArgumentError::new(format!("{message}\n{usage}")).into())
}

/// Select and parse one positional-or-flag gate key.
pub fn resolve_gate_key_for(
    positional: Option<String>,
    flag: Option<String>,
    command: &str,
) -> anyhow::Result<crate::domain::identity::GateKey> {
    Ok(resolve_gate_key(positional, flag, command)?.parse()?)
}

/// The `output_schema` lookup of every migrated group, consulted by
/// [`crate::CommandSchema::generate`] before its legacy table. Test builds
/// also register the test-only groups, whose leaf paths no real command uses.
pub const REGISTERED_GROUPS: &[fn(&str) -> Option<OutputSchema>] = &[
    apply::output_schema,
    archive::output_schema,
    claim::output_schema,
    claim::recover::output_schema,
    config::output_schema,
    dep::output_schema,
    doc::output_schema,
    events::output_schema,
    gate::output_schema,
    graph::output_schema,
    graph::rdeps::output_schema,
    hooks::output_schema,
    init::output_schema,
    invariant::output_schema,
    profile::output_schema,
    query::output_schema,
    query::bare::output_schema,
    query::status::output_schema,
    issue::output_schema,
    issue::alias_output_schema,
    snapshot::output_schema,
    item::output_schema,
    label::output_schema,
    project::output_schema,
    search::output_schema,
    serve::output_schema,
    validate::output_schema,
    version::output_schema,
    worktree::output_schema,
    #[cfg(test)]
    test_group::output_schema,
    #[cfg(test)]
    test_forms::output_schema,
];

/// The registered success schema of the leaf at `cmd_path`, if any group
/// registers it.
pub fn registered_output_schema(cmd_path: &str) -> Option<OutputSchema> {
    REGISTERED_GROUPS.iter().find_map(|lookup| lookup(cmd_path))
}

/// Register one command group from its single leaf list.
///
/// Each leaf names its schema path, the command pattern, the handler call over
/// the pattern's bindings, and its response type:
///
/// ```text
/// command_group! {
///     command: ItemCommands,
///     storage: [IssueStore],
///     leaves: {
///         "item_show" => ItemCommands::Show { id } => show(id) -> ItemShowResponse,
///     }
/// }
/// ```
///
/// The `storage:` entry picks the group's form, which generates
/// `dispatch(command, ctx, out, err) -> anyhow::Result<RunStatus>` and
/// `output_schema(cmd_path)`. `dispatch` runs the leaf's handler and renders
/// its outcome through [`crate::output::emit_outcome`]; `output_schema`
/// returns the leaf's response schema.
///
/// - `storage: [Bounds]`: handlers are generic over `S: Bounds` and take the
///   [`CommandContext`] first.
/// - `storage: Type`: handlers take a `CommandContext<'_, Type>`, for
///   operations only that store implements.
/// - No `storage:` entry (executor-free): for leaves that run before
///   repository bootstrap. Handlers take no context and `dispatch(command,
///   mode, out, err)` takes the [`OutputMode`] in its place.
///
/// A top-level `Commands` variant registers as a single leaf: `command:` is
/// the variant's argument type (a `clap::Args` struct carrying its fields, or
/// `()` for a unit variant), and the leaf's pattern binds the arguments
/// (`args`) or matches `()`.
///
/// A leaf whose success is one of several shapes ends with
/// `as union [A, B] "description"`; the schema is a `oneOf` over the listed
/// types, and the handler's response type serializes as whichever applies.
// Child modules declared below reach it by textual scope.
macro_rules! command_group {
    (
        command: $command:ty,
        storage: [$($bound:tt)+],
        leaves: { $($leaves:tt)+ } $(,)?
    ) => {
        command_group!(@body
            generics [<S: $($bound)+>]
            params [ctx: &$crate::dispatch::CommandContext<'_, S>,]
            mode [ctx.mode]
            handler_args [ctx,]
            command $command,
            leaves { $($leaves)+ }
        );
    };
    (
        command: $command:ty,
        storage: $storage:ty,
        leaves: { $($leaves:tt)+ } $(,)?
    ) => {
        command_group!(@body
            generics []
            params [ctx: &$crate::dispatch::CommandContext<'_, $storage>,]
            mode [ctx.mode]
            handler_args [ctx,]
            command $command,
            leaves { $($leaves)+ }
        );
    };
    (
        command: $command:ty,
        leaves: { $($leaves:tt)+ } $(,)?
    ) => {
        command_group!(@body
            generics []
            params [mode: $crate::output::OutputMode,]
            mode [mode]
            handler_args []
            command $command,
            leaves { $($leaves)+ }
        );
    };
    (@body
        generics [$($generics:tt)*] params [$($params:tt)*] mode [$mode:expr]
        handler_args $handler_args:tt
        command $command:ty,
        leaves {
            $(
                $path:literal => $pattern:pat => $handler:ident ( $($arg:expr),* $(,)? ) -> $response:ty
                    $(as union [$($member:ty),+ $(,)?] $description:expr)?
            ),+ $(,)?
        }
    ) => {
        #[allow(unused_parens)]
        pub fn dispatch $($generics)* (
            command: $command,
            $($params)*
            out: &mut dyn ::std::io::Write,
            err: &mut dyn ::std::io::Write,
        ) -> ::anyhow::Result<$crate::output::RunStatus> {
            match command {
                $( $pattern => {
                    let outcome: $crate::dispatch::Outcome<$response> =
                        command_group!(@call $handler $handler_args ($($arg),*))?;
                    $crate::output::emit_outcome(&outcome, $mode, out, err)
                } )+
            }
        }

        pub fn output_schema(cmd_path: &str) -> Option<$crate::schema::OutputSchema> {
            match cmd_path {
                $( $path => Some(command_group!(@schema $response $(, [$($member),+] $description)?)), )+
                _ => None,
            }
        }
    };
    (@call $handler:ident [$($handler_args:tt)*] ($($arg:expr),*)) => {
        $handler($($handler_args)* $($arg),*)
    };
    (@schema $response:ty) => {
        $crate::schema::OutputSchema::of::<$response>()
    };
    (@schema $response:ty, [$($member:ty),+] $description:expr) => {
        $crate::schema::OutputSchema::union_of(
            vec![$($crate::schema::UnionMember::of::<$member>()),+],
            $description,
        )
    };
}

pub mod apply;
pub mod archive;
pub mod claim;
pub mod config;
pub mod dep;
pub mod doc;
pub mod events;
pub mod gate;
pub mod graph;
pub mod hooks;
pub mod init;
pub mod invariant;
pub mod issue;
pub mod item;
pub mod label;
pub mod profile;
pub mod project;
pub mod query;
pub mod search;
pub mod serve;
pub mod snapshot;
#[cfg(test)]
mod test_forms;
#[cfg(test)]
mod test_group;
pub mod validate;
pub mod version;
pub mod worktree;
