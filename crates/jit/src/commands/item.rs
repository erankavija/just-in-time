//! Addressable-item query operations (`jit item list/show/search/resolve`).
//!
//! These commands surface the pure [`item`](crate::domain::item) model across the
//! whole repository: they read the `[item_kinds]` registry from config, index
//! every issue's description through the storage boundary, and return derived
//! qualified ids. All projection logic lives in `domain/item.rs`; this module
//! only orchestrates config + storage + the pure index (the layer boundary in
//! AGENTS.md "Separation of Concerns").

use super::*;
use crate::config::SourceOfTruth;
use crate::domain::item::{
    index_items, index_project_sources, load_toml_scope_items, parse_kind_segmented_address,
    resolve_item_kinds, resolve_kind_alias, resolve_sugar_address, AddressableItem,
    IssueItemAddress, ItemKind, ProjectSource, Scope, PROJECT_SCOPE_SENTINEL,
};
use crate::errors::ItemNotFoundError;

/// Collection key of [`ItemListResult`].
pub enum ItemsKey {}

impl CollectionKey for ItemsKey {
    const KEY: &'static str = "items";
    const SCHEMA_NAME: Option<&'static str> = Some("ItemListResult");
}

/// Result of a `jit item list` / `search` query: the matched addressable items,
/// in (issue, section, item) order.
pub type ItemListResult = ListEnvelope<ItemsKey, AddressableItem>;

/// Result of a `jit item show` / `resolve` query for one qualified id.
///
/// `issue_full_id` / `issue_title` are populated only for an issue-scoped item;
/// for a project-scoped item (`@/<kind>/<self-id>`) both are `None`, since no single
/// issue owns it (REQ-01).
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ItemShowResult {
    /// The resolved addressable item.
    pub item: AddressableItem,
    /// Full id of the owning issue, or `None` for a project-scoped item.
    pub issue_full_id: Option<IssueId>,
    /// Title of the owning issue, or `None` for a project-scoped item.
    pub issue_title: Option<String>,
}

/// An address classified for project or issue item lookup.
enum ResolvedItemAddress {
    /// A project-scope address `@/<kind>/<self-id>` of an explicit kind.
    Project {
        kind: String,
        self_id: String,
    },
    Issue(IssueItemAddress),
}

/// Addressable-item operations of a [`CommandExecutor`].
pub struct ItemOps<'a, S: IssueStore> {
    pub(super) exec: &'a CommandExecutor<S>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// The addressable-item family.
    pub fn items(&self) -> ItemOps<'_, S> {
        ItemOps { exec: self }
    }
}

impl<S: IssueStore> ItemOps<'_, S> {
    /// Resolve the effective item kinds from the cached `[item_kinds]` registry.
    ///
    /// The engine bakes in no kinds: with no `[item_kinds]` table the result is
    /// empty (see [`resolve_item_kinds`]). A profile package or repository-authored
    /// config can supply the table.
    pub(crate) fn item_kinds(&self) -> Result<Vec<ItemKind>> {
        let config = self.exec.cached_config()?;
        resolve_item_kinds(config.item_kinds.as_ref()).map_err(|err| {
            crate::errors::InvalidArgumentError::new(format!(
                "invalid [item_kinds] configuration: {err}"
            ))
            .into()
        })
    }

    /// Canonicalize a kind name or config-declared alias to the registry name it
    /// addresses ([`resolve_kind_alias`]), or return the input verbatim when it
    /// matches no configured kind.
    ///
    /// Callers compare the result against the canonical [`AddressableItem::kind`],
    /// so an alias (`inv`) resolves to the same items as the registry name
    /// (`invariant`); an unresolved input stays as typed, so an unknown kind still
    /// yields an empty result / descriptive not-found rather than an error here.
    fn canonical_kind_name(&self, name: &str) -> Result<String> {
        let kinds = self.item_kinds()?;
        Ok(resolve_kind_alias(&kinds, name).unwrap_or(name).to_string())
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
    /// `kind_filter` keeps only kinds whose name matches — a config-declared alias
    /// is accepted and resolved to the registry name first ([`resolve_kind_alias`]),
    /// so `--kind inv` filters the same items as `--kind invariant`. An unknown
    /// kind name yields an empty result (not an error) so callers can probe freely.
    pub fn list_items(&self, kind_filter: Option<&str>) -> Result<ItemListResult> {
        // Resolve a kind alias in the filter to the canonical registry name so both
        // the kind-set filter and the item filter compare against canonical names
        // (items always carry the registry name). An unresolved filter is kept
        // verbatim, so an unknown kind still yields an empty result.
        let canonical_filter: Option<String> = match kind_filter {
            Some(f) => Some(self.canonical_kind_name(f)?),
            None => None,
        };
        let filter = canonical_filter.as_deref();
        // Only issue-scope kinds index issue descriptions; project-scope kinds are
        // sourced from their config-declared file via `project_items`.
        let kinds: Vec<ItemKind> = self
            .issue_item_kinds()?
            .into_iter()
            .filter(|k| filter.is_none_or(|f| k.name() == f))
            .collect();
        let repo_format = self.exec.cached_config()?.content_format();
        let mut issues = self.exec.storage.list_issues()?;
        // Deterministic output: order by short id, then the index's own order.
        issues.sort_by_key(|a| a.short_id());

        let mut items = Vec::new();
        // Project-scope (`@`) items first, then issue-scope items in short-id
        // order, so both substrates surface through one list (REQ-01). The same
        // `kind_filter` applies to both substrates.
        items.extend(
            self.project_items()?
                .into_iter()
                .filter(|i| filter.is_none_or(|f| i.kind == f)),
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
        Ok(ItemListResult::new(items))
    }

    /// Search addressable items whose self-id or text contains `query`
    /// (case-insensitive), optionally narrowed to one kind.
    ///
    /// An empty `query` matches every item, so the `kind_filter` can be used
    /// alone. Builds on [`list_items`](Self::list_items) so the same indexing path
    /// serves both.
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
        Ok(ItemListResult::new(items))
    }

    /// Index the project-scope (`@`) addressable items, routing each kind by its
    /// `source-of-truth` so both authoring substrates surface through one path.
    ///
    /// For every configured kind with `scope = "project"`:
    /// - **markdown-first**: its `source` file (a repository-local path) is read
    ///   through the storage boundary
    ///   ([`IssueStore::read_repo_file`])
    ///   and scanned the SAME way an issue description is. The source path comes
    ///   ONLY from config — no filename is hardcoded — and this command does NO
    ///   direct filesystem I/O: path-safety (repository-local enforcement) and
    ///   reading both live in storage. An absent source file contributes no items
    ///   (storage returns `Ok(None)`), never an error; a source that is present but
    ///   unreadable, or a path that escapes the repo, IS reported.
    /// - **registry-first**: items are projected directly from a structured toml
    ///   registry, NOT a markdown source. The registry is the AUTHORITATIVE source
    ///   and NO markdown index is read or produced for them (REQ-02). The kind's
    ///   toml `source` descriptor ([`ItemKind::toml_source`]) names the backing
    ///   `.toml`: its file is read through the storage boundary and each entry
    ///   projected through the descriptor's field mapping by
    ///   [`load_toml_scope_items`] (REQ-02). An absent file contributes no items
    ///   (graceful); a present-but-malformed file is a typed error. Any
    ///   descriptor-backed kind routes through this same generic path with no
    ///   reserved-name branch (REQ-03). A
    ///   registry-first project kind that declares no descriptor is rejected at kind
    ///   resolution ([`ItemError::MissingProjectSource`]), so `toml_source` is
    ///   `Some` here.
    ///
    /// Both substrates' candidates are deduped once through the single
    /// [`index_project_sources`] derivation, so per-(scope, kind) uniqueness and
    /// qualified-id derivation are identical across them.
    ///
    /// [`ItemError::MissingProjectSource`]: crate::domain::item::ItemError::MissingProjectSource
    fn project_items(&self) -> Result<Vec<AddressableItem>> {
        let mut sources = Vec::new();
        let mut registry_items = Vec::new();
        for kind in self.item_kinds()? {
            if !kind.kind_scope().is_project() {
                continue;
            }
            match kind.source_of_truth() {
                // Registry-first: project items come ONLY from the kind's structured
                // toml `source` descriptor, never from a markdown source file. Kind
                // resolution guarantees a registry-first project kind carries a
                // descriptor (MissingProjectSource otherwise), so the `if let` binds;
                // the defensive `else` keeps this non-panicking without a re-fetch.
                SourceOfTruth::RegistryFirst => {
                    if let Some(descriptor) = kind.toml_source() {
                        // Read the named `.toml` through the storage boundary and
                        // project each entry through the descriptor's field mapping.
                        // An absent file contributes no items (graceful), mirroring
                        // the markdown-first path; a present-but-malformed file is a
                        // typed error.
                        let toml = self
                            .exec
                            .storage
                            .read_repo_file(&descriptor.toml)
                            .with_context(|| {
                                format!(
                                    "cannot read toml source '{}' for item kind '{}'",
                                    descriptor.toml,
                                    kind.name()
                                )
                            })?;
                        if let Some(toml) = toml {
                            let rows = load_toml_scope_items(kind.name(), descriptor, &toml)
                                .with_context(|| {
                                    format!(
                                        "cannot project toml source '{}' for item kind '{}'",
                                        descriptor.toml,
                                        kind.name()
                                    )
                                })?;
                            registry_items.extend(rows);
                        }
                    }
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
                    let markdown =
                        self.exec
                            .storage
                            .read_repo_file(source_rel)
                            .with_context(|| {
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

        let repo_format = self.exec.cached_config()?.content_format();
        // Project-scope source files use the repo's content format (Markdown by
        // default); reuse the same parser selection as issue descriptions.
        let parser = crate::document::content_parser_for(None, repo_format)
            .map_err(|err| anyhow!("cannot parse project-scope source: {err}"))?;
        index_project_sources(&sources, registry_items, parser.as_ref())
            .map_err(|err| anyhow!("indexing project-scope items failed: {err}"))
    }

    /// Resolve an item address to its addressable item, filtering by kind AND
    /// self-id (REQ-01, REQ-02).
    ///
    /// Accepts the full address grammar:
    /// - `@/<kind>/<self-id>` — a project-scope item of an explicit kind.
    /// - `@<name>/<kind>/<self-id>` — a named-project form, routed through
    ///   [`AddressScope::bind_local`](crate::domain::item::AddressScope::bind_local):
    ///   it resolves identically to the bare-`@`
    ///   form when `<name>` is the repo's declared `[project] name`, and is an
    ///   [`ItemError::NotResolvable`](crate::domain::item::ItemError::NotResolvable)
    ///   error otherwise (no federation or remote lookup is attempted).
    /// - `@/issue/<short-id>/<kind>/<self-id>` — an issue-scope item of an explicit
    ///   kind; the issue reference is resolved through the SAME issue-id resolver the
    ///   rest of the CLI uses (full id, short id, or unique prefix).
    /// - `<short-id>/<self-id>` — sugar for an issue-scope item whose kind is
    ///   inferred from the self-id's shape and actual items.
    ///
    /// Every minted qualified id (`@/<kind>/<self-id>` for project scope,
    /// `@/issue/<short-id>/<kind>/<self-id>` for issue scope) is itself an explicit
    /// kind-segmented address, so `jit item show` round-trips every id `jit item
    /// list` prints. A kindless `@/<self-id>` input is not a valid address: it is a
    /// clear parse error from [`parse_kind_segmented_address`], never a self-id-only
    /// match.
    ///
    /// Explicit-kind and sugar addresses filter candidates by BOTH kind and self-id,
    /// so two same-scope same-self-id items of different kinds (e.g. a `rule` and a
    /// `gate` both named `coverage-preview`) each resolve to their own distinct item,
    /// never a silent first-match. A request for a kind that does not own the matched
    /// self-id in that scope is a descriptive not-found error, never a fallback to a
    /// different kind's item. An unresolvable scope or an unknown self-id is a
    /// descriptive error rather than a silent miss.
    pub fn show_item(&self, qualified: &str) -> Result<ItemShowResult> {
        match self.resolve_item_address(qualified)? {
            // Project scope: sourced from a config-declared file, no owning issue
            // (REQ-01). The address always names its kind, so filter by kind AND
            // self-id.
            ResolvedItemAddress::Project { kind, self_id } => {
                let item = self
                    .project_items()?
                    .into_iter()
                    .find(|i| i.kind == kind && i.self_id == self_id)
                    .ok_or_else(|| {
                        ItemNotFoundError::new(format!(
                            "project scope '@' declares no addressable item of kind '{kind}' \
                             with self-id '{self_id}'"
                        ))
                    })?;
                Ok(ItemShowResult {
                    item,
                    issue_full_id: None,
                    issue_title: None,
                })
            }
            // Issue scope: resolve the issue, index its description, then filter by
            // kind AND self-id (REQ-01, REQ-02).
            ResolvedItemAddress::Issue(IssueItemAddress {
                issue_ref,
                kind,
                self_id,
            }) => {
                let full_id = self
                    .exec
                    .storage
                    .resolve_issue_id(&issue_ref)
                    .with_context(|| {
                        format!(
                            "cannot resolve issue scope '{}' in '{qualified}'",
                            issue_ref.as_str()
                        )
                    })?;
                let issue = self.exec.storage.load_issue(&full_id)?;

                let kinds = self.issue_item_kinds()?;
                let repo_format = self.exec.cached_config()?.content_format();
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

                // An explicit or unambiguously-inferred kind filters by kind AND
                // self-id; an open kind (pattern-ambiguous sugar) filters by self-id
                // alone and reports a genuine multi-kind collision rather than
                // silently picking one.
                let mut matched: Vec<AddressableItem> = items
                    .into_iter()
                    .filter(|i| kind.as_deref().is_none_or(|k| i.kind == k) && i.self_id == self_id)
                    .collect();

                let item = match matched.len() {
                    1 => matched.remove(0),
                    0 => {
                        return Err(ItemNotFoundError::new(match &kind {
                            Some(k) => format!(
                                "issue {} declares no addressable item of kind '{k}' \
                                 with self-id '{self_id}'",
                                issue.short_id()
                            ),
                            None => format!(
                                "issue {} declares no addressable item with self-id '{self_id}'",
                                issue.short_id()
                            ),
                        })
                        .into());
                    }
                    _ => {
                        let kinds_found: Vec<&str> =
                            matched.iter().map(|i| i.kind.as_str()).collect();
                        return Err(anyhow!(
                            "self-id '{self_id}' in issue {} is ambiguous across kinds ({}); \
                             use the explicit '@/issue/<short-id>/<kind>/<self-id>' address",
                            issue.short_id(),
                            kinds_found.join(", ")
                        ));
                    }
                };

                Ok(ItemShowResult {
                    item,
                    issue_full_id: Some(full_id),
                    issue_title: Some(issue.title),
                })
            }
        }
    }

    /// Classify an item address into the scope + kind + self-id `show_item`
    /// resolves against, applying the sugar-expansion and kind-segmented grammars.
    ///
    /// A non-`@` input is the `<short-id>/<self-id>` sugar, whose kind is inferred
    /// from the self-id's shape. An `@`-prefixed input is an explicit kind-segmented
    /// address; a malformed one (including a kindless `@/<self-id>`) is the parser's
    /// own [`ItemError::InvalidAddress`](crate::domain::item::ItemError::InvalidAddress),
    /// never a self-id-only match.
    fn resolve_item_address(&self, qualified: &str) -> Result<ResolvedItemAddress> {
        // Sugar `<short-id>/<self-id>`: infer the kind from the self-id's shape
        // against the issue-scoped kinds (project-scoped kinds have no sugar form).
        if !qualified.starts_with(PROJECT_SCOPE_SENTINEL) {
            if !qualified.contains('/') {
                return Err(crate::errors::InvalidArgumentError::new(format!(
                    "'{qualified}' is not a qualified id; expected '<short-id>/<self-id>'"
                ))
                .into());
            }
            return resolve_sugar_address(qualified, &self.item_kinds()?)
                .map(ResolvedItemAddress::Issue)
                .map_err(|err| anyhow!("cannot resolve item address '{qualified}': {err}"));
        }

        // An `@`-prefixed input is an explicit kind-segmented address. A malformed
        // one (including a kindless `@/<self-id>`) surfaces the parser's own
        // InvalidAddress error; there is no self-id-only fallback (story 7f22d6cf
        // REQ-04 retired the legacy kind-agnostic path).
        match parse_kind_segmented_address(qualified) {
            Ok(addr) => {
                // Canonicalize the parsed kind segment: it may be a config-declared
                // alias (`@/inv/...`), which resolves to the registry name items
                // carry, so downstream kind-AND-self-id filtering matches. An
                // unknown kind stays verbatim, giving a descriptive not-found.
                let kind = self.canonical_kind_name(&addr.kind)?;
                // Bind the parsed scope token to local project identity: a bare
                // `@` or a `@<name>` naming the repo's own declared project both
                // collapse to `Scope::Project`; a `@<name>` naming any other
                // project is `ItemError::NotResolvable` (no federation or remote
                // lookup — REQ-09, D5). The wrapping context keeps the kind and
                // self-id visible so a caller can confirm the value was
                // structurally routed, not mis-split, even on the error path.
                let local_name = self
                    .exec
                    .config_manager
                    .project_name_from_config(self.exec.cached_config()?);
                let scope = addr.scope.bind_local(local_name.as_deref()).map_err(|err| {
                    anyhow!(
                        "cannot resolve item address '{qualified}' (kind '{kind}', self-id '{}'): {err}",
                        addr.self_id,
                    )
                })?;
                match scope {
                    Scope::Project => Ok(ResolvedItemAddress::Project {
                        kind,
                        self_id: addr.self_id,
                    }),
                    Scope::Issue(issue_ref) => Ok(ResolvedItemAddress::Issue(IssueItemAddress {
                        issue_ref,
                        kind: Some(kind),
                        self_id: addr.self_id,
                    })),
                }
            }
            // A malformed `@`-address surfaces the parser's own InvalidAddress
            // error. There is no kind-agnostic fallback: a kindless `@/<self-id>`
            // (once the legacy project form) is now just an address missing its kind
            // segment, so it is a clear parse error rather than a self-id-only match
            // (story 7f22d6cf REQ-04, plan amendment D11-A).
            Err(parse_err) => Err(anyhow::Error::new(parse_err)
                .context(format!("cannot resolve item address '{qualified}'"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{seed_issue, MemoryDeclarations};
    use crate::storage::InMemoryStorage;
    use crate::test_harness::seed_memory_repository_file;

    fn issue_with_criteria(title: &str, body: &str) -> Issue {
        crate::test_utils::fixture_issue(title.to_string(), body.to_string())
    }

    const GLOSSARY_CONFIG: &str = "[item_kinds.glossary]\n\
        section = \"success_criteria\"\n\
        id-pattern = \"GLOSS-[0-9]+\"\n\
        markers = []\n\
        link-namespaces = [\"defines\"]\n\
        scope = \"project\"\n\
        source = \"project-items.md\"\n\
        source-of-truth = \"markdown-first\"\n";

    const TWO_PROJECT_KINDS_CONFIG: &str = "[item_kinds.custom-rule]\n\
        section = \"success_criteria\"\n\
        id-pattern = \"[a-z][a-z-]*\"\n\
        markers = []\n\
        link-namespaces = [\"governs\"]\n\
        scope = \"project\"\n\
        source = \"rules.md\"\n\
        source-of-truth = \"markdown-first\"\n\
        \n\
        [item_kinds.custom-gate]\n\
        section = \"success_criteria\"\n\
        id-pattern = \"[a-z][a-z-]*\"\n\
        markers = []\n\
        link-namespaces = [\"checks\"]\n\
        scope = \"project\"\n\
        source = \"gates.md\"\n\
        source-of-truth = \"markdown-first\"\n";

    const POLICY_CONFIG: &str = "[item_kinds.policy]\n\
        section = \"success_criteria\"\n\
        id-pattern = \"POL-[0-9]+\"\n\
        markers = []\n\
        link-namespaces = [\"enforces\"]\n\
        scope = \"project\"\n\
        source = { toml = \"policies.toml\", table = \"policies\", id-field = \"id\", \
        text-field = \"statement\", link-fields = { enforces = \"enforced-by\" } }\n\
        source-of-truth = \"registry-first\"\n";

    #[test]
    fn test_list_items_across_repo() {
        let a = issue_with_criteria(
            "A",
            "## Success Criteria\n\n- [hard] REQ-01: a one\n- [hard] REQ-02: a two\n",
        );
        let b = issue_with_criteria("B", "## Success Criteria\n\n- [hard] REQ-01: b one\n");
        let storage = InMemoryStorage::new();
        seed_issue(&storage, a);
        seed_issue(&storage, b);
        let config = crate::test_utils::shipped_item_kind_tables(&["requirement"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&config),
                ..MemoryDeclarations::default()
            },
        );

        let result = exec.items().list_items(None).unwrap();
        assert_eq!(result.items.len(), 3);
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
        let storage = InMemoryStorage::new();
        seed_issue(&storage, a);
        let config = crate::test_utils::shipped_item_kind_tables(&["requirement"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&config),
                ..MemoryDeclarations::default()
            },
        );
        assert_eq!(
            exec.items()
                .list_items(Some("nonexistent"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            exec.items()
                .list_items(Some("requirement"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn test_list_items_kind_filter_excludes_items_of_other_shipped_kinds() {
        let a = issue_with_criteria(
            "A",
            "## Success Criteria\n\n- [hard] REQ-01: first\n- [hard] REQ-02: second\n",
        );
        let storage = InMemoryStorage::new();
        seed_issue(&storage, a);
        let config = crate::test_utils::shipped_item_kinds_toml();
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&config),
                gates: Some(TWO_GATES),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );

        // Registry-first shipped kinds contribute project items of their own.
        let all = exec.items().list_items(None).unwrap();
        assert!(all.items.iter().any(|i| i.kind == "gate"));
        let requirements = exec.items().list_items(Some("requirement")).unwrap();
        assert_eq!(requirements.items.len(), 2);
        assert!(requirements.items.iter().all(|i| i.kind == "requirement"));
    }

    #[test]
    fn test_search_items_matches_text_and_id() {
        let a = issue_with_criteria(
            "A",
            "## Success Criteria\n\n- [hard] REQ-01: atomic writes\n- [hard] REQ-02: cycle check\n",
        );
        let storage = InMemoryStorage::new();
        seed_issue(&storage, a);
        let config = crate::test_utils::shipped_item_kind_tables(&["requirement"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&config),
                ..MemoryDeclarations::default()
            },
        );
        assert_eq!(
            exec.items().search_items("atomic", None).unwrap().count(),
            1
        );
        assert_eq!(
            exec.items().search_items("REQ-02", None).unwrap().count(),
            1
        );
        assert_eq!(exec.items().search_items("", None).unwrap().count(), 2);
    }

    #[test]
    fn test_show_item_resolves_by_qualified_id() {
        let a = issue_with_criteria("A", "## Success Criteria\n\n- [hard] REQ-01: a one\n");
        let short = a.short_id();
        let a_id = a.id.clone();
        let storage = InMemoryStorage::new();
        seed_issue(&storage, a);
        let config = crate::test_utils::shipped_item_kind_tables(&["requirement"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&config),
                ..MemoryDeclarations::default()
            },
        );

        // The `<short-id>/<self-id>` sugar still resolves as INPUT (D3-A), but the
        // minted qualified id it resolves to is the canonical kind-segmented form.
        let shown = exec.items().show_item(&format!("{short}/REQ-01")).unwrap();
        assert_eq!(shown.item.self_id, "REQ-01");
        assert_eq!(
            shown.item.qualified_id,
            format!("@/issue/{short}/requirement/REQ-01")
        );
        assert_eq!(shown.issue_title.as_deref(), Some("A"));
        // The owning issue id is the FULL id, resolved from the short-id scope.
        assert_eq!(shown.issue_full_id.as_ref(), Some(&a_id));
    }

    #[test]
    fn test_show_item_resolves_project_scope() {
        // REQ-01: `@/<kind>/<self-id>` resolves through the config-driven path with
        // the source served by the storage boundary (in-memory repo-file map, no fs).
        let storage = InMemoryStorage::new();
        seed_memory_repository_file(
            &storage,
            "project-items.md",
            "## Success Criteria\n\n- GLOSS-01: all writes are atomic\n",
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(GLOSSARY_CONFIG),
                ..MemoryDeclarations::default()
            },
        );
        let shown = exec.items().show_item("@/glossary/GLOSS-01").unwrap();
        assert_eq!(shown.item.self_id, "GLOSS-01");
        assert_eq!(shown.item.qualified_id, "@/glossary/GLOSS-01");
        assert_eq!(shown.item.scope, "@");
        assert_eq!(shown.item.kind, "glossary");
        assert_eq!(shown.issue_full_id, None);
        assert_eq!(shown.issue_title, None);
    }

    #[test]
    fn test_show_item_project_scope_missing_self_id_errors() {
        // An unresolvable project-scope id is reported, never silently dropped.
        let storage = InMemoryStorage::new();
        seed_memory_repository_file(
            &storage,
            "project-items.md",
            "## Success Criteria\n\n- GLOSS-01: x\n",
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(GLOSSARY_CONFIG),
                ..MemoryDeclarations::default()
            },
        );
        let err = exec.items().show_item("@/glossary/GLOSS-99").unwrap_err();
        assert!(err.to_string().contains("project scope"));
        assert!(err.to_string().contains("no addressable item"));
    }

    #[test]
    fn test_show_item_project_scope_absent_source_is_graceful() {
        // REQ-01 (degradation): with no source seeded, `read_repo_file` returns
        // None and `@/<kind>/<id>` resolves to a descriptive not-found error (not a
        // panic, not the issue resolver).
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(GLOSSARY_CONFIG),
                ..MemoryDeclarations::default()
            },
        );
        let err = exec.items().show_item("@/glossary/GLOSS-01").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("project scope"));
        assert!(!msg.contains("resolve issue scope"));
    }

    #[test]
    fn test_show_item_same_self_id_distinct_scopes() {
        // REQ-04: the same self-id under an issue scope and the project scope are
        // two distinct items, each resolved by its own qualified id.
        let issue = crate::test_utils::fixture_issue(
            "A".to_string(),
            "## Success Criteria\n\n- [hard] GLOSS-01: issue one\n".to_string(),
        );
        let short = issue.short_id();
        // A custom issue-scope kind must also see GLOSS-* ids.
        let storage = InMemoryStorage::new();
        seed_memory_repository_file(
            &storage,
            "project-items.md",
            "## Success Criteria\n\n- GLOSS-01: project one\n",
        );
        seed_issue(&storage, issue);
        let config = format!(
            "{GLOSSARY_CONFIG}\n[item_kinds.issue-glossary]\n\
             section = \"success_criteria\"\n\
             id-pattern = \"GLOSS-[0-9]+\"\n\
             markers = [\"[hard]\"]\n\
             link-namespaces = [\"satisfies\"]\n\
             scope = \"issue\"\n\
             source-of-truth = \"markdown-first\"\n"
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&config),
                ..MemoryDeclarations::default()
            },
        );

        let from_issue = exec
            .items()
            .show_item(&format!("{short}/GLOSS-01"))
            .unwrap();
        let from_project = exec.items().show_item("@/glossary/GLOSS-01").unwrap();
        assert!(from_issue.item.text.contains("issue one"));
        assert!(from_project.item.text.contains("project one"));
        assert_ne!(from_issue.item.qualified_id, from_project.item.qualified_id);
    }

    #[test]
    fn test_show_item_explicit_kind_disambiguates_same_self_id() {
        // REQ-01: a `custom-rule` and a `custom-gate` both self-id `coverage-preview` in the
        // project scope each resolve to their OWN distinct item via the explicit
        // `@/<kind>/<self-id>` address, never a silent first-match.
        let storage = InMemoryStorage::new();
        seed_memory_repository_file(
            &storage,
            "rules.md",
            "## Success Criteria\n\n- coverage-preview: the rule form\n",
        );
        seed_memory_repository_file(
            &storage,
            "gates.md",
            "## Success Criteria\n\n- coverage-preview: the gate form\n",
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(TWO_PROJECT_KINDS_CONFIG),
                ..MemoryDeclarations::default()
            },
        );

        let rule = exec
            .items()
            .show_item("@/custom-rule/coverage-preview")
            .unwrap();
        assert_eq!(rule.item.kind, "custom-rule");
        assert_eq!(rule.item.self_id, "coverage-preview");
        assert!(rule.item.text.contains("rule form"));

        let gate = exec
            .items()
            .show_item("@/custom-gate/coverage-preview")
            .unwrap();
        assert_eq!(gate.item.kind, "custom-gate");
        assert_eq!(gate.item.self_id, "coverage-preview");
        assert!(gate.item.text.contains("gate form"));

        // Distinct items sharing a scope and self-id: the kind is a segment of the
        // minted id, so they even derive DISTINCT qualified ids.
        assert_ne!(rule.item.text, gate.item.text);
        assert_eq!(rule.item.qualified_id, "@/custom-rule/coverage-preview");
        assert_eq!(gate.item.qualified_id, "@/custom-gate/coverage-preview");
        assert_ne!(rule.item.qualified_id, gate.item.qualified_id);
    }

    #[test]
    fn test_show_item_wrong_kind_is_not_found() {
        // REQ-02: asking for a kind that does not own the matched self-id in that
        // scope is a clear not-found error, never a silent fallback to the kind that
        // DOES own it.
        let storage = InMemoryStorage::new();
        seed_memory_repository_file(
            &storage,
            "rules.md",
            "## Success Criteria\n\n- only-rule: exists only as a rule\n",
        );
        seed_memory_repository_file(
            &storage,
            "gates.md",
            "## Success Criteria\n\n- other-gate: an unrelated gate\n",
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(TWO_PROJECT_KINDS_CONFIG),
                ..MemoryDeclarations::default()
            },
        );

        // The self-id `only-rule` belongs to `custom-rule`, not `custom-gate`.
        let err = exec
            .items()
            .show_item("@/custom-gate/only-rule")
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no addressable item"), "message: {msg}");
        assert!(msg.contains("gate"), "message: {msg}");

        // Sanity: the correct kind DOES resolve it.
        assert_eq!(
            exec.items()
                .show_item("@/custom-rule/only-rule")
                .unwrap()
                .item
                .kind,
            "custom-rule"
        );
    }

    #[test]
    fn test_show_item_named_project_matching_local_name_resolves_like_bare_at() {
        // REQ-03 (task a1b6b3da): `@<declared-name>/<kind>/<self-id>` routes
        // through `AddressScope::bind_local` and resolves identically to the
        // bare-`@/<kind>/<self-id>` form when the repo declares that name.
        let storage = InMemoryStorage::new();
        seed_memory_repository_file(
            &storage,
            "project-items.md",
            "## Success Criteria\n\n- GLOSS-01: all writes are atomic\n",
        );
        let config = format!("[project]\nname = \"acme\"\n\n{GLOSSARY_CONFIG}");
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&config),
                ..MemoryDeclarations::default()
            },
        );
        let named = exec.items().show_item("@acme/glossary/GLOSS-01").unwrap();
        let bare = exec.items().show_item("@/glossary/GLOSS-01").unwrap();
        assert_eq!(named.item, bare.item);
        assert_eq!(named.item.qualified_id, "@/glossary/GLOSS-01");
    }

    #[test]
    fn test_show_item_named_project_other_name_is_not_resolvable() {
        // REQ-03 (task a1b6b3da): a named-project address naming a project OTHER
        // than the locally declared one is syntactically valid but not
        // resolvable — the stopgap "not yet supported" error is replaced by
        // `ItemError::NotResolvable`, which names the unresolvable project and
        // states resolution is local-only.
        let storage = InMemoryStorage::new();
        seed_memory_repository_file(
            &storage,
            "project-items.md",
            "## Success Criteria\n\n- GLOSS-01: all writes are atomic\n",
        );
        let config = format!("[project]\nname = \"acme\"\n\n{GLOSSARY_CONFIG}");
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&config),
                ..MemoryDeclarations::default()
            },
        );
        let err = exec
            .items()
            .show_item("@other-project/glossary/GLOSS-01")
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("other-project"), "got: {msg}");
        assert!(msg.contains("local-only"), "got: {msg}");
    }

    #[test]
    fn test_show_item_named_project_no_declared_name_is_not_resolvable() {
        // REQ-02/REQ-03: with no `[project] name` declared at all, ANY
        // named-project reference is not resolvable, even one that happens to
        // name an item kind and self-id that do exist locally.
        let config = crate::test_utils::shipped_item_kind_tables(&["requirement"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                ..MemoryDeclarations::default()
            },
        );
        let err = exec
            .items()
            .show_item("@acme/requirement/REQ-01")
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("acme"), "got: {msg}");
        assert!(msg.contains("local-only"), "got: {msg}");
    }

    #[test]
    fn test_show_item_named_project_two_segment_form_is_not_legacy_project() {
        // Audit (jit:7a2bbe4f) REQ-02: a named-project two-segment form
        // `@<name>/<self-id>` (no kind segment) must NOT be mis-split into the
        // legacy bare-`@` project fallback: that fallback guards on
        // `scope == PROJECT_SCOPE_SENTINEL` exactly ("@"), not "@acme", so the
        // parser's own segment-count `InvalidAddress` surfaces instead of a
        // wrong "no addressable item" not-found.
        let config = crate::test_utils::shipped_item_kind_tables(&["requirement"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                ..MemoryDeclarations::default()
            },
        );
        let err = exec.items().show_item("@acme/GLOSS-01").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("segments"), "got: {msg}");
        assert!(!msg.contains("no addressable item"), "got: {msg}");
    }

    #[test]
    fn test_list_items_includes_project_scope() {
        // REQ-01: project-scoped items surface in the cross-repo list alongside
        // issue-scoped ones.
        let storage = InMemoryStorage::new();
        seed_memory_repository_file(
            &storage,
            "project-items.md",
            "## Success Criteria\n\n- GLOSS-01: project gloss\n",
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(GLOSSARY_CONFIG),
                ..MemoryDeclarations::default()
            },
        );
        let result = exec.items().list_items(None).unwrap();
        let qids: Vec<&str> = result
            .items
            .iter()
            .map(|i| i.qualified_id.as_str())
            .collect();
        assert!(qids.contains(&"@/glossary/GLOSS-01"));
    }

    #[test]
    fn test_project_items_rejects_path_traversal_source() {
        // Path-safety: a `..`-traversal source path is rejected by the storage
        // boundary, so `@/<kind>/<id>` resolution surfaces a typed InvalidPath error
        // rather than reading a file outside the repository.
        let config = "[item_kinds.glossary]\n\
            section = \"success_criteria\"\n\
            id-pattern = \"GLOSS-[0-9]+\"\n\
            markers = []\n\
            link-namespaces = [\"defines\"]\n\
            scope = \"project\"\n\
            source = \"../escape.md\"\n\
            source-of-truth = \"markdown-first\"\n";
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(config),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let err = exec.items().show_item("@/glossary/GLOSS-01").unwrap_err();
        let invalid_path = err
            .downcast_ref::<crate::storage::PathReadError>()
            .is_some_and(|error| matches!(error, crate::storage::PathReadError::InvalidPath(_)));
        assert!(
            invalid_path,
            "expected a typed invalid-path rejection, got: {err:#}"
        );
    }

    #[test]
    fn test_project_items_rejects_absolute_source() {
        // Path-safety: an absolute source path is rejected by the storage boundary.
        let config = "[item_kinds.glossary]\n\
            section = \"success_criteria\"\n\
            id-pattern = \"GLOSS-[0-9]+\"\n\
            markers = []\n\
            link-namespaces = [\"defines\"]\n\
            scope = \"project\"\n\
            source = \"/etc/passwd\"\n\
            source-of-truth = \"markdown-first\"\n";
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(config),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let err = exec.items().show_item("@/glossary/GLOSS-01").unwrap_err();
        let invalid_path = err
            .downcast_ref::<crate::storage::PathReadError>()
            .is_some_and(|error| matches!(error, crate::storage::PathReadError::InvalidPath(_)));
        assert!(
            invalid_path,
            "expected a typed invalid-path rejection, got: {err:#}"
        );
    }

    const TWO_POLICIES: &str = "\
[[policies]]
id = \"POL-01\"
statement = \"All writes are atomic.\"
enforced-by = [\"cargo-ci\", \"jit-validate\"]

[[policies]]
id = \"POL-02\"
statement = \"Every dependency edge stays acyclic.\"
";

    #[test]
    fn test_project_items_indexes_custom_toml_registry_kind() {
        // REQ-02: a NON-invariant registry-first kind backed by a custom `.toml`
        // through the declared field mapping indexes `@/<kind>/<self-id>` items, with
        // the toml read going through the storage boundary and link-fields surfacing
        // as namespace-qualified link labels.
        let storage = InMemoryStorage::new();
        seed_memory_repository_file(&storage, "policies.toml", TWO_POLICIES);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(POLICY_CONFIG),
                ..MemoryDeclarations::default()
            },
        );

        let first = exec.items().show_item("@/policy/POL-01").unwrap();
        assert_eq!(first.item.self_id, "POL-01");
        assert_eq!(first.item.qualified_id, "@/policy/POL-01");
        assert_eq!(first.item.scope, "@");
        assert_eq!(first.item.kind, "policy");
        assert!(first.item.text.contains("atomic"));
        // The `enforced-by` toml field maps to the `enforces` link namespace.
        assert_eq!(
            first.item.links,
            vec![
                "enforces:cargo-ci".to_string(),
                "enforces:jit-validate".to_string()
            ]
        );
        // No owning issue: a project-scope item.
        assert_eq!(first.issue_full_id, None);

        // An entry with no link field is graceful: it still indexes, with no labels.
        let second = exec.items().show_item("@/policy/POL-02").unwrap();
        assert_eq!(second.item.self_id, "POL-02");
        assert!(second.item.links.is_empty());

        // Both surface in the cross-repo list under the `@` project scope.
        let listed = exec.items().list_items(Some("policy")).unwrap();
        assert_eq!(listed.items.len(), 2);
    }

    #[test]
    fn test_project_items_custom_toml_absent_file_is_graceful() {
        // REQ-02 (degradation): with no descriptor toml seeded, the storage read
        // returns None and `@/<kind>/<id>` resolves to a descriptive not-found error,
        // never a panic and never the issue resolver.
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(POLICY_CONFIG),
                ..MemoryDeclarations::default()
            },
        );
        let err = exec.items().show_item("@/policy/POL-01").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("project scope"), "got: {msg}");
        assert!(!msg.contains("resolve issue scope"), "got: {msg}");
    }

    const TWO_INVARIANTS: &str = "\
[[invariants]]
id = \"sample-invariant\"
statement = \"Every dependency edge stays acyclic.\"
kind = \"enforced\"
enforced-by = \"dag-no-cycles\"

[[invariants]]
id = \"second-invariant\"
statement = \"Issues prefer functional style.\"
kind = \"advisory\"
";

    #[test]
    fn test_list_items_kind_invariant_returns_registry_entry() {
        // REQ-01: in a repo with the canonical `[item_kinds]` table and a
        // `.jit/invariants.toml`, `jit item list --kind invariant` returns each
        // loaded invariant addressed as `@/<kind>/<self-id>`.
        let config = crate::test_utils::shipped_item_kind_tables(&["invariant"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                invariants: Some(TWO_INVARIANTS),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let result = exec.items().list_items(Some("invariant")).unwrap();
        assert_eq!(result.items.len(), 2);
        let qids: Vec<&str> = result
            .items
            .iter()
            .map(|i| i.qualified_id.as_str())
            .collect();
        assert!(qids.contains(&"@/invariant/sample-invariant"));
        assert!(qids.contains(&"@/invariant/second-invariant"));
        // The self-id is the invariant's id; the statement is its text; the kind is
        // `invariant`; the scope is `@`.
        let first = result
            .items
            .iter()
            .find(|i| i.self_id == "sample-invariant")
            .unwrap();
        assert_eq!(first.kind, "invariant");
        assert_eq!(first.scope, "@");
        assert_eq!(first.text, "Every dependency edge stays acyclic.");
    }

    #[test]
    fn test_show_item_resolves_invariant_by_qualified_id() {
        // REQ-01: the generic resolver returns an invariant by its
        // `@/<kind>/<self-id>` address.
        let config = crate::test_utils::shipped_item_kind_tables(&["invariant"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                invariants: Some(TWO_INVARIANTS),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let shown = exec
            .items()
            .show_item("@/invariant/second-invariant")
            .unwrap();
        assert_eq!(shown.item.self_id, "second-invariant");
        assert_eq!(shown.item.qualified_id, "@/invariant/second-invariant");
        assert_eq!(shown.item.kind, "invariant");
        assert_eq!(shown.item.scope, "@");
        // No owning issue for a project-scope item.
        assert_eq!(shown.issue_full_id, None);
        assert_eq!(shown.issue_title, None);
    }

    /// A single migrated colon-free rule entry (`name`, no `default:`-style
    /// prefix), mirroring a real `.jit/rules.toml` row well enough to exercise
    /// the `rule` item kind's toml projection without pulling in the full
    /// `RawRule` schema (irrelevant here: `load_toml_scope_items` reads this as a
    /// generic `toml::Table`, never through the strict rule-engine parser).
    const ONE_RULE: &str = "\
[[rules]]
name = \"coverage-preview\"
origin = \"bracket\"
severity = \"error\"
enforce = true
";

    #[test]
    fn test_show_item_resolves_rule_by_qualified_id() {
        // REQ-03 (cdc33a0f): the generic resolver returns a migrated colon-free
        // rule id (e.g. `coverage-preview`) as a project-scope `rule` item via
        // the kind-segmented `@/rule/<self-id>` address (story 71ebd1e8 REQ-01),
        // mirroring the invariant kind's resolution path above but sourced from
        // `.jit/rules.toml`'s `rules` table.
        let config = crate::test_utils::shipped_item_kind_tables(&["rule"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                rules: Some(ONE_RULE),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let shown = exec.items().show_item("@/rule/coverage-preview").unwrap();
        assert_eq!(shown.item.self_id, "coverage-preview");
        // The minted qualified id carries the kind segment, so it equals the
        // kind-segmented address it resolved from (round-trip).
        assert_eq!(shown.item.qualified_id, "@/rule/coverage-preview");
        assert_eq!(shown.item.kind, "rule");
        assert_eq!(shown.item.scope, "@");
        assert!(!shown.item.text.is_empty());
        // No owning issue for a project-scope item.
        assert_eq!(shown.issue_full_id, None);
        assert_eq!(shown.issue_title, None);
    }

    /// Two gate registry entries shaped like real `.jit/gates.toml` rows well
    /// enough to exercise the `gate` kind's toml projection: only `key` and
    /// `description` matter here (`load_toml_scope_items` reads this as a generic
    /// `toml::Table`, never through the gate engine's stricter checker schema).
    const TWO_GATES: &str = "\
[[gates]]
key = \"cargo-ci\"
title = \"Cargo CI (fmt + clippy + tests)\"
description = \"Full Rust CI pipeline: formatting check, zero-warning clippy, and the workspace test suite must all pass.\"
stage = \"postcheck\"

[[gates]]
key = \"tests\"
title = \"All Tests Pass\"
description = \"Full test suite must pass\"
stage = \"postcheck\"
";

    #[test]
    fn test_list_items_kind_gate_returns_registry_entries() {
        // REQ-03: `jit item list --kind gate` returns every gate in `.jit/gates.toml`
        // as an addressable item, mirroring the existing invariant/rule behavior for
        // other project-scoped registry-first kinds.
        let config = crate::test_utils::shipped_item_kind_tables(&["gate"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                gates: Some(TWO_GATES),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let result = exec.items().list_items(Some("gate")).unwrap();
        assert_eq!(result.items.len(), 2);
        let qids: Vec<&str> = result
            .items
            .iter()
            .map(|i| i.qualified_id.as_str())
            .collect();
        assert!(qids.contains(&"@/gate/cargo-ci"));
        assert!(qids.contains(&"@/gate/tests"));
        let cargo_ci = result
            .items
            .iter()
            .find(|i| i.self_id == "cargo-ci")
            .unwrap();
        assert_eq!(cargo_ci.kind, "gate");
        assert_eq!(cargo_ci.scope, "@");
        assert_eq!(
            cargo_ci.text,
            "Full Rust CI pipeline: formatting check, zero-warning clippy, and the \
             workspace test suite must all pass."
        );
    }

    #[test]
    fn test_show_item_resolves_gate_by_qualified_id() {
        // REQ-02: `jit item show @/gate/<key>` resolves a configured gate's item,
        // whose text is that gate's description. Uses the kind-segmented address
        // (story 71ebd1e8 REQ-01); the kindless `@/<key>` form is no longer a valid
        // address at all.
        let config = crate::test_utils::shipped_item_kind_tables(&["gate"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                gates: Some(TWO_GATES),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let shown = exec.items().show_item("@/gate/cargo-ci").unwrap();
        assert_eq!(shown.item.self_id, "cargo-ci");
        // The minted qualified id carries the kind segment, so it round-trips with
        // the kind-segmented address it resolved from.
        assert_eq!(shown.item.qualified_id, "@/gate/cargo-ci");
        assert_eq!(shown.item.kind, "gate");
        assert_eq!(shown.item.scope, "@");
        assert_eq!(
            shown.item.text,
            "Full Rust CI pipeline: formatting check, zero-warning clippy, and the \
             workspace test suite must all pass."
        );
        // No owning issue for a project-scope item.
        assert_eq!(shown.issue_full_id, None);
        assert_eq!(shown.issue_title, None);
    }

    const COVERAGE_PREVIEW_GATE: &str = "\
[[gates]]
key = \"coverage-preview\"
title = \"Coverage Preview\"
description = \"Run scoped validation for the container resolved from the breakdown node's brackets label; blocks when a [hard] criterion is uncovered at plan time\"
stage = \"postcheck\"
";

    #[test]
    fn test_show_item_rule_and_gate_share_self_id_resolve_to_distinct_items() {
        // Load-bearing cross-kind collision (jit:42898915): `coverage-preview` is
        // BOTH a rule and a gate. Per-(scope, kind) uniqueness
        // (`domain::item::derive_scope_items`) keeps them distinct, and because the
        // kind is a segment of the minted id they now derive DISTINCT qualified-id
        // strings (`@/rule/coverage-preview` vs `@/gate/coverage-preview`). Each
        // kind-segmented address must resolve to its OWN item, never a silent
        // first-match or a spurious `DuplicateSelfId`.
        let config = crate::test_utils::shipped_item_kind_tables(&["rule", "gate"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                gates: Some(COVERAGE_PREVIEW_GATE),
                rules: Some(ONE_RULE),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );

        let rule = exec.items().show_item("@/rule/coverage-preview").unwrap();
        assert_eq!(rule.item.kind, "rule");
        assert_eq!(rule.item.self_id, "coverage-preview");
        assert_eq!(rule.item.text, "coverage-preview");
        assert_eq!(rule.item.qualified_id, "@/rule/coverage-preview");

        let gate = exec.items().show_item("@/gate/coverage-preview").unwrap();
        assert_eq!(gate.item.kind, "gate");
        assert_eq!(gate.item.self_id, "coverage-preview");
        assert_eq!(gate.item.qualified_id, "@/gate/coverage-preview");
        assert_eq!(
            gate.item.text,
            "Run scoped validation for the container resolved from the breakdown \
             node's brackets label; blocks when a [hard] criterion is uncovered at \
             plan time"
        );
        assert_ne!(rule.item.text, gate.item.text);
        assert_ne!(rule.item.qualified_id, gate.item.qualified_id);

        // Both coexist in one unfiltered list with no DuplicateSelfId despite the
        // shared self-id (they differ by kind).
        let all = exec.items().list_items(None).unwrap();
        assert_eq!(all.items.len(), 2);
    }

    #[test]
    fn test_search_items_finds_invariant_by_statement() {
        // The generic search path reaches registry-first invariants too.
        let config = crate::test_utils::shipped_item_kind_tables(&["invariant"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                invariants: Some(TWO_INVARIANTS),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let hits = exec
            .items()
            .search_items("acyclic", Some("invariant"))
            .unwrap();
        assert_eq!(hits.items.len(), 1);
        assert_eq!(hits.items[0].qualified_id, "@/invariant/sample-invariant");
    }

    #[test]
    fn test_invariant_registry_is_authoritative_no_markdown_index() {
        // REQ-02: the registry is the authoritative source for invariants and NO
        // markdown index is produced for them. An issue description containing an
        // invariant-looking line is NOT projected as an invariant item — invariants
        // come ONLY from `.jit/invariants.toml`.
        let issue = crate::test_utils::fixture_issue(
            "A".to_string(),
            "## Success Criteria\n\n- [hard] missing-invariant: a markdown line that LOOKS like an invariant\n"
                .to_string(),
        );
        let storage = InMemoryStorage::new();
        seed_issue(&storage, issue);
        let config = crate::test_utils::shipped_item_kind_tables(&["invariant"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&config),
                invariants: Some(TWO_INVARIANTS),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );

        let invariants = exec.items().list_items(Some("invariant")).unwrap();
        let qids: Vec<&str> = invariants
            .items
            .iter()
            .map(|i| i.qualified_id.as_str())
            .collect();
        // Only the two registry entries are invariants; the issue's missing-invariant line is
        // NOT among them (no markdown index for invariants).
        assert_eq!(invariants.items.len(), 2);
        assert!(qids.contains(&"@/invariant/sample-invariant"));
        assert!(qids.contains(&"@/invariant/second-invariant"));
        assert!(!qids.iter().any(|q| q.ends_with("/missing-invariant")));
        assert!(!qids.iter().any(|q| q.contains("missing-invariant")));
    }

    #[test]
    fn test_list_items_without_invariants_file_has_no_invariants() {
        // Registry-first: with no `.jit/invariants.toml`, the invariant kind yields
        // no items (graceful), and nothing is read from any markdown source.
        let issue = issue_with_criteria("A", "## Success Criteria\n\n- [hard] REQ-01: a\n");
        let storage = InMemoryStorage::new();
        seed_issue(&storage, issue);
        let config = crate::test_utils::shipped_item_kind_tables(&["invariant"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&config),
                ..MemoryDeclarations::default()
            },
        );
        assert_eq!(
            exec.items().list_items(Some("invariant")).unwrap().count(),
            0
        );
    }

    #[test]
    fn test_invariant_config_resolves_through_its_toml_descriptor() {
        // REQ-03: the `invariant` name carries no special routing — a config-declared
        // `[item_kinds.invariant]` (shipped) with a toml `source` descriptor indexes its
        // registry items exactly like any other registry-first kind. Here the
        // descriptor names `.jit/invariants.toml`, so `item list --kind invariant`
        // returns the registry entries through the GENERIC toml path.
        let config = crate::test_utils::shipped_item_kind_tables(&["invariant"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                invariants: Some(TWO_INVARIANTS),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        seed_issue(
            exec.storage(),
            issue_with_criteria(
                "A",
                "## Success Criteria\n\n- [hard] missing-invariant: looks like an invariant\n",
            ),
        );
        let result = exec.items().list_items(Some("invariant")).unwrap();
        let qids: Vec<&str> = result
            .items
            .iter()
            .map(|i| i.qualified_id.as_str())
            .collect();
        assert_eq!(result.items.len(), 2);
        assert!(qids.contains(&"@/invariant/sample-invariant"));
        assert!(qids.contains(&"@/invariant/second-invariant"));
        // The issue's missing-invariant line is NOT a project invariant (registry-first reads
        // only the toml, never a markdown section).
        assert!(!qids.iter().any(|q| q.contains("missing-invariant")));
    }

    #[test]
    fn test_issue_scoped_invariant_config_is_no_longer_reserved() {
        // REQ-03: a custom `[item_kinds.tenet]` declared registry-first BUT issue-scoped
        // (as the shipped `invariant` once was) was once rejected to keep invariants out of the issue-description parser.
        // With the reserved-name branch gone it resolves as an ORDINARY issue-scope
        // kind, indexing its items from issue descriptions like any other.
        let issue = issue_with_criteria(
            "A",
            "## Success Criteria\n\n- sample-invariant: an issue-scoped invariant line\n",
        );
        let short = issue.short_id();
        let config = "[item_kinds.tenet]\n\
            section = \"success_criteria\"\n\
            id-pattern = \"[a-z][a-z0-9-]*\"\n\
            markers = []\n\
            link-namespaces = [\"enforces\"]\n\
            scope = \"issue\"\n\
            source-of-truth = \"registry-first\"\n";
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(config),
                invariants: Some(TWO_INVARIANTS),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        seed_issue(exec.storage(), issue);
        // No rejection: the kind resolves and indexes from the issue description.
        let result = exec.items().list_items(Some("tenet")).unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(
            result.items[0].qualified_id,
            format!("@/issue/{short}/tenet/sample-invariant")
        );
    }

    #[test]
    fn test_registry_first_project_kind_without_descriptor_is_typed_error() {
        // REQ-03: a registry-first project kind that declares no toml `source`
        // descriptor has nothing to project from. It is rejected at kind resolution
        // with a typed MissingProjectSource (symmetric to a markdown-first project
        // kind missing its source) rather than silently producing no items.
        // `foo` is a custom kind.
        let config = "[item_kinds.foo]\n\
            section = \"success_criteria\"\n\
            id-pattern = \"FOO-[0-9]+\"\n\
            markers = []\n\
            link-namespaces = [\"foos\"]\n\
            scope = \"project\"\n\
            source-of-truth = \"registry-first\"\n";
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(config),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let err = exec.items().list_items(None).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("declares no 'source'") && msg.contains("foo"),
            "expected MissingProjectSource for 'foo', got: {msg}"
        );
        // The same typed rejection surfaces when the kind is filtered directly.
        assert!(exec.items().list_items(Some("foo")).is_err());
    }

    #[test]
    fn test_show_item_missing_self_id_errors() {
        let a = issue_with_criteria("A", "## Success Criteria\n\n- [hard] REQ-01: a\n");
        let short = a.short_id();
        let storage = InMemoryStorage::new();
        seed_issue(&storage, a);
        let config = crate::test_utils::shipped_item_kind_tables(&["requirement"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&config),
                ..MemoryDeclarations::default()
            },
        );
        let err = exec
            .items()
            .show_item(&format!("{short}/REQ-99"))
            .unwrap_err();
        assert!(err.to_string().contains("no addressable item"));
    }

    #[test]
    fn test_show_item_not_qualified_errors() {
        let config = crate::test_utils::shipped_item_kind_tables(&["requirement"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                ..MemoryDeclarations::default()
            },
        );
        let err = exec.items().show_item("REQ-01").unwrap_err();
        assert!(err.to_string().contains("not a qualified id"));
    }

    #[test]
    fn test_show_item_colon_in_project_address_self_id_surfaces_parser_error() {
        // A colon anywhere in an `@`-address surfaces the parser's own
        // InvalidAddress rejection (colon is the label separator, never valid
        // inside an address), not a generic not-found.
        let config = crate::test_utils::shipped_item_kind_tables(&["requirement"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                ..MemoryDeclarations::default()
            },
        );
        let err = exec.items().show_item("@/rule/foo:bar").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("colon"), "got: {msg}");
    }

    #[test]
    fn test_show_item_kindless_at_address_is_parse_error_not_self_id_match() {
        // REQ-02: the legacy kind-agnostic resolution arm is gone. A kindless
        // `@/<self-id>` is now just an address missing its kind segment, so it is a
        // clear InvalidAddress parse error naming the input, never a self-id-only
        // project match. The `.jit/rules.toml`-backed `coverage-preview` rule exists
        // and DOES resolve via the kind-segmented `@/rule/coverage-preview`, proving
        // the kindless form fails on grammar, not on a missing item.
        let config = crate::test_utils::shipped_item_kind_tables(&["rule"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                rules: Some(ONE_RULE),
                after_initialization: true,
                ..MemoryDeclarations::default()
            },
        );
        let err = exec.items().show_item("@/coverage-preview").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("cannot resolve item address '@/coverage-preview'"),
            "got: {msg}"
        );
        assert!(
            msg.contains("not a valid kind-segmented address"),
            "got: {msg}"
        );
        assert!(!msg.contains("no addressable item"), "got: {msg}");
        // The kind-segmented address still resolves the same item.
        assert_eq!(
            exec.items()
                .show_item("@/rule/coverage-preview")
                .unwrap()
                .item
                .kind,
            "rule"
        );
    }

    #[test]
    fn test_show_item_colon_in_sugar_self_id_surfaces_parser_error() {
        let config = crate::test_utils::shipped_item_kind_tables(&["requirement"]);
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            MemoryDeclarations {
                config: Some(&config),
                ..MemoryDeclarations::default()
            },
        );
        let err = exec.items().show_item("56ab0224/REQ:01").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("colon"), "got: {msg}");
    }
}
