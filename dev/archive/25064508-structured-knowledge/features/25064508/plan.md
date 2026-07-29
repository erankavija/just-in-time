# Plan: Structured project knowledge in jit: addressable items, qualified ids, and projected invariants (25064508)

> Planning node: 0d753ba5. Container criteria source: 25064508 `## Success Criteria`.

## Success Criteria

- [hard] REQ-01: Every addressable item has a globally-unique qualified id `<scope>/<self-id>`, scope = issue short-id or `@` (project); the id is derived (not separately stored); self-id is human-authored and unique within scope.
- [hard] REQ-02: Item kinds are declared in config as `(section, id-pattern, marker(s), link-namespace(s), scope, source-of-truth)`; no kind name is hardcoded in engine logic.
- [hard] REQ-03: Source-of-truth direction is per kind — requirement/decision/risk are markdown-first (authored in descriptions, indexed); invariant is registry-first.
- [hard] REQ-04: `requirement`, `decision`, `risk`, and `invariant` are all implemented, indexed, and queryable, demonstrating generality across the issue-scoped and project-scoped substrates.
- [hard] REQ-05: Invariants live in `.jit/invariants.toml` with `statement`, `kind` (enforced|advisory), and optional `enforced-by` rule/gate binding.
- [hard] REQ-06: Invariants project into a configurable doc target (delimited region of an existing file, or a separate file); the target is config-driven and no documentation filename is hardcoded in engine logic.
- [hard] REQ-07: A bidirectional enforcement-drift check (declared-but-unenforced + enforced-but-undeclared) is runnable via validate.
- [hard] REQ-08: Generic node→item link namespaces reference qualified ids; the existing `label-coverage` / `criteria-to-check` rules are re-expressed as instances over the model or shown compatible.
- [hard] REQ-09: CLI exposes `jit item list/show/search/resolve` (with `--kind`) and `jit invariant render/check`.
- [hard] REQ-10: Indexes and qualified ids are pure projections; addressing degrades gracefully (a line without a self-id is not required to be addressable).

## 1. Completeness vs criteria

How the plan addresses every `[hard]` criterion. The foundational child `56ab0224` (already `Ready`, gated, unimplemented) carries the issue-scoped half of the model; new groups extend it to project scope, add three further kinds, the invariant registry, doc projection, drift, and the `jit invariant` CLI. No criterion is silently narrowed.

| Criterion | Addressed by | Notes |
|---|---|---|
| REQ-01 | 56ab0224 (issue-scope `<issue-id>/<self-id>`) + Group A (widen to `<scope>/<self-id>` with `@` project scope) | Derivation stays a pure projection over `short_id()` + parsed self-id; `@` scope is the superset 56ab0224 does not deliver. |
| REQ-02 | 56ab0224 (issue kinds) + Group A (six-tuple schema adds `scope`, `source-of-truth`) | `[item_kinds]` registry mirroring `[namespaces.*]`; no kind name in engine logic. |
| REQ-03 | Group A (per-kind `source-of-truth` field) + Group B (markdown-first decision/risk) + Group C (registry-first invariant read path) | Both directions exercised: markdown-first index for requirement/decision/risk, registry-first for invariant. |
| REQ-04 | 56ab0224 (`requirement`) + Group B (`decision`, `risk`) + Group C (`invariant`) + Group E (cross-substrate generality demo) | All four kinds indexed and queryable; Group E is the explicit generality assertion across both substrates. |
| REQ-05 | Group C (`.jit/invariants.toml` schema + loader) | `statement`, `kind` (enforced\|advisory), optional `enforced-by`. |
| REQ-06 | Group D (config-driven doc target, both modes, no hardcoded filename) | Default ships a separate jit-owned file (D3); region mode also shipped. |
| REQ-07 | Group D (bidirectional drift validate rule + `jit invariant check`) | Declared-but-unenforced and enforced-but-undeclared, both directions tested. |
| REQ-08 | 56ab0224 (link namespaces over qualified ids at issue scope) + Group A (opt-in `kind=` for free-form-table rules) + Group E (compatibility assertion + cross-kind link resolution) | `kind=` sugar is additive for `label-coverage` only; the typed `criteria-to-check`/`criteria-label-match` are shown compatible (same triple), not rewritten. See D2. |
| REQ-09 | 56ab0224 (`jit item list/show/search/resolve`) + Group D (`jit invariant render/check`) | `--kind` filter present; `--json` per-command. |
| REQ-10 | 56ab0224 (issue-scope projection + graceful degradation) + Group A (project-scope projection purity) | Nothing newly stored; a line without a self-id is plain prose. |

Gap flagged (not invented): the foundational child `56ab0224` is unimplemented (zero code, gates `Pending`). Its delivery is a hard prerequisite for every group here; it is treated as the existing foundation item in the sketch (not recreated) and must land first. See Risk R1.

## 2. Technical soundness and architectural fit

The approach reuses the engine's existing genericity rather than adding parallel machinery. The validation engine is already a pure `(ruleset, projection) → findings` function with no notion of "requirement" (`docs/concepts/validation-engine.md`); this epic lifts the latent `(section, marker, id-pattern)` triple into a named registry and adds one new registry (invariants) plus a doc-projection writer.

- **Approach:** (1) Add an `[item_kinds]` config registry capturing the six-tuple per kind; existing rules MAY reference a kind name that expands to the triple, while the inline-triple form stays valid as an anonymous kind (D2, REQ-02/08). (2) Widen qualified-id derivation to `<scope>/<self-id>` with `@` project scope on top of 56ab0224's issue-scope form (REQ-01). (3) Add `decision` and `risk` as markdown-first kinds (REQ-04). (4) Add `.jit/invariants.toml` as a registry-first kind loaded at config time (REQ-03/05). (5) Build a config-driven doc-projection writer (region-replace + separate-file modes) and a bidirectional drift validate rule plus `jit invariant render/check` (REQ-06/07/09).

- **Reuses / integrates with:**
  - `criterion_ids(source, section_slug, marker, id_pattern, repo_default_format, plan_content)` at `crates/jit/src/validation/graph.rs:795` — already pure and parameterized over the triple; the three rule kinds reuse it (`label-coverage` `graph.rs:731`, `criteria-to-check` `graph.rs:1317`, `criteria-label-match` `graph.rs:1482`). `[item_kinds]` resolution feeds these same params.
  - `JitConfig::load(jit_root)` at `crates/jit/src/config.rs:429`, which chain-loads templates at `config.rs:464` — the exact hook to also load `.jit/invariants.toml` and `[item_kinds]`.
  - `[namespaces.*]` as `HashMap<String, NamespaceConfig>` (`config.rs:23`, `config.rs:175`) — the precedent struct to mirror for `[item_kinds]` as `Option<HashMap<String, ItemKindConfig>>` and for the invariants table shape.
  - `Issue::short_id()` (`crates/jit/src/domain/types.rs:237`) and `resolve_issue_id(partial_id)` (`storage/mod.rs:158`, impls `json.rs:508` + `memory.rs:102`) — the base for `<scope>` resolution and `jit show <issue>/<self-id>`.
  - `project()` → `with_sections(content, parser)` → `Section.items: Vec<String>` (`domain/projection.rs:196`, `document/parser/mod.rs:70`) — the parse path qualified ids project over.
  - `validation/serialize.rs:180` `write_atomic(path, &str)` — extract a public `write_file_atomic` and reuse for the invariants-registry and doc-projection writers (string render-then-write idiom mirrors `serialize.rs:60`). Issue-store atomic write `storage/json.rs:103` is the Serialize-side precedent.
  - `Assertion` enum (`validation/rules.rs`, variants at `:628`, `:660`) dispatched by the graph evaluator match at `graph.rs:355` — the extension point for the new drift assertion (Scope::Graph).
  - `CommandExecutor::search_issues_with_filters` (`commands/search.rs:54`) — reused by `jit item search`.

- **Grounding (from investigation):**
  - "Engine is generic over the triple" → valid-and-open; `criterion_ids` at `graph.rs:795` takes `(section, marker, id_pattern)` as params; the word "requirement" is not hardcoded in engine logic. Defaults exist only as defaults: `criteria_section` default `"success_criteria"` (`rules.rs:1176`, `:633`), id_pattern default `"[A-Z][A-Z0-9]*-[0-9]+"` (`rules.rs:1180`).
  - "Config has a load hook for a new registry" → valid-and-open; `config.rs:464` already chain-loads templates; invariants load mirrors `RuleSet::load` (`rules.rs:902`).
  - "Qualified ids are a pure projection, nothing newly stored" → valid-and-open; `short_id()` + parsed self-id, no new persisted field.
  - "No documentation filename is hardcoded in engine logic" → invalid-as-stated for the WHOLE tree, valid as scoped: `README.md` appears in live (non-test) source — a link-validator allowlist (`document/link_validator.rs:188`) and the snapshot-export command (`commands/snapshot.rs:562`). Both are unrelated to invariant projection. So REQ-06's "no hardcoded doc filename" is scoped to the NEW invariant-projection code path (which must ship clean), not the entire tree; LOC-D1's grep is scoped accordingly.
  - "Delimited-region replacement machinery exists" → invalid-as-stated; none exists, must be built (region-marker parser + atomic region-replace writer + target-config schema + orchestrator). Reusable precedent for the render-then-write half only: `serialize.rs:60`, `commands/snapshot.rs:381`.
  - "REQ-08 forces rewriting every live rule" → invalid-as-stated; the consumer sweep measured ~50 files / 150+ references (engine `rules.rs`/`graph.rs`/`serialize.rs:314`/`desugar.rs:83`/`local.rs:520`; gate-preset derivation `gate_presets/planning.rs:225`, `gate_presets/builtin.rs:314`; commands `breakdown.rs`, `validate.rs:649`, `template.rs`; live `.jit/rules.toml` + `.jit/templates.toml`; example rulesets `docs/examples/{sdd,research,nyquist}/`; ~15 test files; skills). REQ-08 permits "or shown compatible," so D2 takes the opt-in indirection that leaves all of these untouched.
  - "`checker-command` may be parsed-but-never-executed" → confirmed: `Assertion::CheckerCommand` is never executed — the local write path explicitly skips it (`validation/local.rs:288`) and `commands/validate.rs` has no execution site (its only `Command::new` calls are `git`). The `local.rs:300` docstring claiming it "runs in `jit validate`" is stale. Consequence: `enforced-by` drift cannot rely on execution; REQ-07 is delivered as declaration-consistency drift. See Decision D5.

Layer boundaries respected: qualified-id derivation and item indexing are pure and belong in `domain/` (per CLAUDE.md separation of concerns), not in command handlers or `main.rs` arms; registry/doc writes go through the storage/atomic-write boundary; new CLI logic lives in `commands/item.rs` and `commands/invariant.rs` (registered in `commands/mod.rs`), with the `main.rs` dispatch arm a thin delegation.

## 3. Decomposition sketch (near-ready; jit-breakdown instantiates — no issues created here)

Each group is independently landable (green at every boundary). Ordering is expressed only through `depends-on`. The foundational child `56ab0224` is the EXISTING foundation item — it is not recreated here; the sketch builds on it.

**Existing foundation (account for, do not recreate): `56ab0224`** — "Addressable structured items in descriptions, with qualified ids", state `Ready`, gates cargo-ci + code-review, currently unimplemented. It delivers the issue-scoped model: `[item_kinds]` config registry (issue-scope four-tuple), qualified ids `<issue-id>/<self-id>`, `jit item list/show/search/resolve`, generic link namespaces, `label-coverage`/`criteria-to-check` compatibility, graceful degradation. It substantially delivers epic REQ-02, REQ-08, REQ-10, the issue-scope subset of REQ-01, the `requirement` slice of REQ-04, and the `jit item *` slice of REQ-09. At breakdown it must carry `satisfies:REQ-02, satisfies:REQ-08, satisfies:REQ-10` and the issue-scope shares of REQ-01/04/09. It must land before any group below.

### Group A: Project-scope addressing and the kind registry six-tuple — covers REQ-01, REQ-02

- **Widen qualified ids to scoped addressing with project scope**  `type: task`  `satisfies: REQ-01`  `depends-on: 56ab0224`
  Outcome: a qualified id resolves as `<scope>/<self-id>` where scope is an issue short-id or `@` for project scope, derived not stored, with self-id uniqueness enforced within each scope.
  Own criteria: `[hard] LOC-A1: jit resolve of @/<self-id> and <issue>/<self-id> both succeed; a duplicate self-id within one scope is reported; no new persisted id field is introduced.`
  Blast radius: extends 56ab0224's resolver and item-index projection; updates resolve/show paths and their tests. Self-contained otherwise.
- **Extend the item-kind registry to the full six-tuple**  `type: task`  `satisfies: REQ-02`  `depends-on: 56ab0224`
  Outcome: each kind in `[item_kinds]` declares `(section, id-pattern, marker(s), link-namespace(s), scope, source-of-truth)`; a free-form-table rule (e.g. `label-coverage`) may carry an optional `kind=` key that expands to that kind's triple while the inline-triple form stays valid as an anonymous kind; no kind name appears in engine logic.
  Own criteria: `[hard] LOC-A2: a label-coverage rule carrying kind = "requirement" evaluates identically to the inline (section, marker, id-pattern) form; a grep scoped to engine logic finds no kind-name literal.`
  Blast radius: adds two fields to the kind config struct and a name→triple expansion read by `label-coverage` evaluation. The `kind=` sugar is added ONLY to free-form-table rules — `label-coverage`'s assert is an `Option<toml::value::Table>` (`rules.rs:1032`) that round-trips an extra key untouched (`serialize.rs:313`), so existing inline-triple rules and tests stay green unchanged. The typed `criteria-to-check`/`criteria-label-match` (`#[serde(deny_unknown_fields)]`, `rules.rs:1146`) are deliberately NOT given `kind=` syntax (would force a serde + serializer change D2 avoids); their compatibility is shown, not re-expressed (Group E).

### Group B: Decision and risk kinds (markdown-first) — covers REQ-03 (markdown side), REQ-04

- **Add the decision and risk item kinds**  `type: story`  `satisfies: REQ-04`  `depends-on: A`
  Outcome: `decision` and `risk` are configured as markdown-first item kinds, indexed and queryable alongside `requirement`, with their link namespaces (`per:`, `mitigates:`/`resolves:`) referencing qualified ids.
  Own criteria: `[hard] LOC-B1: jit item list --kind decision and --kind risk return items parsed from descriptions; a per:<id> and a mitigates:<id> label resolve to the addressed item; markdown remains the sole source (no structured duplicate).`
  Blast radius: config additions plus index/query coverage for the two kinds; reuses `criterion_ids` over the new triples. Self-contained.

### Group C: Invariant registry (registry-first) — covers REQ-03 (registry side), REQ-04, REQ-05

- **Add the invariant registry and registry-first read path**  `type: story`  `satisfies: REQ-05`  `depends-on: A`
  Outcome: `.jit/invariants.toml` declares entries with `statement`, `kind` (enforced\|advisory), and optional `enforced-by` binding; the file loads at config time and invariants are indexed and queryable as a project-scoped, registry-first kind.
  Own criteria: `[hard] LOC-C1: an invariants.toml entry is loaded by config load (including when config.toml is absent); jit item list --kind invariant returns it addressed as @/<self-id>; an invalid entry fails config load with a typed, descriptive error; the registry is authoritative (no markdown index for invariants).`
  Blast radius: new `invariants.rs` module mirroring `RuleSet::load`. `JitConfig::load` has TWO return paths — config-absent early return (`config.rs:432-451`) and config-present (`config.rs:474`), both of which chain-load `TemplateRegistry`; the invariants load must be added to BOTH so invariants load even without a config.toml. Self-contained.
- **Demonstrate cross-substrate generality across all four kinds**  `type: task`  `satisfies: REQ-04`  `depends-on: B, C`
  Outcome: `requirement`, `decision`, `risk` (issue-scoped, markdown-first) and `invariant` (project-scoped, registry-first) are all indexed and queryable through the same generic operations, with a test asserting the same engine serves both substrates.
  Own criteria: `[hard] LOC-C2: a single test enumerates all four kinds through jit item list/show/search and asserts both substrates (issue-scope markdown-first and project-scope registry-first) resolve through one code path.`
  Blast radius: test-and-wiring only; touches no engine internals beyond what B and C add. Self-contained.

### Group D: Projection, drift, and the invariant CLI — covers REQ-06, REQ-07, REQ-09 (invariant side)

- **Project invariants into a configurable doc target**  `type: task`  `satisfies: REQ-06`  `depends-on: C`
  Outcome: invariants render into a config-driven documentation target supporting both a delimited region of an existing file and a separate jit-owned file, with no documentation filename hardcoded in engine logic; the shipped default targets a separate jit-owned file.
  Own criteria: `[hard] LOC-D1: with region mode configured, only the delimited region is rewritten and surrounding content is byte-preserved; with separate-file mode the file is written atomically; the invariant-projection code path contains no documentation-filename literal (the target path comes only from config) — grep scoped to that path, since pre-existing unrelated literals exist at link_validator.rs:188 and snapshot.rs:562; writes use the shared atomic writer.`
  Blast radius: new region-marker parser + region-replace writer + target-config schema + orchestrator; extracts a public `write_file_atomic` from `serialize.rs:180` (private today). Extraction updates the one existing caller in the same change.
- **Add the bidirectional enforcement-drift check and the invariant CLI**  `type: task`  `satisfies: REQ-07, REQ-09`  `depends-on: C`
  Outcome: validate reports drift in both directions (an invariant whose `enforced-by` binding is missing or broken, and a rule/gate no invariant claims), and `jit invariant render/check` expose projection and the drift check, each with `--json`.
  Own criteria: `[hard] LOC-D2: a declared-but-unenforced invariant and an enforced-but-undeclared rule are each reported as findings; both directions have explicit tests; jit invariant check exits non-zero on drift and supports --json.`
  Blast radius: new `Assertion` variant dispatched at `graph.rs:355`; new `commands/invariant.rs` registered in `commands/mod.rs`; thin dispatch arm in `main.rs`. Reuses validate plumbing. Also corrects the stale `validation/local.rs:300` docstring (claims `checker-command` runs in `jit validate`; it does not). Self-contained.

### Group E: Link-namespace resolution and coverage-rule compatibility — covers REQ-08 (both clauses)

REQ-08 has two clauses: (i) generic node→item link namespaces reference qualified ids, and (ii) the existing `label-coverage` / `criteria-to-check` rules are re-expressed as instances over the model OR shown compatible. This group delivers an epic-scope acceptance for both.

- **Resolve node→item links by qualified id across kinds**  `type: task`  `satisfies: REQ-08`  `depends-on: B, C`
  Outcome: a link label in each shipped namespace (`satisfies:`, `per:`, `mitigates:`/`resolves:`, `enforces:`) on a node resolves to its addressed item by qualified id, across the `requirement`, `decision`, `risk`, and `invariant` kinds.
  Own criteria: `[hard] LOC-E1: for each link namespace, a label of the form <ns>:<scope>/<self-id> resolves to the addressed item of the corresponding kind; an unresolvable qualified id is reported, not silently dropped.`
  Blast radius: integration test plus any resolver wiring over the kinds added by B and C; touches no live ruleset. Self-contained.
- **Show coverage rules compatible over the kind model**  `type: task`  `satisfies: REQ-08`  `depends-on: A`
  Outcome: `label-coverage` evaluates identically whether expressed inline or via a `kind=` reference; the typed `criteria-to-check`/`criteria-label-match` are shown to consume the same `(section, marker, id-pattern)` triple a declared kind expands to (compatibility shown, not rewritten), preserving the planning-bracket semantics they back.
  Own criteria: `[hard] LOC-E2: a label-coverage rule with kind = "requirement" produces findings identical to its inline form; a test asserts the inline triple of a criteria-to-check (or criteria-label-match) rule equals the declared kind's expansion; no live rule under .jit/ or docs/examples/ is modified.`
  Blast radius: assertion-and-test only over the indirection added in Group A; explicitly touches no live ruleset. Self-contained.

**Coverage map** (every `[hard]` criterion → ≥1 item):

| Criterion | Satisfied by |
|---|---|
| REQ-01 | 56ab0224 (issue scope) + Group A "Widen qualified ids to scoped addressing with project scope" |
| REQ-02 | 56ab0224 + Group A "Extend the item-kind registry to the full six-tuple" |
| REQ-03 | Group B "Add the decision and risk item kinds" (markdown side) + Group C "Add the invariant registry and registry-first read path" (registry side) |
| REQ-04 | 56ab0224 (requirement) + Group B + Group C "Demonstrate cross-substrate generality across all four kinds" |
| REQ-05 | Group C "Add the invariant registry and registry-first read path" |
| REQ-06 | Group D "Project invariants into a configurable doc target" |
| REQ-07 | Group D "Add the bidirectional enforcement-drift check and the invariant CLI" |
| REQ-08 | 56ab0224 + Group E "Resolve node→item links by qualified id across kinds" (clause i) + Group E "Show coverage rules compatible over the kind model" (clause ii) |
| REQ-09 | 56ab0224 (jit item *) + Group D "Add the bidirectional enforcement-drift check and the invariant CLI" (jit invariant *) |
| REQ-10 | 56ab0224 + Group A "Widen qualified ids to scoped addressing with project scope" |

> Repo-wide acceptance checks for the "no hardcoded name" criteria are SCOPED, not tree-wide (pre-existing unrelated literals exist — `README.md` at `link_validator.rs:188` and `snapshot.rs:562`): a grep scoped to engine logic must find no kind-name literal (REQ-02, LOC-A2), and a grep scoped to the NEW invariant-projection code path must find no documentation-filename literal (REQ-06, LOC-D1). No group removes or renames any existing rule kind or live rule: the `kind=` indirection is added only to free-form-table rules and is purely additive; the typed rules are shown compatible without modification — so there is no "remove/rewrite X before consumers migrate" hazard. The one extraction (private `write_atomic` → public `write_file_atomic`) updates its single existing caller in the same change.

## 4. Risks and actionability

| Risk / open question | Severity | Mitigation or decision |
|---|---|---|
| R1 — Foundation `56ab0224` is unimplemented (zero code, gates Pending); every group depends on it. | High | Sketch builds on it as the existing foundation and gates Group A..E behind it via `depends-on: 56ab0224`. Breakdown must wire `56ab0224` as the dependency root and ensure it carries its `satisfies:` labels. It must land and go green before any group starts. |
| R2 — Per-kind source-of-truth direction (markdown-first vs registry-first) shares one engine; a lifecycle-inversion bug shipped once in this engine (MEMORY 2026-06-10). | High | Isolate the two read paths and test BOTH directions explicitly (LOC-C2 cross-substrate test; LOC-D2 both drift directions). Do not let one direction's defaults leak into the other. |
| R3 — `enforced-by` cannot rely on execution: `Assertion::CheckerCommand` is confirmed never executed (`validation/local.rs:288`; no execution site in `commands/validate.rs`; stale docstring `local.rs:300`). | Medium → settled | Decided (D5): REQ-07 drift is delivered as declaration-consistency — an `enforced-by` binding must reference a real, loadable rule/gate; the inverse direction reports rules/gates no invariant claims. Recorded limitation: a binding naming a real-but-disabled rule reads as "enforced". The stale `local.rs:300` docstring is corrected as part of Group D. |
| R4 — Region-replace doc projection is new machinery (no precedent); a buggy writer could clobber surrounding doc content. | Medium | Ship separate-file mode as the default (D3) so the default never touches existing docs; region mode byte-preserves content outside the delimiters (LOC-D1) and writes through the shared atomic writer. |
| R5 — Public `write_file_atomic` extraction touches a shared writer. | Low | Single existing caller; update it in the same change; behavior-preserving extraction with existing tests as the guard. |
| R6 — Default doc target choice (separate file vs region) is a product default, not a hard constraint. | Low (minor) | Reversible default per D3; flagged as minor, not a blocker. |

Every load-bearing question is resolved or owned: R1/R2 are owned by sketch dependency wiring and explicit both-direction tests; R3 is an explicit pre-implementation verification with a fallback scope. Each sketch item states a single outcome and local `[hard]` criteria executable without re-deriving the design.

## Decisions

- **D1 — Epic scope:** chosen **full scope as authored** (all 10 `[hard]` criteria, four kinds, `jit item *`, invariants registry/projection/drift). Rejected: "narrow to invariants-only" (the repo study `dev/studies/jit-vs-agentic-trends-2026.md` Rec 4 was judged invalid and lacking context by the owner on 2026-06-25; it is not a constraint and is not cited as a scope-cutting reason anywhere in this plan). Rejected: "two-substrate middle path" (requirement + invariant only) — the owner wants the full four-kind generality.
- **D2 — REQ-08 approach:** chosen **opt-in kind indirection for free-form-table rules + "shown compatible" for typed rules**. `label-coverage`'s assert is a free-form `Option<toml::value::Table>` (`rules.rs:1032`) that round-trips an extra key untouched (`serialize.rs:313`), so it MAY carry an optional `kind = "requirement"` that expands to the `(section, marker, id-pattern, link-ns)` triple — purely additive. The typed `criteria-to-check`/`criteria-label-match` use `#[serde(deny_unknown_fields)]` structs round-tripped field-by-field (`rules.rs:1146`, `serialize.rs:373-397`); they are deliberately NOT given `kind=` syntax (it would force a serde + serializer change) and are instead **shown compatible** — a test asserts their inline triple equals the declared kind's expansion. Rejected: hard re-expression rewriting every live rule — a ~50-file / 150+-reference blast radius (live `.jit/rules.toml`, gate-preset derivation `gate_presets/planning.rs:225`, example rulesets `docs/examples/{sdd,research,nyquist}/`, ~15 test files, breakdown/validate commands, skills) for no added capability and high regression risk; REQ-08 explicitly permits "or shown compatible."
- **D5 — REQ-07 drift semantics:** chosen **declaration-consistency drift, not execution-based**. `Assertion::CheckerCommand` and therefore `enforced-by` bindings are confirmed never executed (`validation/local.rs:288` skips on the write path; no execution site in `commands/validate.rs`; the `local.rs:300` docstring is stale and is corrected in Group D). Drift therefore checks both directions over declarations: an invariant whose `enforced-by` names a missing/unloadable rule or gate (declared-but-unenforced), and a rule/gate no invariant claims (enforced-but-undeclared). Recorded limitation: a binding naming a real-but-disabled rule reads as "enforced". Rationale: REQ-07 asks for a runnable bidirectional drift check, not enforcement execution; this delivers the criterion against the system as it actually is.
- **D3 — Doc projection default target (REQ-06):** chosen **config-driven with no hardcoded documentation filename** (currently clean — keep it), shipping BOTH modes (delimited-region-in-existing-file and separate-file) and defaulting the shipped config to a **separate jit-owned file** to avoid clobbering existing docs. Reversible default — minor.
- **D4 — Foundation reuse:** chosen **build on existing child `56ab0224`** rather than re-planning the item foundation. The issue-scope widening to `<scope>/<self-id>` with `@` project scope lands as a small follow-on task in Group A (not inside `56ab0224`), so `56ab0224` ships its issue-scope contract independently and Group A supersets it.
- **Assumptions** (where intent stayed underspecified): none open. The one prior assumption — `enforced-by` execution semantics — was verified against the code during planning and is now settled as Decision D5 (drift is declaration-consistency, with the disabled-rule limitation recorded).
