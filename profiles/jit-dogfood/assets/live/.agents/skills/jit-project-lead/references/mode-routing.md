# Mode routing

The steward's front door. It classifies the opening request into exactly one of
four invocation modes and hands off to that mode's body. Routing only: it
resolves what to run and supplies the context, then the mode's own section owns
the work. It never runs two modes, and never guesses a mode the request does not
clearly signal.

Run this after pre-flight and tier derivation, once, on the opening request.
Tier derivation has already produced the **ANCHOR TYPE** (steward anchor) and
**BOUNDARY SET**; routing uses ANCHOR TYPE when it resolves a container.

## The four modes

| Mode | Request intent | Signal | Handoff |
|---|---|---|---|
| 1 | Lead an already-existing strategic container | The request names or points at a strategic container that already exists (an id, or a name/label that resolves to one) | Resolve the container, hand it + context to `## Sub-strategic dispatch` |
| 2 | Plan and execute a vague high-level goal | A goal with no container yet — a cold start needing scoping before any work exists | Charter step (`references/charter-facilitation.md`), then `jit-planning-lead` at the strategic altitude (its interview / research-and-plan entry) |
| 3 | Steering discussion | Interactive vision or decision work with **no workers dispatched** — think through direction, not drive execution | Steward-owned charter facilitation (`references/charter-facilitation.md`) |
| 4 | Standards sweep | Audit the project against the content standards and fix the mechanical violations | `## Standards sweep mode` |

## Classifying the opening request

Read the request for the one intent it expresses. Each mode has a distinct
signal; match the request to exactly one, or stop and ask (below).

- **Mode 1 — lead existing strategic work.** The request tells the steward to
  drive, lead, run, or own a strategic container that **already exists**, and
  identifies it: an issue id, or a name/label that resolves to one existing
  strategic-tier container. The defining signal is a *resolvable existing
  container* at the anchor tier.
  Representative request: *"Drive milestone `<container-id>` to
  done — steward it across its epics."* An id is given and it resolves to an
  existing anchor-tier container (here the `type:milestone` v1.0 container).

- **Mode 2 — plan and execute a vague goal.** The request states a high-level
  objective with **no container yet**: a cold start where the work has to be
  scoped and planned before anything exists to lead. The defining signal is a
  *goal without a resolvable container*, asking to both plan and get it built.
  Representative request: *"Stand up multi-tenant support for the product —
  figure out what that means and get it delivered."* No container exists; the
  goal must be scoped first.

- **Mode 3 — steering discussion.** The request asks for **interactive vision or
  decision work with no workers dispatched**: talk through direction, weigh
  options, settle the vision or a decision before any execution is committed. The
  defining signal is *deliberation, not delivery* — the requester wants to think,
  not to dispatch a team.
  Representative request: *"Let's work through the direction for next quarter and
  settle the vision before we commit anyone to it."* Vision/decision work only,
  explicitly no dispatch.

- **Mode 4 — standards sweep.** The request asks to **audit the project against
  the content standards** and fix the mechanical violations: bring the tracked
  issues and documents into content-standards compliance, surface the judgment
  calls. The defining signal is a *project-wide content-standards audit*.
  Representative request: *"Sweep the whole project for content-standards
  violations, auto-fix the mechanical ones, and list what needs a decision."*

Modes 2 and 3 both run the charter facilitation step; the split is delivery
intent. Mode 2 asks to plan **and get it built** (a cold-start goal to carry
through), so after the charter step it hands the scoped goal to
`jit-planning-lead`; mode 3 asks only to **deliberate** with no execution
committed, so it stops at the charter — no planning, no dispatch. When the request
wants a team put to work, it is mode 2; when it wants to think first with no
dispatch, it is mode 3.

## Handoffs

### Mode 1 — resolve and hand off

Mode 1 is resolve-and-handoff only. Resolve the container, then hand it to the
sub-strategic dispatch protocol; the dispatch section (and the sections it reads)
owns wave execution, coherence review, and charter/progress maintenance. Do not
run any of that here.

1. **Resolve the strategic container** from the identifier in the request:
   - An issue id → `jit issue show <id>`.
   - A name or strategic label → resolve it to a single issue (e.g.
     `jit issue list --json` filtered by the name/label the request gives). If it
     resolves to zero or more than one issue, stop and ask (Stop and ask, below).
2. **Confirm it is the steward anchor tier.** The resolved container's `type`
   must equal the **ANCHOR TYPE** from tier derivation. A container below the
   anchor tier is a sub-strategic container an execution lead owns, not the
   steward's to lead directly; if the resolved container is not at the anchor
   tier, stop and ask rather than steward the wrong altitude.
3. **Hand off.** Supply the resolved container id and the invocation context (the
   opening request and any constraints in it) to `## Sub-strategic dispatch`.
   That section resolves the vision/progress artifacts, layers the waves, and
   drives them. Routing stops here; it does not enter the dispatch loop itself.

### Modes 2 and 3 — run the charter step

Both run the steward-owned charter facilitation body
(`references/charter-facilitation.md`) at the **strategic altitude** — the vision
and its decisions are settled for the anchor-tier container, not below it. Mode 2
continues into planning after the charter step; mode 3 stops at the charter.

- **Mode 2** runs the charter step first: the facilitation body produces or
  evolves the charter with its owner (posture selected from the entry state), so a
  cold start yields a charter before planning begins. It then hands the scoped
  goal to the `jit-planning-lead` skill at its cold-start entry (its
  research-and-plan / interview path), which refines the goal into a
  strategic-tier container and plans behind it. Read that skill's `SKILL.md` and
  follow it for the plan; do not reimplement planning here.
- **Mode 3** runs the charter facilitation body end to end: the interactive
  strategic-altitude vision and decision work the requester asked for, every
  outcome landing in the charter, with no planning skill invoked. No
  `jit-execution-lead` is dispatched from the front door in either mode;
  execution, if it follows, is a separate later invocation (mode 1 over the
  now-existing container).

### Mode 4 — hand to the standards sweep

Route to `## Standards sweep mode` and run it end to end per
`references/standards-sweep.md`. The sweep is project-wide; it neither dispatches
leads nor changes issue lifecycle state. Do not reimplement the scan or fix.

## Stop and ask (do not guess a mode)

Route only when the request signals **exactly one** mode. Otherwise stop and ask
the invoker which mode, reporting what the request did and did not signal. Never
default to a mode.

Stop and ask when:

- **No explicit mode signal.** The request is generic ("steward this project",
  "follow the skill and run it") with no signal that distinguishes lead-existing,
  plan-a-goal, steer, or sweep. There is no mode to route to; ask which of the
  four the invoker means and stop. (The skeleton tier-derivation scenarios land
  here: the derived tiers are reported, then the front door asks which mode.)
- **More than one mode signalled.** The request mixes signals (e.g. "audit the
  standards and also drive the milestone"). Ask which to run first; do not run
  both, and do not pick one.
- **Mode 1 container does not resolve to exactly one anchor-tier container.** The
  identifier resolves to zero issues, to more than one, or to an issue whose type
  is not the ANCHOR TYPE. Report what was found and ask for the specific
  anchor-tier container to lead.

A stop-and-ask reports the derived tiers (already in hand) and the ambiguity, and
halts before any mode body runs. It writes no issue state.

## Red flags

- **Guessing a mode the request does not clearly signal.** A generic or
  ambiguous request stops and asks; it never defaults to a mode. Guessing is the
  primary failure this front door prevents.
- **Running mode 1 as more than resolve-and-handoff.** Mode 1 resolves the
  container and hands it to `## Sub-strategic dispatch`; it does not itself run
  waves, coherence review, or charter/progress maintenance.
- **Reimplementing planning, facilitation, or the sweep in the front door.** Mode
  2 runs the charter step then hands to `jit-planning-lead`; mode 3 runs
  `references/charter-facilitation.md`; mode 4 hands to `## Standards sweep mode`.
  The front door routes; each mode's work lives in its own referenced section.
- **Leading a sub-strategic container directly in mode 1.** If the resolved
  container is below the anchor tier, that is an execution lead's target, not the
  steward's. Stop and ask for the anchor-tier container.
- **Dispatching an execution lead from modes 2 or 3.** Neither dispatches
  workers; execution is a later mode-1 invocation over the planned container.
- **Running two modes for one request.** Exactly one mode per opening request.
