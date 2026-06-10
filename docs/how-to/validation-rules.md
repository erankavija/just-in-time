# How-To: Author Validation Rules

> **Diátaxis Type:** How-To Guide
> **Audience:** Users who want to enforce their own conventions on issues

JIT ships ONE declarative validation engine. You describe the conventions your
project cares about in `.jit/rules.toml` (optionally backed by JSON Schemas in
`.jit/schemas/`), and the engine enforces them on writes and on demand. The
engine is **methodology-agnostic**: it knows nothing about any particular way of
working. Spec-Driven Development (SDD) is just the first methodology *shipped as
configuration* — the same grammar expresses bug-triage hygiene, release
checklists, or whatever your team needs. See
[Methodology-Agnostic Validation](../concepts/validation-engine.md) for the
"why".

This guide shows you how to write rules, using ready-to-copy examples:

- [`docs/examples/sdd/`](../examples/sdd/rules.toml) — Spec-Driven Development
- [`docs/examples/bug-repro/`](../examples/bug-repro/rules.toml) — bug triage
- [`docs/examples/release-checklist/`](../examples/release-checklist/rules.toml) — release gating
- [`docs/examples/fresh-evidence/`](../examples/fresh-evidence/rules.toml) — fresh-evidence-before-done
- [`docs/examples/nyquist/`](../examples/nyquist/rules.toml) — criteria-to-check mapping
- [`docs/examples/cross-epic/`](../examples/cross-epic/rules.toml) — cross-epic requirement-id collision detection
- [`docs/examples/research/`](../examples/research/rules.toml) — research program: non-software hierarchy with `type:goal` / `type:experiment` and `hyp:` / `tests:` namespaces

> The files under `docs/examples/` are EXAMPLES. They are not active on this
> repository. To use one, copy its `rules.toml` to your project's `.jit/rules.toml`
> and its `schemas/` directory (if any) to `.jit/schemas/`.

## How validation works

When you create or update an issue, the engine:

1. **Projects** the issue into a canonical JSON shape (see
   [the projection](#the-projection) below).
2. **Selects** the rules whose `when` selector matches the issue.
3. **Evaluates** each matching *local* rule against the projection.
4. **Blocks** the write if any matching rule has `severity = "error"` AND
   `enforce = true` — unless you pass `--force` (which logs a bypass event).

Some rules are *graph* rules: they need cross-issue context (e.g. "is every
requirement covered by a child issue?"). Graph rules never run on plain field
writes; they run when you invoke `jit validate` or a gate checker, and at
**state transitions**, where a matching rule with `severity = "error"` and
`enforce = true` blocks the transition (exit 4) unless you pass `--force`
(which logs a bypass event). Scope a rule to a transition with a state
predicate, e.g. `when = { state = "done" }`.

```bash
jit validate            # run all rules (local + graph) over the whole repo
jit validate <id>       # validate a single issue
jit validate --explain  # show which rule produced each finding
```

## Anatomy of a rule

Every rule is a `[[rules]]` table in `.jit/rules.toml`:

```toml
[[rules]]
name = "epic-has-criteria"          # unique, human-readable
when = { type = "epic" }            # selector (see below); empty = matches all
severity = "error"                  # off | warn | error   (default: warn)
enforce = false                     # true => an `error` finding blocks writes
assert = { require-section = { heading = "Success Criteria" } }  # exactly one kind
```

- **`name`** must be unique across the file (so each finding attributes to one
  rule).
- **`severity`**: `off` disables the rule; `warn` reports but never blocks;
  `error` reports and (with `enforce = true`) can block a write.
- **`enforce`** defaults to `false` — an `error` rule warns until you opt into
  blocking.
- **`assert`** holds **exactly one** assertion kind. A shorthand kind and a
  `json-schema` file reference cannot coexist in one rule.

### Selectors (`when`)

A selector is the AND of any dimensions you specify; an empty `when` matches
every issue:

| Key            | Matches issues…                                          |
|----------------|----------------------------------------------------------|
| `type`         | whose `type:<value>` label equals this                   |
| `label`        | carrying this label; supports `ns:*` wildcards           |
| `state`        | in one of these lifecycle states (single value or list)  |
| `has-doc-type` | with an attached document of this `doc_type`             |

#### State predicates

`state` scopes a rule to a lifecycle phase. It accepts either a single state or
a list of states; the rule matches when the issue is in any of them:

```toml
when = { type = "epic", state = "in_progress" }              # single state
when = { state = ["ready", "in_progress", "gated"] }         # any of several
```

The valid state tokens are `backlog`, `ready`, `in_progress`, `gated`, `done`,
`rejected`, and `archived`. An unknown state name is rejected when the ruleset
loads, with an error naming the offending rule and listing the valid tokens, so
a typo cannot silently turn a rule into one that never matches.

State predicates combine (AND) with the other selector dimensions and are
evaluated everywhere a rule is selected — on the write path and in
`jit validate`. `jit validate <id> --explain` shows the matched predicate in the
rule's selector, rendering a list as `state=ready|in_progress`.

`--explain` lists **every** rule in the ruleset, not just the ones that fired:
matched rules render as `[PASS]`/`[FAIL]`, and rules whose selector excluded the
issue render as `[SKIP]` with the reason their selector did not apply. The state
dimension is called out explicitly, so you can see at a glance whether a
state-scoped rule applied. For example, asking `--explain` about an
`in_progress` issue against a rule scoped to `done`:

```text
[SKIP] done-needs-summary (local, error) selector: state=done — state predicate did not match (issue is 'in_progress', wants 'done')
```

In `--json`, each outcome carries `matched` (bool) and, when skipped, a
`skip_reason` string naming the excluding dimension(s); `skip_reason` is omitted
for matched rules.

### Assertion kinds

**Shorthand kinds** carry simple scalars in TOML and desugar to JSON Schema:

| Kind                  | Scope | Asserts…                                            |
|-----------------------|-------|-----------------------------------------------------|
| `require-label`       | local | a label (or `ns:*`) is present, with optional `min`/`max` count |
| `require-section`     | local | the body has a section with this heading            |
| `require-doc-type`    | local | a document of this `doc-type` is attached           |
| `label-value-pattern` | local | every value in a namespace matches a regex          |

**Raw JSON Schema** validates the projection directly:

| Kind          | Scope | Asserts…                                                  |
|---------------|-------|----------------------------------------------------------|
| `json-schema` | local | the projection conforms to a `.jit/schemas/<name>.json`  |

Raw schemas live in files only (TOML cannot faithfully express constructs like
`contains`/`pattern`). The reference must be a relative path under `schemas/`
ending in `.json`.

**Graph kinds** need cross-issue context and run in `jit validate`, gate
checkers, and at state transitions (where enforcing failures block):

| Kind                    | Asserts…                                                                    |
|-------------------------|-----------------------------------------------------------------------------|
| `label-coverage`        | every source criterion is satisfied by at least one child                   |
| `label-reference`       | a `from:`-namespace label resolves to a declared `to:` source               |
| `dependency-shape`      | issues matching a selector depend on issues matching a target               |
| `gate-recency`          | recorded gate results are no older than a configured age                    |
| `criteria-label-match`  | a namespace label's value names a real criterion id (stray-req detection)   |
| `criteria-to-check`     | every criterion in a section maps to a verifiable check (a required gate or a label) |
| `label-uniqueness`      | each value in a namespace is declared by at most one matching issue         |

`label-uniqueness` is repo-wide (`scope = "all"` is required and is the only
valid value). It runs ONLY in `jit validate` — not at transition time — because
repo-wide uniqueness cannot be determined from a single issue's dependency
neighborhood. See the [scope semantics reference](../reference/configuration.md#graph-rule-scope-semantics-jitrulestoml-scope)
and the [`docs/examples/cross-epic/`](../examples/cross-epic/rules.toml) example.

**Escape hatch:** `checker-command` runs an external command. It is applied by
`jit validate`, not on the write path.

## The projection

Schemas and shorthand kinds validate against a normalized JSON view of the
issue — the **projection** — not the raw issue. Its shape (stable contract):

```jsonc
{
  "type": "epic",                  // from the `type:*` label
  "state": "ready",
  "priority": "high",
  "labels": {                      // every label grouped by namespace
    "type": ["epic"],
    "req":  ["REQ-01", "REQ-02"]
  },
  "doc_types": ["design"],         // distinct attached document types
  "sections": {                    // parsed from the description (Markdown)
    "success_criteria": {
      "heading": "Success Criteria",
      "level": 2,
      "items": ["[hard] REQ-01: ...", "[aspirational] REQ-02: ..."]
    }
  }
}
```

Section keys are the *slugified* heading (`## Success Criteria` →
`success_criteria`). `items` are the raw text of the top-level list entries under
that heading. A JSON Schema can therefore express "at least one `[hard]`
criterion" with `contains` + `pattern` over `sections.success_criteria.items`.

## Worked example: Spec-Driven Development

SDD treats an epic's specification body as the single canonical source of truth.
The body has three structured sections — `## Requirements`, `## Scenarios`, and
`## Success Criteria` — and the `req:` / `satisfies:` labels are **derived** from
the criteria, not authored independently:

```markdown
## Requirements

- REQ-01: the loader rejects mixed shorthand and raw schema
- REQ-02: a nicety we would like

## Scenarios

- Given a rule mixing shorthand and a raw schema When the loader runs Then it errors

## Success Criteria

- [hard] REQ-01: the loader rejects mixed shorthand and raw schema
- [aspirational] REQ-02: a nicety we would like
```

The epic carries `req:REQ-01`; a child that implements it carries
`satisfies:REQ-01`. The full ruleset is in
[`docs/examples/sdd/rules.toml`](../examples/sdd/rules.toml); the highlights:

- **`require-section`** (local, enforced) — the epic must have a
  `## Success Criteria` section at all.
- **`json-schema`** (local, enforced) — the spec **structure** is well-formed:
  the body must contain non-empty `## Requirements` (each item `REQ-N: ...`),
  `## Scenarios` (each item a single-line `Given ... When ... Then ...`), and
  `## Success Criteria` sections, every Success Criteria item carries a
  `[hard]`/`[aspirational]` marker and a `REQ-N:` id, and at least one item is
  `[hard]`. See [`schemas/spec-body.json`](../examples/sdd/schemas/spec-body.json),
  which uses `pattern` over `sections.<slug>.items` plus a `contains` +
  `minContains` hard-criterion check.
- **`label-value-pattern`** (local) — `req:` ids look like `REQ-01`.
- **`criteria-label-match`** (graph, always-on) — every `req:<id>` label value
  is compared directly against the criterion ids extracted from `## Success
  Criteria` prose. A `req:REQ-77` absent from the criteria is reported as a
  **stray** immediately at any lifecycle phase.
- **`label-coverage`** (graph, at done, `enforce = true`) — every `[hard]`
  criterion is satisfied by some `done` child carrying `satisfies:<id>`. Scoped
  to `state = "done"` so planning is quiet; the done transition is blocked if any
  `[hard]` criterion is uncovered.
- **`label-reference`** (graph, `req` → `satisfies`, at done, `enforce = true`)
  — every declared `req:<id>` is actually used by some `satisfies:<id>` child at
  done. An unsatisfied `req:` label blocks the done transition.
- **`label-reference`** (graph, `satisfies` → `req`, always-on, warn) — every
  `satisfies:<id>` points at a declared `req:<id>` (reference integrity); a typo
  surfaces as a warning at any state.

### Lifecycle behavior

The SDD ruleset is designed so **planning is quiet and the done transition is
where coverage bites**:

- During planning (any state other than `done`) an in-flight epic with incomplete
  children — correct structure, matching `req:` labels, children still in progress
  — produces **zero error-severity graph findings** from `jit validate`. Only the
  stray-req check (`criteria-label-match`) fires immediately for fabricated ids.
- The done transition runs coverage and derivation rules with `enforce = true`,
  blocking (exit 4) if any `[hard]` criterion is uncovered or any `req:` is
  unsatisfied.
- `--force` bypasses the block and logs a bypass event in the audit log.

Two finding types are now textually distinct: a **stray req** (id not in the
criteria prose, `sdd-req-matches-a-criterion`) versus an **unsatisfied req** (id
is real, no child yet, `sdd-req-is-satisfied`). Together all five rules close the
derivation loop so a `req:` label cannot float free of the canonical criteria.

## The engine is methodology-agnostic

The same grammar expresses unrelated workflows with no engine changes:

### Bug triage — "every bug must have a repro"

[`docs/examples/bug-repro/rules.toml`](../examples/bug-repro/rules.toml) enforces
that every `type:bug` carries a `## Reproduction` section
(`require-section`) with at least one listed step
([`schemas/bug-body.json`](../examples/bug-repro/schemas/bug-body.json), a
`json-schema` rule over `sections.reproduction.items`).

### Release gating — a checklist methodology

[`docs/examples/release-checklist/rules.toml`](../examples/release-checklist/rules.toml)
requires a `type:release` to have a `## Checklist` section, an attached
`release-notes` document (`require-doc-type`), and a dependency on a
`type:qa-signoff` issue (`dependency-shape`, a graph rule).

### Fresh evidence — "don't finish on stale gates"

[`docs/examples/fresh-evidence/rules.toml`](../examples/fresh-evidence/rules.toml)
requires that a done issue's recorded gate results are recent (`gate-recency`, a
graph rule). It asserts the `code-review` gate result is at most 7 days old,
computed against the gate's recorded timestamp:

```toml
[[rules]]
name = "fresh-evidence-before-done"
when = { state = "done" }
severity = "error"
enforce = true
assert = { gate-recency = { max-age-days = 7, gates = ["code-review"] } }
```

A stale result yields `gate 'code-review' result is <N> days old (max 7)`; a
required gate with no recorded result yields `gate 'code-review' has no recorded
result`. Use `max-age-hours` for sub-day windows, and omit `gates` to require
freshness for ALL of the issue's required gates. Recency is relative to the clock,
so real rulesets usually keep these rules at `severity = "warn"` (the example
uses `enforce = true` to demonstrate blocking completion).

### Nyquist — "every criterion has a verification path"

[`docs/examples/nyquist/rules.toml`](../examples/nyquist/rules.toml) uses
`criteria-to-check` (a graph rule) to assert that every `[hard]` criterion in
an epic's `## Success Criteria` section has at least one verifiable check before
the epic is marked done:

```toml
[[rules]]
name = "nyquist-criteria-verified-at-done"
when = { type = "epic", state = "done" }
severity = "error"
enforce = true
assert = { criteria-to-check = {
  marker = "[hard]",
  gate-prefix = "verify:",
  check-namespace = "checks",
} }
```

A criterion `REQ-01` is "checked" when the issue has `"verify:REQ-01"` in
`gates_required` OR a label `"checks:REQ-01"`. Either mechanism alone satisfies
the rule. An unmapped criterion yields a finding naming the criterion id and
the expected gate or label:

```
criterion 'REQ-01' has no verification: expected gate 'verify:REQ-01' or label 'checks:REQ-01'
```

When only one mechanism is configured the message names only that one:

```
criterion 'REQ-01' has no verification: expected gate 'verify:REQ-01'
criterion 'REQ-01' has no verification: expected label 'checks:REQ-01'
```

The example ships a second (always-on, warn-only) variant of the rule that fires
during planning so authors catch missing verification paths early, long before the
done transition. Configure `criteria-section`, `marker`, `id-pattern`,
`gate-prefix`, and `check-namespace` to match your team's conventions; omit
`gate-prefix` or `check-namespace` to require only one mechanism. At least one
of the two must be set (the loader rejects a rule with neither).

### Research program — a non-software hierarchy

[`docs/examples/research/rules.toml`](../examples/research/rules.toml) models a
research program using only research vocabulary. There is no "epic" or
"milestone" anywhere in the ruleset. The two issue types are `type:goal` and
`type:experiment`; the label namespaces are `hyp:` (declared hypotheses on a
goal) and `tests:` (which hypothesis an experiment addresses).

A goal body has two required sections:

```markdown
## Hypotheses

- [hard] H-1: increasing training data improves accuracy
- [exploratory] H-2: model size is the primary performance driver

## Success Criteria

- accuracy exceeds 95% on the held-out test set
```

An experiment body has `## Method` (the protocol) and `## Evidence` (observations).

The ruleset demonstrates the full set of lifecycle-aware primitives in research
vocabulary:

- **`require-section`** (local, enforced) — a goal must have `## Hypotheses` and
  `## Success Criteria`; an experiment must have `## Method` and `## Evidence`.
- **`json-schema`** (local, enforced) — Hypotheses items must be shaped
  `[hard] H-N: <text>` or `[exploratory] H-N: <text>`, with at least one
  `[hard]` item. See [`schemas/research-bodies.json`](../examples/research/schemas/research-bodies.json).
- **`criteria-label-match`** (graph) — every `hyp:<id>` label on a goal must
  name a real hypothesis id extracted from `## Hypotheses`. A fabricated
  `hyp:H-99` not present in the body is reported immediately as stray.
- **`label-coverage`** (graph, scoped `state = "done"`, `enforce = true`) — when
  a goal reaches done, every `[hard]` hypothesis must be tested by at least one
  done experiment carrying `tests:<id>`. An in-flight goal produces zero error
  findings because the rule does not match until the done transition.
- **`label-reference`** (graph, warn) — every `tests:<id>` on an experiment
  resolves to a declared `hyp:<id>` on the linked goal. A typo surfaces as a
  warning without blocking.

None of these is built into JIT. They are configuration over one engine.

## Verifying your own ruleset

A ruleset is "real" when it loads and behaves. The shipped examples are checked
by [`crates/jit/tests/example_rulesets_tests.rs`](../../crates/jit/tests/example_rulesets_tests.rs),
which loads each `rules.toml` through the production loader and asserts a
compliant sample issue passes and a non-compliant one fails through the real
engine. To check your own rules in your project, run:

```bash
jit validate --explain
```

## See Also

- [Methodology-Agnostic Validation](../concepts/validation-engine.md) — why the engine is config-driven
- [Custom Gates](custom-gates.md) — wire `jit validate` into a quality gate
- [Labels](../reference/labels.md) — the `namespace:value` label model selectors use
- [Configuration](../reference/configuration.md) — `.jit/` configuration files
