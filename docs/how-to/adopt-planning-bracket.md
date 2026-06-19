# How-To: Adopt the Planning Bracket

> **Diátaxis Type:** How-To Guide
> **Audience:** Ruleset authors who want plan review and a coverage check sequenced *before* work fans out

This guide walks you through adopting the **plan-before-fan-out bracket** in your
own project: declaring the breakable container and bracket node types, wiring the
three gates, adding the preview and closure coverage rules, then scaffolding a
container and breaking it down. For the *why* — the spine, the three gates, and the
preview-vs-closure split — read
[The Plan-Before-Fan-Out Bracket](../concepts/planning-bracket.md) first.

The bracket is **configuration**, not engine behaviour. Two complete, copy-ready
rulesets ship it; this guide points you at the exact blocks to lift:

- [`docs/examples/sdd/`](../examples/sdd/config.toml) — software, `epic` breakable.
- [`docs/examples/research/`](../examples/research/config.toml) — research, `goal`
  breakable, with **no** software vocabulary anywhere (proof the bracket carries
  no baked-in altitude).

> The files under `docs/examples/` are EXAMPLES, not active on this repository. To
> use one, copy its `config.toml` to `.jit/config.toml`, its `rules.toml` to
> `.jit/rules.toml`, and its `schemas/` directory to `.jit/schemas/`.

## Prerequisites

- A JIT repository (`jit init`).
- A methodology already expressed (or about to be) as a ruleset — the bracket
  *adds to* a coverage ruleset; it does not replace one. If you are starting from
  scratch, copy `docs/examples/sdd/` or `docs/examples/research/` wholesale and
  skip to [Step 5](#step-5-scaffold-a-container).
- `jq` and an agent CLI on `PATH` if you want the `plan-review` and
  `breakdown-review` agent gates to actually invoke a reviewer (see
  [Step 4](#step-4-supply-the-gate-checker-scripts)).

## Step 1 — Declare the breakable container and the two bracket types

Add `planning` and `breakdown` to your type hierarchy as valid children of the
breakable container, and add a `[planning]` block naming the vocabulary. From
[`docs/examples/sdd/config.toml`](../examples/sdd/config.toml):

```toml
[type_hierarchy]
# `planning` and `breakdown` are the two bracket node types — function-typed
# children of the breakable `epic`.
types = { epic = 2, story = 3, planning = 3, breakdown = 3, task = 4, bug = 4 }

[planning]
breakable_types = ["epic"]          # container types that require a bracket
planning_type = "planning"          # the type of node P
breakdown_type = "breakdown"        # the type of node B
plan_doc_location = "dev/active/{id}-plan.md"   # where P's plan doc lives ({id} -> container id)
plan_gate_preset = "plan-review"        # agent gate applied to P
coverage_gate_preset = "coverage-preview"     # deterministic gate applied to B
breakdown_review_gate_preset = "breakdown-review"  # agent gate also applied to B
```

The research example is identical in shape, with `goal` substituted for `epic`:

```toml
[type_hierarchy]
types = { goal = 2, experiment = 3, planning = 3, breakdown = 3 }

[planning]
breakable_types = ["goal"]
planning_type = "planning"
breakdown_type = "breakdown"
plan_doc_location = "dev/active/{id}-plan.md"
plan_gate_preset = "plan-review"
coverage_gate_preset = "coverage-preview"
breakdown_review_gate_preset = "breakdown-review"
```

The engine hardcodes **none** of these names — `epic`/`goal`, `planning`,
`breakdown`, and the preset names are all read from this block.
`breakdown_review_gate_preset` is optional: omit it and it defaults to the builtin
`breakdown-review` preset, so an existing bracket config gains the review gate
without an edit.

> **Sync the type-known schema.** Adding a type to `[type_hierarchy].types`
> updates the graph hierarchy, but if your project has a baked
> `.jit/schemas/default-type-hierarchy-known.json`, the write path reads *that*
> frozen enum. The shipped examples regenerate it from `[type_hierarchy]`, so
> their `schemas/` directories already list `planning`/`breakdown`. If you build a
> ruleset by hand, copy the example `schemas/` directory too, or the write path
> warns on every `type:planning`/`type:breakdown` issue.

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
   `brackets:<C-id>` label. The rule is keyed on `type:breakdown`, so it fires only
   while `B` exists — never on an in-progress container.

Keep every other knob identical to the closure rule (`criteria-section`, `marker`,
`id-pattern`, the satisfies namespace, `child-link`, `child-type-exclude`). The
research example mirrors this exactly with its `hypotheses` section and `tests`
namespace (`research-hypotheses-covered-preview`).

Verify the ruleset loads:

```bash
jit validate --explain
```

## Step 4 — Supply the gate checker scripts

The three gate presets (`plan-review`, `coverage-preview`, `breakdown-review`) are
**built in** to JIT, so you do not define the gates by hand — but their checkers
shell out to scripts that must exist at the root of **your** repository. The
scripts ship in the JIT source tree, so copy them from a checkout of the JIT
repository into your project. Point `JIT_SRC` at that checkout (clone it first if
you do not already have one):

```bash
# In your adopting project's root. JIT_SRC = a local checkout of the jit repo.
JIT_SRC=${JIT_SRC:-/path/to/jit}            # e.g. git clone https://…/jit /tmp/jit && JIT_SRC=/tmp/jit
mkdir -p scripts
cp "$JIT_SRC"/scripts/coverage-preview.sh scripts/        # deterministic coverage gate (on B)
cp "$JIT_SRC"/scripts/ai-review.sh scripts/               # agent review runner (plan-review + breakdown-review)
cp "$JIT_SRC"/scripts/plan-review-prompt.md scripts/      # the plan-review prompt (on P)
cp "$JIT_SRC"/scripts/breakdown-review-prompt.md scripts/ # the breakdown-review prompt (on B)
chmod +x scripts/coverage-preview.sh scripts/ai-review.sh
```

(If you are adopting the bracket *inside* the JIT repository itself, these scripts
are already present at `scripts/` — skip this step.)

What each does:

- **`coverage-preview.sh`** (the `coverage-preview` gate's checker) reads the gated
  breakdown node's `brackets:<C-id>` label via `jit issue show ... --json | jq`,
  recovers the container `C`, and runs `jit validate --scope <C>`. That scoped
  validation evaluates your **preview rule** from Step 3 and exits 4 — failing the
  gate — when a `[hard]` criterion is left uncovered.
- **`ai-review.sh`** (the checker shared by the `plan-review` and `breakdown-review`
  gates) pipes the gate context into an agent CLI and parses a `VERDICT:
  PASS`/`VERDICT: FAIL`. Set the reviewer agent via the `REVIEWER_AGENT` env var;
  each preset points it at its own prompt file (`scripts/plan-review-prompt.md` for
  `P`, `scripts/breakdown-review-prompt.md` for `B`). See
  [Custom Gates](custom-gates.md#context-aware-gates) for the agent-gate mechanism.
- **`breakdown-review-prompt.md`** drives the agent review of the *decomposition* on
  `B`: per-child content standards, dependency-DAG coherence (missing **and**
  over-constraining edges), right-sized depth, and blank-workspace reachability. It
  does not re-check `[hard]` coverage — that is `coverage-preview.sh`'s job.

You can inspect any preset before applying it:

```bash
jit gate preset show plan-review
jit gate preset show coverage-preview
jit gate preset show breakdown-review
```

## Step 5 — Scaffold a container

The scaffolding commands apply the `plan-review` preset to `P` automatically. Two
entry points (both read the breakable types and gate presets from your
`[planning]` block):

**Create a new container already bracketed:**

```bash
jit issue create --type epic --title "Payment service" --with-planning
```

This creates the container `C`, creates the planning node `P` (`type:planning`),
wires `C → P`, sets `P`'s plan-doc location from `plan_doc_location`, and applies
the `plan-review` preset to `P`. The container's `type:` label must be one of
`[planning].breakable_types`. It does **not** create the breakdown node `B` —
that is the breakdown step's job (Step 7).

**Retrofit an existing container:**

```bash
jit plan epic-123
```

This brackets an already-existing container `C`: it creates `P`, wires `C → P`,
**moves `C`'s pre-existing upstream dependencies onto `P`** (so planning waits on
that upstream work and `C` becomes the pure closure node), applies the
`plan-review` preset, and sets `P`'s plan-doc location. Add `--json` for a
machine-readable result.

After scaffolding the graph is just `C → P`:

```bash
jit graph deps epic-123
```

## Step 6 — Write and review the plan

Author the plan document at the configured location (e.g.
`dev/active/<C-id>-plan.md`), then drive `P` through its `plan-review` gate. The
agent gate judges plan *quality* against `P`'s success criteria and the linked
design document. A FAIL leaves the drafts in place for revision — nothing is
archived on rejection. See [Custom Gates](custom-gates.md) for running and
inspecting agent gates.

## Step 7 — Break down behind the approved plan

Once `P`'s plan-review gate passes, break the container down. The breakdown step:

1. creates the breakdown node `B` (`type:breakdown`, labelled `brackets:<C-id>`,
   carrying **both** the `coverage-preview` and `breakdown-review` gates) depending
   on `P`;
2. drafts the implementation children in Backlog, each carrying the satisfies
   label (`satisfies:<id>` / `tests:<id>`) for the criterion it covers;
3. wires the **spine** — entry impl issues depend on `B` (sources), and the impl
   sinks are what `C` depends on; transitive reduction drops the now-redundant
   `C → P` edge — yielding `C → impl → B → P`;
4. runs `B`'s gates.

`coverage-preview` runs `jit validate --scope <C>`, which fires your preview rule.
If the drafted children leave a `[hard]` criterion with no satisfying child (in any
state), the gate **blocks** (exit 4) and names the uncovered criteria. The
`breakdown-review` agent gate separately judges the decomposition's *quality* —
content standards, dependency-DAG coherence, right-sized depth — and reports
concrete fixes on a FAIL. Because the impl subgraph transitively depends on `B`,
**both** gates must pass before any implementation issue becomes ready, so jit's
gate enforcement sequences the review without extra scripting. Fix what either gate
flags (add a missing child or satisfies label; apply the review's proposed edge or
content fixes) and re-run.

You can run the scoped check directly at any time:

```bash
jit validate --scope epic-123
```

The `jit-breakdown` skill performs this spine wiring and runs the gate
automatically; if you wire it by hand, follow the source/sink edge geometry in
[the concept page](../concepts/planning-bracket.md#edge-geometry).

## Step 8 — Implement, then close

With the breakdown approved, the impl children become ready in dependency order
and the work proceeds normally. When the container finally transitions to `done`,
your **closure** rule from Step 2 fires: every `[hard]` criterion must now be
satisfied by a **done** child. An uncovered criterion blocks the `→ done`
transition (exit 4); `--force` bypasses it and records a bypass event in the audit
log.

So coverage is checked at **both ends** of the bracket — *mapping exists* at the
breakdown gate (plan time), *mapping done* at the container's done transition
(closure) — by one rule kind instanced twice.

## See Also

- [The Plan-Before-Fan-Out Bracket](../concepts/planning-bracket.md) — the spine, the three gates, and the preview-vs-closure split
- [How-To: Author Validation Rules](validation-rules.md) — the `label-coverage` rule kind and selectors
- [How-To: Custom Gates](custom-gates.md) — the agent-gate mechanism and gate presets
- [Methodology-Agnostic Validation](../concepts/validation-engine.md) — why coverage is configuration
- [`docs/examples/sdd/`](../examples/sdd/config.toml) — the complete software ruleset
- [`docs/examples/research/`](../examples/research/config.toml) — the complete research ruleset (no software vocabulary)
