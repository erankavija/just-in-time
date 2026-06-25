//! Addressable-item query operations (`jit item list/show/search/resolve`).
//!
//! These commands surface the pure [`item`](crate::domain::item) model across the
//! whole repository: they read the `[item_kinds]` registry from config, index
//! every issue's description through the storage boundary, and return derived
//! qualified ids. All projection logic lives in `domain/item.rs`; this module
//! only orchestrates config + storage + the pure index (the layer boundary in
//! CLAUDE.md "Separation of Concerns").

use super::*;
use crate::domain::item::{
    index_items, resolve_item_kinds, split_qualified_id, AddressableItem, ItemKind,
};

/// Result of a `jit item list` / `search` query.
#[derive(Debug, Serialize)]
pub struct ItemListResult {
    /// The matched addressable items, in (issue, section, item) order.
    pub items: Vec<AddressableItem>,
    /// Number of matched items.
    pub count: usize,
}

/// Result of a `jit item show` / `resolve` query for one qualified id.
#[derive(Debug, Serialize)]
pub struct ItemShowResult {
    /// The resolved addressable item.
    pub item: AddressableItem,
    /// Full id of the owning issue (the qualified id's scope is its short form).
    pub issue_full_id: String,
    /// Title of the owning issue, for human-readable output.
    pub issue_title: String,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Resolve the effective item kinds from the cached `[item_kinds]` registry,
    /// falling back to the built-in `requirement` kind when none is declared.
    fn item_kinds(&self) -> Result<Vec<ItemKind>> {
        let config = self.cached_config()?;
        resolve_item_kinds(config.item_kinds.as_ref())
            .map_err(|err| anyhow!("invalid [item_kinds] configuration: {err}"))
    }

    /// Index every addressable item across the repository, optionally narrowed to
    /// one kind by NAME.
    ///
    /// Reads every issue through storage and projects items via the pure
    /// [`index_items`]; the qualified ids are derived, nothing is persisted. A
    /// `kind_filter` keeps only kinds whose name matches; an unknown kind name
    /// yields an empty result (not an error) so callers can probe freely.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let all = executor.list_items(None)?;
    /// println!("{} addressable item(s)", all.count);
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn list_items(&self, kind_filter: Option<&str>) -> Result<ItemListResult> {
        let kinds: Vec<ItemKind> = self
            .item_kinds()?
            .into_iter()
            .filter(|k| kind_filter.is_none_or(|f| k.name() == f))
            .collect();
        let repo_format = self.repo_content_format()?;
        let mut issues = self.storage.list_issues()?;
        // Deterministic output: order by short id, then the index's own order.
        issues.sort_by(|a, b| a.short_id().cmp(&b.short_id()));

        let mut items = Vec::new();
        for issue in &issues {
            let parser = crate::document::content_parser_for(issue.content_format, repo_format)
                .map_err(|err| {
                    anyhow!(
                        "cannot parse description of issue {}: {err}",
                        issue.short_id()
                    )
                })?;
            let indexed = index_items(issue, &kinds, parser.as_ref()).with_context(|| {
                format!("indexing items of issue {} failed", issue.short_id())
            })?;
            items.extend(indexed);
        }
        let count = items.len();
        Ok(ItemListResult { items, count })
    }

    /// Search addressable items whose self-id or text contains `query`
    /// (case-insensitive), optionally narrowed to one kind.
    ///
    /// An empty `query` matches every item, so the `kind_filter` can be used
    /// alone. Builds on [`list_items`](Self::list_items) so the same indexing path
    /// serves both.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let hits = executor.search_items("atomic", None)?;
    /// println!("{} item(s) mention 'atomic'", hits.count);
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn search_items(&self, query: &str, kind_filter: Option<&str>) -> Result<ItemListResult> {
        let q = query.to_lowercase();
        let all = self.list_items(kind_filter)?;
        let items: Vec<AddressableItem> = all
            .items
            .into_iter()
            .filter(|item| {
                q.is_empty()
                    || item.self_id.to_lowercase().contains(&q)
                    || item.text.to_lowercase().contains(&q)
                    || item.qualified_id.to_lowercase().contains(&q)
            })
            .collect();
        let count = items.len();
        Ok(ItemListResult { items, count })
    }

    /// Resolve a qualified id `<issue>/<self-id>` to its addressable item.
    ///
    /// The scope segment is resolved through the SAME issue-id resolver the rest
    /// of the CLI uses (full id, short id, or unique prefix), so `jit item show
    /// 56ab/REQ-01` works just like `jit show 56ab`. An input without a `/` is a
    /// usage error; an unresolvable scope or an unknown self-id is a descriptive
    /// error rather than a silent miss (REQ-04).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let shown = executor.show_item("56ab0224/REQ-01")?;
    /// assert_eq!(shown.item.self_id, "REQ-01");
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn show_item(&self, qualified: &str) -> Result<ItemShowResult> {
        let (scope, self_id) = split_qualified_id(qualified).ok_or_else(|| {
            anyhow!("'{qualified}' is not a qualified id; expected <issue>/<self-id>")
        })?;

        let full_id = self
            .storage
            .resolve_issue_id(scope)
            .with_context(|| format!("cannot resolve issue scope '{scope}' in '{qualified}'"))?;
        let issue = self.storage.load_issue(&full_id)?;

        let kinds = self.item_kinds()?;
        let repo_format = self.repo_content_format()?;
        let parser = crate::document::content_parser_for(issue.content_format, repo_format)
            .map_err(|err| {
                anyhow!(
                    "cannot parse description of issue {}: {err}",
                    issue.short_id()
                )
            })?;
        let items = index_items(&issue, &kinds, parser.as_ref())
            .with_context(|| format!("indexing items of issue {} failed", issue.short_id()))?;

        let item = items
            .into_iter()
            .find(|i| i.self_id == self_id)
            .ok_or_else(|| {
                anyhow!(
                    "issue {} declares no addressable item with self-id '{self_id}'",
                    issue.short_id()
                )
            })?;

        Ok(ItemShowResult {
            item,
            issue_full_id: full_id,
            issue_title: issue.title,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::InMemoryStorage;

    fn executor_with(issues: Vec<Issue>) -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        for issue in issues {
            storage.save_issue(issue).unwrap();
        }
        CommandExecutor::new(storage)
    }

    fn issue_with_criteria(title: &str, body: &str) -> Issue {
        Issue::new(title.to_string(), body.to_string())
    }

    #[test]
    fn test_list_items_across_repo() {
        let a = issue_with_criteria(
            "A",
            "## Success Criteria\n\n- [hard] REQ-01: a one\n- [hard] REQ-02: a two\n",
        );
        let b = issue_with_criteria("B", "## Success Criteria\n\n- [hard] REQ-01: b one\n");
        let exec = executor_with(vec![a, b]);

        let result = exec.list_items(None).unwrap();
        assert_eq!(result.count, 3);
        // Qualified ids disambiguate the two REQ-01s by their issue scope.
        let qids: Vec<&str> = result.items.iter().map(|i| i.qualified_id.as_str()).collect();
        assert!(qids.iter().filter(|q| q.ends_with("/REQ-01")).count() == 2);
    }

    #[test]
    fn test_list_items_kind_filter_unknown_is_empty() {
        let a = issue_with_criteria("A", "## Success Criteria\n\n- [hard] REQ-01: a\n");
        let exec = executor_with(vec![a]);
        assert_eq!(exec.list_items(Some("nonexistent")).unwrap().count, 0);
        assert_eq!(exec.list_items(Some("requirement")).unwrap().count, 1);
    }

    #[test]
    fn test_search_items_matches_text_and_id() {
        let a = issue_with_criteria(
            "A",
            "## Success Criteria\n\n- [hard] REQ-01: atomic writes\n- [hard] REQ-02: cycle check\n",
        );
        let exec = executor_with(vec![a]);
        assert_eq!(exec.search_items("atomic", None).unwrap().count, 1);
        assert_eq!(exec.search_items("REQ-02", None).unwrap().count, 1);
        assert_eq!(exec.search_items("", None).unwrap().count, 2);
    }

    #[test]
    fn test_show_item_resolves_by_qualified_id() {
        let a = issue_with_criteria("A", "## Success Criteria\n\n- [hard] REQ-01: a one\n");
        let short = a.short_id();
        let exec = executor_with(vec![a]);

        let qualified = format!("{short}/REQ-01");
        let shown = exec.show_item(&qualified).unwrap();
        assert_eq!(shown.item.self_id, "REQ-01");
        assert_eq!(shown.item.qualified_id, qualified);
        assert_eq!(shown.issue_title, "A");
    }

    #[test]
    fn test_show_item_missing_self_id_errors() {
        let a = issue_with_criteria("A", "## Success Criteria\n\n- [hard] REQ-01: a\n");
        let short = a.short_id();
        let exec = executor_with(vec![a]);
        let err = exec.show_item(&format!("{short}/REQ-99")).unwrap_err();
        assert!(err.to_string().contains("no addressable item"));
    }

    #[test]
    fn test_show_item_not_qualified_errors() {
        let exec = executor_with(vec![]);
        let err = exec.show_item("REQ-01").unwrap_err();
        assert!(err.to_string().contains("not a qualified id"));
    }
}
