# Breakdown spec: Label-value grammar widening and re-split-site audit (0efbc594)

Story under epic 2821e177 (Addressing v2). Full plan: dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md (§2 grounding, §4 risks). Grounding investigation: dev/active/2821e177-investigation.md.

## Story description and criteria

Widen the label-value grammar so values may begin with `@`, optionally followed by a project name, and a deeper `/`-delimited path, while keeping the single-colon split that separates a label's `namespace:value` unchanged. Today jit's label-value regex admits no `@` character, so addresses of the new form cannot be authored as label values at all: a link such as `enforces:@/rule/label-format` would fail label validation before it ever reaches resolution.

Audit every site in the codebase that re-splits a label value beyond the namespace split, to confirm each routes correctly under the wider grammar: colon-splitting sites (namespace parsing) are unaffected, and slash-decoding sites must route through the structural address parser. The canonical label-value regex exists in two places today, the runtime validator and a separately maintained constant used to generate a JSON Schema; both must change together, with the generated schema regenerated, or the write path and validation diverge.

## Success Criteria

- [hard] REQ-01: the value grammar accepts `@/kind/self-id` and `@project/kind/self-id` forms at the grammar level and rejects a bare colon inside a value; grammar tests cover both accepted and rejected values. Resolution of named-project addresses is separate work outside this item.
- [hard] REQ-02: the two copies of the canonical label regex and the generated label-format JSON Schema change together and stay in lockstep.
- [hard] REQ-03: every value re-split site in the codebase is audited; colon-split (namespace) sites are confirmed unaffected, slash-decode sites route through the structural address parser, and the audit confirms the wider grammar breaks no existing parsing or resolution path.

Blast radius: the label-value regex and its duplicated constant, plus the generated label-format JSON Schema file.

## Plan §3 group context (the reviewed decomposition contract)

### Group A: Addressing core  — covers REQ-01, REQ-02

> Ownership split for the named-project segment (`@<name>`), amendment after cross-sibling
> coherence review: the Group A parser item owns the STRUCTURAL parse of every address form,
> including recognizing `@<name>` as a scope token; the Group A grammar item admits the form at
> the regex level only; the Group E item owns project identity (the config field, `jit init`
> seeding, write validation) and the local vs not-resolvable RESOLUTION of that scope token.

- **Kind-segmented scope-aware address parser and resolver**  `type: story`
  `satisfies: REQ-01`  `depends-on: —`  `gate: cargo-ci, code-review`
  `source: §1 REQ-01; §2 (grounding: split_qualified_id + 4 callers, derive_scope_items)`
  Outcome: an address of the form `@[<project>]/<kind>/<self-id>`, `@/issue/<short-id>/<kind>/<self-id>`,
  or `<short-id>/<self-id>` sugar is parsed, kind-resolved, and matched to its addressable item;
  the colon never appears inside a value.
  Own criteria:
  `[hard] REQ-01: project-item and issue-item forms parse into scope, kind, and self-id components.`
  `[hard] REQ-02: the sugar form expands to the canonical issue form by inferring kind from the id-pattern; ambiguous inference errors clearly.`
  `[hard] REQ-03: resolution matches on (scope, kind, self-id); per-scope self-id uniqueness is rescoped to per-(scope, kind) so same-slug different-kind items coexist.`
  Blast radius: `split_qualified_id` (`domain/item.rs:678-680`) and its four callers
  (`commands/item.rs` show_item/`:402`, `commands/validate.rs:317`, `validation/graph.rs:90`) plus
  the qualified path of `label_credits_id` (`validation/graph.rs:73-91`) are updated in this change;
  `derive_scope_items`/`DuplicateSelfId` (`domain/item.rs:746-750,776-786`, contract note at `:746`)
  uniqueness key gains the kind dimension; any existing test asserting `DuplicateSelfId` across
  kinds within one scope breaks under the rescope and is updated in the same change. The resolver
  takes the configured kind list as a parameter (like `resolve_item_kinds`) so the domain layer
  stays I/O-free and no kind literal leaks into engine code.

- **Label-value grammar widening and re-split-site audit**  `type: story`
  `satisfies: REQ-02`  `depends-on: Kind-segmented scope-aware address parser and resolver`
  `gate: cargo-ci, code-review, repo-validate`
  `source: §1 REQ-02; §2 (grounding: labels.rs regex, dual-source schema)`
  Outcome: label values admit a leading `@`, an optional project-name, and the deeper `/`-path while
  the single-colon namespace split is retained; the canonical regex and its generated JSON Schema
  stay in sync.
  Own criteria:
  `[hard] REQ-01: the value grammar accepts @/kind/self-id and @project/kind/self-id and rejects colon-in-value; grammar tests cover accepted and rejected values.`
  `[hard] REQ-02: the two copies of the canonical label regex (labels.rs and CANONICAL_LABEL_REGEX) and the generated default-label-format schema change together.`
  `[hard] REQ-03: every value re-split site is audited; the `:`-split sites are confirmed unaffected and the `/`-decode sites route through the new parser.`
  Blast radius: `labels.rs:24`, `CANONICAL_LABEL_REGEX` (`defaults.rs:55`),
  `.jit/schemas/default-label-format.json`; audit set enumerated in investigation §2 REQ-02
  (unrelated `:`-split and file-path `/`-split sites confirmed out of scope).

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
  convention beyond AGENTS.md's invariant region; it is config-declared so the target can be chosen
  at the REQ-07 item without reopening the plan. Risk if wrong: cosmetic, contained to that item.

