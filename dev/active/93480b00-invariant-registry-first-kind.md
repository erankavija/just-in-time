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

## Rework (code-review attempt 1)

### Finding 1 — `invariant` reserved as registry-first

The `invariant` kind name is RESERVED as registry-first in
`ItemKind::from_config`: declaring `[item_kinds.invariant]` with any
`source-of-truth` other than `registry-first` (including the unset default,
`markdown-first`) is rejected with the typed `ItemError::InvariantMustBeRegistryFirst`.
This makes markdown-indexing of invariants impossible — invariants can ONLY come
from `.jit/invariants.toml` (REQ-02).

### Finding 2 — explicit registry binding in `project_items`

`project_items()` no longer treats every registry-first project kind as backed by
`config.invariants`. Routing binds by kind name: a registry-first kind that IS
`invariant` projects from the invariant registry; any OTHER registry-first project
kind has no registered registry source and is rejected with the typed
`ItemError::UnknownRegistrySource` (no invariant rows mislabeled under a foreign
kind name). The kind-name binding lives in the COMMANDS layer, keeping
`crates/jit/src/validation/` free of the `invariant` literal.

Pre-existing tests that had used `invariant` merely as a stand-in for a generic
markdown-first project-scope kind were renamed to the non-reserved `glossary`
kind, since `invariant` is now strictly registry-first.
