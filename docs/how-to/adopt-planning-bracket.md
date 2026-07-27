# How-To: Manually Adopt the Planning Bracket

> **Diátaxis Type:** How-To Guide
> **Audience:** Ruleset authors who want plan review and a coverage check sequenced *before* work fans out

This guide walks you through adopting the **plan-before-fan-out bracket** in your
own project: declaring the breakable container and bracket node types, wiring the
three gates, adding the preview and closure coverage rules, then scaffolding a
container and breaking it down. For the *why* — the spine, the three gates, and the
preview-vs-closure split — read
[The Plan-Before-Fan-Out Bracket](../concepts/planning-bracket.md) first.

For JIT's portable recommended workflow, use
`jit init --profile jit-dogfood` instead. The
[Repository Profiles reference](../reference/profiles.md) owns that package's
commands, inventory, and guarantees. Continue with this guide when you need an
alternative container type, taxonomy, coverage convention, template, or gate
integration.

The bracket is **configuration**, not engine behaviour. Two complete, copy-ready
rulesets ship it; this guide points you at the exact blocks to lift:

- [`docs/examples/sdd/`](../examples/sdd/config.toml) — software, `epic` breakable.
- [`docs/examples/research/`](../examples/research/config.toml) — research, `goal`
  breakable, with **no** software vocabulary anywhere (proof the bracket carries
  no baked-in altitude).

> The files under `docs/examples/` are EXAMPLES, not active on this repository. To
> use one, copy its `config.toml` to `.jit/config.toml`, its `rules.toml` to
> `.jit/rules.toml`, its `templates.toml` to `.jit/templates.toml`, and its
> `schemas/` directory to `.jit/schemas/`.

**Bracket configuration is optional and affects issues only when applied.** A
project with no `planning`/`breakdown` types and no `plan` template needs nothing
in this guide. The opt-in path is the three additions below: declare the `planning` and
`breakdown` types ([Step 1](#step-1--declare-the-breakable-container-and-the-two-bracket-types)),
add a `plan` template ([Step 1](#step-1--declare-the-breakable-container-and-the-two-bracket-types)),
and add the preview + closure coverage rules ([Steps 2–3](#step-2--add-the-closure-coverage-rule)).
Existing issues are untouched until you bracket one with `jit apply plan <C>`.

## Prerequisites

- A JIT repository (`jit init`).
- A methodology already expressed (or about to be) as a ruleset — the bracket
  *adds to* a coverage ruleset; it does not replace one. If you are starting from
  scratch, copy `docs/examples/sdd/` or `docs/examples/research/` wholesale and
  skip to [Step 5](#step-5--scaffold-a-container).
- A repository-owned review integration if `plan-review` and
  `breakdown-review` must provide real approval rather than the built-in
  warning-only placeholders (see
  [Step 4](#step-4--understand-and-replace-the-review-placeholders)).

## Step 1 — Declare the breakable container and the two bracket types

First add `planning` and `breakdown` to your type hierarchy in `.jit/config.toml`,
as valid children of the breakable container. From
[`docs/examples/sdd/config.toml`](../examples/sdd/config.toml):

```toml
[type_hierarchy]
# `planning` and `breakdown` are the two bracket node types — function-typed
# children of the breakable `epic`.
types = { epic = 2, story = 3, planning = 3, breakdown = 3, task = 4, bug = 4 }
```

Then declare the bracket itself as a `plan` `[[template]]` in
`.jit/templates.toml`. `jit apply plan <C>` instantiates this subgraph onto a
container. From [`docs/examples/sdd/templates.toml`](../examples/sdd/templates.toml):

```toml
[[template]]
name        = "plan"
applies_to  = ["epic"]           # container types that require a bracket

  [[template.anchors]]
  name = "container"             # bound at apply time to the target container

  [[template.nodes]]
  role        = "planning"       # node P
  type        = "planning"       # P's issue type (must exist in [type_hierarchy])
  gates       = ["plan-review"]  # review checkpoint applied to P
  doc_area    = "dev/active"     # issue-scoped area {container.dir} resolves in
  doc         = "{container.dir}/plan.md"

  [[template.nodes]]
  role        = "breakdown"      # node B
  type        = "breakdown"      # B's issue type
  gates       = ["coverage-preview", "breakdown-review"]  # the TWO gates on B
  labels      = ["brackets:{container.short_id}"]         # B's container pointer
  depends_on  = ["planning"]     # internal edge B → P

  [[template.anchor_edges]]
  from = "container"             # container depends on the breakdown node (C → B)
  to   = "breakdown"

  [[template.transforms]]
  kind = "move-upstream-to-role"
  role = "planning"             # move C's pre-existing upstream deps onto P
```

The research example is identical in shape, with `goal` substituted for `epic`
(`applies_to = ["goal"]`) — see
[`docs/examples/research/templates.toml`](../examples/research/templates.toml).

The engine hardcodes **none** of these names — `epic`/`goal`, `planning`,
`breakdown`, and the preset names are all read from the template.

`doc_area` is the node's area declaration: it names one of the issue-scoped areas
your [`[documentation]` table](../reference/configuration.md#documentation)
declares, and an area the registry does not declare fails the apply. Inside a
`doc` path, `{container.dir}` interpolates to the canonical artifact directory
`C` owns in that area — the directory `jit doc dir <C> dev/active` prints — so
the filename beside it is the artifact's own name, `plan.md`. To make `P`'s plan
an inline body rather than an external file, omit the `doc` field.

The `planning` / `breakdown` node `role`s and the `container` anchor `name` above
are the defaults the bracket tooling assumes. To use your own vocabulary, name
them in `.jit/templates.toml`'s top-level `[roles]` and `[anchors]` tables — see
[Template bindings](../reference/configuration.md#template-bindings-jittemplatestoml).

> **New types take effect immediately.** The `type-hierarchy-known` rule (a
> built-in default) derives its allowed-type enum from `[type_hierarchy]` in
> memory at load, so adding `planning` / `breakdown` to config recognizes them on
> the next command with no regeneration step. The `schemas/default-*.json` files
> `jit init` scaffolds are regenerated projections for external consumers, not the
> validation authority.

Declare the `brackets:` namespace so `B`'s container pointer validates cleanly:

```toml
[namespaces.brackets]
description = "On a breakdown node B: names the container C it brackets (the sole scope pointer for `validate --scope`)."
unique = true
examples = ["brackets:2fbd2a82"]
```

## Step 2 — Add the closure coverage rule

The bracket front-ends an existing `→ done` coverage check, so you need a
**closure** `label-coverage` rule keyed on the container at `state = "done"`. This
is the same rule you would write for done-transition coverage without a bracket.
From [`docs/examples/sdd/rules.toml`](../examples/sdd/rules.toml):

```toml
[[rules]]
name = "sdd-hard-criteria-covered"
when = { type = "epic", state = "done" }
severity = "error"
enforce = true
assert = { label-coverage = { criteria-section = "success_criteria", marker = "[hard]", id-pattern = "REQ-[0-9]+", satisfies-namespace = "satisfies", child-state = "done", child-link = "dependencies", child-type-exclude = ["planning", "breakdown"] } }
```

Two knobs make this bracket-aware:

- **`child-state = "done"`** — closure asks *mapping DONE*: a covering child must
  itself be done.
- **`child-type-exclude = ["planning", "breakdown"]`** — drops the bracket nodes
  `P`/`B` from coverage candidates and halts the transitive walk at `B`, so
  coverage tallies exactly the impl interior between `C` and `B`. In a plain
  (unbracketed) container this exclusion is simply a no-op.

`child-link = "dependencies"` makes the walk transitive, so a criterion satisfied
by a non-sink impl issue deep in the subgraph is still credited.

## Step 3 — Add the preview coverage rule

The **preview** rule is the same `label-coverage` kind, keyed on the **breakdown
node** and checking *mapping EXISTS* instead of *mapping DONE*. From
[`docs/examples/sdd/rules.toml`](../examples/sdd/rules.toml):

```toml
[[rules]]
name = "sdd-coverage-preview"
when = { type = "breakdown" }
severity = "error"
enforce = true
assert = { label-coverage = { criteria-section = "success_criteria", marker = "[hard]", id-pattern = "REQ-[0-9]+", satisfies-namespace = "satisfies", child-link = "dependencies", child-type-exclude = ["planning", "breakdown"], container-from-label = "brackets" } }
```

It differs from the closure rule in exactly the two ways the
[concept page](../concepts/planning-bracket.md#coverage-at-both-ends-preview-vs-closure)
describes:

1. **`child-state` is OMITTED.** An absent `child-state` means "any state", so a
   drafted child in Backlog counts. (Do **not** write `child-state = "any"` — that
   is not a valid token; the *absence* of the key is how "any state" is expressed.)
2. **`container-from-label = "brackets"`** redirects the criteria source from `B`
   (which has no criteria of its own) to its container `C`, recovered from the
   `brackets:<C-short-id>` label. The rule is keyed on `type:breakdown`, so it fires only
   while `B` exists — never on an in-progress container.

Keep every other knob identical to the closure rule (`criteria-section`, `marker`,
`id-pattern`, the satisfies namespace, `child-link`, `child-type-exclude`). The
research example mirrors this exactly with its `hypotheses` section and `tests`
namespace (`research-hypotheses-covered-preview`).

Verify the ruleset loads:

```bash
jit validate --explain
```

## Step 4 — Understand and replace the review placeholders

The three gate presets (`plan-review`, `coverage-preview`, `breakdown-review`) are
**built in** to JIT, so you do not define them by hand and no checker scripts or
JIT source checkout are required.

What each does:

- **`coverage-preview`** uses the in-process `label_target_validation` checker. It
  reads `B`'s `brackets:<C-short-id>` label and runs scoped validation for `C`.
  Your preview rule from Step 3 therefore blocks when a `[hard]` criterion is
  uncovered.
- **`plan-review`** and **`breakdown-review`** use the in-process
  `review_placeholder` checker. Each passes with an advisory structured finding
  and prints `WARNING: EXTERNAL REVIEW PLACEHOLDER`. This is an unmistakable
  installation placeholder, not review evidence.

Before relying on either review gate, replace its checker with your own external
integration. For example:

```bash
jit gate update plan-review \
  --checker-command './scripts/review-plan.sh'
jit gate update breakdown-review \
  --checker-command './scripts/review-breakdown.sh'
```

The commands and scripts are repository policy; JIT does not require a particular
agent, JSON processor, or source checkout. See
[Custom Gates](custom-gates.md#context-aware-gates) for passing gate context to an
external reviewer.

You can inspect any preset before applying it:

```bash
jit gate preset show plan-review
jit gate preset show coverage-preview
jit gate preset show breakdown-review
```

## Step 5 — Scaffold a container

Bracket an existing container `C` by applying the `plan` template (its `type:`
label must be one of the template's `applies_to` types):

```bash
jit apply plan epic-123
```

This reads the `plan` template (the node types, gate presets, doc location, and
the `brackets:` label all come from `.jit/templates.toml`) and in one step:

- creates the planning node `P` (`type:planning`), applies the `plan-review`
  preset, and sets `P`'s plan-doc location from the template's `doc`;
- creates the breakdown node `B` (`type:breakdown`, labelled `brackets:<C-short-id>`),
  carrying the `coverage-preview` and `breakdown-review` presets, depending on `P`;
- wires the anchor edge `C → B` and **moves `C`'s pre-existing upstream
  dependencies onto `P`** (so planning waits on that upstream work and `C` becomes
  the pure closure node).

Add `--json` for a machine-readable result; re-applying requires `--force`, which
refreshes the existing nodes' prose in place. After scaffolding the bracket is
`C → B → P`:

```bash
jit graph deps epic-123
```

## Step 6 — Write and review the plan

Author and link two artifacts to `P`, both inside the directory
`jit doc dir <C> dev/active` resolves:

- `plan.md`: concise shared architecture, decisions, risks, sources, and a
  generated overview;
- `breakdown.json`: the authoritative bare batch-create array, including
  complete issue bodies, edges, and planning metadata.

Validate the manifest, check the generated region, and run native validation
without allocating ids or writing issues/events:

```bash
DIR="$(jit doc dir <C> dev/active)"
.agents/skills/jit-planning-lead/scripts/breakdown_manifest.py validate \
  "$DIR/breakdown.json" --config .jit/config.toml \
  --plan "$DIR/plan.md" \
  --known-source <every-valid-source-id> ... \
  --required-source <mandatory-source-id> ... --deny-warnings
jit issue batch-create --from-json "$DIR/breakdown.json" --dry-run --json
```

Then drive `P` through `plan-review`. Review fails missing/invalid manifests,
stale generated output, duplicated task prose, and non-worker-sized terminal
tasks. Missing hierarchy/source universes, invented references, tier-laundered
leaves, and malformed/duplicate contract headings also fail. Per-code sizing
overrides remain visible for reviewer judgment. Shared contracts are marked
`plan-fixed` or `implementation-produced`; produced contracts require one
dependency-reachable producer. Each terminal discloses created/touched paths or
footprint uncertainty, including greenfield files. The built-in placeholder is
not approval; replace it as described in Step 4. Correct the manifest first,
regenerate the plan, and rerun validation.

## Step 7 — Break down after the plan checkpoint

Once a real `plan-review` passes, the breakdown consumes both the pre-created `B`
and the linked manifest. Markdown-only plans are rejected; there is no legacy
reinterpretation path. It then:

1. creates every implementation issue and intra-manifest edge in one
   `jit issue batch-create` call, using its returned semantic key→id map;
2. wires the **spine** — entry impl issues depend on `B` (sources), and the impl
   sinks are what `C` depends on; transitive reduction drops the now-redundant
   `C → B` anchor edge (`jit dep add --reduce <C> <sinks...>` when wiring by
   hand) — yielding `C → impl → B → P`;
3. re-homes recorded external dependencies through the key→id map, verifies
   issues and edges against the manifest, then runs `B`'s gates.

`coverage-preview` runs `jit validate --scope <C>`, which fires your preview rule.
If the drafted children leave a `[hard]` criterion with no satisfying child (in any
state), the gate **blocks** (exit 4) and names the uncovered criteria. The
`breakdown-review` placeholder separately reserves the decomposition-quality
checkpoint but performs no judgment until you replace its checker. Because the
impl subgraph transitively depends on `B`, all configured gates must pass before
any implementation issue becomes ready. Replace the placeholder before relying
on that sequencing as review approval.

You can run the scoped check directly at any time:

```bash
jit validate --scope epic-123
```

The `jit-breakdown` skill performs the batch and external wiring. Planning
metadata is accepted for authoring but is neither persisted nor exported.

## Step 8 — Implement, then close

With the breakdown gates passed, the impl children become ready in dependency
order and the work proceeds normally. The warning-only `breakdown-review`
placeholder is not approval evidence until replaced with a real reviewer. When
the container finally transitions to `done`, your **closure** rule from Step 2
fires: every `[hard]` criterion must now be satisfied by a **done** child. An
uncovered criterion blocks the `→ done` transition (exit 4); `--force` bypasses
it and records a bypass event in the audit log.

So coverage is checked at **both ends** of the bracket — *mapping exists* at the
breakdown gate (plan time), *mapping done* at the container's done transition
(closure) — by one rule kind instanced twice.

## See Also

- [Repository Profiles](../reference/profiles.md) — preferred embedded workflow setup
- [The Plan-Before-Fan-Out Bracket](../concepts/planning-bracket.md) — the spine, the three gates, and the preview-vs-closure split
- [How-To: Author Validation Rules](validation-rules.md) — the `label-coverage` rule kind and selectors
- [How-To: Custom Gates](custom-gates.md) — the agent-gate mechanism and gate presets
- [Methodology-Agnostic Validation](../concepts/validation-engine.md) — why coverage is configuration
- [`docs/examples/sdd/`](../examples/sdd/config.toml) — the complete software ruleset
- [`docs/examples/research/`](../examples/research/config.toml) — the complete research ruleset (no software vocabulary)
