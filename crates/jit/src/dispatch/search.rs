//! `jit search` handler.

use super::{CommandContext, Outcome};
use crate::output::{SearchHit, SearchQuery, SearchResponse};
use crate::search::SearchOptions;
use crate::storage::IssueStore;

/// The matches are the payload; the count is the JSON `message`.
fn search<S: IssueStore>(
    ctx: &CommandContext<'_, S>,
    query: String,
    options: SearchOptions,
) -> anyhow::Result<Outcome<SearchResponse>> {
    let matches = ctx.executor.queries().search_content(&query, options)?;
    let message = format!("Found {} result(s)", matches.len());
    let items = matches
        .into_iter()
        .map(|found| SearchHit {
            result: found.result,
            issue_title: found.issue_title,
        })
        .collect();
    Ok(Outcome::Success {
        response: SearchResponse::with_extras(items, SearchQuery { query }),
        message: Some(message),
    })
}
