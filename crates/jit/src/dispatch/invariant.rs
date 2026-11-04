//! `jit invariant` handlers.

use super::{CommandContext, HandlerResult, HandlerStore, Outcome};
use crate::cli::InvariantCommands;
use crate::commands::InvariantCheckResult;
use crate::output::ErrorCode;

/// Enforcement drift is a findings outcome; a clean registry succeeds.
fn check<S: HandlerStore>(ctx: &CommandContext<'_, S>) -> HandlerResult<InvariantCheckResult> {
    let result = ctx.executor.items().check_invariants()?;
    Ok(if result.has_drift() {
        Outcome::findings(
            ErrorCode::ValidationFailed,
            format!("Enforcement drift: {} finding(s)", result.count()),
            result,
        )
    } else {
        Outcome::success_with(result, "No enforcement drift")
    })
}

command_group! {
    command: InvariantCommands,
    storage: [HandlerStore],
    leaves: {
        "invariant_check" => InvariantCommands::Check => check()
            findings ValidationFailed "Enforcement drift was found (declared enforcement not backed by an enforcing rule)." -> InvariantCheckResult,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{memory_executor_declaring, MemoryDeclarations};
    use crate::output::{ExitCode, OutputMode};
    use crate::storage::InMemoryStorage;

    #[test]
    fn test_dispatch_check_drift_returns_validation_failed_findings_exit_4() {
        let executor = memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                invariants: Some(
                    "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\n\
                     kind = \"enforced\"\nenforced-by = \"@/rule/ghost-rule\"\n",
                ),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let ctx = CommandContext {
            executor: &executor,
            mode: OutputMode {
                json: true,
                quiet: false,
            },
        };
        let mut out = Vec::new();

        let status = dispatch(InvariantCommands::Check, &ctx, &mut out, &mut Vec::new()).unwrap();

        let printed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(
            printed["error"]["code"],
            serde_json::to_value(ErrorCode::ValidationFailed).unwrap()
        );
        assert_eq!(status, ExitCode::ValidationFailed.into());
        assert_eq!(status.code(), 4);
    }

    #[test]
    fn test_registered_output_schema_invariant_check_is_named_for_its_result_type() {
        let registered = crate::dispatch::registered_output_schema("invariant_check").unwrap();

        assert_eq!(registered.success, "InvariantCheckResult");
    }
}
