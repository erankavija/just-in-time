# Design: Addressable structured items, issue scope (56ab0224)

Foundational child of epic 25064508. Delivers the issue-scoped half of the
addressable-item model that Groups A–E extend (project scope, more kinds,
invariant registry/projection/drift). This note is the durable contract for that
foundation; the epic plan
(`dev/active/25064508-d563-4073-a970-296607a01adc-plan.md`) is the authoritative
spec.

## What ships

- **`[item_kinds]` config registry** (`crates/jit/src/config.rs`,
  `ItemKindConfig`): a `Option<HashMap<String, ItemKindConfig>>` mirroring
  `[namespaces.*]`. A kind is the four-tuple `(section, id-pattern, marker(s),
  link-namespace(s))`. No kind NAME is interpreted by engine logic; `requirement`
  is just one configuration (REQ-01).
- **Pure item model** (`crates/jit/src/domain/item.rs`): `ItemKind` (resolved
  four-tuple with repo defaults), `AddressableItem`, `qualified_id`/
  `split_qualified_id`, `index_items` (pure projection over the issue body via
  the existing `ContentParser`), `resolve_item_kinds`. Self-id uniqueness within
  an issue is validated; a list line with no self-id is plain prose (REQ-02,
  REQ-03, REQ-06).
- **`jit item list/show/search/resolve`** (`crates/jit/src/commands/item.rs`,
  CLI in `cli.rs`, dispatch in `main.rs`): query and resolve items across the
  repo by kind and qualified id, each with `--json` (REQ-04).

## Key decisions

- **Qualified id is derived, never stored.** `qualified_id(issue.short_id(),
  self_id)` = `<issue-id>/<self-id>`. The markdown description stays the single
  source of truth; the index is recomputed on demand (REQ-03).
- **Reuse, don't fork, the engine's genericity.** `ItemKind::as_triple()`
  exposes the exact `(section, marker, id-pattern)` triple that `criterion_ids`
  / `label-coverage` (`validation/graph.rs`) already consume. The built-in
  `requirement` default reproduces that rule's defaults
  (`success_criteria` / `[hard]` / `[A-Z][A-Z0-9]*-[0-9]+` / `satisfies`), so the
  coverage machinery is compatible with the item model with no rule rewrite —
  shown by a test, not by modifying any live ruleset (REQ-05).
- **Layer boundaries.** Derivation + indexing are pure and live in `domain/`;
  registry/storage reads go through config + the `IssueStore` trait; the
  `main.rs` arm is a thin delegation to `commands/item.rs`.

## Out of scope (later epic groups)

Project (`@`) scope, the full six-tuple (`scope`, `source-of-truth`),
decision/risk/invariant kinds, the invariant registry/projection/drift, and the
`jit invariant` CLI. The kind model is generic enough that those slot in as
config plus additive code, not a redesign.
