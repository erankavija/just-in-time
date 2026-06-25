# Design note: `.jit/invariants.toml` schema and config-time loader (21558ace)

Group C of epic 25064508 (REQ-05). Scope: the registry schema, the config-time
loader, and typed errors only. Item-indexing / `jit item` wiring and the
`@/<self-id>` projection are the NEXT issue (93480b00).

## Schema

`.jit/invariants.toml` is an array-of-tables, mirroring the `[[rules]]` idiom of
`.jit/rules.toml` (NOT the `[namespaces.*]` map idiom): an array preserves
authored order and lets the loader detect duplicate ids, which a map keyed by id
cannot. Each entry:

```toml
[[invariants]]
id = "INV-01"                                   # self-id; @/INV-01 is its qualified id
statement = "Every dependency edge stays acyclic."  # required
kind = "enforced"                               # required: "enforced" | "advisory"
enforced-by = "dag-no-cycles"                   # optional: rule name or gate key
```

- `id` is the entry's SELF-ID. The project-scoped qualified id `@/<id>` is derived
  from it by the downstream indexer (93480b00). Ids must be unique.
- `kind` is a typed enum (`InvariantKind`) deserialized via `#[serde(rename_all =
  "kebab-case")]` with no `Default`, so the field is required and an unknown token
  (`"mandatory"`, `"both"`, …) is a descriptive serde parse error listing the
  valid values.
- `enforced-by` (snake-cased to `enforced_by` in Rust) is `Option<String>`.

## Code shape

- `crates/jit/src/validation/invariants.rs` — pure parse/load, mirroring
  `RuleSet::load` (`validation/rules.rs:903`):
  - `InvariantRegistry { invariants: Vec<Invariant> }` with `empty()`, `load(jit_root)`,
    `from_toml_str(content)`.
  - `load` returns an empty registry when the file is absent (graceful, NOT an
    error), exactly like `RuleSet::load`.
  - `Invariant { id, statement, kind, enforced_by }`, `InvariantKind { Enforced, Advisory }`.
  - `InvariantConfigError` (`thiserror`): `Io { path, source }`, `Toml(#[from]
    toml::de::Error)` (covers missing required field and bad `kind` token), and
    `DuplicateId { id }`.
  - Registered in `validation/mod.rs`.

- `crates/jit/src/config.rs` — wiring (config layer):
  - New `#[serde(skip)] pub invariants: InvariantRegistry` field on `JitConfig`,
    defaulting to empty, mirroring the `templates` field precedent. Kept here so
    93480b00 can consume the loaded registry.
  - Loaded on BOTH `JitConfig::load` return paths: the config-absent early return
    (struct literal) and the config-present path (chain-load after
    `validate_item_kinds`). Both `.context("invalid .jit/invariants.toml")` so a
    malformed/invalid entry fails config load with a path-bearing message.

## Layer boundaries

The registry struct + loader are pure (no config-layer knowledge); the
`JitConfig::load` wiring is the config layer. No `.jit/` lifecycle state is
touched and no live `.jit/invariants.toml` is created (the loader handles its
absence; tests use temp fixtures).
