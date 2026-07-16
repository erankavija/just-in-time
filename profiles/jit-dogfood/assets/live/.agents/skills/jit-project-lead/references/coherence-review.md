# Cross-container coherence review

Before a sub-strategic container is accepted as complete, review it against
every **other** sub-strategic container already accepted under the same
strategic-tier parent, and sweep the accepted containers' artifacts for
forward-looking references the just-landed container has made stale. This is the
one-tier-up analogue of `jit-execution-lead`'s per-issue review
(`../../jit-execution-lead/references/lead-review-protocol.md`): its **Tier 3**
(holistic cross-issue coherence) and its **Tier 2.5** (pre-close stale-narrative
sweep), lifted from cross-issue to cross-container.

It reviews containers that have already been dispatched and report completion.
It does **not** dispatch, break down, or drive execution of any container; that
is `references/container-dispatch.md`. This review is the separate cross-container
concern that `container-dispatch.md` step 8 explicitly defers to.

The review has three tiers (C1, C2, C3), applied in order. A **FAIL** at any tier
is an automatic overall FAIL; the container is not accepted (see Stop condition).
Do not silently continue past a tier that failed to manufacture a pass at a later
one.

## When it runs

Run this review on a candidate container `K` after `K` passes per-container
acceptance and before the wave advances over it:

- **After** `container-dispatch.md` step 8: `K` is `done`, its own gates are
  `passed`, and its `[hard]` success criteria are met. A container that has not
  cleared its own acceptance never reaches this review; send it back first.
- **Before** `container-dispatch.md` step 9 (advance): the wave does not cross the
  boundary over `K` until this review returns PASS.

A FAIL here is handled exactly as an unmet per-container criterion in step 8:
`K` is sent back to its dispatched lead for rework, or the steward escalates. The
wave does not advance over a FAIL.

## Inputs

- **`K`**: the candidate sub-strategic container reporting completion
  (`<candidate-short-id>`).
- **`P`**: the strategic-tier parent (the steward anchor), held from tier
  derivation.
- **membership label**: the parent's membership label, read as in
  `references/wave-layering.md` (the label whose namespace is
  `[type_hierarchy.label_associations][<parent-type>]`, carried by every direct
  child of `P`).
- **`D`**: the delegation-boundary types, held from tier derivation.
- **project conventions**: `AGENTS.md` at the repository root and the canonical
  content standards (`.jit/reference/content-standards.md`), the
  authority for naming, style, and document conventions across containers.

## Tier C1: Enumerate the accepted-sibling comparison set

Establish `A`, the set of already-accepted siblings `K` is compared against. This
is the concrete answer to "which containers": reuse `wave-layering.md`'s roster
method, then keep only the accepted, non-candidate members.

1. **Roster the sibling set `S`** exactly as `references/wave-layering.md` step 1:
   from `jit graph deps <P-short-id> --depth 0 --json`, collect the distinct
   `tree` nodes, keep a node iff its type is in `D` **and** it carries the
   parent's membership label (confirm both per node with
   `jit issue show <id> --json`; `tree` nodes carry no `labels` field).

2. **Filter to accepted siblings.** `A = { Ci in S : Ci.state == "done" and
   Ci != K }`. Cross-check the roster against the label query so the comparison
   set is exactly the accepted subset of the parent's children.

   ```bash
   jit issue list --label <membership-label> --state done
   ```

   Every id this returns whose type is in `D`, minus `K`, must equal `A`. A
   mismatch between the graph-deps roster and the label query means the roster or
   the labels drifted; stop and escalate (do not review against a wrong set).

3. **Baseline case.** If `A` is empty, `K` is the first accepted sibling under
   `P`. There is no peer to compare against, so Tier C2 records `K` as the
   convention baseline (PASS) and captures its produced interfaces for later
   siblings to be measured against. Tier C3 still runs (K's own artifacts may
   carry stale forward-references to siblings not yet accepted). Do not skip the
   review because `A` is empty.

**Verdict:** FAIL only if the two enumerations disagree (drifted roster/labels).
Otherwise proceed to C2 with the set `A`.

## Tier C2: Cross-container coherence (REQ-01)

Compare `K`'s naming, conventions, and produced interfaces against **every** `Ci`
in `A`. Gather each container's artifact set once, then run the checks.

**Gather `K`'s and each `Ci`'s artifact set**, the files a container produced:

```bash
# Documents linked to the container (design docs, plan docs, reports)
jit doc list <container-short-id>
```

Add any produced files the container's completion report records (scripts,
schemas, shared documents) that are not linked as jit docs. The artifact set is
the union of the linked docs and those recorded outputs.

Run these concrete checks, `K` against each `Ci` in `A`:

1. **Naming.** Do the two containers name the same concept the same way? For each
   shared concept, compare the term used across both artifact sets:

   ```bash
   # A concept K introduces; confirm siblings do not name the same thing differently
   git grep -nEi '<concept-term-A>|<concept-term-B>' -- <K-and-Ci-artifact-paths>
   ```

   FAIL if `K` calls a thing `X` and an accepted `Ci` calls the same thing `Y`
   (the `user_profile` vs `account_data` split in `lead-review-protocol.md` Tier
   3, one tier up). Also compare label namespaces and values, container and
   document titles, and command/flag names for the same drift.

2. **Conventions and style.** Does `K` follow the same conventions as the
   accepted siblings and the project authorities? Read `AGENTS.md` and the content
   standards, then confirm `K`'s artifacts match the register, structure, and
   invariant vocabulary the accepted siblings already established. FAIL on a
   convention `K` breaks that its siblings hold (heading structure, verdict/report
   shape, and the repository's configured label and assignee formats,
   terse-imperative doc register, no em-dashes). Every `@/…` address `K`'s
   artifacts cite must resolve via `jit item show`; a dangling citation is a FAIL.

3. **Produced interfaces.** For every interface `K` produces that a sibling
   consumes, or consumes from a sibling (an API such as a CLI command, script or
   function signature; a data structure such as a JSON shape or file format; or a
   shared document one container writes and another reads), confirm the two sides
   match:

   ```bash
   # The interface identifier as K defines it, seen across every sibling's artifacts
   git grep -nE '<interface-identifier>' -- <all-A-artifact-paths> <K-artifact-paths>
   ```

   FAIL if `K`'s produced shape (field names, section headings, argument order,
   file path) does not match what an accepted `Ci` was built to consume, or if
   `K` consumes a sibling interface under a shape that sibling does not emit.

Record a PASS/FAIL for the tier with **specific findings**: for each FAIL, cite
`file:line` in both `K`'s and the conflicting `Ci`'s artifacts, name both
containers, and state the concrete mismatch. A tier-level PASS requires that
every check above cleared against every `Ci` in `A`.

## Tier C3: Cross-container stale forward-reference sweep (REQ-02)

The one-tier-up analogue of `lead-review-protocol.md` Tier 2.5. When `K` lands,
forward-looking narrative in the **already-accepted** containers' artifacts that
pointed at `K`'s work ("planned", "future work", "once `K` lands") becomes false.
Reviewers surface these one per cycle unless swept proactively.

**Artifact scope:** the container-level artifacts of every accepted sibling in
`A` **and** of `K` itself: the design docs, plan docs, and completion reports in
each container's artifact set gathered in Tier C2. Interior issue-level artifacts
are the dispatched lead's Tier 2.5 concern, not this sweep's.

**Sweep**, at minimum:

```bash
# Forward-looking phrasing left in accepted containers' and K's artifacts
git grep -nEi \
  "planned|future work|once .+ lands?|will (land|be (added|implemented|available))|upcoming|is planned|not yet (implemented|available)|forthcoming|to be (added|implemented|done)|pending .+ (work|container)" \
  -- <all-A-artifact-paths> <K-artifact-paths>
```

Then filter the matches to those referencing `K`'s now-landed work: grep the same
artifact set for `K`'s short id and `K`'s key terms (its title terms, its
produced interface names, its container name) in forward-looking framing.

```bash
git grep -nEi "(planned|future|once|pending|upcoming).{0,60}(<K-short-id>|<K-key-term>)" \
  -- <all-A-artifact-paths> <K-artifact-paths>
```

Any match naming work `K` has now landed is a stale reference. FAIL the tier with
every match listed (`file:line` plus the stale phrase). The rework resolves every
match in a single submission, not one per cycle. A legitimately still-future
reference (to a container not yet accepted) is not stale; document each such
match and why it stands.

This tier is cheap (two greps) and removes the largest source of multi-cycle
rework at the container tier, exactly as Tier 2.5 does per issue.

## Stop condition (REQ-03)

A **FAIL** verdict blocks acceptance of `K` as complete. The wave does not
advance over `K` (`container-dispatch.md` step 9 does not run) until **every**
listed finding is resolved and the review is re-run to a clean PASS. The review
never downgrades a FAIL to a pass:

- Every numbered finding must be closed at HEAD and re-verified on re-review.
  Partial resolution keeps the verdict at FAIL.
- The steward may not soften a finding by argument. Wording such as "cosmetic
  naming difference", "the sibling can adapt", "the stale note is only
  documentation", or "close enough across containers" is argue-mode and is
  forbidden here exactly as in `lead-review-protocol.md`'s no-argue discipline.
  Either fix the artifact so the finding is literally resolved, or, if the
  finding is genuinely wrong, escalate; there is no soft middle.
- On FAIL, hand the full verdict back to `K`'s dispatched execution lead for
  rework (the container tier's analogue of dispatching issue rework) or escalate
  per `../../jit-execution-lead/references/escalation-policy.md`. Re-run this
  review from Tier C1 on the reworked container.

## Recording the verdict

Record a structured verdict, the one-tier-up analogue of
`lead-review-protocol.md`'s "Recording the Verdict":

```
## Cross-container coherence review: [CONTAINER_TITLE] ([K_SHORT_ID])

**Verdict:** PASS | FAIL

### Comparison set (Tier C1)
Accepted siblings under [P_SHORT_ID]: [C1, C2, …] | none (K is the baseline)
Roster/label cross-check: consistent | DRIFTED (details)

### Cross-container coherence (Tier C2)
- Naming: PASS | FAIL [finding: K file:line vs Ci file:line, the mismatch]
- Conventions/style: PASS | FAIL [finding]
- Produced interfaces: PASS | FAIL [finding: interface, both sides]

### Stale forward-reference sweep (Tier C3)
[Zero matches / list each: file:line, stale phrase, the container it names]

### Required changes (if FAIL)
1. [Specific change with file:line and the container that must change]
2. [Next change]
```

The verdict is passed to `K`'s dispatched lead on a FAIL and recorded in the
container-level progress file on a PASS, so a resumed session reads which
containers cleared cross-container review.

## Red flags

- Accepting `K` as complete while any Tier C2 or C3 finding is open. A FAIL is a
  hard block on advancing the wave; there is no non-blocking coherence finding.
- Downgrading a FAIL by argument ("cosmetic", "the sibling can adapt", "only
  documentation"). Fix the artifact or escalate; never soften the finding.
- Comparing `K` against the full sibling roster `S` instead of the accepted
  subset `A`. A not-yet-accepted sibling has no fixed interface to match against;
  compare only against `done` siblings.
- Reviewing at the default `jit graph deps` depth 1. Transitive reduction hides
  siblings; enumerate the roster at `--depth 0`, as `wave-layering.md` requires.
- Sweeping interior issue-level artifacts for stale references. That is the
  dispatched lead's per-issue Tier 2.5; this sweep is container-level artifacts
  only.
- Dispatching or driving `K` from this review. It reviews an already-completed
  container; execution belongs to `container-dispatch.md` and the dispatched lead.
- Skipping the review because `A` is empty. The first accepted container is the
  convention baseline and still gets the Tier C3 sweep.

## Stop and escalate

Stop and report to the invoker (the human when the steward runs standalone) when:

- The graph-deps roster and the `--state done --label <membership-label>` query
  disagree on the accepted-sibling set (Tier C1 drift): the roster or the labels
  are corrupt.
- `K`'s dispatched lead exhausts rework on a cross-container finding, or a finding
  requires a convention change that belongs to the vision rather than to `K`
  (escalate against the vision, do not soften the finding).
- A produced-interface mismatch cannot be resolved inside `K` because an
  already-accepted sibling would also have to change: a cross-container contract
  decision the steward owns, not a single container's rework.
