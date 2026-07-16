# Parent-escalation policy

The steward's decision tree for an escalation it **receives** from a dispatched
planning or execution lead. It is the receiving side, one tier up, of the lead
skills' escalation contracts: those skills decide what to escalate to their
invoker; this policy is how the invoking steward decides what to do with what
arrives.

The default is **resolve it yourself against the vision and decision log**, and
record the resolution as a new charter decision-log entry. Only three categories
of subordinate escalation are bigger than the strategic container and forward to
the human. Every other escalation is resolved from the vision without human
involvement.

## The invoker relationship

Every dispatched planning and execution lead names the steward as its invoker
(per `container-dispatch.md` steps 4 and 5). So each lead reports every
escalation to the steward, not to the human. The steward is the party that
resolves it against the vision or raises it onward.

When the steward itself runs standalone, the steward's own invoker is the human.
"Forward to the human" therefore means the steward raises the escalation
interactively with the human (e.g. via `AskUserQuestion`); it does not mean
handing it back to the lead.

## Decision tree

For each subordinate escalation, classify it. If it falls in one of the three
forward-to-human categories, forward it. Otherwise resolve it from the vision and
record a decision-log entry.

### Resolve from the vision (the default, no human)

These fit inside the strategic container's scope. The vision and the existing
decision log already answer them, or answer them once the steward decides. Resolve
each directly, then record the resolution as a new `### D-N` charter entry (see
The recording step). Non-exhaustive categories:

1. **Cross-sub-strategic-container dependency within this strategic container.**
   The lead surfaces work in a sibling sub-strategic container under the same
   strategic parent (an execution lead's cross-epic-but-within-this-milestone
   dependency, in graph terms). It stays inside the strategic container the
   steward owns, so the steward orders it against the vision: reorder the waves,
   add the dependency edge, or fold the work in. No human input.

2. **Architectural choice the vision settles or can settle.** The escalation asks
   which of several valid approaches to take. If the vision or a landed `D-N`
   already decides it, cite that decision back to the lead. If not, decide it
   against the vision's yardstick and log a new `D-N`. This is exactly the
   escalation the charter exists to answer.

3. **Rework-exhausted issue whose direction the vision determines.** The lead
   hits `MAX_REWORK_ATTEMPTS` (or a repeated same-root-cause finding) and
   escalates because the requirements or design are unclear. When the resolution
   is a direction the vision fixes (which behavior is correct, which of two
   contracts the project wants), the steward decides it and returns the guidance;
   log the call as a `D-N`.

4. **Scope question answerable from the charter.** "Is this in scope for the
   container?" The vision statement is the yardstick. Answer it (in-scope /
   out-of-scope), and log the boundary interpretation if it is consequential
   enough that someone would re-litigate it.

5. **Any other escalation contained by the strategic container.** If resolving it
   does not contradict the vision, does not reach beyond the strategic container
   into another top-level container, and does not change project-wide
   infrastructure, the steward owns it. Resolve from the vision and log it.

### Forward to the human (exactly these three)

Forward, and only forward, when the escalation is one of these. Do not add a
fourth category; do not drop one. These are genuinely bigger than any single
sub-strategic container, so the steward cannot resolve them against a vision it
does not own the authority to rewrite.

1. **Vision-level conflicts.** The escalation contradicts the vision or charter,
   or resolving it would rewrite the vision statement or overturn a landed
   decision the vision depends on. The steward resolves *against* the vision; it
   does not get to change the vision. A change to the yardstick is the human's.

2. **Cross-strategic-container dependencies.** The work spans more than one
   top-level strategic container, beyond any single sub-strategic container's
   scope. The steward owns one strategic container; a dependency reaching into
   another top-level strategic container is outside that authority and forwards to
   the human.

3. **Project-wide infrastructure changes.** CI/CD configuration, shared tooling,
   build systems, or project-wide config whose blast radius exceeds one container.
   Small or large, the change affects work beyond the strategic container, so the
   human decides.

**Every other subordinate escalation is resolved from the vision without human
involvement.** If an escalation is not one of the three above, the steward does
not forward it. Forwarding a resolvable escalation is a red flag.

## The recording step

A resolved escalation is recorded as a new charter decision, in the exact format
defined by `references/vision-charter.md` (Decision-log entry format): a one-line
summary row under `## Decision Log` and the full `### D-N` entry under
`## Decision Details`, sharing the same `D-N` id. The entry states the option
chosen, the option(s) rejected, and the reasoning:

```markdown
## Decision Log

- D-N: <one-line summary of the resolved escalation>

## Decision Details

### D-N: <one-line title of the resolved escalation>

- **Chosen:** <the resolution the steward gave the lead, stated concretely>
- **Rejected:** <the alternative(s) the lead raised>: <why each lost>
- **Reasoning:** <why the chosen option is coherent with the vision; the tradeoff that decided it>
- **Date:** <ISO-8601 date>
```

Refer to the summary row as `D-N` in the linked charter; add it together with
the full entry. Follow `vision-charter.md`'s rules verbatim: continue the
numbering from the highest existing `D-N`; ids are append-only and never reused;
`Rejected` is never empty. The lead escalated because a real alternative existed,
so name it. After appending, re-link the charter to the strategic container with
`jit doc add` per `vision-charter.md`.

A resolution the steward gives without logging it is not recorded: a resumed
session re-reads the charter, not the lead's chat, so an unlogged decision is lost
and the same escalation returns. Resolve and record are one step.

## Stop and escalate

The forward-to-human set is exactly the three categories above; nothing here adds
a fourth. Two clarifications on the boundary, then the operational halts:

- An escalation the charter is silent on, whose resolution would set **new**
  project direction rather than apply the existing vision, is a **vision-level
  conflict** (category 1). Forward it as that category; do not guess a resolution
  the vision does not support. This is not a separate forward category.
- The other two categories bind the same way: a dependency reaching another
  top-level strategic container, or a project-wide infrastructure change,
  forwards; everything else the steward resolves.

Operational halts are **not** subordinate-escalation forwards; they stop the
steward on a tooling failure and are reported to the steward's own invoker:

- The charter cannot be read or appended (unresolvable path, or a corrupt log per
  `vision-charter.md`'s stop conditions), so a resolution cannot be recorded. Stop
  and report the failure rather than resolve without recording.

## Red flags

- Forwarding a resolvable escalation to the human. If it is not one of the three
  forward categories, the steward resolves it from the vision. Forwarding
  everything defeats the steward's purpose.
- Resolving a vision-level conflict yourself. The steward resolves against the
  vision, never rewrites it; a change to the yardstick is the human's.
- Treating a cross-sub-strategic-container dependency inside this strategic
  container as if it were cross-strategic-container. The former stays inside the
  steward's scope and is resolved from the vision; only a dependency reaching into
  another top-level strategic container forwards.
- Resolving an escalation without recording its `## Decision Log` row and
  `### D-N` entry. An unlogged resolution is lost on resume and the escalation
  recurs.
- Logging the resolution with an empty `Rejected` field. The lead escalated
  because a real alternative existed; name it and say why it lost.
- Adding a fourth forward-to-human category, or dropping one of the three. The set
  is exactly three.
