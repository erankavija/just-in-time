//! Response and human rendering of the top-level `jit search` command.

use super::{HumanRender, OutputMode};
use crate::domain::list_envelope::{CollectionKey, ListEnvelope};
use crate::search::SearchResult;
use schemars::JsonSchema;
use serde::Serialize;

/// The collection key of a search response. Its schema name is its own, since
/// [`ResultsKey`](super::ResultsKey) is shared by other responses.
pub enum SearchKey {}

impl CollectionKey for SearchKey {
    const KEY: &'static str = "results";
    const SCHEMA_NAME: Option<&'static str> = Some("SearchResponse");
}

/// A match beside the title of its issue. The title is for human output only,
/// so the JSON entry is the match alone.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchHit {
    #[serde(flatten)]
    pub result: SearchResult,
    #[serde(skip)]
    pub issue_title: Option<String>,
}

/// Response for top-level `search` command
pub type SearchResponse = ListEnvelope<SearchKey, SearchHit, SearchQuery>;

/// The query a [`SearchResponse`] echoes.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchQuery {
    pub query: String,
}

impl HumanRender for SearchResponse {
    /// Quiet mode drops the header and the no-match line; the matches stay.
    fn render_human(&self, mode: OutputMode) -> String {
        let quiet = mode.quiet;
        let query = &self.extras.query;
        if self.items.is_empty() {
            return if quiet {
                String::new()
            } else {
                format!("No matches found for \"{query}\"\n")
            };
        }
        let mut out = String::new();
        if !quiet {
            out.push_str(&format!(
                "Search results for \"{query}\" ({} matches):\n\n",
                self.count()
            ));
        }
        let mut current_file = "";
        for hit in &self.items {
            let result = &hit.result;
            if result.path != current_file {
                current_file = &result.path;
                out.push_str(&match (&result.issue_id, &hit.issue_title) {
                    (Some(issue_id), Some(title)) => format!("Issue {issue_id} | {title}\n"),
                    (Some(issue_id), None) => format!("Issue {issue_id}\n"),
                    (None, _) => format!("Document {}\n", result.path),
                });
            }
            out.push_str(&format!(
                "  Line {}: {}\n",
                result.line_number,
                result.line_text.trim()
            ));
        }
        out.push('\n');
        out
    }
}
