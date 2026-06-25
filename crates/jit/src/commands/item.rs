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
    index_items, index_project_sources, resolve_item_kinds, split_qualified_id, AddressableItem,
    ItemKind, ProjectSource, Scope,
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
///
/// `issue_full_id` / `issue_title` are populated only for an issue-scoped item;
/// for a project-scoped item (`@/<self-id>`) both are `None`, since no single
/// issue owns it (REQ-01).
#[derive(Debug, Serialize)]
pub struct ItemShowResult {
    /// The resolved addressable item.
    pub item: AddressableItem,
    /// Full id of the owning issue, or `None` for a project-scoped item.
    pub issue_full_id: Option<String>,
    /// Title of the owning issue, or `None` for a project-scoped item.
    pub issue_title: Option<String>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Resolve the effective item kinds from the cached `[item_kinds]` registry,
    /// falling back to the built-in `requirement` kind when none is declared.
    fn item_kinds(&self) -> Result<Vec<ItemKind>> {
        let config = self.cached_config()?;
        resolve_item_kinds(config.item_kinds.as_ref())
            .map_err(|err| anyhow!("invalid [item_kinds] configuration: {err}"))
    }

    /// The issue-scope subset of the configured kinds.
    ///
    /// Project-scope kinds read a config-declared file, not issue descriptions, so
    /// indexing an issue must apply only issue-scope kinds — otherwise a project
    /// kind's id-pattern could spuriously match an issue line and collide with an
    /// issue-scope self-id.
    fn issue_item_kinds(&self) -> Result<Vec<ItemKind>> {
        Ok(self
            .item_kinds()?
            .into_iter()
            .filter(|k| !k.kind_scope().is_project())
            .collect())
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
        // Only issue-scope kinds index issue descriptions; project-scope kinds are
        // sourced from their config-declared file via `project_items`.
        let kinds: Vec<ItemKind> = self
            .issue_item_kinds()?
            .into_iter()
            .filter(|k| kind_filter.is_none_or(|f| k.name() == f))
            .collect();
        let repo_format = self.repo_content_format()?;
        let mut issues = self.storage.list_issues()?;
        // Deterministic output: order by short id, then the index's own order.
        issues.sort_by_key(|a| a.short_id());

        let mut items = Vec::new();
        // Project-scope (`@`) items first, then issue-scope items in short-id
        // order, so both substrates surface through one list (REQ-01). The same
        // `kind_filter` applies to both substrates.
        items.extend(
            self.project_items()?
                .into_iter()
                .filter(|i| kind_filter.is_none_or(|f| i.kind == f)),
        );
        for issue in &issues {
            let parser = crate::document::content_parser_for(issue.content_format, repo_format)
                .map_err(|err| {
                    anyhow!(
                        "cannot parse description of issue {}: {err}",
                        issue.short_id()
                    )
                })?;
            let indexed = index_items(issue, &kinds, parser.as_ref())
                .with_context(|| format!("indexing items of issue {} failed", issue.short_id()))?;
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

    /// Index the project-scope (`@`) addressable items from config-declared
    /// sources.
    ///
    /// For every configured kind with `scope = "project"`, its `source` file
    /// (a repository-local path) is read through the storage boundary
    /// ([`IssueStore::read_repo_file`](crate::storage::IssueStore::read_repo_file))
    /// and scanned the SAME way an issue description is, then all candidates are
    /// deduped once through [`index_project_sources`] (REQ-01). The source path
    /// comes ONLY from config — no filename is hardcoded here — and this command
    /// does NO direct filesystem I/O: path-safety (repository-local enforcement)
    /// and reading both live in storage. An absent source file contributes no
    /// items (storage returns `Ok(None)`), never an error; a source that is present
    /// but unreadable, or a path that escapes the repo, IS reported.
    fn project_items(&self) -> Result<Vec<AddressableItem>> {
        let mut sources = Vec::new();
        for kind in self.item_kinds()? {
            if !kind.kind_scope().is_project() {
                continue;
            }
            // A project kind without a source is rejected at kind resolution
            // (MissingProjectSource), so `source()` is Some here; treat a None
            // defensively as no items rather than panicking.
            let Some(source_rel) = kind.source() else {
                continue;
            };
            // Read via the storage boundary: absent -> None (graceful, REQ-01),
            // invalid/escaping path or unreadable file -> typed error.
            let markdown = self.storage.read_repo_file(source_rel).with_context(|| {
                format!(
                    "cannot read project-scope source '{source_rel}' for item kind '{}'",
                    kind.name()
                )
            })?;
            let Some(markdown) = markdown else {
                continue;
            };
            sources.push(ProjectSource { kind, markdown });
        }

        let repo_format = self.repo_content_format()?;
        // Project-scope source files use the repo's content format (Markdown by
        // default); reuse the same parser selection as issue descriptions.
        let parser = crate::document::content_parser_for(None, repo_format)
            .map_err(|err| anyhow!("cannot parse project-scope source: {err}"))?;
        index_project_sources(&sources, parser.as_ref())
            .map_err(|err| anyhow!("indexing project-scope items failed: {err}"))
    }

    /// Resolve a qualified id `<scope>/<self-id>` to its addressable item.
    ///
    /// The scope is `@` for the project scope, or any issue reference (full id,
    /// short id, or unique prefix) resolved through the SAME issue-id resolver the
    /// rest of the CLI uses — so `jit item show 56ab/REQ-01` works just like `jit
    /// show 56ab`, and `jit item show @/INV-01` resolves the project-scoped item
    /// (REQ-01, REQ-02). An input without a `/` is a usage error; an unresolvable
    /// scope or an unknown self-id is a descriptive error rather than a silent miss
    /// (an unresolvable qualified id is reported, never dropped).
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
        let (scope_segment, self_id) = split_qualified_id(qualified).ok_or_else(|| {
            anyhow!("'{qualified}' is not a qualified id; expected <scope>/<self-id>")
        })?;

        match Scope::parse(scope_segment) {
            // Project scope: sourced from a config-declared file, no owning issue
            // (REQ-01).
            Scope::Project => {
                let item = self
                    .project_items()?
                    .into_iter()
                    .find(|i| i.self_id == self_id)
                    .ok_or_else(|| {
                        anyhow!(
                            "project scope '@' declares no addressable item \
                             with self-id '{self_id}'"
                        )
                    })?;
                Ok(ItemShowResult {
                    item,
                    issue_full_id: None,
                    issue_title: None,
                })
            }
            // Issue scope: resolve the issue, index its description (REQ-02).
            Scope::Issue(issue_ref) => {
                let full_id = self.storage.resolve_issue_id(&issue_ref).with_context(|| {
                    format!("cannot resolve issue scope '{issue_ref}' in '{qualified}'")
                })?;
                let issue = self.storage.load_issue(&full_id)?;

                let kinds = self.issue_item_kinds()?;
                let repo_format = self.repo_content_format()?;
                let parser = crate::document::content_parser_for(issue.content_format, repo_format)
                    .map_err(|err| {
                        anyhow!(
                            "cannot parse description of issue {}: {err}",
                            issue.short_id()
                        )
                    })?;
                let items = index_items(&issue, &kinds, parser.as_ref()).with_context(|| {
                    format!("indexing items of issue {} failed", issue.short_id())
                })?;

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
                    issue_full_id: Some(full_id),
                    issue_title: Some(issue.title),
                })
            }
        }
    }

    /// Resolve a generic node→item link label of the form
    /// `<namespace>:<issue>/<self-id>` to its addressed item (REQ-05).
    ///
    /// The namespace must be a `link-namespace` of some configured item kind
    /// (e.g. the `requirement` kind's `satisfies`), so an arbitrary label is not
    /// mistaken for an item reference. The value after the namespace must be a
    /// qualified id `<issue>/<self-id>`; it is resolved through the SAME
    /// [`show_item`](Self::show_item) path, so an unresolvable qualified id is
    /// reported as an error rather than silently dropped.
    ///
    /// Returns `Ok(None)` when the label's namespace is not a registered link
    /// namespace OR when the value is unqualified (the legacy `satisfies:REQ-01`
    /// shape) — those are not generic qualified references and the caller may
    /// handle them by the existing unqualified rules. A namespace that DOES match
    /// but whose qualified id cannot be resolved is an `Err`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// // A qualified `satisfies:` reference resolves to the addressed item.
    /// if let Some(resolved) = executor.resolve_link_label("satisfies:56ab0224/REQ-01")? {
    ///     assert_eq!(resolved.item.self_id, "REQ-01");
    /// }
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn resolve_link_label(&self, label: &str) -> Result<Option<ItemShowResult>> {
        let Some((namespace, value)) = label.split_once(':') else {
            return Ok(None);
        };

        // The namespace must belong to some kind's link-namespaces, else this is
        // not an item reference at all.
        let is_link_namespace = self
            .item_kinds()?
            .iter()
            .any(|kind| kind.link_namespaces().iter().any(|ns| ns == namespace));
        if !is_link_namespace {
            return Ok(None);
        }

        // A generic qualified reference carries `<issue>/<self-id>`; the legacy
        // unqualified `satisfies:REQ-01` shape has no scope and is left to the
        // existing rules.
        if split_qualified_id(value).is_none() {
            return Ok(None);
        }

        // A registered link namespace with a qualified value MUST resolve; an
        // unresolvable qualified id is an error, never a silent drop (REQ-05).
        let resolved = self.show_item(value).with_context(|| {
            format!("link label '{label}' references an unresolvable qualified item id")
        })?;
        Ok(Some(resolved))
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
        let qids: Vec<&str> = result
            .items
            .iter()
            .map(|i| i.qualified_id.as_str())
            .collect();
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
        assert_eq!(shown.issue_title.as_deref(), Some("A"));
        // The owning issue id is the FULL id, resolved from the short-id scope.
        assert!(shown.issue_full_id.as_deref().unwrap().starts_with(&short));
    }

    /// Build an [`InMemoryStorage`] executor that declares a project-scope
    /// `invariant` kind sourced from `project-items.md`, seeding the source through
    /// the in-memory repo-file map (NO real source file). `config.toml` is written
    /// to the synthetic root (the established `InMemoryStorage` config pattern, the
    /// only on-disk file config loading requires). `extra_config` is appended to
    /// the `[item_kinds]` block; `source_md` (when `Some`) seeds the source.
    fn project_exec(
        source_md: Option<&str>,
        extra_config: &str,
        issues: Vec<Issue>,
    ) -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();
        let config = format!(
            "[item_kinds.invariant]\n\
             section = \"success_criteria\"\n\
             id-pattern = \"INV-[0-9]+\"\n\
             markers = []\n\
             link-namespaces = [\"upholds\"]\n\
             scope = \"project\"\n\
             source = \"project-items.md\"\n\
             source-of-truth = \"markdown-first\"\n{extra_config}"
        );
        std::fs::write(storage.root().join("config.toml"), config).unwrap();
        if let Some(md) = source_md {
            storage.add_repo_file("project-items.md", md);
        }
        for issue in issues {
            storage.save_issue(issue).unwrap();
        }
        CommandExecutor::new(storage)
    }

    #[test]
    fn test_show_item_resolves_project_scope() {
        // REQ-01: `@/<self-id>` resolves through the config-driven path with the
        // source served by the storage boundary (in-memory repo-file map, no fs).
        let exec = project_exec(
            Some("## Success Criteria\n\n- INV-01: all writes are atomic\n"),
            "",
            vec![],
        );
        let shown = exec.show_item("@/INV-01").unwrap();
        assert_eq!(shown.item.self_id, "INV-01");
        assert_eq!(shown.item.qualified_id, "@/INV-01");
        assert_eq!(shown.item.scope, "@");
        assert_eq!(shown.item.kind, "invariant");
        assert_eq!(shown.issue_full_id, None);
        assert_eq!(shown.issue_title, None);
    }

    #[test]
    fn test_show_item_project_scope_missing_self_id_errors() {
        // An unresolvable project-scope id is reported, never silently dropped.
        let exec = project_exec(Some("## Success Criteria\n\n- INV-01: x\n"), "", vec![]);
        let err = exec.show_item("@/INV-99").unwrap_err();
        assert!(err.to_string().contains("project scope"));
        assert!(err.to_string().contains("no addressable item"));
    }

    #[test]
    fn test_show_item_project_scope_absent_source_is_graceful() {
        // REQ-01 (degradation): with no source seeded, `read_repo_file` returns
        // None and `@/<id>` resolves to a descriptive not-found error (not a panic,
        // not the issue resolver).
        let exec = project_exec(None, "", vec![]);
        let err = exec.show_item("@/INV-01").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("project scope"));
        assert!(!msg.contains("resolve issue scope"));
    }

    #[test]
    fn test_show_item_same_self_id_distinct_scopes() {
        // REQ-04: the same self-id under an issue scope and the project scope are
        // two distinct items, each resolved by its own qualified id.
        let issue = Issue::new(
            "A".to_string(),
            "## Success Criteria\n\n- [hard] INV-01: issue one\n".to_string(),
        );
        let short = issue.short_id();
        // The issue-scope `requirement` kind must also see INV-* ids.
        let exec = project_exec(
            Some("## Success Criteria\n\n- INV-01: project one\n"),
            "\n[item_kinds.requirement]\n\
             section = \"success_criteria\"\n\
             id-pattern = \"INV-[0-9]+\"\n\
             markers = [\"[hard]\"]\n\
             link-namespaces = [\"satisfies\"]\n\
             scope = \"issue\"\n\
             source-of-truth = \"markdown-first\"\n",
            vec![issue],
        );

        let from_issue = exec.show_item(&format!("{short}/INV-01")).unwrap();
        let from_project = exec.show_item("@/INV-01").unwrap();
        assert!(from_issue.item.text.contains("issue one"));
        assert!(from_project.item.text.contains("project one"));
        assert_ne!(from_issue.item.qualified_id, from_project.item.qualified_id);
    }

    #[test]
    fn test_list_items_includes_project_scope() {
        // REQ-01: project-scoped items surface in the cross-repo list alongside
        // issue-scoped ones.
        let exec = project_exec(
            Some("## Success Criteria\n\n- INV-01: project inv\n"),
            "",
            vec![],
        );
        let result = exec.list_items(None).unwrap();
        let qids: Vec<&str> = result
            .items
            .iter()
            .map(|i| i.qualified_id.as_str())
            .collect();
        assert!(qids.contains(&"@/INV-01"));
    }

    #[test]
    fn test_project_items_rejects_path_traversal_source() {
        // Path-safety: a `..`-traversal source path is rejected by the storage
        // boundary, so `@/<id>` resolution surfaces a typed InvalidPath error
        // rather than reading a file outside the repository.
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();
        std::fs::write(
            storage.root().join("config.toml"),
            "[item_kinds.invariant]\n\
             section = \"success_criteria\"\n\
             id-pattern = \"INV-[0-9]+\"\n\
             markers = []\n\
             link-namespaces = [\"upholds\"]\n\
             scope = \"project\"\n\
             source = \"../escape.md\"\n\
             source-of-truth = \"markdown-first\"\n",
        )
        .unwrap();
        let exec = CommandExecutor::new(storage);
        let err = exec.show_item("@/INV-01").unwrap_err();
        // The typed InvalidPath cause is in the error chain (alternate Display
        // renders the full anyhow context chain).
        let msg = format!("{err:#}");
        assert!(
            msg.contains("'..' segment not permitted"),
            "expected traversal rejection, got: {msg}"
        );
    }

    #[test]
    fn test_project_items_rejects_absolute_source() {
        // Path-safety: an absolute source path is rejected by the storage boundary.
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();
        std::fs::write(
            storage.root().join("config.toml"),
            "[item_kinds.invariant]\n\
             section = \"success_criteria\"\n\
             id-pattern = \"INV-[0-9]+\"\n\
             markers = []\n\
             link-namespaces = [\"upholds\"]\n\
             scope = \"project\"\n\
             source = \"/etc/passwd\"\n\
             source-of-truth = \"markdown-first\"\n",
        )
        .unwrap();
        let exec = CommandExecutor::new(storage);
        let err = exec.show_item("@/INV-01").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("absolute paths are not permitted"),
            "expected absolute-path rejection, got: {msg}"
        );
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

    #[test]
    fn test_resolve_link_label_qualified_satisfies() {
        // REQ-05: a generic link label `satisfies:<issue>/<self-id>` resolves to
        // the addressed item via the qualified id.
        let a = issue_with_criteria("A", "## Success Criteria\n\n- [hard] REQ-01: a one\n");
        let short = a.short_id();
        let exec = executor_with(vec![a]);

        let label = format!("satisfies:{short}/REQ-01");
        let resolved = exec.resolve_link_label(&label).unwrap().expect("resolves");
        assert_eq!(resolved.item.self_id, "REQ-01");
        assert_eq!(resolved.item.qualified_id, format!("{short}/REQ-01"));
    }

    #[test]
    fn test_resolve_link_label_unresolvable_qualified_is_error() {
        // A registered link namespace with a qualified-but-unresolvable id must
        // be reported, not silently dropped.
        let a = issue_with_criteria("A", "## Success Criteria\n\n- [hard] REQ-01: a\n");
        let short = a.short_id();
        let exec = executor_with(vec![a]);
        let err = exec
            .resolve_link_label(&format!("satisfies:{short}/REQ-99"))
            .unwrap_err();
        assert!(err.to_string().contains("unresolvable qualified item id"));
    }

    #[test]
    fn test_resolve_link_label_unqualified_and_unknown_ns_are_none() {
        let a = issue_with_criteria("A", "## Success Criteria\n\n- [hard] REQ-01: a\n");
        let exec = executor_with(vec![a]);
        // Legacy unqualified shape: not a generic qualified reference.
        assert!(exec
            .resolve_link_label("satisfies:REQ-01")
            .unwrap()
            .is_none());
        // A namespace that is not a kind link-namespace is not an item reference.
        assert!(exec
            .resolve_link_label("type:epic/REQ-01")
            .unwrap()
            .is_none());
        // A label with no `:` is not a reference.
        assert!(exec.resolve_link_label("nope").unwrap().is_none());
    }
}
