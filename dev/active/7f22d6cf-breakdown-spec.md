# Breakdown spec: Uniform re-addressing of all kinds with repo data and doc migration (7f22d6cf)

Story under epic 2821e177 (Addressing v2). Full plan: dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md (§2 grounding, §4 risks). Grounding investigation: dev/active/2821e177-investigation.md.

## Story description and criteria

Re-address the existing invariant, requirement, decision, risk, and definition kinds uniformly under the new `@[<project>]/<kind>/<self-id>` address scheme. Migrate this repository's own live data and documentation so no legacy-form address remains: the legacy forms are the old kindless project address (for example `@/INV-01`, with no kind segment) and colon-prefixed rule identifiers. The `<short-id>/<self-id>` sugar form (for example `56ab0224/REQ-01`) is not legacy and remains valid; it is excluded from the legacy-address scan.

Confirm that bare `satisfies:REQ-NN` coverage-credit labels are unaffected by the address change: they are container-criterion credits, not qualified-id address links, and no qualified-form `satisfies:` label exists in live data.

## Success Criteria

- [hard] REQ-01: all existing kinds (invariant, requirement, decision, risk, definition) resolve under the uniform `@[<project>]/<kind>/<self-id>` scheme; bare `satisfies:REQ-NN` coverage labels are confirmed unchanged by the migration.
- [hard] REQ-02: `jit validate` runs green after migration.
- [hard] REQ-03: a tree-wide grep across source, `.jit/` data, docs, and test fixtures shows no legacy-form address (the old kindless project form such as `@/INV-*`, and colon-prefixed rule ids) remains; the `<short-id>/<self-id>` sugar form is confirmed valid and excluded from this grep.

Blast radius: the four `invariants.toml` `enforced-by` bindings, already migrated as a prerequisite and verified green here; a documentation hygiene sweep of the design brief and prior investigation/completion-report docs that reference the old address forms; stale CLI help text and examples that show pre-migration address forms.

## Plan §3 group context (the reviewed decomposition contract)

### Group D: Projection and migration  — covers REQ-07, REQ-08

- **Rules and gates reference projection**  `type: task`
  `satisfies: REQ-07`  `depends-on: Rule as an addressable kind with origin lifted out of the id, Gate as an addressable kind over migrated gates.toml`
  `gate: cargo-ci, code-review`
  `source: §1 REQ-07; §2 (grounding: projection mechanism, not the invariant renderer); D8`
  Outcome: a rules/gates reference document renders from the registry into a config-declared target
  region.
  Own criteria:
  `[hard] REQ-01: a new render function produces the rules/gates reference from the registry using the projection mechanism (ProjectionMode/ProjectionStyle/splice_region/atomic write), not the invariant-typed renderer.`
  `[hard] REQ-02: the target file and style are config-declared; a render test produces the reference from the registry.`
  Blast radius: new renderer beside `projection.rs:143-199`; reuses `splice_region`
  (`:223-263`) and the `project_invariants` orchestration (`:305-344`). Self-contained given Group B.

- **Uniform re-addressing of all kinds with repo data and doc migration**  `type: story`
  `satisfies: REQ-08`  `depends-on: Rebind invariant enforced-by bindings to addressable items`
  `gate: cargo-ci, code-review, repo-validate`
  `source: §1 REQ-08; §2 (grounding: near-zero live blast radius, satisfies bare form untouched); D9, D11`
  Outcome: invariant/requirement/decision/risk/definition are addressed uniformly under the new
  scheme, this repo's live data and docs carry no legacy-form address, and `jit validate` is green.
  Own criteria:
  `[hard] REQ-01: all existing kinds resolve under the uniform @[<project>]/<kind>/<self-id> scheme; bare satisfies:REQ-NN coverage labels are confirmed unchanged.`
  `[hard] REQ-02: `jit validate` is green after migration.`
  `[hard] REQ-03: a tree-wide grep (including docs and fixtures) shows no legacy-form address (the old kindless project form such as @/INV-*, and colon-prefixed rule ids) remains in live .jit/ data or docs; <short-id>/<self-id> sugar is valid per the address scheme and is outside this grep.`
  Blast radius: live data is only the four `enforced-by` strings (handled upstream by the rebind
  item, verified green here) plus doc hygiene sweep of `dev/active/21558ace-invariants-registry.md`,
  `dev/active/90a2dbfd-kinds-over-sources.md`, the design brief, and `cli.rs:413-442` help examples.

## Plan decisions (binding)

## Decisions

Resolves all 12 open points of brief §8. Provisional; premise-shaky ones marked REOPEN.

- **D1 — Value-grammar exact form (point 1):** chosen **extend the value class to admit an optional
  leading `@` followed by an optional project-name and `/`-delimited segments**, keeping `/` (already
  allowed) and the single-colon namespace split unchanged. Concretely the value grammar accepts
  `@`, `@<project-name>`, and `<segment>(/<segment>)*`; the exact regex is finalized in the REQ-02
  item and regenerated into the schema in lockstep. Rejected: widening the value grammar to allow `:`
  in values (parse-safe via `splitn(2,':')` but reads ambiguously and keeps legacy colon ids alive,
  contradicting the colon-reserved decision).

- **D2 — Rule identity refactor + collision (points 2):** chosen **split `origin` and a clean `id`
  into separate fields in `rules.toml`'s schema** (a data migration of the 7 seed rules), and
  **rescope per-scope self-id uniqueness to per-(scope, kind)** so the kind segment disambiguates
  `@/rule/coverage-preview` from `@/gate/coverage-preview`. The `namespace-unique:{param}` family
  becomes a colon-free slug (`namespace-unique-<param>`). No rule rename is needed for correctness
  once the kind segment is address-significant. Rejected: adding a transform capability to
  `TomlSourceDescriptor` (`config.rs:589-608` has none; a schema split is simpler and keeps the
  descriptor a verbatim reader); relying on a flat per-scope id space with prefix stripping (breaks
  on the `coverage-preview` collision, `DuplicateSelfId`).

- **D3 — Sugar + inference (point 3):** chosen **sugar `<short-id>/<self-id>` expands to
  `@/issue/<short-id>/<kind>/<self-id>` with kind inferred by id-pattern for issue-scoped items
  only; project-scoped rule/gate addresses require the explicit kind segment (no sugar)**; ambiguous
  inference is a clear error. Rationale: the four shipped kinds have visually distinct id-patterns
  (`REQ-\d+`, `D-\d+`, `RISK-\d+`, `[A-Z][A-Z0-9]*-\d+`) so issue-scoped inference is unambiguous,
  while rule/gate slugs (`label-format`, `cargo-ci`) carry no distinguishing shape and
  `id_pattern` is not enforced at index time for registry-first kinds
  (`load_toml_scope_items`/`project_toml_entry`, `domain/item.rs:1117-1122`; the markdown extractor
  `extract_raw_items` at `:870` is the path that applies it). Rejected: project-item sugar
  `@/<self-id>` without kind (collision-prone for rule/gate, per investigation §5.3).

- **D4 — `issue` segment status (point 4):** chosen **`issue` is a reserved, built-in first-class
  address segment, not an `[item_kinds]` entry; `@/issue/<short-id>/…` is a prefix for issue-scoped
  items**. A bare `@/issue/<id>` as a standalone resolvable "issue view" is not built this round
  (keeps scope minimal; not required by any REQ). Rejected: declaring `issue` as an item kind
  (issues are a separate storage substrate, `IssueStore`, with no `[item_kinds.issue]` today,
  investigation §5.4).

- **D5 — Project identity (point 5):** chosen **add a canonical `[project] name` field to
  `config.toml`, seeded by `jit init` (default: slugified directory basename), validated as
  `[a-z][a-z0-9-]*` on write**. Bare `@` is the local shorthand; `@<name>` where `name` equals the
  declared project resolves locally; any other `@<other>` is parse-valid and returns a
  not-resolvable error. Rejected: putting the name in `index.json` (machine metadata:
  `schema_version`/`all_ids`/`deleted_ids` only, not human-editable); reusing the server's
  display-only `project_name` (`crates/server/src/main.rs:83-90`, not read by `crates/jit`, not
  persisted). This is engineered, not deferred — greenfield per investigation §5.5.

- **D6 — Gate substrate migration (point 6):** chosen **migrate `gates.json` → `gates.toml` with a
  dedicated store mirroring `storage/ruleset_store.rs`, and declare `gate` as a registry-first kind
  over it (id-field = key, text-field = description)**; the typed `Gate`/`GateChecker` load path and
  the item-projection path both read `gates.toml`. Guard `Gate.reserved` against JSON null on TOML
  serialization. Rejected: adding a JSON source descriptor to the item-kind engine (would fork the
  descriptor away from its toml-only design, `config.rs:589-608`, and keep gates on JSON contrary to
  the no-legacy decision).

- **D7 — `enforced-by` mechanics (point 7):** chosen **`enforced-by` stays a typed single-string
  binding in `invariants.toml`, rebound to the new `@/rule/…`/`@/gate/…` addresses; it is not
  additionally surfaced as an `enforces:` item link**. The authoring path `enforces:@/…` (REQ-06) is
  the separate, work-item-side link. Rejected: double-wiring `enforced-by` as both a typed binding
  and an `enforces:` label (redundant; `enforced_by` is `Option<String>`, consumed by validation
  only, `projection.rs:165-169`).

- **D8 — Reference projection (point 8):** chosen **a new `render_rules_gates_markdown` function
  following the projection pattern, rendered into a config-declared target region using the existing
  `ProjectionStyle` field**; default target a dedicated reference doc region. Rejected: calling the
  existing `render_invariants_markdown` (it is `&InvariantRegistry`-typed, not generic,
  `projection.rs:143-199`).

- **D9 — Re-addressing migration approach (point 9):** chosen **manual migration**: the live blast
  radius is the four `enforced-by` strings (handled by the rebind item); doc updates are a hygiene
  sweep; acceptance is green `jit validate` plus a tree-wide legacy-address grep. Rejected: a
  migration script (over-engineered for four live strings; investigation §REQ-08 confirms
  `satisfies:REQ-NN` bare labels and all other link namespaces need no data migration).

- **D10 — `satisfies:`/coverage interaction (point 10):** chosen **confirm `satisfies:REQ-NN` is
  unaffected**; its bare-form path in `label_credits_id` (`validation/graph.rs:73-91`) is untouched,
  and its qualified path shares `split_qualified_id`, updated in lockstep by the REQ-01 item. No
  coverage-id remapping. Rejected: remapping coverage ids under the address scheme (they are
  container-criterion credits, not qualified-id links; zero qualified `satisfies:` labels exist).

- **D11 — Backward-compat window (point 11):** **clean cut, no shim** — settled by container REQ-08
  ("no legacy-form addresses remain"). The legacy-form set is exactly (a) the old kindless project
  form `@/<self-id>` (e.g. `@/INV-01`) and (b) colon-prefixed rule ids. `<short-id>/<self-id>`
  (e.g. `56ab0224/REQ-01`) is NOT legacy: container REQ-01 mandates it remains valid sugar for the
  canonical issue form. No dual-form/legacy-resolution scaffolding exists to preserve
  (investigation §5.11). Rejected: a compatibility window resolving old kindless `@/INV-*` forms
  during migration (contradicts REQ-08; no existing shim to build on).

- **D12 — Scope of one effort (point 12):** **one epic** — settled by the container's existence and
  its "this epic states the end-state requirements." The code dependency order (parser core →
  rule/gate kinds → enforcement/projection/migration; multi-jit off the parser) matches the §3
  grouping. Rejected: splitting into a milestone of several epics (unnecessary; the waves are
  internal groups, each independently landable).

- **Assumptions:** the rules/gates reference doc's exact target file/heading (D8) has no existing
  convention beyond CLAUDE.md's invariant region; it is config-declared so the target can be chosen
  at the REQ-07 item without reopening the plan. Risk if wrong: cosmetic, contained to that item.

