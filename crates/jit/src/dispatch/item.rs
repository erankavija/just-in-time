//! `jit item` handlers.

use super::{CommandContext, HandlerResult, Outcome};
use crate::cli::ItemCommands;
use crate::commands::{ItemListResult, ItemShowResult};
use crate::storage::IssueStore;

fn list<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    kind: Option<String>,
) -> HandlerResult<ItemListResult> {
    let response = ctx.executor.items().list_items(kind.as_deref())?;
    Ok(list_outcome(response))
}

fn search<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    query: String,
    kind: Option<String>,
) -> HandlerResult<ItemListResult> {
    let response = ctx.executor.items().search_items(&query, kind.as_deref())?;
    Ok(list_outcome(response))
}

/// A list's `--json` success carries its count as the top-level message.
fn list_outcome(response: ItemListResult) -> Outcome<ItemListResult> {
    let message = format!("Found {} item(s)", response.count());
    Outcome::success_with(response, message)
}

fn show<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    qualified_id: String,
) -> HandlerResult<ItemShowResult> {
    Ok(ctx.executor.items().show_item(&qualified_id)?.into())
}

command_group! {
    command: ItemCommands,
    storage: [IssueStore],
    leaves: {
        "item_list" => ItemCommands::List { kind } => list(kind) -> ItemListResult,
        "item_search" => ItemCommands::Search { query, kind } => search(query, kind) -> ItemListResult,
        "item_show" => ItemCommands::Show { qualified_id } => show(qualified_id) -> ItemShowResult,
        "item_resolve" => ItemCommands::Resolve { qualified_id } => show(qualified_id) -> ItemShowResult,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{memory_executor_declaring, MemoryDeclarations};
    use crate::output::failure::classify;
    use crate::output::{ErrorCode, ExitCode, OutputMode};
    use crate::storage::InMemoryStorage;

    fn run_with_config(
        config: &str,
        command: ItemCommands,
        json: bool,
    ) -> anyhow::Result<ExitCode> {
        let executor = memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(config),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let ctx = CommandContext {
            executor: &executor,
            mode: OutputMode { json, quiet: false },
        };
        dispatch(command, &ctx, &mut Vec::new(), &mut Vec::new())
    }

    #[test]
    fn test_dispatch_list_malformed_config_returns_invalid_argument_in_json_and_human() {
        for json in [true, false] {
            let error = run_with_config(
                "not = valid = toml\n",
                ItemCommands::List { kind: None },
                json,
            )
            .unwrap_err();

            assert_eq!(classify(&error).error.code, ErrorCode::InvalidArgument);
        }
    }

    #[test]
    fn test_dispatch_show_missing_item_returns_item_not_found_in_json_and_human() {
        for json in [true, false] {
            let error = run_with_config(
                "",
                ItemCommands::Show {
                    qualified_id: "@/invariant/absent".to_owned(),
                },
                json,
            )
            .unwrap_err();

            assert_eq!(classify(&error).error.code, ErrorCode::ItemNotFound);
        }
    }
}
