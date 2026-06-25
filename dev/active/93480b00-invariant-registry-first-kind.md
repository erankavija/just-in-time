# 93480b00 — Index and query invariants as a registry-first kind

## Design

Invariants are surfaced through the generic addressable-item operations as a
**project-scoped, registry-first** built-in kind. Unlike `requirement`,
`decision`, and `risk` (markdown-first, parsed from issue-description sections),
the `invariant` kind derives its items directly from the loaded invariant
registry (`.jit/invariants.toml`, surfaced as `JitConfig::invariants`). The
registry is the authoritative source; NO markdown index is produced or consulted
for invariants.

### Routing by source-of-truth

`ItemKind` now carries the resolved `SourceOfTruth` (threaded from
`ItemKindConfig::source_of_truth()`), with a `source_of_truth()` accessor.
`commands/item.rs::project_items()` routes each project-scope kind:

- `RegistryFirst` (invariant): projects `config.invariants` into `RawScopeItem`s
  (`self_id = id`, `text = statement`) via the free `invariant_raw_items` helper.
- `MarkdownFirst`: the existing `read_repo_file` + scan path, unchanged.

Both substrates feed ONE `index_project_sources(sources, registry_items, parser)`
call, which pools all candidates into a single `derive_scope_items(Scope::Project,
..)` pass — so per-scope uniqueness and `@/<self-id>` derivation are identical
across substrates (and cross-substrate self-id collisions are detected).

### Guard relaxation

`ItemKind::from_config` previously rejected any project-scope kind without a
`source` file (`MissingProjectSource`). That guard now applies only to
markdown-first project kinds; a registry-first project kind is exempt because its
items come from the registry, not a markdown file.

### Why issue descriptions never produce invariant items

`issue_item_kinds()` filters out project-scope kinds, so the project-scoped
`invariant` kind is never applied to issue-description parsing. An `INV-`-looking
line in an issue description is therefore NOT projected as an invariant (REQ-02).

### Engine name-freedom

The `invariant` kind name and its constructor live in the DOMAIN layer
(`domain/item.rs::invariant_default`, `INVARIANT_KIND_NAME`); no kind-name literal
appears in `crates/jit/src/validation/`.
