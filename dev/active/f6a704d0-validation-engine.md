# Design / Decision Record: Generic Validation Engine

Issue: `f6a704d0` — Generic issue & label validation engine (JSON-Schema-based), with SDD as first application.
Blocks milestone: `9db27a3a` (Version 1.0 Production Release).

This document is the **single authoritative record** of the design. Issues
point here; they do not restate it. Each decision is numbered for traceability.

> Revision 2 (post adversarial review, agent a00f98c3). Changes: corrected the
> invented gate-checker placeholder (§9.2); completed the existing-check
> inventory and switched to hard migration with no backward compatibility
> (§8.3/§8.4); `enforce` now defaults to false (§7.2); HTML/XML parsers behind
> optional features (§6.2); description criteria made canonical, labels derived
> (§6.3/§10); custom-keyword point split out of the core task (§11); added
> `jit validate --explain` and rejected shorthand+raw-schema mixing (§8.1/§9.1).

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

- **5.1** Validate with the `jsonschema` crate (pin `~0.46` at task start, not
  "impl time"). `schemars` 0.8 (already a dep) generates schemas for jit's own
  types. No new schema-generation code needed.
- **5.2** Shorthand kinds (§4.1) desugar to JSON Schema; one validator
  underneath (consistency) with ergonomic surface on top.
- **5.3 Long tail**: checker-command (§4.3) now. Future declarative cross-field
  logic -> `x-jit-*` **custom keywords** registered with the validator (unknown
  keywords are ignored by standard validators, so schemas degrade gracefully);
  a keyword MAY embed CEL internally. **No CEL top-level type in v1.** This is a
  separate task (§11), not part of the core validator.
- **5.4 Why not CEL now**: its only marginal value within one issue is value
  comparison / arithmetic / cross-collection predicates — already covered by the
  checker-command hatch. Avoid a second formalism.

## 6. Validation target: enriched projection

- **6.1** Issues normalize to a canonical JSON projection before validation:
  labels grouped by namespace (`labels.req = [...]`), structured fields, plus
  parsed content fields.
- **6.2 Per-format parsers**: a NEW content-parser trait (independent of the
  existing `DocFormatAdapter`, which is an asset/link tool and does NOT parse
  content to sections — but the new trait SHOULD reuse `DocFormatAdapter::detect`
  for format detection rather than duplicate it). Each parser yields ONE
  canonical structure (sections -> items, headings, attributes, text). The
  **Markdown** parser (pulldown-cmark; review/bump the stale 0.9 pin) is in the
  default build. **HTML and XML parsers are implemented but behind OPTIONAL cargo
  features** — included to force the architecture to be genuinely
  format-agnostic (not markdown-coupled), without bloating the default build.
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

## 8. Configuration & storage

- **8.1** Rules in `.jit/rules.toml`; schemas inline OR in `.jit/schemas/`. A
  single rule is shorthand XOR raw-schema XOR file-reference — the loader
  REJECTS mixing shorthand and raw schema in one rule, to keep one definition
  per constraint.
- **8.2 No hard-coded rules.** `jit init` scaffolds a default `.jit/rules.toml`
  reproducing today's checks, fully editable.
- **8.3 Hard migration, NO backward compatibility.** ALL existing enforcement is
  re-expressed as default rules and the legacy config keys are REMOVED (not
  aliased). Complete inventory to migrate:
  - From `validation.rs`: type-label requirement, label format
    (`label_regex`/`reject_malformed_labels`), namespace-registry enforcement.
  - From `validate.rs`: orphan-leaf warning, strategic-label consistency, and
    per-namespace allowed-`values`, value-`pattern`, uniqueness, and `required`
    constraints (`config.rs` `NamespaceConfig`, enforced ~`validate.rs:286-388`).
  - From `[validation]` (`config.rs:67-84`): `strictness`, `default_type`,
    `require_type_label`, `label_regex`, `reject_malformed_labels`,
    `enforce_namespace_registry`, `warn_orphaned_leaves`,
    `warn_strategic_consistency`.
  `rules.toml` becomes the SINGLE source of truth for issue/label validation.
  Default rules preserve today's behavior (warn vs reject) via `enforce` (§7.2).
- **8.4 Migration**: a one-time migration converts existing `[validation]` and
  `[namespaces].{values,pattern,required}` config into `.jit/rules.toml`, then
  drops the old enforcement keys. `jit init` scaffolds defaults for new repos.

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
