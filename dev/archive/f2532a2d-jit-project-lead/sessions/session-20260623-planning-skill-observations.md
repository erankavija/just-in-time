# Planning-skill observations from a live plan-before-fan-out run (2026-06-23)

Observations for the future `jit-plan` skill (the skill that owns the **P** node:
take a container, produce a reviewed plan, before the breakdown fans out).
Drawn from a full session that mined real usage into an epic, planned it, drove
the plan through its review gate, broke it down, and dogfooded coverage.

Each was validated interactively with the user. The framing is deliberately
**domain-agnostic** — jit (and this skill) apply to any work domain, not just
software, so nothing below should assume code/CLI specifics.

## Validated observations

1. **Ground the plan in the current state of the work, not the input artifact.**
   The planning input is a hypothesis, not truth. Before specifying any work,
   verify its claims against the actual current state and split them into
   *already-done* / *valid-and-open* / *invalid-as-stated*. The input here was
   substantially stale — several proposals were already satisfied, some were
   factually wrong about the system, one "problem" was intended behavior. A plan
   built on an unverified input fans out work that is already done or impossible.

2. **Elicit the real decisions; record a living decision log.**
   A plan is not mechanical — it hits genuine forks the planner must not resolve
   silently (how far a breaking change goes, which alternatives to drop, where a
   responsibility belongs, how to reconcile a choice that conflicts with the
   existing system). Distinguish defaults-you-can-pick from decisions-the-owner-
   owns; ask the latter; persist each answer **and the rejected options with
   reasons** as a first-class section of the plan. Downstream review and
   breakdown consume this rationale.

3. **Decisions are provisional — re-open them when new evidence contradicts a premise.**
   Verification or review will sometimes invalidate the premise of a decision the
   owner already made. Do not silently override it, and do not stubbornly keep a
   now-broken one: surface the conflict to the decision-owner, let them re-decide,
   and update the log. (In this run, two early decisions were reversed this way.)

4. **Express outcomes as discrete, addressable, verifiable criteria.**
   Write the plan's goals as atomic success criteria — one observable outcome
   each, a stable id, an importance marker (required vs optional) — not as prose.
   This makes the plan→breakdown handoff a checkable one-to-one contract: every
   required criterion is credited by exactly one downstream item, and that
   correspondence can be verified mechanically. (Where the criteria physically
   live is in flux in jit; the durable point is their *shape and addressability*.)

5. **Plan the structure to match the size: intermediate groupings, not a flat list.**
   Multi-area work needs an intermediate grouping level (by area / capability /
   wave) so the structure stays legible and each group is a coherent checkpoint.
   This is a *planning* judgment, not a mechanical breakdown step — the plan
   should already organize outcomes into natural clusters sized to the work, so
   the fan-out inherits a sensible depth. Keep depth proportional: small work
   stays shallow; large work earns the extra level.

6. **Bound each item's blast radius and state how ripple effects are handled.**
   Right-size each work item to one coherent change. For anything that perturbs
   something others depend on, enumerate the affected consumers and how they are
   **updated in the same change**. Keep genuinely separate contracts separate
   rather than lumping them. *Caveat:* prefer clean breaking changes over
   compatibility shims — "handle ripple effects" means sweep-and-update the
   dependents, not preserve the old behavior. Do not pad plans with
   backward-compat/migration scaffolding that isn't wanted.

7. **Plan and seeded items must be standalone-readable and must not restate
   system-tracked structure.** A reader with no other context should understand
   each item. Express relationships *through the system's structures* (the
   dependency graph), never by narrating dependencies, ordering, or ordinals in
   prose — that is redundant, divergence-prone, and makes items un-reusable out
   of context.

8. **Treat plan approval as an adversarial review loop, and address every concern.**
   The plan-quality review is a genuine adversary and will rarely pass first try;
   expect several rounds of real, evidence-backed findings. Two behaviors are
   non-negotiable: **(a) address ALL blocking findings, no cherry-picking** —
   including ones buried in secondary sections — revise, and re-run until a
   genuine pass; and **(b) confirm the review's actual recorded verdict**, never
   a proxy signal (a wrapper/notification reported success even when the gate had
   failed). "Approved" is inferred only from the recorded outcome.

## Dropped during validation

- **"Scaffold the bracket via the CLI scaffolding command, not by hand."**
  Superseded by the graph-template work — bracket scaffolding will be template
  instantiation, so the specific command is not a durable skill lesson.
- **"Negative-test that a quality gate actually enforces."** Dropped by the user
  for this doc's scope (the gate-mechanism specifics are jit-internal and in
  flux).

## Provenance

Observations distilled from the 2026-06-23 session that planned and broke down
the `agent-ergonomics` epic (`53e3fa36`) through its plan-review and
breakdown-review gates. Companion to
[session-20260620-planning-skill-design.md](session-20260620-planning-skill-design.md).
