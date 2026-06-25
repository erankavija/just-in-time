# Item-kind six-tuple + `kind=` coverage sugar (jit:1e1ea81d)

Design note for the Group A registry extension of epic 25064508.

## What changed

1. **Sixth field `source-of-truth`** (config layer, `config.rs`). A new typed
   `SourceOfTruth` enum (`markdown-first` | `registry-first`, serde-renamed
   kebab-case tokens, default `MarkdownFirst`) added to `ItemKindConfig` as
   `source_of_truth`. It is the authoring DIRECTION and is DISTINCT from the
   existing `source` PATH field. `ItemKindConfig::source_of_truth()` resolves the
   default. An invalid token is a descriptive serde parse error.

2. **Pure kind→triple resolver** (`domain/item.rs`). `expand_kind_triple(registry,
   name) -> Result<KindTriple, ItemError>` resolves a named kind through the
   existing `ItemKind::from_config` + `ItemKind::as_triple` (one code path, repo
   defaults and id-pattern validation applied identically to indexing). An
   undeclared name is the new typed `ItemError::UnknownKind`.

3. **`kind=` sugar for `label-coverage`** (`validation/rules.rs`). At rule-load
   time (`expand_kind_sugar`), a `label-coverage` assert table carrying
   `kind = "<name>"` has the kind's `(section, marker, id-pattern)` triple spliced
   in as the `criteria-section` / `marker` / `id-pattern` keys and the `kind` key
   removed. The engine (`validation/graph.rs`) then evaluates a plain inline-triple
   table and never sees a kind NAME.

## Key design choices

- **Expansion happens in the config layer, not the engine.** This is what keeps
  REQ-05 true: the kind name is read only where `[item_kinds]` is available
  (`rules.rs` reading the generic `"kind"` key), and the engine consumes the
  resolved triple. The REQ-05 grep over `crates/jit/src/validation/` finds no
  `"requirement"`/`"decision"`/`"risk"`/`"invariant"` literal outside
  `#[cfg(test)]`.

- **Additive and scoped to free-form tables.** Only `label-coverage` (whose assert
  is `Option<toml::value::Table>`) gets the sugar. Inline-triple rules carry no
  `kind` key and pass through `expand_kind_sugar` untouched (REQ-03); an
  unrecognized extra key still round-trips. The typed `criteria-to-check` /
  `criteria-label-match` rules keep `deny_unknown_fields` and get no `kind=`
  (REQ-04).

- **Mixing `kind` with an inline triple key is a config error** (ambiguous: the
  sugar REPLACES the triple). Unknown kind / non-string `kind` are config errors,
  never silent passes.

- **One resolver, no duplicated triple logic.** Both the config sugar and the
  domain layer go through `ItemKind::as_triple`.

## REQ→evidence

- REQ-01: six fields on `ItemKindConfig`; `SourceOfTruth` + `source_of_truth()`.
- REQ-02: `expand_kind_triple` == `as_triple`; end-to-end eval test asserts a
  `kind="requirement"` rule yields identical findings to the inline form.
- REQ-03: inline rules untouched; extra-key round-trip test.
- REQ-04: typed rules keep `deny_unknown_fields`, no `kind=`.
- REQ-05: engine reads the triple; grep clean outside `#[cfg(test)]`.
- REQ-06: cross-scope same-self-id resolves to distinct qualified ids; bare
  `satisfies:REQ-NN` still credits the container criterion.
