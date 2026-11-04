//! Library-side command handlers, one module per command group.
//!
//! A handler takes a [`CommandContext`] and its parsed arguments and returns
//! `anyhow::Result<Outcome<R>>` for its registered response type `R`. Each
//! group declares its leaves once with `command_group!`, which generates the
//! group's `dispatch` and its `output_schema` lookup; a group joins the schema
//! by adding that lookup to [`REGISTERED_GROUPS`].
//!
//! `main.rs` builds one [`CommandContext`] after bootstrap and gives each
//! migrated group one arm,
//! `return group::dispatch(command, &ctx, &mut io::stdout(), &mut io::stderr())`;
//! the returned [`ExitCode`](crate::output::ExitCode) becomes the process
//! status in `main()`. Handlers build successes with [`Outcome::success`] and
//! [`Outcome::success_with`]; a `message` becomes the `--json` envelope's
//! top-level `message`, and [`Outcome::with_warnings`] attaches warnings the
//! emitter renders (human) or lists (`--json`). Handlers resolve issue-id arguments with
//! [`resolve_issue`].

use crate::commands::CommandExecutor;
use crate::domain::identity::{IssueId, IssueIdPrefix};
use crate::output::{ErrorCode, ExitCode, OutputMode, Report};
use crate::schema::OutputSchema;
use crate::storage::{IssueNotFoundError, IssueStore};

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
}

impl<R> Outcome<R> {
    /// A success with no `--json` message.
    pub fn success(response: R) -> Self {
        Self::Success {
            response,
            message: None,
            warnings: Vec::new(),
        }
    }

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

    /// This outcome with `warnings` appended.
    #[must_use]
    pub fn with_warnings<I>(mut self, warnings: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        let (Self::Success { warnings: own, .. } | Self::Findings { warnings: own, .. }) =
            &mut self;
        own.extend(warnings.into_iter().map(Into::into));
        self
    }
}

/// What a split group's `dispatch` returns: the exit status of a variant it
/// handled, or the command back for the caller's remaining arm.
///
/// Transitional: part of the `unmigrated:` form of `command_group!`, which
/// 5687e281 removes once every group owns all its variants.
pub enum Dispatched<C> {
    Handled(ExitCode),
    Unhandled(C),
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

/// The `output_schema` lookup of every migrated group, consulted by
/// [`crate::CommandSchema::generate`] before its legacy table. Test builds
/// also register the test-only groups, whose leaf paths no real command uses.
pub const REGISTERED_GROUPS: &[fn(&str) -> Option<OutputSchema>] = &[
    config::output_schema,
    invariant::output_schema,
    snapshot::output_schema,
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
/// `dispatch(command, ctx, out, err) -> anyhow::Result<ExitCode>` and
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
///
/// A split group (transitional; 5687e281 removes this clause) ends its
/// invocation with `, unmigrated: <pattern>` naming the variants it does not
/// own. Its `dispatch` returns `anyhow::Result<`[`Dispatched`]`<command>>`,
/// giving those variants back unchanged.
// Child modules declared below reach it by textual scope.
macro_rules! command_group {
    (
        command: $command:ty,
        storage: [$($bound:tt)+],
        leaves: { $($leaves:tt)+ }
        $(, unmigrated: $rest:pat)? $(,)?
    ) => {
        command_group!(@generate
            generics [<S: $($bound)+>]
            params [ctx: &$crate::dispatch::CommandContext<'_, S>,]
            mode [ctx.mode]
            handler_args [ctx,]
            command $command,
            unmigrated [$($rest)?]
            leaves { $($leaves)+ }
        );
    };
    (
        command: $command:ty,
        storage: $storage:ty,
        leaves: { $($leaves:tt)+ }
        $(, unmigrated: $rest:pat)? $(,)?
    ) => {
        command_group!(@generate
            generics []
            params [ctx: &$crate::dispatch::CommandContext<'_, $storage>,]
            mode [ctx.mode]
            handler_args [ctx,]
            command $command,
            unmigrated [$($rest)?]
            leaves { $($leaves)+ }
        );
    };
    (
        command: $command:ty,
        leaves: { $($leaves:tt)+ }
        $(, unmigrated: $rest:pat)? $(,)?
    ) => {
        command_group!(@generate
            generics []
            params [mode: $crate::output::OutputMode,]
            mode [mode]
            handler_args []
            command $command,
            unmigrated [$($rest)?]
            leaves { $($leaves)+ }
        );
    };
    (@generate
        generics $generics:tt params $params:tt mode $mode:tt handler_args $handler_args:tt
        command $command:ty,
        unmigrated []
        leaves $leaves:tt
    ) => {
        command_group!(@body
            generics $generics params $params mode $mode handler_args $handler_args
            command $command,
            returns [::anyhow::Result<$crate::output::ExitCode>]
            lift [::std::convert::identity]
            rest []
            leaves $leaves
        );
    };
    (@generate
        generics $generics:tt params $params:tt mode $mode:tt handler_args $handler_args:tt
        command $command:ty,
        unmigrated [$rest:pat]
        leaves $leaves:tt
    ) => {
        command_group!(@body
            generics $generics params $params mode $mode handler_args $handler_args
            command $command,
            returns [::anyhow::Result<$crate::dispatch::Dispatched<$command>>]
            lift [$crate::dispatch::Dispatched::Handled]
            rest [unhandled @ ($rest) => Ok($crate::dispatch::Dispatched::Unhandled(unhandled)),]
            leaves $leaves
        );
    };
    (@body
        generics [$($generics:tt)*] params [$($params:tt)*] mode [$mode:expr]
        handler_args $handler_args:tt
        command $command:ty,
        returns [$returns:ty]
        lift [$lift:expr]
        rest [$($rest:tt)*]
        leaves {
            $(
                $path:literal => $pattern:pat => $handler:ident ( $($arg:expr),* $(,)? ) -> $response:ty
                    $(as union [$($member:ty),+ $(,)?] $description:literal)?
            ),+ $(,)?
        }
    ) => {
        #[allow(unused_parens)]
        pub fn dispatch $($generics)* (
            command: $command,
            $($params)*
            out: &mut dyn ::std::io::Write,
            err: &mut dyn ::std::io::Write,
        ) -> $returns {
            match command {
                $( $pattern => {
                    let outcome: $crate::dispatch::Outcome<$response> =
                        command_group!(@call $handler $handler_args ($($arg),*))?;
                    $crate::output::emit_outcome(&outcome, $mode, out, err).map($lift)
                } )+
                $($rest)*
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
    (@schema $response:ty, [$($member:ty),+] $description:literal) => {
        $crate::schema::OutputSchema::union_of(
            vec![$($crate::schema::UnionMember::of::<$member>()),+],
            $description,
        )
    };
}

pub mod config;
pub mod invariant;
pub mod snapshot;
#[cfg(test)]
mod test_forms;
#[cfg(test)]
mod test_group;
