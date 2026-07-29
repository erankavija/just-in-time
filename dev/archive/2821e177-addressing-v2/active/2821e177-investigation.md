# Investigation: epic 2821e177 — Addressing v2 (rule/gate as addressable items)

Investigator report. Grounds the design brief (`dev/studies/addressing-v2-rule-gate-items.md`)
and the epic's REQ-01..REQ-09 against the current system. All citations are `file:line` as of
HEAD (`751da7f5`). No plan is written here; no code or `.jit/` state was changed.

## 1. Claim classification — §2 brief findings

1. **"`@` is not valid in label values today."** CONFIRMED.
   `crates/jit/src/labels.rs:24`: `^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._/-]*$` — the value
   character class (`[a-zA-Z0-9._/-]`) has no `@`. Verified there is no other label-value
   validator in the codebase (`labels.rs` is the sole `validate_label`/`parse_label` source; all
   other split sites call into it or duplicate only the colon split — see §3b). **valid-and-open**
   (real work: the value grammar must change).

2. **"`@` is already a parsed scope sentinel elsewhere."** CONFIRMED.
   `crates/jit/src/domain/item.rs:62` (`PROJECT_SCOPE_SENTINEL = "@"`) and `:113-119`
   (`Scope::parse`). **already-true-context**, not itself work.

3. **"Label parsing splits on the first colon only."** CONFIRMED.
   `crates/jit/src/labels.rs:77`: `label.splitn(2, ':')`. This split is unaffected by the
   value-grammar widening (the brief's design keeps the colon split unchanged) — confirmed no
   code needs to touch this line for REQ-02.

4. **"Rule names are colon-namespaced... baked into `validation/defaults.rs`."** CONFIRMED but
   **understated** — the brief cites only `defaults.rs`. The `default:` literal-string
   dependency is wider:
   - `crates/jit/src/validation/defaults.rs:104,150,285` — builds names via
     `"default:label-format"` literal and `format!("default:namespace-unique:{name}")`, and a
     `format!("<default:{name}>")` reference/path string.
   - `crates/jit/src/validation/serialize.rs:561` — `.filter(|r| r.name == "default:label-format")`.
   - `crates/jit/src/commands/issue.rs:34` — `.find(|finding| finding.rule ==
     "default:type-hierarchy-known")`.
   - `crates/jit/src/main.rs:1296` — `gf.finding.rule == "default:orphan-leaf"`.
   - Fixed-contract tests pin the exact strings: `crates/jit/src/validation/defaults.rs:402-459`
     (`test_fixed_default_contract_mf1`, `test_empty_registry_emits_exactly_the_unconditional_rules`)
     assert the literal names `default:label-format`, `default:namespace-registry`,
     `default:type-hierarchy-known`, `default:namespace-unique:<ns>`, `default:orphan-leaf`,
     `default:strategic-consistency`, in order.
   - Rule names can carry **two** colons, not one: `default:namespace-unique:resolution`,
     `default:namespace-unique:team`, `default:namespace-unique:type`
     (`.jit/rules.toml:26,32,38`) — a dynamically-generated family (one rule per unique
     namespace declared in `config.toml`, so the family is open-ended, not a fixed enum). REQ-03's
     "origin moves out of the id into a field" must account for a THREE-segment name
     (origin:family:param), not just origin:name. **valid-and-open, broader surface than stated.**

5. **"A concrete cross-kind collision exists: `bracket:coverage-preview` (rule) vs `coverage-preview`
   (gate key)."** CONFIRMED as data (`.jit/rules.toml:44`, `.jit/gates.json:221-240`). But note a
   nuance the brief doesn't draw out: today's per-scope uniqueness (`ItemError::DuplicateSelfId`,
   `derive_scope_items`, `crates/jit/src/domain/item.rs:746-750,777-786`) is enforced **across all
   kinds sharing one scope** — "the kind is not part of the qualified id" (comment at line 746).
   Under REQ-01's target grammar, `kind` becomes part of the address path
   (`@/rule/coverage-preview` vs `@/gate/coverage-preview`), so this specific collision is
   naturally resolved by the new grammar *if* per-scope uniqueness is also rescoped to
   per-(scope, kind). That rescoping is not automatic — it is a real design decision the plan
   must make explicit (open point 2's final question — see §6/§7).

6. **"Gate keys are already clean (no colons)."** CONFIRMED against every key in
   `.jit/gates.json` (`code-review`, `npm-ci`, `cargo-ci`, `jit-validate`, `plan-review`,
   `tdd-reminder`, `cargo-ci-features`, `clippy`, `tests`, `fmt`, `repo-validate`,
   `coverage-preview`, `breakdown-review`).

7. **"Gates are JSON, not TOML... needs a JSON descriptor or a JSON→TOML migration."** CONFIRMED,
   and I found an additional concrete constraint: `TomlSourceDescriptor` (`crates/jit/src/config.rs:589-608`)
   only maps `{toml, table, id-field, text-field, link-fields}` — a **direct field→self-id
   mapping**, no transform/strip capability (see §4).

8. **"`enforced-by` bindings today: two rules... and one gate (`cargo-ci`, twice)."** CONFIRMED
   exactly: `.jit/invariants.toml` — `INV-LABEL-FORMAT → default:label-format`,
   `INV-NAMESPACE-REGISTRY → default:namespace-registry`, `INV-DAG-ACYCLIC → cargo-ci`,
   `INV-GATE-SEMANTICS → cargo-ci`. `InvariantRegistry`'s `enforced_by` field is `Option<String>`
   (single value per invariant) — confirmed via `crates/jit/src/validation/projection.rs:165-169`
   (`inv.enforced_by.as_deref()`), so REQ-05's rebind is exactly 4 single-string edits.

## 2. Claim classification — REQ-01..REQ-09 factual premises

- **REQ-01** (uniform address mint/parse/resolve). **valid-and-open, and larger than it reads.**
  The current qualified-id parser (`split_qualified_id`, `domain/item.rs:678-680`) splits on the
  **first** `/` only into `(scope, self_id)` — there is no kind segment, no `@<project>` segment,
  no `issue/<short-id>/<kind>/<self-id>` deep form, and no sugar-expansion function. Every caller
  (`commands/item.rs:290-337` `show_item`, `commands/item.rs:402`, `commands/validate.rs:317`,
  `validation/graph.rs:90`) treats "everything after the first slash" as the literal self-id and
  matches it by equality against `AddressableItem.self_id` — filtering by kind never happens
  today. This is a from-scratch parser/resolver, not an extension of an existing kind-aware one.

- **REQ-02** (value grammar + re-split-site audit). **valid-and-open.** Full re-split-site
  inventory (grep for `splitn`/`split_once`/`split(` over `crates/jit/src`, excluding tests):
  - `/`-splitters that decode a **qualified id** and are in the direct blast radius of the deeper
    grammar: `domain/item.rs:679` (`split_qualified_id`, the core one), and its four callers above.
  - `:`-splitters (namespace split, **unaffected** — the brief's design keeps the colon split
    unchanged, confirmed no code here needs the deeper grammar): `labels.rs:77,187`,
    `commands/item.rs:385`, `commands/validate.rs:308`, `validation/desugar.rs:125`,
    `commands/query.rs:46`, `snapshot.rs:177`, `commands/claim.rs:937` (assignee, unrelated
    namespace), `domain/types.rs:301,336` (assignee parsing, unrelated).
  - `/`-splitters unrelated to qualified ids (file paths, not addresses — no change needed):
    `storage/memory.rs:335` (repo-relative file path segments), `storage/path_errors.rs:94`
    (`..`-traversal check), `validation/engine.rs:698,737,807,914` (JSON-pointer-style query
    paths), `server/routes.rs:384` (unrelated route path).

- **REQ-03** (rule as addressable kind, no colon in self-id). **valid-and-open**, and the
  descriptor primitive does not yet support what's needed (see §4 primitive verification):
  `TomlSourceDescriptor.id_field` reads a raw string field verbatim as the self-id
  (`domain/item.rs:1122`, `required_toml_str`) — no regex-strip or transform. Moving "origin" out
  of the id means either (a) changing `rules.toml`'s schema to store `origin` and a clean `id` as
  separate fields (a data migration of the 7 seed rules), or (b) adding transform capability to
  the descriptor (an engine change). Also affects the 2-colon `namespace-unique:{param}` family
  (see item 4 above).

- **REQ-04** (gate kind; `gates.json` → `gates.toml`). **valid-and-open.** Concrete findings:
  - `storage/json.rs:128-145` — `write_json`/`read_json` are hardcoded to `serde_json`, not
    format-generic. `load_gate_registry`/`save_gate_registry` (`storage/json.rs:623-634`,
    `GATES_FILE = "gates.json"` at line 21) will need a TOML-specific store, mirroring the
    existing pattern `storage/ruleset_store.rs` already established for `rules.toml`
    (`has_validation_ruleset`, `write_validation_ruleset`, `write_baked_schema`).
  - **Empirically verified TOML gotcha** (built and ran a standalone repro against `toml = "0.8"`,
    the exact version pinned in `crates/jit/Cargo.toml:32`): a struct field `Option<T> = None`
    serializes fine (correctly omitted) even without `skip_serializing_if` — so
    `Gate.example_integration: Option<String>` (currently always `null` in `gates.json`) is **not**
    a blocker. However `Gate.reserved: HashMap<String, serde_json::Value>`
    (`domain/types.rs:760-762`) **does** fail TOML serialization if it ever holds a JSON `null`
    (`toml::to_string_pretty` → `Err("unsupported unit type")`, reproduced directly). Every gate in
    the current seed has `"reserved": {}` (empty), so this is not a live blocker for today's data,
    but it is a real fragility TOML imposes that JSON does not: any future `reserved` value that is
    `null` will hard-fail serialization, and `serde_json::Value` provides no compile-time guard
    against it.
  - `Gate`/`GateChecker` field shapes confirmed (`domain/types.rs:740-761,1000-1024`): `version,
    key, title, description, stage (GateStage), mode (GateMode), checker (Option<GateChecker>),
    priority, reserved, auto, example_integration`; `GateChecker` is internally tagged
    (`#[serde(tag = "type")]`) — serializes to TOML as an inline table, no known issue.
  - Concrete test/fixture consumers that hardcode the literal filename/format (must be swept):
    `crates/jit/tests/init_tests.rs:62` (asserts `gates.json` exists), `gate_update_test.rs:306-310`
    (parses `gates.json` as `serde_json::Value` and asserts on it), `invariant_check_cli_tests.rs:228`
    (writes malformed JSON to `.jit/gates.json` to test an error path), `template_apply_tests.rs:823,831`
    (reads **this repo's own** `.jit/gates.json` via `CARGO_MANIFEST_DIR`-relative path to assert
    `repo-validate` is declared there). Also `commands/snapshot.rs:486-489` (copies `gates.json` by
    literal name during export) and doc-comment mentions in `commands/template.rs:51,276,881` (stale
    after migration, no functional dependency — `template.rs` only ever touches gates through
    `self.storage.get_gate_preset`/the registry trait, never the raw file).
  - **mcp-server and web/ have zero direct consumers** of `gates.json`/`rules.toml`/
    `invariants.toml`/`item_kinds` (grepped `mcp-server/lib/*.js`, `mcp-server/*.js`, `web/src`) —
    they operate only through the CLI / server HTTP API, so REQ-04's "no consumer still reads
    gates.json" is already true outside `crates/jit` proper; the surface is fully contained in
    `crates/jit/src/storage/json.rs` + the 4 test files + `commands/snapshot.rs`.

- **REQ-05** (rebind `enforced-by`). **valid-and-open, small**: exactly 3 unique bindings (2 rules
  + 1 gate used twice) across 4 invariants, confirmed in item 8 above.

- **REQ-06** (`enforces:@/<rule-or-gate>` resolves like other link namespaces). **mostly
  already-architected, blocked on REQ-01/02.** `resolve_link_label`
  (`commands/validate.rs:290-320`-ish / `commands/item.rs`) already generalizes over
  `item_kinds()... link_namespaces()` with no kind-name literal — once `rule`/`gate` are declared
  kinds with `link-namespaces = ["enforces"]`, resolution should fall out of the existing
  mechanism, **provided** the qualified-id parser is updated for the deeper grammar first (REQ-01
  is a hard prerequisite, not parallel work).

- **REQ-07** (rules/gates reference doc, "reusing the projection style mechanism"). **valid-and-open,
  and the brief's phrasing needs precision.** `render_invariants_markdown`
  (`validation/projection.rs:143-199`) is hard-typed to `&InvariantRegistry` — it is not a generic
  "any registry" renderer. What IS genuinely reusable: `ProjectionMode`/`ProjectionStyle`
  (`config.rs`), `splice_region` (pure, format-agnostic, `projection.rs:223-263`), and the
  orchestration pattern in `project_invariants` (`projection.rs:305-344`: render → optionally
  splice into an existing target → atomic write through `IssueStore::write_repo_file`). A new
  `render_rules_gates_markdown`-shaped function following the *same pattern* is what REQ-07
  needs — not a call into the existing invariant-typed renderer. The plan should word this
  precisely (reuse the *mechanism/pattern*, not the *function*) so a reviewer checking the
  citation against code doesn't read it as "calls `render_invariants_markdown`."

- **REQ-08** (re-address all kinds; migrate this repo's data; `jit validate` green). **valid-and-open,
  but the live blast radius is much smaller than the brief implies.** Direct grep of
  `.jit/issues/*.json` for the actual label data (not docs/examples):
  - `satisfies:` — 86 occurrences, **all** in the bare unqualified form `satisfies:REQ-NN` (no
    scope, no `/`). Zero qualified-form `satisfies:<scope>/<self-id>` labels exist.
  - `per:`, `mitigates:`, `resolves:`, `enforces:` — **zero** occurrences anywhere in
    `.jit/issues/*.json`.
  - AGENTS.md's live projected invariants region (`AGENTS.md:125-133`) uses the `id-anchor` render
    style, which emits `- **{id}** — {statement}` with **no address prefix at all**
    (confirmed against `render_id_anchor`, `projection.rs:184-198`) — there is nothing to migrate
    there; it re-renders automatically from whatever the registry's ids become.
  - Remaining `@/INV-*`/`<id>/REQ-*` mentions are confined to **historical prose docs**
    (`dev/active/21558ace-invariants-registry.md`, `dev/active/90a2dbfd-kinds-over-sources.md`, the
    brief itself, one archived completion report) — none of these are `jit validate`-checked data;
    updating them is documentation hygiene, not a correctness requirement for "jit validate is
    green."
  - Net: the actual re-addressing migration REQ-08 must perform is (1) the 4 `enforced-by`
    bindings in `invariants.toml`, and (2) nothing else in live `.jit/` data, because
    `satisfies:REQ-NN`'s bare form is untouched by the address change (see REQ-10-adjacent open
    point 10 below — confirmed by code, not assumption).

- **REQ-09** (multi-jit address form, parse-only). **valid-and-open, clean slate.** `Scope::parse`
  (`domain/item.rs:113-119`) today only distinguishes the literal string `"@"` from "anything
  else" (treated as an issue scope) — there is no support for `@<project>` at all, parsed or
  otherwise. No existing federation/project-registry scaffolding exists anywhere to interact with.

## 3. Prior-art sweep

- `dev/active/90a2dbfd-kinds-over-sources.md` — the immediate predecessor design doc; already read
  in full. Its "Risks and Open Questions" section explicitly flagged the `:` collision
  (line 224-226) and named the decompose-if-it-grows caveat (line 227-229) — this epic is that
  split happening.
- `dev/archive/features/90a2dbfd-completion-report.md` — confirms REQ-07 (rule/gate-as-item) was
  the ONLY deferred criterion of that epic (line 30, decision D7), and records a durable process
  lesson directly relevant here: **"Recurring stale-documentation pattern: every behavior-change
  task first failed code-review solely on stale prose describing the old behavior — most often CLI
  help text (`cli.rs`) and `config.rs` doc comments."** This epic's CLI help text
  (`cli.rs:413-442`, `jit item show 56ab0224/REQ-01` / `@/INV-01` examples) is exactly this kind of
  stale-doc risk once the grammar changes — flag for the plan to pre-sweep, per that lesson.
  It also records that the 129 pre-existing addressable items were migrated byte-identically with
  zero indexing regressions — a precedent worth citing for REQ-08's "no regression" bar.
- `dev/archive/features/25064508/plan.md` and `.jit/issues/25064508-...json` — the origin epic that
  shipped the addressable-items model itself (kinds, qualified ids, link namespaces). Its own
  criteria describe the CURRENT two-segment `<scope>/<self-id>` scheme as the baseline REQ-01
  extends.
- `dev/active/21558ace-invariants-registry.md` — design doc for the invariants registry itself;
  contains a stray `@/INV-01` comment (line 16) that is prose only, not live data (see REQ-08
  finding above).
- No existing study/session doc records the specific facts this report surfaces (the TOML-null
  gotcha, the "kind not part of qualified id" uniqueness scoping, the id-field-has-no-transform
  descriptor limit, or the near-zero live blast radius for REQ-08) — these are new to this
  investigation, not previously written down.

## 4. Primitive verification

- **Config toml source descriptor (90a2dbfd REQ-02).** Exactly:
  `TomlSourceDescriptor { toml: String, table: String, id_field: String, text_field: String,
  link_fields: BTreeMap<String,String> }` (`config.rs:589-608`). Confirmed behavior via
  `load_toml_scope_items`/`project_toml_entry`/`required_toml_str`/`toml_link_labels`
  (`domain/item.rs:1086-1198`):
  - `toml` only — no other structured format is supported.
  - `id-field`/`text-field` are **direct verbatim string reads**, not regex-matched or
    transformed. Contrast with the markdown path (`extract_raw_items`, `domain/item.rs:857-882`),
    which DOES apply `kind.id_pattern.find(text)` to extract a self-id — **registry-first kinds
    bypass `id_pattern` entirely for self-id extraction**; `id_pattern` only feeds `as_triple()`
    for the label-coverage engine, not indexing. This means a registry-first kind's declared
    `id-pattern` is cosmetic/coverage-only, not enforced at index time — worth the plan knowing
    before assuming id-pattern validates rule/gate self-ids on ingest.
  - `link-fields` maps a namespace to ONE field name; that field's value may be a string or an
    array of strings, each becoming a `<namespace>:<target>` label (`toml_link_labels`,
    `domain/item.rs:1169-1198`). No id-pattern applied to link targets either.
  - Scope (`issue`/`project`) and `source-of-truth` (`markdown-first`/`registry-first`) are
    separate `ItemKindConfig` fields, not part of the descriptor (`config.rs:326,346`).
  - A project-scope kind MUST declare a source (markdown path OR toml descriptor) or
    `ItemKind::from_config` returns `MissingProjectSource` (`domain/item.rs:473-477`) — confirmed
    by test `test_registry_first_project_kind_requires_toml_descriptor`
    (`domain/item.rs:1619-1653`).

- **Projection/render mechanism (REQ-07 premise).** See REQ-07 classification above — confirmed
  `render_invariants_markdown` is `InvariantRegistry`-typed, not generic; the reusable primitives
  are `ProjectionMode`/`ProjectionStyle`/`splice_region`/the atomic-write orchestration, not the
  render function itself.

- **`Scope::parse` / `PROJECT_SCOPE_SENTINEL`.** Confirmed exact behavior
  (`domain/item.rs:62,97-152`): `Scope::parse(s)` returns `Scope::Project` iff `s == "@"` exactly;
  every other string becomes `Scope::Issue(s)` verbatim, with resolution of a short-id/prefix to a
  full issue id deferred to the storage layer (`commands/item.rs:315-317`,
  `storage.resolve_issue_id`). No `@<project>` handling exists at all — confirmed clean slate for
  REQ-09.

- **`split_qualified_id`.** Confirmed (`domain/item.rs:678-680`): splits on the FIRST `/` only,
  into exactly two segments `(scope, self_id)`; the doc comment explicitly notes "the self-id is
  the rest (so a self-id may itself contain slashes)" — i.e. it is NOT kind-aware today. Every
  caller (§2 REQ-01/REQ-02 above) treats the second segment as the whole self-id and matches it by
  `==` against `AddressableItem.self_id`, without any kind filter. This is the single function
  REQ-01's deeper grammar must generalize (or replace) — and every one of its 4 call sites must be
  updated in lockstep, not just the function itself.

## 5. Facts for the §8 open points

1. **Value-grammar exact form.** Current regex and full re-split-site list in §1/§2 item 1 and
   REQ-02 above.
2. **Rule identity refactor.** `rules.toml`'s 7 seed rules, colon counts, and the dynamically
   generated `namespace-unique:{param}` family are enumerated in §1 item 4. The toml descriptor has
   no transform capability (§4) — REQ-03 needs either a schema change to `rules.toml` (split
   `origin`/`id` fields) or a descriptor extension.
3. **Sugar + inference rules.** Concrete architecture caution: today's 4 shipped kinds
   (`requirement`, `decision`, `risk`, `invariant`) each use a visually-distinct id-pattern
   (`REQ-\d+`, `D-\d+`, `RISK-\d+`, `[A-Z][A-Z0-9]*-\d+`), so kind-inference-by-pattern is
   unambiguous by construction today. Rule/gate self-ids will almost certainly be free-form slugs
   (`label-format`, `cargo-ci`) with no distinguishing shape — kind-inference-by-id-pattern is
   fundamentally more collision-prone for these two kinds than for the existing four. The plan
   should weigh requiring an explicit kind segment for rule/gate addresses (no sugar) rather than
   inferring.
4. **`issue` segment status.** Confirmed: no `[item_kinds.issue]` or similar exists anywhere;
   issues are a wholly separate storage substrate (`storage/issues/<id>.json`, `IssueStore` trait)
   from the `[item_kinds]`-projected model. Supports treating `issue` as a reserved, built-in
   address segment rather than an item kind, as the brief converges on.
5. **Project identity.** Confirmed **no existing candidate**: `.jit/index.json` has only
   `schema_version`, `all_ids`, `deleted_ids` (read in full — no name field).
   `JitConfig` (`config.rs:13-43`) has no `[project]` section or name field of any kind. The only
   "project_name" concept in the whole tree is `crates/server/src/main.rs:83-90`, which derives a
   **display-only** string from the OS basename of the parent directory of `--data-dir` — unrelated
   to jit's domain model, not read by `crates/jit` at all, and not persisted anywhere. Open point 5
   is genuinely greenfield, not a "pick the existing field" decision.
6. **Gate substrate migration.** Full schema, storage-layer, and consumer/test details in REQ-04
   above, including the empirically-verified TOML-null gotcha for the `reserved` field.
7. **`enforced-by` migration mechanics.** Exactly 4 single-string edits (§1 item 8); no list-typed
   bindings exist to worry about.
8. **Reference projection.** See REQ-07 above — no target file/style decision is blocked on any
   missing primitive; the pattern to reuse is `ProjectionMode`/`splice_region`/atomic-write, a new
   render function is needed regardless of target choice.
9. **Re-addressing blast radius + migration.** See REQ-08 above — live blast radius is 4
   `enforced-by` strings; docs are hygiene, not correctness.
10. **`satisfies:`/label-coverage interaction.** CONFIRMED by code, not inference:
    `label_credits_id` (`validation/graph.rs:73-91`) has two paths — unqualified `value == id`
    (what every live `satisfies:REQ-NN` label uses today) and qualified
    `split_qualified_id(value)` with scope+self-id match. The unqualified path is untouched by any
    address-grammar change. The qualified path calls the SAME `split_qualified_id` that REQ-01
    must generalize — so IF anyone ever authors a qualified `satisfies:` label under the new deeper
    grammar, this site needs the same update as the other 3 callers in §2 REQ-02. Today, zero such
    labels exist in this repo (confirmed), so it's a forward-looking consistency point, not a live
    break.
11. **Backward-compat window.** No dual-form/legacy-resolution shim exists anywhere in the current
    code for qualified ids (unlike, say, `label_credits_id`'s explicit unqualified/qualified
    duality, which IS a deliberate two-form design, not a legacy shim). A clean cut is the simpler
    implementation path structurally — there is no existing compat scaffolding to either preserve
    or remove.
12. **Scope of one effort.** Not a code fact; noted only that the codebase's actual dependency
    order matches the brief's suggested decomposition (§6): the parser/grammar core is a hard
    prerequisite for rule/gate kinds (REQ-06 depends on REQ-01/02), which are a hard prerequisite
    for re-addressing migration (REQ-08 needs `rule`/`gate` kinds to exist before it can migrate
    `enforced-by` to them), which precedes/parallels the multi-jit form (REQ-09, fully independent
    of the others — touches only `Scope::parse` and the parser, not indexing).

## 6. Architecture fit

- **Reuse candidates confirmed real:** `resolve_item_kinds`/`ItemKind::from_config`
  (`domain/item.rs`) — already domain-agnostic, zero kind-name literals, directly supports adding
  `rule`/`gate` as ordinary config entries once the descriptor/grammar work lands.
  `resolve_link_label` and the `enforces:`-generalizing validate-time link scan
  (`commands/validate.rs`) already iterate `item_kinds()...link_namespaces()` generically — no
  changes needed there beyond the parser dependency.
  `storage/ruleset_store.rs` is the pattern to mirror for a new `gates.toml` store (same
  atomic-write, same "sole source when present" idiom already used for `rules.toml`).
- **Layer boundaries to respect:** the new address parser (kind-segmented, `@<project>`-aware)
  belongs in `domain/item.rs` alongside `Scope`/`split_qualified_id`, staying pure/I-O-free per
  AGENTS.md's domain-layer rule — consistent with how the existing parser is built. Storage access
  for a `gates.toml` store belongs behind the `IssueStore` trait exactly as `load_gate_registry`
  already is (`storage/mod.rs:188,195`) — the trait signature does not need to change, only the
  `JsonFileStorage` implementation (and, if kept, `InMemoryStorage`'s in-memory equivalent).
  `commands/` orchestrates (issue resolution + item indexing + link resolution), never doing raw
  file I/O — already true of `commands/item.rs`, must stay true for whatever new
  `commands/rule.rs`/gate-item glue is added.
- **A genuinely new component, not a lookup-and-extend:** the resolver
  (`show_item`/`resolve_link_label`/graph-rule `label_credits_id`) currently has **no concept of
  kind** in address resolution at all — self-id equality is the entire mechanism. REQ-01 is not
  "widen an existing kind-aware resolver," it's "add kind-awareness that doesn't exist yet,"
  because the per-scope dedup (`derive_scope_items`) was deliberately designed so **kind is not
  part of the qualified id** (comment at `domain/item.rs:746`). The plan should treat this as the
  single largest architectural decision in the epic: whether the addressing v2 scheme changes that
  invariant (kind becomes address-significant) or keeps self-id globally unique per scope
  regardless of kind (in which case the coverage-preview collision in §1 item 5 is NOT resolved by
  the kind segment alone, contrary to what the brief's phrasing might suggest).

## 7. Architectural-invariant check

- **INV-LABEL-FORMAT** (`AGENTS.md`/`invariants.toml`, enforced by `default:label-format`) is the
  literal target of REQ-02's grammar change — the rule's backing JSON Schema
  (`.jit/schemas/default-label-format.json`, generated from `CANONICAL_LABEL_REGEX` in
  `validation/defaults.rs:55`) must be regenerated in lockstep with `labels.rs:24`'s regex, or the
  write-path rule and the read-path `jit validate` rule diverge (exactly the R5 dual-source trap
  `defaults.rs`'s own docs warn about at line 190-192, `TYPE_HIERARCHY_SCHEMA_FILE`/
  `regenerate_type_hierarchy_schema`). This is a concrete, code-confirmed risk for REQ-02: two
  copies of the canonical label regex exist today (`labels.rs:24` and
  `defaults.rs:55` `CANONICAL_LABEL_REGEX`, explicitly documented as "Kept in sync with
  `labels::label_regex`" — a manual-sync comment, not an enforced one) and both must change
  together.
- **INV-NAMESPACE-REGISTRY** — unaffected; namespace-side parsing is untouched by the value-grammar
  change.
- **INV-ATOMIC-WRITES** — the existing `write_file_atomic`/`IssueStore::write_repo_file` primitives
  are format-agnostic (they write bytes) and are already used correctly by
  `project_invariants`/`ruleset_store`; a new `gates.toml` writer following the same path preserves
  this invariant with no special handling needed.
- **INV-EVENT-LOG** — not implicated; addressing/config changes are not issue state transitions and
  don't append events today (neither does the existing `rules.toml`/`invariants.toml` write path).
- **Config-driven-engine / no-domain-literal invariant:** the `[item_kinds]` engine itself is
  already clean (confirmed, §6). The place a domain literal genuinely lives today is the **fixed
  default ruleset** (`validation/defaults.rs`'s `default_ruleset`), which is a **deliberately
  hardcoded contract** (module docs literally call it "the FIXED default rule set," MF1) — this is
  by design, not a violation, and is a **different subsystem** from the domain-agnostic item-kind
  engine. REQ-03's rule-identity refactor must not conflate the two: changing how a rule's self-id
  is addressed (a `rule` item kind) is separate from changing what the fixed default ruleset
  generates (which has its own pinned-string tests, §1 item 4) — the plan should scope REQ-03 to
  avoid quietly breaking `test_fixed_default_contract_mf1`'s literal-string assertions as a
  side-effect of an unrelated addressing change.

## 8. Unresolved / needs a plan decision (not a fact this investigation can settle)

- Whether per-scope self-id uniqueness becomes per-(scope, kind) (see §6) — a design decision, not
  a code fact.
- Whether `TomlSourceDescriptor` gains a transform capability or `rules.toml`'s schema changes
  instead (§4/§2 REQ-03) — both are structurally possible; no code fact favors one over the other.
- Exact target file/heading for the rules/gates reference doc (REQ-07) — no existing convention
  beyond `AGENTS.md`'s invariants region to draw an analogy from.
