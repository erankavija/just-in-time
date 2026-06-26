//! Addressable-item query operations (`jit item list/show/search/resolve`).
//!
//! These commands surface the pure [`item`](crate::domain::item) model across the
//! whole repository: they read the `[item_kinds]` registry from config, index
//! every issue's description through the storage boundary, and return derived
//! qualified ids. All projection logic lives in `domain/item.rs`; this module
//! only orchestrates config + storage + the pure index (the layer boundary in
//! CLAUDE.md "Separation of Concerns").

use super::*;
use crate::config::SourceOfTruth;
use crate::domain::item::{
    index_items, index_project_sources, resolve_item_kinds, split_qualified_id, AddressableItem,
    ItemError, ItemKind, ProjectSource, RawScopeItem, Scope, INVARIANT_KIND_NAME,
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
    /// falling back to the built-in default kinds (`requirement`, `decision`, ...)
    /// when none is declared.
    pub(crate) fn item_kinds(&self) -> Result<Vec<ItemKind>> {
        let config = self.cached_config()?;
        resolve_item_kinds(config.item_kinds.as_ref()).map_err(|err| {
            crate::errors::InvalidArgumentError::new(format!(
                "invalid [item_kinds] configuration: {err}"
            ))
            .into()
        })
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

    /// Index the project-scope (`@`) addressable items, routing each kind by its
    /// `source-of-truth` so both authoring substrates surface through one path.
    ///
    /// For every configured kind with `scope = "project"`:
    /// - **markdown-first**: its `source` file (a repository-local path) is read
    ///   through the storage boundary
    ///   ([`IssueStore::read_repo_file`](crate::storage::IssueStore::read_repo_file))
    ///   and scanned the SAME way an issue description is. The source path comes
    ///   ONLY from config — no filename is hardcoded — and this command does NO
    ///   direct filesystem I/O: path-safety (repository-local enforcement) and
    ///   reading both live in storage. An absent source file contributes no items
    ///   (storage returns `Ok(None)`), never an error; a source that is present but
    ///   unreadable, or a path that escapes the repo, IS reported.
    /// - **registry-first**: items are projected directly from a registered
    ///   registry, NOT a markdown source — the registry is the AUTHORITATIVE source
    ///   and NO markdown index is read or produced for them (REQ-02). Routing binds
    ///   to an EXPLICIT registry by kind name: the only registered source is the
    ///   invariant registry ([`JitConfig::invariants`](crate::config::JitConfig)),
    ///   bound to the [`invariant`](crate::domain::item::INVARIANT_KIND_NAME) kind.
    ///   Each invariant entry's `id` is its self-id, so its qualified id is `@/<id>`
    ///   (REQ-01). Any OTHER registry-first project kind has no bound registry and
    ///   is rejected with [`ItemError::UnknownRegistrySource`], never a silent
    ///   projection of the invariant registry under the wrong kind name.
    ///
    /// Both substrates' candidates are deduped once through the single
    /// [`index_project_sources`] derivation, so per-scope uniqueness and
    /// qualified-id derivation are identical across them.
    fn project_items(&self) -> Result<Vec<AddressableItem>> {
        let config = self.cached_config()?;
        let mut sources = Vec::new();
        let mut registry_items = Vec::new();
        for kind in self.item_kinds()? {
            if !kind.kind_scope().is_project() {
                continue;
            }
            match kind.source_of_truth() {
                // Registry-first: project items come ONLY from a registered
                // registry, never from a markdown source file. The binding is
                // explicit by kind name — only `invariant` is backed by a registry
                // (the invariant registry); any other registry-first project kind is
                // a typed error rather than a mislabeled invariant projection (REQ-02).
                SourceOfTruth::RegistryFirst if kind.name() == INVARIANT_KIND_NAME => {
                    registry_items.extend(invariant_raw_items(kind.name(), config));
                }
                SourceOfTruth::RegistryFirst => {
                    return Err(anyhow!(ItemError::UnknownRegistrySource {
                        kind: kind.name().to_string(),
                    }))
                    .with_context(|| {
                        format!("cannot index registry-first project kind '{}'", kind.name())
                    });
                }
                // Markdown-first: read and scan the config-declared source file.
                SourceOfTruth::MarkdownFirst => {
                    // A markdown-first project kind without a source is rejected at
                    // kind resolution (MissingProjectSource), so `source()` is Some
                    // here; treat a None defensively as no items rather than panicking.
                    let Some(source_rel) = kind.source() else {
                        continue;
                    };
                    // Read via the storage boundary: absent -> None (graceful,
                    // REQ-01), invalid/escaping path or unreadable file -> typed error.
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
            }
        }

        let repo_format = self.repo_content_format()?;
        // Project-scope source files use the repo's content format (Markdown by
        // default); reuse the same parser selection as issue descriptions.
        let parser = crate::document::content_parser_for(None, repo_format)
            .map_err(|err| anyhow!("cannot parse project-scope source: {err}"))?;
        index_project_sources(&sources, registry_items, parser.as_ref())
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

/// Project the loaded invariant registry into raw project-scope candidates for the
/// registry-first `invariant` kind.
///
/// A pure projection over [`JitConfig::invariants`](crate::config::JitConfig): each
/// [`Invariant`](crate::validation::invariants::Invariant) becomes a
/// [`RawScopeItem`] whose `self_id` is the entry's `id` (so its qualified id is
/// `@/<id>`) and whose `text` is the entry's `statement`. The registry is the
/// authoritative source — nothing is parsed from any markdown section (REQ-02).
/// `kind_name` tags each candidate so the derived item reports the right kind.
fn invariant_raw_items(kind_name: &str, config: &JitConfig) -> Vec<RawScopeItem> {
    config
        .invariants
        .invariants
        .iter()
        .map(|inv| RawScopeItem {
            kind: kind_name.to_string(),
            self_id: inv.id.clone(),
            text: inv.statement.clone(),
        })
        .collect()
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

    /// Build an [`InMemoryStorage`] executor that declares a markdown-first
    /// project-scope `glossary` kind sourced from `project-items.md`, seeding the
    /// source through the in-memory repo-file map (NO real source file). `config.toml`
    /// is written to the synthetic root (the established `InMemoryStorage` config
    /// pattern, the only on-disk file config loading requires). `extra_config` is
    /// appended to the `[item_kinds]` block; `source_md` (when `Some`) seeds the
    /// source.
    ///
    /// Uses the non-reserved name `glossary` (not `invariant`, which is reserved as
    /// a registry-first kind) to exercise generic markdown-first project-scope
    /// sourcing.
    fn project_exec(
        source_md: Option<&str>,
        extra_config: &str,
        issues: Vec<Issue>,
    ) -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();
        let config = format!(
            "[item_kinds.glossary]\n\
             section = \"success_criteria\"\n\
             id-pattern = \"GLOSS-[0-9]+\"\n\
             markers = []\n\
             link-namespaces = [\"defines\"]\n\
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
            Some("## Success Criteria\n\n- GLOSS-01: all writes are atomic\n"),
            "",
            vec![],
        );
        let shown = exec.show_item("@/GLOSS-01").unwrap();
        assert_eq!(shown.item.self_id, "GLOSS-01");
        assert_eq!(shown.item.qualified_id, "@/GLOSS-01");
        assert_eq!(shown.item.scope, "@");
        assert_eq!(shown.item.kind, "glossary");
        assert_eq!(shown.issue_full_id, None);
        assert_eq!(shown.issue_title, None);
    }

    #[test]
    fn test_show_item_project_scope_missing_self_id_errors() {
        // An unresolvable project-scope id is reported, never silently dropped.
        let exec = project_exec(Some("## Success Criteria\n\n- GLOSS-01: x\n"), "", vec![]);
        let err = exec.show_item("@/GLOSS-99").unwrap_err();
        assert!(err.to_string().contains("project scope"));
        assert!(err.to_string().contains("no addressable item"));
    }

    #[test]
    fn test_show_item_project_scope_absent_source_is_graceful() {
        // REQ-01 (degradation): with no source seeded, `read_repo_file` returns
        // None and `@/<id>` resolves to a descriptive not-found error (not a panic,
        // not the issue resolver).
        let exec = project_exec(None, "", vec![]);
        let err = exec.show_item("@/GLOSS-01").unwrap_err();
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
            "## Success Criteria\n\n- [hard] GLOSS-01: issue one\n".to_string(),
        );
        let short = issue.short_id();
        // The issue-scope `requirement` kind must also see GLOSS-* ids.
        let exec = project_exec(
            Some("## Success Criteria\n\n- GLOSS-01: project one\n"),
            "\n[item_kinds.requirement]\n\
             section = \"success_criteria\"\n\
             id-pattern = \"GLOSS-[0-9]+\"\n\
             markers = [\"[hard]\"]\n\
             link-namespaces = [\"satisfies\"]\n\
             scope = \"issue\"\n\
             source-of-truth = \"markdown-first\"\n",
            vec![issue],
        );

        let from_issue = exec.show_item(&format!("{short}/GLOSS-01")).unwrap();
        let from_project = exec.show_item("@/GLOSS-01").unwrap();
        assert!(from_issue.item.text.contains("issue one"));
        assert!(from_project.item.text.contains("project one"));
        assert_ne!(from_issue.item.qualified_id, from_project.item.qualified_id);
    }

    #[test]
    fn test_list_items_includes_project_scope() {
        // REQ-01: project-scoped items surface in the cross-repo list alongside
        // issue-scoped ones.
        let exec = project_exec(
            Some("## Success Criteria\n\n- GLOSS-01: project gloss\n"),
            "",
            vec![],
        );
        let result = exec.list_items(None).unwrap();
        let qids: Vec<&str> = result
            .items
            .iter()
            .map(|i| i.qualified_id.as_str())
            .collect();
        assert!(qids.contains(&"@/GLOSS-01"));
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
            "[item_kinds.glossary]\n\
             section = \"success_criteria\"\n\
             id-pattern = \"GLOSS-[0-9]+\"\n\
             markers = []\n\
             link-namespaces = [\"defines\"]\n\
             scope = \"project\"\n\
             source = \"../escape.md\"\n\
             source-of-truth = \"markdown-first\"\n",
        )
        .unwrap();
        let exec = CommandExecutor::new(storage);
        let err = exec.show_item("@/GLOSS-01").unwrap_err();
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
            "[item_kinds.glossary]\n\
             section = \"success_criteria\"\n\
             id-pattern = \"GLOSS-[0-9]+\"\n\
             markers = []\n\
             link-namespaces = [\"defines\"]\n\
             scope = \"project\"\n\
             source = \"/etc/passwd\"\n\
             source-of-truth = \"markdown-first\"\n",
        )
        .unwrap();
        let exec = CommandExecutor::new(storage);
        let err = exec.show_item("@/GLOSS-01").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("absolute paths are not permitted"),
            "expected absolute-path rejection, got: {msg}"
        );
    }

    /// Build an executor whose synthetic `.jit` root carries an
    /// `invariants.toml` with `invariants_toml` and NO `config.toml`, exercising
    /// the default-repo, registry-first invariant path (the built-in `invariant`
    /// kind is active because no `[item_kinds]` table replaces the defaults).
    fn registry_exec(
        invariants_toml: &str,
        issues: Vec<Issue>,
    ) -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();
        std::fs::write(storage.root().join("invariants.toml"), invariants_toml).unwrap();
        for issue in issues {
            storage.save_issue(issue).unwrap();
        }
        CommandExecutor::new(storage)
    }

    const TWO_INVARIANTS: &str = "\
[[invariants]]
id = \"INV-01\"
statement = \"Every dependency edge stays acyclic.\"
kind = \"enforced\"
enforced-by = \"dag-no-cycles\"

[[invariants]]
id = \"INV-02\"
statement = \"Issues prefer functional style.\"
kind = \"advisory\"
";

    #[test]
    fn test_list_items_kind_invariant_returns_registry_entry() {
        // REQ-01: in a default repo with a `.jit/invariants.toml` (no config.toml),
        // `jit item list --kind invariant` returns each loaded invariant addressed
        // as `@/<self-id>`.
        let exec = registry_exec(TWO_INVARIANTS, vec![]);
        let result = exec.list_items(Some("invariant")).unwrap();
        assert_eq!(result.count, 2);
        let qids: Vec<&str> = result
            .items
            .iter()
            .map(|i| i.qualified_id.as_str())
            .collect();
        assert!(qids.contains(&"@/INV-01"));
        assert!(qids.contains(&"@/INV-02"));
        // The self-id is the invariant's id; the statement is its text; the kind is
        // `invariant`; the scope is `@`.
        let first = result.items.iter().find(|i| i.self_id == "INV-01").unwrap();
        assert_eq!(first.kind, "invariant");
        assert_eq!(first.scope, "@");
        assert_eq!(first.text, "Every dependency edge stays acyclic.");
    }

    #[test]
    fn test_show_item_resolves_invariant_by_qualified_id() {
        // REQ-01: the generic resolver returns an invariant by its `@/<self-id>`.
        let exec = registry_exec(TWO_INVARIANTS, vec![]);
        let shown = exec.show_item("@/INV-02").unwrap();
        assert_eq!(shown.item.self_id, "INV-02");
        assert_eq!(shown.item.qualified_id, "@/INV-02");
        assert_eq!(shown.item.kind, "invariant");
        assert_eq!(shown.item.scope, "@");
        // No owning issue for a project-scope item.
        assert_eq!(shown.issue_full_id, None);
        assert_eq!(shown.issue_title, None);
    }

    #[test]
    fn test_search_items_finds_invariant_by_statement() {
        // The generic search path reaches registry-first invariants too.
        let exec = registry_exec(TWO_INVARIANTS, vec![]);
        let hits = exec.search_items("acyclic", Some("invariant")).unwrap();
        assert_eq!(hits.count, 1);
        assert_eq!(hits.items[0].qualified_id, "@/INV-01");
    }

    #[test]
    fn test_invariant_registry_is_authoritative_no_markdown_index() {
        // REQ-02: the registry is the authoritative source for invariants and NO
        // markdown index is produced for them. An issue description containing an
        // `INV-`-looking line is NOT projected as an invariant item — invariants
        // come ONLY from `.jit/invariants.toml`.
        let issue = Issue::new(
            "A".to_string(),
            "## Success Criteria\n\n- [hard] INV-99: a markdown line that LOOKS like an invariant\n"
                .to_string(),
        );
        let exec = registry_exec(TWO_INVARIANTS, vec![issue]);

        let invariants = exec.list_items(Some("invariant")).unwrap();
        let qids: Vec<&str> = invariants
            .items
            .iter()
            .map(|i| i.qualified_id.as_str())
            .collect();
        // Only the two registry entries are invariants; the issue's INV-99 line is
        // NOT among them (no markdown index for invariants).
        assert_eq!(invariants.count, 2);
        assert!(qids.contains(&"@/INV-01"));
        assert!(qids.contains(&"@/INV-02"));
        assert!(!qids.iter().any(|q| q.ends_with("/INV-99")));
        assert!(!qids.iter().any(|q| q.contains("INV-99")));
    }

    #[test]
    fn test_list_items_without_invariants_file_has_no_invariants() {
        // Registry-first: with no `.jit/invariants.toml`, the invariant kind yields
        // no items (graceful), and nothing is read from any markdown source.
        let issue = issue_with_criteria("A", "## Success Criteria\n\n- [hard] REQ-01: a\n");
        let exec = executor_with(vec![issue]);
        assert_eq!(exec.list_items(Some("invariant")).unwrap().count, 0);
    }

    /// Build an executor whose synthetic `.jit` root carries an explicit
    /// `config.toml` (`config`) and an optional `invariants.toml`, for the
    /// reserved-name and registry-binding rework tests.
    fn config_exec(
        config: &str,
        invariants_toml: Option<&str>,
        issues: Vec<Issue>,
    ) -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();
        std::fs::write(storage.root().join("config.toml"), config).unwrap();
        if let Some(inv) = invariants_toml {
            std::fs::write(storage.root().join("invariants.toml"), inv).unwrap();
        }
        for issue in issues {
            storage.save_issue(issue).unwrap();
        }
        CommandExecutor::new(storage)
    }

    #[test]
    fn test_markdown_first_invariant_config_is_rejected() {
        // Finding 1: declaring `[item_kinds.invariant]` as markdown-first is
        // rejected — the invariant kind is reserved as registry-first, so no markdown
        // index can ever be produced for invariants (REQ-02). The rejection surfaces
        // through the kind-resolution path that `list_items` uses.
        let exec = config_exec(
            "[item_kinds.invariant]\n\
             section = \"success_criteria\"\n\
             id-pattern = \"INV-[0-9]+\"\n\
             markers = []\n\
             link-namespaces = [\"enforces\"]\n\
             scope = \"project\"\n\
             source = \"project-items.md\"\n\
             source-of-truth = \"markdown-first\"\n",
            // Even with a registry present, the markdown-first declaration is invalid.
            Some(TWO_INVARIANTS),
            // An issue with an INV- line: it must NEVER become an invariant item.
            vec![issue_with_criteria(
                "A",
                "## Success Criteria\n\n- [hard] INV-99: looks like an invariant\n",
            )],
        );
        let err = exec.list_items(Some("invariant")).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("project-scoped") && msg.contains("registry-first"),
            "expected reserved-name rejection naming both requirements, got: {msg}"
        );
    }

    #[test]
    fn test_registry_first_issue_scoped_invariant_config_is_rejected() {
        // REQ-02 (final hole): `[item_kinds.invariant]` declared registry-first BUT
        // issue-scoped is rejected. Otherwise the kind would pass through
        // `issue_item_kinds()` and parse invariants from issue descriptions — a
        // markdown index for invariants by another route.
        let exec = config_exec(
            "[item_kinds.invariant]\n\
             section = \"success_criteria\"\n\
             id-pattern = \"INV-[0-9]+\"\n\
             markers = []\n\
             link-namespaces = [\"enforces\"]\n\
             scope = \"issue\"\n\
             source-of-truth = \"registry-first\"\n",
            Some(TWO_INVARIANTS),
            // An issue carrying an INV- line: it must NEVER become an invariant item.
            vec![issue_with_criteria(
                "A",
                "## Success Criteria\n\n- INV-99: looks like an invariant\n",
            )],
        );
        let err = exec.list_items(Some("invariant")).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("project-scoped") && msg.contains("registry-first"),
            "expected reserved-name rejection naming both requirements, got: {msg}"
        );
    }

    #[test]
    fn test_registry_first_non_invariant_kind_is_typed_error() {
        // Finding 2: a custom registry-first project kind that is NOT `invariant` has
        // no bound registry source. It must produce a typed error rather than
        // silently projecting the invariant registry under the wrong kind name.
        let exec = config_exec(
            "[item_kinds.foo]\n\
             section = \"success_criteria\"\n\
             id-pattern = \"FOO-[0-9]+\"\n\
             markers = []\n\
             link-namespaces = [\"foos\"]\n\
             scope = \"project\"\n\
             source-of-truth = \"registry-first\"\n",
            // The invariant registry exists, but `foo` must NOT borrow its rows.
            Some(TWO_INVARIANTS),
            vec![],
        );
        let err = exec.list_items(None).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("no registered registry source") && msg.contains("foo"),
            "expected UnknownRegistrySource for 'foo', got: {msg}"
        );
        // No invariant rows leaked under the `foo` kind: the whole list errors out
        // rather than returning mislabeled rows.
        assert!(exec.list_items(Some("foo")).is_err());
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
    fn test_resolve_link_label_across_all_four_kinds() {
        // REQ-01/REQ-02: for EACH shipped namespace a `<ns>:<scope>/<self-id>`
        // label resolves to the addressed item of the corresponding kind, across
        // all four kinds — requirement/decision/risk (issue-scope, markdown-first)
        // and invariant (project-scope, registry-first). The default kinds are
        // active (no `[item_kinds]` table), and a `.jit/invariants.toml` is loaded.
        let issue = Issue::new(
            "A".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: atomic writes\n\n\
             ## Decisions\n\n- D-01: use json storage\n\n\
             ## Risks\n\n- RISK-01: lock contention\n"
                .to_string(),
        );
        let short = issue.short_id();
        let exec = registry_exec(TWO_INVARIANTS, vec![issue]);

        // requirement / `satisfies:` -> the requirement item.
        let req = exec
            .resolve_link_label(&format!("satisfies:{short}/REQ-01"))
            .unwrap()
            .expect("satisfies resolves");
        assert_eq!(req.item.kind, "requirement");
        assert_eq!(req.item.qualified_id, format!("{short}/REQ-01"));

        // decision / `per:` -> the decision item.
        let dec = exec
            .resolve_link_label(&format!("per:{short}/D-01"))
            .unwrap()
            .expect("per resolves");
        assert_eq!(dec.item.kind, "decision");
        assert_eq!(dec.item.qualified_id, format!("{short}/D-01"));

        // risk / `mitigates:` and `resolves:` -> the risk item (both namespaces).
        let mit = exec
            .resolve_link_label(&format!("mitigates:{short}/RISK-01"))
            .unwrap()
            .expect("mitigates resolves");
        assert_eq!(mit.item.kind, "risk");
        assert_eq!(mit.item.qualified_id, format!("{short}/RISK-01"));
        let res = exec
            .resolve_link_label(&format!("resolves:{short}/RISK-01"))
            .unwrap()
            .expect("resolves resolves");
        assert_eq!(res.item.kind, "risk");

        // invariant / `enforces:@/<id>` -> the registry-first invariant item.
        let inv = exec
            .resolve_link_label("enforces:@/INV-01")
            .unwrap()
            .expect("enforces resolves");
        assert_eq!(inv.item.kind, "invariant");
        assert_eq!(inv.item.qualified_id, "@/INV-01");
        assert_eq!(inv.item.scope, "@");
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
