# Plan — 0abaddc0 full single-source-of-truth migration (scope-expanded)

**Issue:** 0abaddc0 — jit init default ruleset scaffolding and existing-repo migration
**Epic:** f6a704d0 — Generic issue & label validation engine
**Status:** planning (awaiting plan-review green before dispatch)
**Date:** 2026-06-07

## 0. Why this plan exists (planning-failure correction)

The original 0abaddc0 breakdown assumed the legacy `[validation]` config could be
fully migrated into `.jit/rules.toml` so that "rules.toml becomes the SINGLE
source of truth" with "no hard-coded rules" (decision record DR §8.2/§8.3). The
first implementation discovered this is **not achievable as scoped**: two default
rules — the orphan-leaf and strategic-consistency graph warnings
(`Assertion::TypeHierarchy`) — have **no `rules.toml` surface** (they are
default-only, constructed in code; `rules.rs` `RawAssert` cannot parse them). The
first worker therefore shipped a PARTIAL split (kept 6 of 8 `[validation]` keys
live with hard-coded rules still generated from them), which the adversarial
review correctly flagged as a literal DR §8.2/§8.3 violation.

User decision (2026-06-07): this was a planning failure — **expand the scope** to
do the migration properly (add the missing engine surface so the full migration
is achievable), plan it, get the plan reviewed, then drive to completion.

The partial branch `worktree-agent-0abaddc0` @ `5cb6d3c` is **superseded** by this
plan. Its migration mechanics (toml_edit key-stripping, atomic writes, idempotent
re-run guard, deprecated-key scan skeleton, parity-test approach) are good and
should be REUSED, but the architecture changes (file becomes the operative
source; ALL defaults serialized; all 8 keys removed).

## 1. Goal (literal DR §8.2/§8.3 conformance)

1. `.jit/rules.toml` is the **operative single source of truth** for issue/label
   validation. `effective_rules()` derives the rule set from the file.
2. **All** default checks are expressible in `rules.toml` (no default rule lacks a
   TOML surface).
3. `jit init` scaffolds a **complete** `.jit/rules.toml` (every default rule
   serialized) + any referenced `.jit/schemas/*.json`.
4. Re-running `jit init` on an existing repo migrates the full `[validation]` table
   and `[namespaces].{values,pattern,required}` into `rules.toml` and strips **all**
   of them from `config.toml`. Behavior (accept/reject + warn/block + severity)
   is identical before vs after, proven by an exhaustive parity battery.
5. Stale configs never hard-error (no `deny_unknown_fields` on `JitConfig`); a
   pre-parse scan warns on **every** removed key and prompts a `jit init` re-run.
6. Fresh `jit init` produces a coherent end state: no self-contradictory
   "deprecated, re-run init" warning on a brand-new repo; `config.toml` does not
   document fields it no longer carries.

## 1a. Plan-review revisions (v2 — incorporated after adversarial plan review)

The plan reviewer returned PLAN-NEEDS-REVISION with 9 must-fixes; all are folded
into v2 below. The headline corrections:

- **R1 (hard contradiction):** `RuleSet::load` REJECTS any rule named `default:*`
  via the `ReservedRuleName` guard (`rules.rs:615-622`). A file containing the
  serialized defaults would therefore fail to load. **Resolution:** REMOVE the
  `default:` reservation (and its unit test `rules.rs:1212-1223` + the error-doc
  at `rules.rs:118-123`). Its only rationale was avoiding collisions "when the two
  sets are combined" — under file-as-source there is no separate combining, and
  making the `default:*` rules live and editable in the file is exactly DR §8.2
  ("fully editable"). The separate uniqueness guard (`rules.rs:~630`) still
  prevents duplicate names. Default rules keep their `default:*` names (no rename),
  so the round-trip comparator needs no name remapping.
- **R2:** the D4 round-trip cannot use `==` (see revised D4).
- **R3:** D3 fallback is valid only because D8 (dogfood this repo) is mandatory
  and in-scope (see revised D3).
- **R4:** §2 has TWO gaps, not one (corrected below).
- **R5:** init behavior when a user `rules.toml` AND legacy keys coexist is now
  specified (revised D5).
- **R6:** `Assertion::TypeHierarchy` drops inline config; `HierarchyConfig` is
  threaded into `evaluate_graph` (revised D1).
- **R7:** shipped example rulesets get a validation/update phase (Phase E2).
- **R8:** the reused `migration.rs` is rewritten from "subset split" to "complete
  serialization"; deprecated-key lists widened; emptied `[validation]` header
  removed (revised D5/D7).
- **R9:** DR amendment moves EARLIER (revised Phase order: F before C/D).

Authoring-surface note (resolves the reviewer's `--schema`/MCP question): DR §9.1
mandates NO `jit rule` subcommands — rules are authored by editing `.jit/rules.toml`
directly. So the new `type-hierarchy` assert kind needs NO `jit --schema`/MCP
surface; §9.3 parity covers only `jit validate [<id>]`/`--explain`, which already
exist and are out of scope here. A `rules.toml` grammar schema is explicitly
DEFERRED (not silently omitted).

## 2. Current-state facts (verified)

Default rules emitted by `default_ruleset` (defaults.rs) and their TOML-surface
status. **Two engine gaps, not one:** (a) no `type-hierarchy` parse kind for the
two graph warnings; AND (b) the canonical/registry/unknown-type/values rules are
built as `Assertion::JsonSchema` with an INLINE schema and a PLACEHOLDER
`reference`/`path` of `<default:NAME>` (`defaults.rs:434-445`) — they are NOT
file-backed today, so the D4 serializer must MATERIALIZE those inline schemas to
real `.jit/schemas/*.json` files.

| Rule | Assertion kind | TOML-serializable today? |
|---|---|---|
| `default:label-format` (canonical) | json-schema (raw_labels regex) | yes (json-schema file) |
| `default:label-format-custom` | json-schema (custom regex) | yes (json-schema file) |
| `default:require-type-label` | require-label shorthand | yes |
| `default:namespace-unique:<ns>` | require-label `{ns:* , min0 max1}` | yes |
| `default:namespace-registry` | json-schema (ns enum) | yes (json-schema file) |
| `default:type-hierarchy-known` (unknown-type) | json-schema (type enum) | yes (json-schema file) |
| `default:namespace-values:<ns>` | json-schema | yes (json-schema file) |
| `default:namespace-pattern:<ns>` | label-value-pattern shorthand | yes |
| `default:namespace-required:<ns>` | require-label shorthand | yes |
| **orphan-leaf** | `TypeHierarchy{OrphanLeaf}` | **NO surface** |
| **strategic-consistency** | `TypeHierarchy{StrategicConsistency}` | **NO surface** |

`effective_rules()` (mod.rs:342-369) today = `default_ruleset(config) ++ rules()`.
**Load-bearing edit (plan-review note):** this concat at `mod.rs:361-364` MUST be
removed under D2/D3 so defaults and the file are never combined when the file is
present — otherwise the file's `default:*` rules collide with concatenated
defaults (`DuplicateRuleName`).
`rules()` (mod.rs:310-314) loads `.jit/rules.toml` via `RuleSet::load`.
`RawAssert` (rules.rs:701-722) is the serde parse surface (deny_unknown_fields).
This repo (just-in-time) has **no** `.jit/rules.toml` today.

## 3. Design decisions

### D1 — Add a `type-hierarchy` TOML assert kind (R6)
Extend `RawAssert` with `#[serde(rename = "type-hierarchy")] type_hierarchy:
Option<RawTypeHierarchy>` where `RawTypeHierarchy { kind: String }` (with
`#[serde(deny_unknown_fields)]`) and `kind` is `"orphan-leaf"` |
`"strategic-consistency"` (any other value = `RuleConfigError`). `into_assertion`
maps it to `Assertion::TypeHierarchy { kind }`.

**HierarchyConfig injection (verified plumbing).** `Assertion::TypeHierarchy`
currently carries `HierarchyConfig` inline (`rules.rs:422-428`) and the evaluator
reads it straight off the assertion (`graph.rs:750-772`); `evaluate_graph(rules,
issues)` (`graph.rs:264`) has NO config parameter. The reviewer confirmed the only
production call site, `CommandExecutor::evaluate_graph_rules` (`validate.rs:380`),
already has `cached_namespaces()`. Resolution:
- DROP the inline `config` from the `Assertion::TypeHierarchy` variant (carry only
  `kind`). Reject the earlier "default-on-parse, overwrite at eval" fallback (a
  footgun).
- Change `evaluate_graph`/`evaluate_one`/`evaluate_type_hierarchy` to receive the
  repo `HierarchyConfig`. Build it at the call site via the existing
  `defaults::hierarchy_config(namespaces)` helper (currently private at
  `defaults.rs:379` — expose as `pub(crate)`).
- Update `default_ruleset` (no longer passes config into the variant), the doc
  comments + doctest on `Assertion::TypeHierarchy` / `TypeHierarchyKind`
  (`rules.rs:416-450`), and all test call sites of `evaluate_graph`.

### D2 — `default_ruleset` becomes the canonical in-code DEFINITION, not a parallel enforcement source
`default_ruleset(config, namespaces)` stays as the single in-code definition of
the default rule set, but its ONLY runtime consumers become: (a) `jit init`
scaffolding/migration (serialized to the file), and (b) the absent-file fallback
(D3). It is no longer concatenated into `effective_rules` for repos that have a
`rules.toml`.

### D3 — `effective_rules()` reads the file as operative source; in-code defaults are a transient bootstrap fallback (R3, Q1 resolved)
Semantics (note exists-empty vs absent — the reviewer's key distinction):
- **File present, even with zero `[[rules]]`:** `effective_rules` = parsed file
  ONLY (empty file → empty ruleset). This HONORS a user who intentionally emptied
  their rules. The file is authoritative; every rule (including the `default:*`
  ones) is user-editable (DR §8.2 satisfied).
- **File ABSENT (pre-init repo, or deleted):** fall back to in-code
  `default_ruleset(config, namespaces)` AND emit a one-time warning ("no
  .jit/rules.toml; using built-in defaults — run `jit init` to materialize them").
  This prevents silent loss of the always-on canonical-format write block and
  keeps repos safe until init runs. No read-path mutation (do NOT auto-scaffold on
  read).

Q1 RESOLVED: the fallback is acceptable ONLY because it is a proven-TRANSIENT
bootstrap — D8 (dogfood-migrate THIS repo) is MANDATORY and lands in the same
change, so this repo (which today has no rules.toml + a live `[validation]` block)
does not sit in the dual-source state. Without D8 the fallback would silently keep
`config.toml` as the de-facto source on this repo, defeating the expansion. The
exists-empty-vs-absent split ensures intentional-empty is never masked.

### D4 — RuleSet → TOML serializer (round-trippable) (R2)
Add a serializer that emits `[[rules]]` blocks for an arbitrary `RuleSet`,
covering every assertion kind including the new `type-hierarchy`, plus the graph
`config: toml::value::Table` kinds (label-coverage/reference/dependency-shape).
json-schema rules write their schema to `.jit/schemas/<name>.json` and reference
it. Implementation notes (verified against the model):
- **Manual TOML rendering** (the branch's `render_rules_toml` approach). `Severity`
  and `Selector` derive only `Deserialize` (`rules.rs:143,227`) — no `Serialize` —
  so render each field by hand. Reuse the branch's `toml_literal_string` for regex
  literals (DR §8.1).
- **Round-trip is a FIELD-WISE comparator, NOT `==`.** `SchemaSource` derives
  `PartialEq` over `reference`/`path` (`rules.rs:337-346`); in-code defaults carry
  placeholder `reference/path="<default:NAME>"` while reloaded rules carry
  `"schemas/NAME.json"` + an absolute resolved path — never `==`. The round-trip
  test must compare: name, `when` selector, severity, enforce, assertion KIND, and
  for `JsonSchema` the parsed schema VALUE (`SchemaSource.schema`), EXCLUDING
  `reference`/`path`.
- **RawAssert lockstep:** every assert key the serializer emits MUST have a
  matching `RawAssert` field (else `deny_unknown_fields` rejects on reload). The
  new `type-hierarchy` field (D1) closes the only gap. The round-trip test guards
  this invariant.

### D5 — Migration writes the COMPLETE ruleset, strips removed keys, handles the coexistence case (R5, R8, Q2 resolved)
On `jit init`, behavior by repo state:
- **Fresh repo** (no `rules.toml`, post-migration template per D6): write the
  COMPLETE serialized `default_ruleset` to `rules.toml` + schema files. No legacy
  keys to strip, no migration message.
- **Legacy repo** (no `rules.toml`, has legacy `[validation]`/namespace
  constraints — THIS repo's case): serialize the COMPLETE
  `default_ruleset(config, namespaces)` (it already folds in the live flags
  `reject_malformed_labels`, `enforce_namespace_registry`, `require_type_label`,
  `label_regex`, `warn_orphaned_leaves`, `warn_strategic_consistency`, namespace
  values/pattern/required) into `rules.toml`, capturing each rule's
  `enforce`/`severity` as concrete values, then strip from `config.toml`.
- **Coexistence** (user `rules.toml` ALREADY exists AND legacy keys still present):
  do NOT clobber the user file. Compute the COMPLETE default ruleset from the
  legacy config; for each default rule, APPEND it to the user file ONLY if its
  name is not already present; WARN about any skipped (already-present) name; then
  strip the legacy keys. This completes migration without destroying user edits.
- **Already-migrated** (`rules.toml` present, no legacy keys): idempotent no-op.

**Stripping (R8):** remove the legacy ENFORCEMENT keys from `[validation]`
(`require_type_label`, `label_regex`, `reject_malformed_labels`,
`enforce_namespace_registry`, `warn_orphaned_leaves`, `warn_strategic_consistency`)
and `namespaces.*.{values,pattern,required}`. When `[validation]` is left empty,
REMOVE the now-bare table header too (the branch's `strip_keys_from_config` does
NOT do this — add `doc.remove("validation")` when empty). Reuse the branch's
`toml_edit` comment-preserving stripping + `write_atomic` + idempotency guard;
REWRITE its `plan_migration` from the subset-split to complete serialization.

**Q2 RESOLVED:** `default_type` is genuinely behavioral (read at issue creation,
`commands/issue.rs:26-39`) — KEEP it live, do NOT migrate/strip. `strictness` has
NO live consumer (dead/forward-compat) — keep it inert, do NOT strip. DR §8.3
currently lists both for removal; that is a contract defect — amend §8.3 (Phase F)
to scope removal to ENFORCEMENT keys, with `default_type` retained as behavioral
config and `strictness` noted as inert. These two are therefore EXCLUDED from the
deprecated-key warning set (D7).

### D6 — Fresh-init template in post-migration shape
Change the default config template (`hierarchy_templates.rs`) so a brand-new
`jit init` writes a `config.toml` WITHOUT the enforcement keys and WITHOUT
documenting `values`/`pattern`/`required` as live namespace fields, and writes a
COMPLETE `rules.toml` directly (not "write template with keys, then migrate them
in the same run"). Result: fresh init emits no deprecation warning and no
"migrated N keys" message; just "Initialized jit repository" + "scaffolded
.jit/rules.toml". The migration path runs ONLY when a pre-existing legacy config
is detected.

### D7 — Deprecated-key scan covers the full removed set (minus the two kept keys)
`detect_deprecated_keys` warns on the six removed `[validation]` enforcement keys
(`require_type_label`, `label_regex`, `reject_malformed_labels`,
`enforce_namespace_registry`, `warn_orphaned_leaves`, `warn_strategic_consistency`)
and `namespaces.*.{values,pattern,required}`, prompting a `jit init` re-run.
EXCLUDE `default_type` and `strictness` (kept live per Q2). Reuse the branch's pure
`detect_deprecated_keys` scanner mechanism; only the key lists change.

### D8 — Migrate THIS repo as part of completion
After the engine + init work lands and is gate-green, run the migration on the
just-in-time repo itself (generate its `.jit/rules.toml` + `.jit/schemas/`, strip
its `[validation]`/namespace constraint keys), and VERIFY (via `jit validate` +
spot create/update) that the repo's own validation behavior is unchanged. Commit
as a separate, clearly-labeled change. This is the dogfooding step that proves the
migration on a real repo. **Until D8 runs, the D3 fallback keeps the repo safe.**

## 4. Work breakdown

Single expanded effort on 0abaddc0 (the engine surface is small and tightly
coupled to serialization/migration). Phases, each TDD. **Phase F moved FIRST per
R9** (contract decisions must precede the code that implements them):

- **Phase F (DR amendment + criteria) — FIRST:** amend DR §8.2/§8.3/§8.4 to
  reflect: the new `type-hierarchy` TOML surface; file-as-operative-source +
  transient bootstrap fallback (D3); removal of the `default:` reservation;
  removal scoped to ENFORCEMENT keys with `default_type` retained behavioral and
  `strictness` inert (Q2). Update 0abaddc0 success criteria to the expanded scope.
  (Lead does the DR + criteria edits; this is the user-approved scope expansion.)
- **Phase A (engine surface):** D1 type-hierarchy TOML kind + `HierarchyConfig`
  threading into `evaluate_graph` + REMOVE the `default:` reservation (R1) +
  parse/error unit tests. All existing tests green.
- **Phase B (serializer):** D4 RuleSet→TOML + schema-file materialization +
  field-wise round-trip test over the full default set (incl. canonical
  label-format, type-hierarchy, graph-config kinds, `severity=off`).
- **Phase C (effective_rules):** D2/D3 file-as-source + exists-empty-vs-absent +
  bootstrap fallback + warning. Unit/harness tests for all three branches.
- **Phase D (init/migration):** D5/D6/D7 — complete scaffold, full strip (incl.
  empty-table-header removal), template reshape, coexistence case, deprecated-scan
  widening. Reuse (rewritten) branch `5cb6d3c` mechanics.
- **Phase E (parity + CLI tests):** the §5 exhaustive battery + CLI tests for
  fresh init (no warning/no migration message), legacy-config re-init (warns,
  migrates, loads clean), and coexistence re-init.
- **Phase E2 (shipped examples) (R7):** verify `docs/examples/{sdd,bug-repro,
  release-checklist}/rules.toml` still parse after the `RawAssert` change; if any
  demonstrates orphan-leaf/strategic-consistency, express it via the new
  `type-hierarchy` kind. Confirm `example_rulesets_tests` green.
- **Phase G (dogfood) (D8):** migrate THIS repo (generate `.jit/rules.toml` +
  `.jit/schemas/`, strip its enforcement keys), run `jit validate` + spot
  create/update to confirm unchanged behavior; commit separately and clearly.

**Criteria change (0abaddc0, Phase F):** expand to require: complete rules.toml
scaffold; full enforcement-key migration & strip (six `[validation]` keys +
namespace constraints; `default_type`/`strictness` retained); `type-hierarchy`
TOML surface; `default:` reservation removed; effective_rules file-as-source with
documented empty-vs-absent semantics; exhaustive before/after parity; fresh-init
coherence; this-repo dogfood migration.

## 5. Exhaustive parity test battery (Phase E — mandatory)

Build a rich legacy config exercising EVERY dimension, snapshot per-issue
`(is_blocking, error_count, warning_count)` over an issue battery that actually
TRIGGERS each rule (positive AND negative cases — not possibly-empty count
equality), run migration, reload from the file, assert identical outcomes + equal
rule count + no duplicate names + no dropped rule. Battery must include issues
that fire: require-type-label, canonical-format reject, custom-regex reject (with
reject_malformed on and off), namespace values violation, pattern violation,
required missing, unique collision, unknown namespace (registry, both flags),
unknown type, orphan leaf, strategic-consistency. The graph-rule parity test MUST
assert the warnings actually FIRE before and after (not count-equality of empty
sets — the prior weakness).

Additional cases (R6 suggestions): empty-namespace-registry repo (registry rule
skipped; `registered_namespace_schema` empty branch); `severity=off` rule
serialize+reload (emitted and skipped); round-trip of the always-on
`default:label-format` canonical rule specifically (highest-traffic, most exposed
to the SchemaSource path/reference exclusion); idempotent double-`init`; and the
coexistence re-init (user file preserved, legacy rules appended by name, keys
stripped).

## 6. Resolved questions (post plan-review)

- **Q1 (fallback):** RESOLVED — file present (even empty) is authoritative;
  absent → transient bootstrap fallback + warn, valid only because D8 is
  mandatory/in-scope. See D3.
- **Q2 (default_type/strictness):** RESOLVED — keep both live (behavioral / inert);
  amend DR §8.3 to scope removal to enforcement keys. See D5/D7, Phase F.
- **Q3 (TypeHierarchy injection):** RESOLVED — drop inline config, thread
  `HierarchyConfig` into `evaluate_graph` from the `evaluate_graph_rules` call
  site; reject the default-on-parse fallback. See D1.
- **Q4 (structure):** one expanded 0abaddc0, but Phase A/B (engine surface +
  serializer with green round-trip) form their own review checkpoint before the
  riskier C–G work.
- **Q5 (schema-file proliferation):** ACCEPTED — json-schema default rules write
  `.jit/schemas/*.json`; this is the cost of file-as-source and is acceptable.
- **R1 (reserved name):** RESOLVED — remove the `default:` reservation. See §1a.
- **R5 (coexistence):** RESOLVED — see D5 coexistence case.
