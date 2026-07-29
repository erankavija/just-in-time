# Item-kind six-tuple + `kind=` coverage sugar (jit:1e1ea81d)

Design note for the Group A registry extension of epic 25064508.

## What changed

1. **Sixth field `source-of-truth`** (config layer, `config.rs`). A new typed
   `SourceOfTruth` enum (`markdown-first` | `registry-first`, serde-renamed
   kebab-case tokens, default `MarkdownFirst`) added to `ItemKindConfig` as
   `source_of_truth`. It is the authoring DIRECTION and is DISTINCT from the
   existing `source` PATH field. `ItemKindConfig::source_of_truth()` resolves the
   default. An invalid token is a descriptive serde parse error.

2. **Required-six validation for explicit declarations** (config layer). Each
   field stays `Option` at the serde layer, but an EXPLICITLY-declared
   `[item_kinds.X]` table must set all six (`section`, `id-pattern`, `markers`,
   `link-namespaces`, `scope`, `source-of-truth`). `ItemKindConfig::
   missing_required_fields()` reports absentees by authored key; `JitConfig::
   validate_item_kinds()` (called from `JitConfig::load`) rejects a partial
   declaration with the typed `ItemKindConfigError::MissingFields` naming the kind
   and the missing keys. The IMPLICIT built-in `requirement` default (no
   `[item_kinds]` table at all) is never validated, so graceful degradation is
   preserved. The optional `source` PATH is NOT one of the six.

3. **Pure kind→triple resolver** (`domain/item.rs`). `expand_kind_triple(registry,
   name) -> Result<KindTriple, ItemError>` resolves a named kind through the
   existing `ItemKind::from_config` + `ItemKind::as_triple` (one code path, repo
   defaults and id-pattern validation applied identically to indexing). An
   undeclared name is the new typed `ItemError::UnknownKind`.

4. **`kind=` sugar for `label-coverage`** (`validation/rules.rs`). Purely additive,
   non-regressing (`expand_kind_sugar`):
   - If the assert table carries ANY inline triple key (`criteria-section` /
     `marker` / `id-pattern`), it is left EXACTLY as authored; a `kind` key, if
     present, stays inert and ignored (precisely how an unrecognized key behaved
     before this feature). An existing inline rule, with or without a stray
     `kind`, evaluates unchanged.
   - Only a kind-ONLY table (no inline triple key) expands: the named kind's
     `(section, marker, id-pattern)` triple is spliced in as the inline keys and
     the `kind` key removed. The engine (`validation/graph.rs`) then evaluates a
     plain inline-triple table and never sees a kind NAME.

## Key design choices

- **Expansion happens in the config layer, not the engine.** This is what keeps
  REQ-05 true: the kind name is read only where `[item_kinds]` is available
  (`rules.rs` reading the generic `"kind"` key), and the engine consumes the
  resolved triple. The REQ-05 grep over `crates/jit/src/validation/` finds no
  `"requirement"`/`"decision"`/`"risk"`/`"invariant"` literal outside
  `#[cfg(test)]`.

- **Inline keys win; `kind` is inert when they are present.** This is the
  non-regression guarantee: no existing inline rule changes behavior, even one
  that already carried a stray `kind` key. Only a kind-ONLY table triggers
  expansion. An unknown kind NAME used as the sole triple source is still a typed
  error; an unknown `kind` next to inline keys is inert (never inspected).

- **One resolver, no duplicated triple logic.** Both the config sugar and the
  domain layer go through `ItemKind::as_triple`.

- **Typed rules untouched.** The `criteria-to-check` / `criteria-label-match`
  rules keep `deny_unknown_fields` and get no `kind=` (REQ-04).

## REQ→evidence

- REQ-01: six fields on `ItemKindConfig`; `SourceOfTruth` + `source_of_truth()`;
  explicit declarations require all six (`validate_item_kinds`,
  `missing_required_fields`, `ItemKindConfigError::MissingFields`); implicit
  default still degrades gracefully.
- REQ-02: `expand_kind_triple` == `as_triple`; end-to-end eval test asserts a
  kind-only `kind="requirement"` rule yields identical findings to the inline
  form.
- REQ-03: inline rules untouched; extra-key round-trip; a stray inert `kind`
  alongside inline keys evaluates identically to no-`kind`.
- REQ-04: typed rules keep `deny_unknown_fields`, no `kind=`.
- REQ-05: engine reads the triple; grep clean outside `#[cfg(test)]`.
- REQ-06: cross-scope same-self-id resolves to distinct qualified ids; bare
  `satisfies:REQ-NN` still credits the container criterion.
