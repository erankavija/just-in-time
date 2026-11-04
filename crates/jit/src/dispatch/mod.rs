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
//! status in `main()`. A leaf whose `--json` success carries a top-level
//! `message` returns it in [`Outcome::Success`]'s `message`, and the others
//! pass `None`.

use crate::commands::CommandExecutor;
use crate::output::{ErrorCode, OutputMode, Report};
use crate::schema::OutputSchema;
use crate::storage::IssueStore;

/// What every handler receives besides its parsed arguments.
pub struct CommandContext<'a, S: IssueStore> {
    pub executor: &'a CommandExecutor<S>,
    pub mode: OutputMode,
}

/// A handler's result, rendered by [`crate::output::emit_outcome`].
pub enum Outcome<R> {
    /// `message`, when present, is the `--json` success envelope's top-level
    /// `message`; the human rendering ignores it.
    Success {
        response: R,
        message: Option<String>,
    },
    /// Findings that fail the command: `code` is the envelope code and exit
    /// status, and `report` is the envelope's `error.details`.
    Findings {
        code: ErrorCode,
        message: String,
        report: Box<dyn Report>,
    },
}

/// The `output_schema` lookup of every migrated group, consulted by
/// [`crate::CommandSchema::generate`] before its legacy table. Test builds
/// also register the test-only group, whose leaf paths no real command uses.
pub const REGISTERED_GROUPS: &[fn(&str) -> Option<OutputSchema>] = &[
    hooks::output_schema,
    invariant::output_schema,
    #[cfg(test)]
    test_group::output_schema,
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
/// It generates `dispatch(command, ctx, out, err) -> anyhow::Result<ExitCode>`,
/// which runs the leaf's handler and renders its outcome through
/// [`crate::output::emit_outcome`], and `output_schema(cmd_path)`, which
/// returns the leaf's response schema.
// Child modules declared below reach it by textual scope.
macro_rules! command_group {
    (
        command: $command:ty,
        storage: [$($bound:tt)+],
        leaves: {
            $( $path:literal => $pattern:pat => $handler:ident ( $($arg:expr),* $(,)? ) -> $response:ty ),+ $(,)?
        }
    ) => {
        pub fn dispatch<S: $($bound)+>(
            command: $command,
            ctx: &$crate::dispatch::CommandContext<'_, S>,
            out: &mut dyn ::std::io::Write,
            err: &mut dyn ::std::io::Write,
        ) -> ::anyhow::Result<$crate::output::ExitCode> {
            match command {
                $( $pattern => {
                    let outcome: $crate::dispatch::Outcome<$response> = $handler(ctx, $($arg),*)?;
                    $crate::output::emit_outcome(&outcome, ctx.mode, out, err)
                } )+
            }
        }

        pub fn output_schema(cmd_path: &str) -> Option<$crate::schema::OutputSchema> {
            match cmd_path {
                $( $path => Some($crate::schema::OutputSchema::of::<$response>()), )+
                _ => None,
            }
        }
    };
}

pub mod hooks;
pub mod invariant;
#[cfg(test)]
mod test_group;
