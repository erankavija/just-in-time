# Design / Decision Record: Generic Validation Engine

Issue: `f6a704d0` — Generic issue & label validation engine (JSON-Schema-based), with SDD as first application.
Blocks milestone: `9db27a3a` (Version 1.0 Production Release).

This document is the authoritative record of the design interview. Each decision
is numbered for traceability from child issues.

## 1. Problem & approach

jit's "plan" is unstructured free text in `Issue.description`; validation is a
few hard-coded checks in `validation.rs`/`validate.rs`. SDD research (OpenSpec,
cc-sdd, get-shit-done, graphify, ruflo) converged on: make plans falsifiable,
structured, traceable. Decision: **do not add SDD features; add one generic,
declarative validation layer and express SDD as configuration.** jit stays lean
and becomes a flexible substrate for many methodologies.

## 2. Rules vs gates (the boundary)

- **Rules**: stateless, structural, evaluated on write (local) or on demand
  (graph). Express "is this issue/label well-formed?".
- **Gates**: stateful, per-issue, transition-blocking, often async/external
  (tests, review). Express "has this passed its checkpoints?".
- A gate MAY invoke `jit validate` via `--checker-command`, but they are
  distinct layers and must not be merged.

## 3. Rule model

A rule is **selector + assertion + severity**.

- **3.1 Selector** — full matrix: `type`, `label`, `state`, `has_doc_type`;
  AND-combinable within one rule.
- **3.2 Profiles are not a concept** — a `profile:sdd` (or any) label is just a
  label that rules select on. An issue's applicable rules = the union of all
  rules whose selectors match. Type-based and label-based rules compose
  additively.
- **3.3 Two scopes**:
  - *issue-local*: pure predicate over one (projected) issue; runs on write.
  - *graph/aggregate*: needs the whole store; runs only in `jit validate` and
    gate checkers, NEVER on write.

## 4. Assertion vocabulary

- **4.1 Local kinds (v1)**: `require-label` (cardinality min/max by selector),
  `label-value-pattern` (regex per namespace), `require-section` (description
  heading present), `require-doc-type` (>=1 attached doc of a type). These are
  **shorthands that desugar to JSON Schema** — not a parallel mechanism.
- **4.2 Graph kinds (v1)**: `label-coverage` (every `source` label satisfied by
  >=1 child in a required state; state configurable per rule),
  `label-reference` (a `from` label must resolve to a declared label, scope
  configurable e.g. ancestors), `dependency-shape` (selector must/should depend
  on a target selector).
- **4.3 Escape hatch (v1)**: a `checker-command` rule kind reusing the gate
  `--checker-command` / `--pass-context` machinery, for logic the declarative
  vocabulary cannot express.
- **4.4 doc-schema**: validates parsed document content (see projection, §6).

## 5. Formalism: JSON Schema

- **5.1** Validate with the `jsonschema` crate (~0.46; pin at impl time).
  `schemars` (already a dep) generates schemas for jit's own types.
- **5.2** Shorthand kinds (§4.1) desugar to JSON Schema; one validator
  underneath (consistency) with ergonomic surface on top.
- **5.3 Long tail**: checker-command now. Future declarative cross-field logic
  (value comparison, arithmetic/ratios, cross-collection) -> `x-jit-*` **custom
  keywords** registered with the validator. Unknown keywords are ignored by
  standard validators, so such schemas degrade gracefully. A custom keyword MAY
  embed CEL internally. **No CEL as a top-level rule type in v1.**
- **5.4 Why not CEL now**: its only marginal value over JSON Schema within a
  single issue is value comparison / arithmetic / cross-collection predicates —
  a narrow band already covered by the checker-command hatch. Avoid a second
  formalism.

## 6. Validation target: enriched projection

- **6.1** Issues are normalized before validation into a canonical JSON
  projection: labels grouped by namespace (`labels.req = [...]`), structured
  fields, plus content fields parsed into a canonical structure.
- **6.2 Per-format parsers**: content (description + attached docs) is parsed by
  a format parser into ONE canonical structure (sections -> items, headings,
  attributes, text). Behind a single trait. **v1 ships Markdown**
  (pulldown-cmark, already a dep); **HTML and XML parsers** are added behind the
  same trait. Format specifics live in user schemas, not in code.
- **6.3** Example: `- [hard] REQ-01: ...` projects to a `success_criteria`
  section item; JSON Schema asserts `contains {pattern:"^\\[hard\\]"}` with
  `minContains:1` for ">=1 hard criterion". No bespoke marker parser required.

## 7. Severity & enforcement

- **7.1 Severity**: `off` / `warn` / `error`.
- **7.2 Write blocking**: per-rule `enforce` flag, **defaulting to enforce
  (blocking)** when unspecified. `error`+enforce fails `create`/`update`.
- **7.3 Bypass**: `--force` bypasses enforce failures (consistent with today's
  `--force` for warnings).
- **7.4 Graph timing**: graph rules run in `jit validate` and gate checkers
  only; never on write.

## 8. Configuration & storage

- **8.1** Rules in `.jit/rules.toml`; schemas in `.jit/schemas/` (referenced by
  path) or inline in a rule.
- **8.2 No hard-coded rules.** `jit init` scaffolds a default `.jit/rules.toml`
  reproducing today's checks (orphan leaves, type-label requirement, strategic
  labels), fully editable.
- **8.3 Backward compat**: the three existing checks become default rules with
  identical behavior; legacy `[validation]` keys keep working as aliases.
- **8.4 Migration**: existing repos lack `rules.toml`; provide a one-time
  scaffold path (e.g. `jit init` on an existing repo, or `--write-defaults`) so
  they do not silently lose validation.

## 9. CLI surface

- **9.1** Extend `jit validate [<id>] [--json]` — whole-repo or single issue,
  reporting rule name + severity + message. No `jit rule` subcommands.
- **9.2** Gates invoke validation via
  `jit gate define ... --checker-command "jit validate {issue} --json"`.

## 10. SDD as first application (docs/examples only)

- **10.1** No preset subsystem. SDD ships as example `.jit/rules.toml` +
  schemas + documentation.
- **10.2** Demonstrates: `req:ID` / `satisfies:ID` namespaces, `label-coverage`
  (every `req:` satisfied by a done child), `label-reference`
  (`satisfies:` resolves to a declared `req:`), a spec content schema using the
  requirement/scenario structure, and the `[hard]`/`[aspirational]` content
  check.
- **10.3** Docs also include at least one non-SDD example to show the engine is
  methodology-agnostic.

## 11. Dropped / deferred

- **Dropped**: delta-specs (ADDED/MODIFIED/REMOVED + merge-on-Done). Specs are
  edited in place.
- **Deferred**: CEL top-level rule type; `x-jit-*` custom keywords ship as an
  extension point only.

## 12. Layering (respect existing boundaries)

- Projection + local rule evaluation are **pure** (`domain/`), testable without
  I/O.
- Graph rules take the store.
- Config loading in storage/config layer.
- `validate` command orchestrates; CLI/output handle user-facing concerns.
