# Design / Decision Record: Generic Validation Engine

Issue: `f6a704d0` — Generic issue & label validation engine (JSON-Schema-based), with SDD as first application.
Blocks milestone: `9db27a3a` (Version 1.0 Production Release).

This document is the **single authoritative record** of the design. Issues
point here; they do not restate it. Each decision is numbered for traceability.

> Revision 2 (post first adversarial review, agent a00f98c3). Changes: corrected
> the invented gate-checker placeholder (§9.2); completed the existing-check
> inventory and switched to hard migration with no backward compatibility
> (§8.3/§8.4); `enforce` now defaults to false (§7.2); HTML/XML parsers behind
> optional features (§6.2); description criteria made canonical, labels derived
> (§6.3/§10); custom-keyword point split out of the core task (§11); added
> `jit validate --explain` and rejected shorthand+raw-schema mixing (§8.1/§9.1).
>
> Revision 3 (post second-pass feasibility review, agent adad1b84). Changes:
> local rules must cover the batch write path (§7.5); `--force` bypasses logged
> to events (§7.6); compiled schemas cached, not per-write (§5.2); softened the
> unknown-keyword degradation claim to a test obligation (§5.3); projection shape
> is a documented schemars contract (§6.1); migration tolerates removed keys with
> a warning, not a hard error (§8.4); MCP `--schema` parity required (§9.3);
> server/web validation surface deferred post-1.0 (§9.4).
>
> Revision 4 (post derisking research, agents ab1d44f3/ac3cd238/a4c709d6).
> Changes: pinned `jsonschema = "0.46"` with explicit Draft 2020-12 and Validator
> caching (§5.1, §5.2); unknown-keyword degradation CONFIRMED, keyword test now a
> regression guard (§5.3); `sections` projection built lazily behind selector
> pre-filtering (§6.1); pulldown-cmark bumped to 0.13.3 workspace-wide, existing
> usages migrated (§6.2); raw JSON Schema must be a `.json` file, inline TOML
> carries shorthands only (§8.1); migration runs at `jit init` re-run, loader
> warns via a pre-parse scan, no `deny_unknown_fields` (§8.4).

## 1. Problem & approach

jit's "plan" is unstructured free text in `Issue.description`; validation is a
set of hard-coded checks split across `validation.rs` (per-issue) and
`validate.rs` (whole-repo). SDD research (OpenSpec, cc-sdd, get-shit-done,
graphify, ruflo) converged on: make plans falsifiable, structured, traceable.
Decision: **do not add SDD features; add one generic, declarative validation
layer and express SDD as configuration.** jit stays lean and becomes a flexible
substrate for many methodologies.

## 2. Rules vs gates (the boundary)

- **Rules**: stateless, structural, evaluated on write (local) or on demand
  (graph). "Is this issue/label well-formed?"
- **Gates**: stateful, per-issue, transition-blocking, often async/external. "Has
  this passed its checkpoints?"
- A gate MAY invoke `jit validate` (see §9.2), but the layers stay distinct.

## 3. Rule model

A rule is **selector + assertion + severity**.

- **3.1 Selector** — full matrix: `type`, `label`, `state`, `has_doc_type`;
  AND-combinable within one rule.
- **3.2 Profiles are not a concept** — a `profile:sdd` (or any) label is just a
  label rules select on. Applicable rules = union of all whose selectors match.
- **3.3 Two scopes**: *issue-local* (pure predicate over one projected issue;
  runs on write) and *graph/aggregate* (needs the store; runs only in
  `jit validate` and gate checkers, NEVER on write).

## 4. Assertion vocabulary

- **4.1 Local kinds (v1)**: `require-label` (cardinality by selector),
  `label-value-pattern` (regex per namespace), `require-section` (heading
  present), `require-doc-type`. These are **shorthands that desugar to JSON
  Schema**, not a parallel mechanism.
- **4.2 Graph kinds (v1)**: `label-coverage` (every source criterion satisfied
  by >=1 child in a configurable state), `label-reference` (a from-reference
  resolves to a declared source, configurable scope), `dependency-shape`
  (selector must/should depend on a target selector).
- **4.3 Escape hatch (v1)**: a `checker-command` rule kind reusing the gate
  checker mechanism (§9.2), for logic the declarative vocabulary cannot express.
- **4.4 doc-schema**: validates parsed document content (see §6).

## 5. Formalism: JSON Schema

- **5.1** Validate with `jsonschema = "0.46"` (latest 0.46.5, actively
  maintained); commit `Cargo.lock` and treat each 0.x minor bump as breaking
  (consult the crate's MIGRATION.md). Build deterministically via
  `jsonschema::options().with_draft(Draft::Draft202012).build(&schema)`; leave
  format validation at its 2020-12 default (annotation-only). `schemars` 0.8
  (already a dep) generates schemas for jit's own types. Note crate renames in
  examples: `JSONSchema` -> `Validator`, `compile()` -> `validator_for()`.
- **5.2** Shorthand kinds (§4.1) desugar to JSON Schema; one validator
  underneath (consistency) with ergonomic surface on top. The compiled
  `Validator` (which is `Clone + Send + Sync`) is cached once per distinct
  schema (keyed by the schema's canonical serialized form, behind an `Arc`),
  NEVER recompiled per write — compiling is the documented perf pitfall,
  validating is cheap. (Revised from the original "once per rule" wording: keying
  by schema identity rather than by rule name makes validator aliasing
  impossible across all construction paths — two rules that happen to share a
  name but carry different schemas can never collide on one validator, while two
  rules with an identical schema correctly reuse a single compiled validator. A
  rule is still compiled at most once; rule-name uniqueness is separately
  enforced by the loader for unambiguous finding attribution.)
- **5.3 Long tail**: checker-command (§4.3) now. Future declarative cross-field
  logic -> `x-jit-*` **custom keywords** registered via
  `ValidationOptions::with_keyword` + the `Keyword` trait. **Confirmed by
  research**: jsonschema treats unrecognized keywords as annotations and has NO
  strict-reject mode (verified in `compiler.rs`), so schemas using `x-jit-*`
  degrade gracefully under a standard validator. Task 33f23ec7's degradation
  test is therefore a regression guard, not a load-bearing unknown. A keyword
  MAY embed CEL internally. **No CEL top-level type in v1.** Separate task (§11),
  not part of the core validator.
- **5.4 Why not CEL now**: its only marginal value within one issue is value
  comparison / arithmetic / cross-collection predicates — already covered by the
  checker-command hatch. Avoid a second formalism.

## 6. Validation target: enriched projection

- **6.1** Issues normalize to a canonical JSON projection before validation:
  labels grouped by namespace (`labels.req = [...]`), structured fields, plus
  parsed content fields. **The projection shape is a documented, stable contract**
  (generated from a Rust type via `schemars`) that every user-authored schema in
  `.jit/schemas/` depends on; it is versioned and not changed casually.
  **Lazy + pre-filtered (perf)**: selector fields (`type`/`label`/`state`) are
  read directly off `Issue` without parsing anything; the `sections` part of the
  projection (the markdown parse) is built lazily and ONLY when a matching rule
  needs a body assertion. The parsed `JitConfig`/ruleset is cached on the
  executor (today config is re-read ~2× per write).
- **6.2 Per-format parsers**: a NEW content-parser trait (independent of the
  existing `DocFormatAdapter`, which is an asset/link tool and does NOT parse
  content to sections — but the new trait SHOULD reuse `DocFormatAdapter::detect`
  for format detection rather than duplicate it). Each parser yields ONE
  canonical structure (sections -> items, headings, attributes, text). The
  **Markdown** parser is in the default build on **pulldown-cmark 0.13.3**
  (bumped workspace-wide from the stale 0.9; existing usages in
  `document/adapter.rs` and `link_validator.rs` migrated — `Event::End` now
  carries `TagEnd`, `Tag::Heading` is a struct). **HTML and XML parsers are
  implemented but behind OPTIONAL cargo features** — included to force the
  architecture to be genuinely format-agnostic, without bloating the default
  build.
- **6.3** A `- [hard] REQ-01: ...` line projects to a `success_criteria`
  section item with marker + embedded id; JSON Schema asserts
  `contains {pattern:"^\\[hard\\]"}` / `minContains:1`. The **description
  success-criteria items are canonical** (see §10.2).

## 7. Severity & enforcement

- **7.1 Severity**: `off` / `warn` / `error`.
- **7.2 Write blocking**: per-rule `enforce` flag, **defaulting to false (warn
  only)** to match jit's current non-blocking culture (`reject_malformed_labels`
  and `enforce_namespace_registry` default false today). Blocking is opt-in;
  SDD/strict rules set `enforce=true` explicitly.
- **7.3 Bypass**: `--force` bypasses enforce failures (consistent with today).
- **7.4 Graph timing**: graph rules run in `jit validate` and gate checkers
  only; never on write.
- **7.5 Write coverage**: local rules MUST run on every issue mutation path —
  `issue create`, `issue update`, AND the batch path (`bulk_update.rs`,
  `jit issue update --filter`), which today has a separate `validate_update()`.
  Otherwise `enforce` rules are bypassable via batch update.
- **7.6 Event logging**: a `--force` bypass of an `enforce` rule IS logged to
  `events.jsonl` (the audit-sensitive override). Ordinary rejections and
  read-only `jit validate` runs are NOT logged.

## 8. Configuration & storage

- **8.1** Rules in `.jit/rules.toml`. A raw `json-schema` assertion MUST
  reference a `.jit/schemas/<name>.json` file — inline raw JSON Schema in TOML is
  NOT allowed, because TOML cannot express `null`/`"type":"null"`, transcodes
  native datetimes to broken JSON, and needs literal strings for regex
  backslashes. Inline TOML carries only the shorthand kinds (§4.1), which are
  simple scalars; a regex-bearing shorthand field (`label-value-pattern`) must
  use a TOML literal string (`'^\[hard\]'`, not a basic string). A rule is
  shorthand XOR file-schema — the loader REJECTS mixing, keeping one definition
  per constraint.
- **8.2 No hard-coded rules (operative source = the file).** `jit init` scaffolds
  a COMPLETE default `.jit/rules.toml` reproducing today's checks, fully editable.
  `effective_rules()` derives the rule set from the file: when `.jit/rules.toml` is
  PRESENT (even if it contains zero rules — honoring an intentionally-emptied
  ruleset) the file is the sole authoritative source; no in-code default rules are
  combined with it. The in-code `default_ruleset` is retained ONLY as (a) the
  canonical definition serialized into the file by `jit init`, and (b) a TRANSIENT
  BOOTSTRAP fallback used when the file is ABSENT (pre-init repo or deleted file),
  which warns the user to run `jit init`. Because the file becomes the single
  operative source, the former `default:` rule-name reservation in
  `RuleSet::load` is REMOVED — the `default:*` rules now live in the file and are
  user-editable like any other (the separate name-uniqueness guard remains). See
  §8.2a for the engine surface this requires (scope-expanded 2026-06-07, issue
  0abaddc0).
- **8.2a Type-hierarchy TOML surface (scope expansion, 2026-06-07).** Making the
  file the complete source required adding a `type-hierarchy` assertion kind to the
  `rules.toml` grammar so the orphan-leaf and strategic-consistency graph warnings
  (previously default-only `Assertion::TypeHierarchy`, with no TOML surface) can be
  serialized and authored. The repo `HierarchyConfig` is injected by the graph
  evaluator at evaluation time (not stored in the parsed rule). This closed the
  original 0abaddc0 PLANNING GAP: the first attempt could not achieve §8.3 because
  two default checks had no TOML representation, forcing a partial split that kept
  hard-coded rules. With this surface, every default rule is file-expressible.
- **8.3 Hard migration, NO backward compatibility.** ALL existing enforcement is
  re-expressed as default rules and the legacy config keys are REMOVED (not
  aliased). Complete inventory to migrate:
  - From `validation.rs`: type-label requirement, label format
    (`label_regex`/`reject_malformed_labels`), namespace-registry enforcement.
  - From `validate.rs`: orphan-leaf warning, strategic-label consistency, and
    per-namespace allowed-`values`, value-`pattern`, uniqueness, and `required`
    constraints (`config.rs` `NamespaceConfig`, enforced ~`validate.rs:286-388`).
  - From `[validation]` (`config.rs:67-84`) — the ENFORCEMENT keys are removed:
    `require_type_label`, `label_regex`, `reject_malformed_labels`,
    `enforce_namespace_registry`, `warn_orphaned_leaves`,
    `warn_strategic_consistency`.
  `rules.toml` becomes the SINGLE source of truth for issue/label validation.
  Default rules preserve today's behavior (warn vs reject) via `enforce` (§7.2),
  modulo the one approved deviation in §8.3a.
  **Amendment (2026-06-07, issue 0abaddc0):** the removal is scoped to the
  ENFORCEMENT keys above. `default_type` is RETAINED as live behavioral config
  (consumed at issue creation, `commands/issue.rs`; not an enforcement rule and
  has no `rules.toml` representation). `strictness` is RETAINED as inert /
  forward-compat (no live consumer). The original §8.3 inventory listed both for
  removal; that was a contract defect — they are not enforcement and cannot be
  expressed as rules, so they stay in `config.toml` and are EXCLUDED from the
  deprecated-key warning (§8.4).
- **8.3a Approved parity deviation — custom `label_regex` on the validate path**
  (user-approved 2026-06-07). Legacy `validate_labels` checked only the FIXED
  canonical regex on the validate path; a custom `validation.label_regex` was
  applied write-time only (gated by `reject_malformed_labels`). Because the rule
  engine has no "write-only" representation for local rules — a write-blocking
  rule needs `severity=error`, and `jit validate` fails on any `severity=error`
  finding regardless of `enforce` (§7 gives local rules no write-only carve-out;
  only graph rules are validate-only, §7.4) — migrating the custom regex makes it
  apply to BOTH the write and validate paths. The resulting behavior is stricter
  and consistent across paths. Most repos are unaffected: when `label_regex`
  equals the canonical regex, the `default:label-format-custom` rule is not
  emitted at all. Achieving exact legacy parity would require adding a write-only
  local-rule concept to the Rule model; that was considered and declined in favor
  of this documented deviation.
- **8.4 Migration**: re-running `jit init` on an existing repo performs the
  one-time migration — it serializes the COMPLETE `default_ruleset` derived from
  the existing `[validation]` + `[namespaces].{values,pattern,required}` config
  into `.jit/rules.toml` (+ `.jit/schemas/*.json`), then strips the removed
  enforcement keys (the six §8.3 `[validation]` keys + the namespace constraints),
  removing the `[validation]` table header when it is left empty.
  - **Fresh repos:** `jit init` writes the config template ALREADY in
    post-migration shape (no enforcement keys) and a complete `rules.toml`, so a
    brand-new repo emits NO deprecation warning and NO migration message.
  - **Coexistence** (a user `rules.toml` already exists AND legacy keys remain):
    the user file is NOT clobbered; migrated rules are appended by name (skipping
    any name already present, with a warning) and the legacy keys are stripped.
  - **Idempotent:** re-running with a `rules.toml` present and no legacy keys is a
    no-op.
  **Until migration runs** (e.g. a config git-synced from an old version): there
  is NO `#[serde(deny_unknown_fields)]`, so serde already ignores the removed keys
  and the config never hard-errors. A pre-parse `toml::Value` scan in
  `JitConfig::load` detects the deprecated keys — the six removed `[validation]`
  ENFORCEMENT keys (NOT `default_type`/`strictness`, which are retained per §8.3)
  and nested `namespaces.*.{values,pattern,required}` — and WARNS, prompting the
  user to re-run `jit init`. Do NOT add `deny_unknown_fields` (it would turn the
  warn into a hard error).

## 9. CLI surface

- **9.1** Extend `jit validate` to take a positional `[<id>]` (NET-NEW surface —
  today it takes none) plus `--json` and `--explain` (lists matched selectors ->
  rule names -> outcomes for an issue; the debugging path that justifies having
  no `jit rule` subcommands). Whole-repo `jit validate` continues to also run
  DAG-integrity checks.
- **9.2** Gates invoke validation via the REAL mechanism — the checker command
  receives the issue id in the `JIT_ISSUE_ID` env var (no `{issue}`
  substitution exists):
  `jit gate define ... --checker-command 'jit validate "$JIT_ISSUE_ID" --json'`.
- **9.3 MCP parity**: the new `jit validate [<id>]` positional and `--explain`
  flag MUST appear in `jit --schema` so the MCP server auto-generates a working
  tool; a parity test asserts this.
- **9.4 Server / web UI**: surfacing rule findings via `crates/server/` is
  **deferred to post-1.0** (today it exposes no rule endpoint; `validate()` there
  is DAG-integrity only). Recorded as a deliberate scope decision, not an
  omission.

## 10. SDD as first application (docs/examples only)

- **10.1** No preset subsystem. SDD ships as example `.jit/rules.toml` + schemas
  + documentation, including >=1 non-SDD example proving methodology-agnosticism.
- **10.2 Criteria SSOT**: the description `## Success Criteria` items (with
  `[hard]`/`[aspirational]` markers and embedded stable ids, e.g. `REQ-01`) are
  the **canonical** source. `req:`/`satisfies:` labels and coverage are DERIVED
  from / checked against those items, never authored as an independent source.
  `label-coverage` (§4.2) treats the canonical criteria as the source set. The
  epic itself dogfoods this format.

## 11. Dropped / deferred

- **Dropped**: delta-specs. Specs are edited in place.
- **Deferred (own task, not on the core critical path)**: the `x-jit-*`
  custom-keyword extension point ships as a standalone task depending on the
  validation core, so speculative keyword-API churn cannot block desugaring or
  graph rules.

## 12. Layering

Projection + local rule evaluation are **pure** (`domain/`), testable without
I/O. Graph rules take the store. Config loading in the storage/config layer.
`validate` orchestrates; CLI/output handle user-facing concerns.
