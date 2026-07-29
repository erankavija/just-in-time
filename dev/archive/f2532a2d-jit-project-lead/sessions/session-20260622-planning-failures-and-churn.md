# Planning failures and churn from a live plan-before-fan-out run (session 7ec82da2)

The failures/churn companion to
[session-20260623-planning-skill-observations.md](session-20260623-planning-skill-observations.md).
Where that doc captures what a good plan run *does*, this one captures where this
session went wrong, looped, or had to be redone, and the discipline `jit-plan`
(the skill that owns the **P** node, `eed6750c`) should encode to prevent each.

Each observation was validated interactively with the user. The framing is
**domain-agnostic**: jit and this skill apply to any work domain, so the lessons
below state behavior, with the concrete software episode kept inline only as
evidence.

## Headline churn metrics

- The core abstraction mutated **5+ times** during design before it stabilized:
  planning skill -> roles split -> addressable requirements -> project invariants
  -> generic addressable items -> graph-template macro layer.
- The live run planned epic `9ac9fdac` (graph-templates). **`plan-review` took 5
  rounds** (R1-R4 FAIL, R5 PASS). **`breakdown-review` took 9 rounds and never
  passed** (accepted manually). ~14 reviewer invocations total, ~30 min/round.
- An adversarial pre-review returned PASS, then the real gate FAILED with three
  deeper findings the pre-review missed.
- The bracket graph was **detached twice** by hand-wiring bugs and later found
  **internally inconsistent** (a false ready root, dangling stories).

## Grounding and verification

1. **Verify every factual claim in the container's success criteria against the
   live system before authoring, and sweep prior study/decision docs first.**
   Treat the container's own criteria as untrusted input. Here a `[hard]`
   criterion asserted "rewrite both real configs" when one target had never
   adopted the feature at all (it was unsatisfiable as written), and a study doc
   in the repo had *already recorded* that exact inaccuracy but was never
   consulted during grounding. Reconcile (edit the container) as an explicit
   pre-plan step. Reinforces observation 1 in the companion doc; the failure mode
   is that a plan built on an unverified premise locks an interview decision that
   verification then reverses.

2. **When a plan claims a property, verify the named primitives actually deliver
   it; do not paraphrase intent.** The plan claimed "atomic, validate-before-
   mutate," but the primitives it named each persisted their effect immediately,
   so there was no rollback and the claim was false. The reviewer caught it. If
   the plan says atomic / transactional / validated-first, the skill must confirm
   the cited operations support that, not assume the words make it so.

3. **For any "remove / rename / migrate X" work, run an exhaustive consumer sweep
   up front and list every consumer in the plan.** Partial grounding has a
   signature: a *different* missed file surfaces each review round. Here the full
   consumer set of a config type leaked out one-per-round across three rounds
   (the author named it "the classic each-round-different-subset failure") until
   an exhaustive sweep was finally run. Front-load the sweep; do not reason from
   the two obvious call sites.

## Convergence before capture

4. **The primary deliverable of planning is correctly extracted intent, not a
   plan document. Do not converge on plan-doc mechanics before intake is
   settled.** The assistant emitted a full 6-step workflow blueprint while the
   concept was still unsettled and was told "No, this is still brewing... The
   most important task of the planner is to extract the intent." The skill must
   carry the burden of convergence from a vague cold start (no pre-existing
   container), ingesting any referenced artifact before interrogating, rather
   than pushing a form at the user.

5. **Do not create the jit container while the abstraction is still
   generalizing; capture at the right altitude after convergence, and let the
   owner set the tier.** Three separate work items were created mid-
   generalization and immediately re-parented or re-scoped; one was self-
   described as "genuinely epic-shaped" yet filed as a task to match an earlier
   preference, then promoted a turn later. The assistant also over-promoted a new
   item to epic and was corrected ("No need to be an epic. Task is fine"). When a
   concept is visibly still moving across turns, hold off on writing the issue.

## Authoring standards (descriptions and criteria)

6. **Descriptions state purpose only: not how the tracker advances the node, not
   the gate command, not the reviewer's identity.** The checker is pluggable, so
   naming it hardcodes today's tool into durable text. The assistant spelled out
   the advance command and named the specific reviewer and was corrected: "The
   issue description does not need to tell how jit works... Even less to talk
   about codex reviewing... Reviewer could be anything." Every seeded description
   the skill writes states what the node delivers, nothing about the machinery.

7. **The task that owns a node runs its own gates to completion; never describe a
   gate as left PENDING for a separate runner.** The assistant invented a
   mythical "standard gate runner" and wrote that `plan-review` was "not run from
   here," and was told "This is rubbish. Of course you run the review from within
   the task." Reviewer independence comes from the checker, not from punting the
   gate. This misconception had propagated into descriptions, summaries, and
   skill files; the skill must bake in "the node's owner runs the node's gates."

8. **Every success criterion carries an explicit marker, `[hard]` (default) or
   `[aspirational]`; never bare, never mixed across siblings.** The assistant
   invented a "non-hard" category out of unmarked criteria, defaulted criteria to
   non-hard "with no basis," and wrote inconsistent formats across four sibling
   issues. There is no implicit tier. Emit criteria in the project's exact marker
   form, defaulting to `[hard]` when in doubt.

9. **Durable authoring conventions live in versioned skill/project docs, not per-
   session memory.** The assistant reflexively saved a correction as private
   cross-session memory and was overruled: "Memory is the wrong place for this
   knowledge. It should live in skills and project conventions." The skill should
   *consume* conventions from their canonical versioned home, not re-derive or
   privately memorize them. (See also the cross-session-memory note: jit's
   structured store is the durable memory, not personal memory files.)

10. **Per-node description templates are first-class; empty seeded nodes are a
    recurring defect.** A planning node was created with an empty description, and
    the asymmetry (one node seeded, its sibling hand-wired by skill prose)
    triggered a whole generalization to the graph-template layer. Each node a
    bracket seeds needs a purpose-stating template, supplied at creation.

## Decomposition invariants

11. **Enforce "every wave/group is independently landable (green); no broken
    intermediate state."** A wave scheduled the removal of something its own later
    waves still consumed, leaving the workspace non-compiling between waves; a
    later round introduced a different broken intermediate when a wave was split.
    Flag any "remove X" step ordered before the steps that migrate X's consumers.
    For "remove X from the codebase" criteria, generate a concrete repo-wide
    acceptance check (grep across the whole tree, including example/fixture dirs)
    as part of the plan, so stragglers surface before the gate finds them.

12. **Emit children that already satisfy the content standards so the breakdown
    review starts near-passing instead of discovering each rule serially.**
    `breakdown-review` ran nine rounds and never passed, partly because each round
    surfaced a *new class* of standards violation: missing `[hard]`/id markers
    (the author's own codified convention), wrong type tier (story-sized work
    filed as leaf tasks, needing an intermediate story layer), unsequenced tasks,
    criteria omitting required contract details. The skill must produce children
    that already carry markers, size-appropriate tiers (container -> story ->
    task), sequencing, and one outcome per item.

13. **"Self-contained" means implementable from the description alone; it is not
    a license to restate dependencies in prose.** Dependency and ordering
    information already lives in the graph, so narrating prerequisites in text is
    redundant and divergence-prone. The review oscillated here: one round added
    "Prerequisite:" sentences to make tasks self-contained, a later round flagged
    those same sentences as dependency duplication. The resolution the skill must
    encode: standalone-readability is about having enough to *do the work*, while
    relationships stay in the structure. Reinforces observation 7 in the companion
    doc.

## Gate-loop discipline

14. **`plan-review` is a legitimate convergent loop; run it as a bounded loop with
    an escalation checkpoint, and read the recorded verdict, never a proxy.** Two
    distinct failures here. First, an adversarial pre-review returned PASS and the
    real gate then FAILED with three deeper findings the pre-review never raised,
    so a pre-gate PASS is not predictive of the gate. Second, the gate wrapper
    reported exit code 0 while the recorded gate status was `failed` (the known
    exit-0-on-FAIL behavior); the author correctly trusted the recorded status.
    The loop genuinely converged in 5 rounds because findings *shrank* each round;
    escalation was promised at ~4 rounds and honored. Encode: bounded loop with an
    escalation checkpoint, distinguish convergence (shrinking findings) from
    pathology (a new class of finding every round), and key "passed" off the
    recorded status only. Reinforces observation 8 in the companion doc.

## Lifecycle handoff

15. **Make the plan -> breakdown handoff an explicit phase transition, and expect
    repo-wide validation coupling.** The assistant jumped from "plan-review passed"
    straight into breakdown "without closing P or claiming B." On `plan-review`
    PASS the skill should mark the planning node done (it blocks breakdown) and
    claim the breakdown node before any breakdown mutation. Separately, marking
    the node done was blocked by an *unrelated* bracket's broken plan-doc, because
    validation runs repo-wide on every transition; the skill must detect this
    coupling and surface it (with a minimal-stub option) rather than treat a
    transition rejection as its own bug.

## Skill structure

16. **Budget for the orchestrator's size from the start; extraction only shrinks
    the body when the moved content loads on a conditional path.** Pushing load-
    bearing sections to `references/` barely moved the token count because a
    pointer plus a re-summary replaced the inline text. The interview runs in the
    main loop; investigate / synthesize / review dispatch as sub-agents whose
    prompts live in `references/` and load only when that path runs. Keep `SKILL.md`
    a thin orchestrator (the `<500` line / ~5k token budget of REQ-04), and only
    extract genuinely conditional content.

## Dropped during validation

- **Bracket-wiring operational hygiene** (atomic apply over incremental edits, ID
  guards, no stderr suppression on mutating calls, never `dep rm` before the
  replacement edges exist, do not hand-splice against eager transitive reduction).
  Real pain in this run (the bracket detached twice and the graph went
  inconsistent), but superseded by the graph-templates epic `9ac9fdac`, which
  makes hand-wiring obsolete. Same reasoning that dropped the "scaffold via CLI
  command" observation in the companion doc. The durable residue is kept as
  observation 10 (description templates) and observation 11 (landable-wave /
  grep-acceptance checks).
- **Generic skill-authoring tips** (avoid hand-numbered internal cross-references
  that break on every reorder; progressive-disclosure caveats). Generic to
  authoring any skill, not planning-specific; only the orchestrator size budget
  (observation 16) was kept because it directly shaped this skill's structure.

## Provenance

Distilled from session `7ec82da2-54d8-4f15-9b66-adaaf9d7f3f8` (2026-06-19 to
06-22): the design churn that produced the `jit-plan` concept, the addressable-
items epic, and the graph-template layer, plus the live plan-before-fan-out run
that planned and broke down the graph-templates epic `9ac9fdac` through its
`plan-review` (5 rounds) and `breakdown-review` (9 rounds) gates. Companion to
[session-20260623-planning-skill-observations.md](session-20260623-planning-skill-observations.md);
both feed `jit-plan` (`eed6750c`).
