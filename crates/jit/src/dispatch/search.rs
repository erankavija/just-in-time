//! `jit search` handler.

use super::{count_message, CommandContext, HandlerResult, Outcome};
use crate::cli::SearchArgs;
use crate::output::{SearchHit, SearchQuery, SearchResponse};
use crate::search::SearchOptions;
use crate::storage::IssueStore;

/// The matches are the payload; the count is the JSON `message`.
fn search<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    args: SearchArgs,
) -> HandlerResult<SearchResponse> {
    let SearchArgs {
        query,
        regex,
        case_sensitive,
        context,
        limit,
        glob,
    } = args;
    let options = SearchOptions {
        case_sensitive,
        regex,
        context_lines: context,
        max_results: limit,
        file_pattern: glob,
        file_patterns: Vec::new(),
    };
    let matches = ctx.executor.queries().search_content(&query, options)?;
    let message = count_message(matches.len(), "Found ", " result(s)");
    let items = matches
        .into_iter()
        .map(|found| SearchHit {
            result: found.result,
            issue_title: found.issue_title,
        })
        .collect();
    Ok(Outcome::success_with(
        SearchResponse::with_extras(items, SearchQuery { query }),
        message,
    ))
}

command_group! {
    command: SearchArgs,
    storage: [IssueStore],
    leaves: {
        "search" => args => search(args) -> SearchResponse,
    }
}
