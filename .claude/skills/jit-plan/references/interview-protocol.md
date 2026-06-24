# Interview Protocol (Phase 2)

The planner's primary deliverable is **correctly extracted intent**, not a document.
A wrong intent extraction means the criteria are wrong, coverage passes the wrong work,
and implementation builds it well: well-executed garbage. Carry the burden of
convergence from a vague cold start; do not push a form at the user.

This runs in the **main loop** (it needs the human). Use `AskUserQuestion` for discrete
forks with real options; ask open elicitation conversationally, one question at a time.

## 1. Ingest before interrogating

Read every referenced artifact first — article, design doc, code, ticket, dataset
description — with generic tools (`Read`, `WebFetch`, repo search). Extract the candidate
**menu**: for an article, the figures/tables/method/datasets/reference-code; for a
feature request, the surfaces it touches. Now you know what is on offer and can ask
grounded questions instead of generic ones.

> Optional ingester hook: if the project provides a richer artifact ingester (e.g. a
> research-librarian skill for papers/DOIs) and it is available, you may delegate ingest
> to it. Default to reading the artifact yourself — jit-plan stays self-contained and
> domain-agnostic, and the hook is never required.

## 2. Interview to extract intent (one question at a time)

Drive toward a **testable definition of done**. Grounded prompts, not a questionnaire:

- Which outcomes count as success? (From the ingested menu — name them.)
- At what **fidelity**? (Exact within tolerance / qualitative trend / ranking / "it runs"
  — make "done" observable.)
- What is explicitly **out of scope**?
- What is the **stakes / depth**? (Scale effort — see leveled discovery below.)
- What must this integrate with or not break?

**Leveled discovery.** Detect signal (a new dependency, a "choose/evaluate", architectural
scope, an unfamiliar domain) and scale effort: small, well-understood work skips research
and stays shallow; large or novel work earns investigation (Phase 4) and possibly a
research artifact (Phase 4b). Do not over-plan small work.

## 3. Elicit owner-owned decisions; keep a living decision log

A plan hits genuine forks the planner must **not** resolve silently. Separate:

- **Defaults you may pick** — reversible, low-stakes, obvious-best. Pick, and note it.
- **Decisions the owner owns** — how far a breaking change goes, which alternative to
  drop, where a responsibility belongs, how to reconcile a choice that conflicts with the
  existing system. **Ask these.**

For each owner decision, persist the **answer and the rejected options with reasons**.
This is a first-class section of the plan (Phase 5), consumed by downstream review and
breakdown. Decisions are **provisional**: if later investigation or review invalidates a
premise, reopen it with the owner — never silently override, never stubbornly keep a
now-broken one.

## 4. Capture at the right altitude — after convergence

Do **not** create the container while the abstraction is still generalizing. If the shape
is visibly moving across turns (re-scoped, re-parented, "actually it's more like…"), keep
eliciting. Premature capture produces items that are immediately re-scoped. When it
settles, create at the right tier and **let the owner set it** — do not over-promote a
task to an epic or vice versa.

## 5. Converge to criteria — the definition-of-done floor

Confirmed outcomes become **atomic success criteria**: one observable outcome each, a
stable id (the ruleset's `id-pattern`, default `REQ-NN`), an explicit marker
(`[hard]` default, `[aspirational]` otherwise — never bare, never mixed), in a
`## Success Criteria` section.

**Floor (hard rule):** author no plan until at least **one owner-confirmed `[hard]`
criterion** exists. Where intent stays underspecified:

- Record an **explicit assumption** in the decision log, with its rationale, and confirm
  it — OR
- **Escalate** after a bounded number of rounds.

Never fabricate a definition of done to move forward. No criterion → nothing to cover,
nothing to review against.

## Headless variant

When dispatched with no human to ask: derive intent from the artifacts and existing
container, record **every** inference as an explicit assumption (do not ask, do not
fabricate-silently), and skip the interactive elicitation. If the artifacts cannot
support a `[hard]` criterion, stop and report — do not invent one.
