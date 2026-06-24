---
name: jit-plan
description: >
  Author a review-passing PLAN for a breakable container before its work fans
  out — the P (planning) node of the plan-before-fan-out bracket. Interview-first
  and cold-start capable: from a vague request or a referenced artifact (no
  goal/epic yet), it ingests the artifact, interviews to extract intent, and
  converges on testable [hard] success criteria before writing anything. Then it
  grounds the design in the real code, synthesizes a plan structured to the
  plan-review rubric, self-reviews adversarially, and runs plan-review to a
  recorded pass — leaving a near-ready decomposition sketch for jit-breakdown to
  consume (it does NOT create child issues). Use when asked to "plan", "write/author
  a plan", "scope this", "plan this epic/goal before breakdown", "turn this
  idea/article into a plan", or to get a plan past plan-review. To decompose an
  already-approved plan into child issues use jit-breakdown; for single-issue
  lifecycle use jit-manage.
compatibility: Requires JIT CLI on PATH. JIT MCP tools used where available.
---

# JIT Plan

You own the **P (planning) node** of the plan-before-fan-out bracket: take a vague
request — or a bare referenced artifact, with no goal/epic yet — and turn it into a
plan that passes `plan-review` and is ready for `jit-breakdown` to decompose. The
primary deliverable of planning is **correctly extracted intent**, not a document;
the plan is what that intent produces once it is settled.

You author the plan and **stop at a decomposition sketch**. You do not create child
issues — that is `jit-breakdown`'s job. Planning recurses **one hierarchy level per
pass**: plan a container, hand off to breakdown, then re-enter on each next-level
container breakdown produced.

**Two entry paths, one skill:**
- **Interactive (default).** A human drives. Run the interview in the main loop to
  extract intent (Phase 2). This is the load-bearing path.
- **Headless (thin).** Dispatched with an existing container + artifacts and no human
  to ask. Derive intent from the artifacts, record every inference as an explicit
  assumption (never fabricate), and skip the interactive elicitation. Composition
  into an autonomous lead is a separate follow-up — keep this path minimal.

## Invariants

1. **Read the bracket vocabulary; never hardcode it.** Breakable container types, the
   P/B node types, P's plan-doc location, and the gate names all come from
   `.jit/templates.toml` (the `plan` template) and the coverage rule in
   `.jit/rules.toml`. The names used below (`epic`, `planning`/`breakdown`,
   `plan-review`, `coverage-preview`, `breakdown-review`, `satisfies`, `REQ-NN`) are
   the **default ruleset's** — substitute whatever the live config declares.
2. **Intent before mechanics.** Do not push a plan template, workflow, or container at
   the user before intake has settled. Ingest the referenced artifact *before*
   interrogating.
3. **Never fabricate a definition of done.** Author no plan until at least one
   **owner-confirmed `[hard]` criterion** exists. Where intent stays underspecified,
   record an explicit assumption with rationale or escalate — do not invent it.
4. **Ground in the current state of the work, not the input.** The request and the
   container's own criteria are untrusted input. Verify every factual claim against the
   live system and split it into already-done / valid-and-open / invalid-as-stated
   before specifying anything.
5. **Descriptions state purpose only.** What a node delivers — never how the tracker
   advances it, never the gate command, never the reviewer's identity. The checker is
   pluggable; naming today's tool hardcodes it into durable text.
6. **Every criterion carries an explicit marker** — `[hard]` (default) or
   `[aspirational]`. Never bare, never mixed across siblings.
7. **The node's owner runs the node's gates** to a recorded verdict. You run
   `plan-review` from here and loop until it records `passed`. Reviewer independence
   comes from the checker being a separate agent, not from punting the run.
8. **Conventions come from versioned docs, not memory.** Authoring standards are read
   from the single canonical `jit-manage/references/content-standards.md` (the SSOT) —
   do not duplicate or privately memorize them.
9. **Stop at the sketch.** Emit a near-ready decomposition sketch; let `jit-breakdown`
   create the children. One level at a time.

---

## Phase 1 — Pre-flight and config grounding

1. Verify `.jit/` exists. Run `jit recover` to clear stale locks. `jit validate` and
   note any pre-existing errors (Phase 7 step 4 explains why they matter here).
2. **Read the bracket vocabulary from the live config** (Invariant 1):
   - `.jit/templates.toml` → the `plan` template: `applies_to` (which container types
     are **breakable**), the planning- and breakdown-role node `type`s, the planning
     node's `doc` (plan-doc location, an `{container.id}`-templated path), and the
     `gates` each node carries.
   - `.jit/rules.toml` → the coverage rule: `criteria-section`, `marker`, `id-pattern`,
     `satisfies-namespace`. These define the exact criterion shape the plan must emit.
3. **Pick the entry path** (interactive vs headless, see above) and whether a container
   already exists. If the user names an existing container, fetch it
   (`jit issue show <id>`); otherwise expect a cold start (no container yet).

If no template applies to any container type, the repo does not bracket — there is no
P node to own. Say so and stop.

---

## Phase 2 — Intake and interview (main loop)

This phase runs in the main loop because it needs the human. Follow
**[references/interview-protocol.md](references/interview-protocol.md)** in full. In brief:

1. **Ingest before interrogating.** Read every referenced artifact (article, doc, code,
   ticket) with generic tools. Extract the candidate menu (results, components,
   constraints) so questions are grounded. *(A project may wire a richer ingester at the
   hook noted in the protocol; default to reading it yourself.)*
2. **Interview to extract intent**, one question at a time, grounded in what you read.
   Converge on a **testable definition of done**: which outcomes count, at what fidelity,
   what is explicitly out of scope.
3. **Elicit owner-owned decisions.** Distinguish defaults you may pick from forks the
   owner owns (how far a breaking change goes, which alternative to drop, where a
   responsibility belongs). Ask the latter. Record each answer **and the rejected options
   with reasons** — this becomes the plan's decision log.
4. **Hold off on writing the container while the abstraction is still moving.** If the
   shape is visibly changing across turns, keep eliciting; capture at the right altitude
   only after convergence, and let the owner set the tier.
5. **Converge to criteria.** Confirmed outcomes become atomic `[hard] REQ-NN` lines (one
   observable outcome each, stable id, explicit marker), in a `## Success Criteria`
   section. **Floor:** at least one owner-confirmed `[hard]` criterion before Phase 3
   (Invariant 3). After a bounded number of rounds without convergence, escalate.

Output of this phase: confirmed intent, the `[hard] REQ-NN` criteria, and the seed
decision log.

---

## Phase 3 — Capture the container and scaffold the bracket

1. **If the container does not exist, create it.** `jit issue create` with the
   breakable `type:` (from `applies_to`, Phase 1), the `## Success Criteria` section of
   `[hard] REQ-NN` lines, and a description that states **purpose only** (Invariant 5).
   Follow the canonical `jit-manage/references/content-standards.md` for title and
   description form (Invariant 8). The owner sets the tier — do not over-promote.
2. **If the container already exists, reconcile it first.** This is an explicit
   pre-plan step (Invariant 4): verify each existing criterion against the live system,
   **sweep prior study/decision docs in the repo first** (a known inaccuracy is often
   already recorded), and edit the container to fix stale, unsatisfiable, or
   already-satisfied criteria before planning behind them.
3. **Scaffold the bracket.** `jit apply plan <C>` instantiates the whole bracket in one
   operation: it creates **P** (planning node, `plan-review` gate, plan-doc location) and
   **B** (breakdown node, `coverage-preview` + `breakdown-review` gates), wires `B → P`
   and `C → B`, and moves `C`'s upstream deps onto `P`. Do not hand-wire. Commit JIT
   state. Record P's plan-doc path (interpolated from the template, also stated in P's
   description) — the plan goes there.

---

## Phase 4 — Investigate (dispatched)

Dispatch a `general-purpose` sub-agent with
**[references/investigator-prompt.md](references/investigator-prompt.md)**. Investigation
is **mandatory** — the `plan-review` area "technical soundness + architectural fit"
fails any ungrounded plan. The investigator:

- Classifies every input claim into **already-done / valid-and-open / invalid-as-stated**
  against the actual code.
- For any **remove / rename / migrate X** intent, runs an **exhaustive consumer sweep**
  up front (whole tree, including example/fixture dirs) and lists every consumer — the
  partial-grounding failure mode is a *different* missed file surfacing each review round.
- **Verifies named primitives deliver claimed properties** (if the plan will say "atomic"
  / "validated-first" / "transactional", confirm the cited operations actually support it;
  do not paraphrase intent into fact).

Returns cited findings (file:line) keyed to the criteria they bear on.

### Phase 4b — Research (conditional, dispatched)

Only when signals fire — a **new external dependency**, a **"choose/evaluate"** decision,
or **architectural-scope** work. Dispatch
**[references/researcher-prompt.md](references/researcher-prompt.md)**; it produces a
**cited research doc, linked to P, separate from the plan** (provenance-tagged), feeding
the decisions in Phase 2/5. Small, well-understood work **skips** this entirely — keep
effort proportional.

---

## Phase 5 — Synthesize the plan (dispatched)

Dispatch a `general-purpose` sub-agent with
**[references/synthesizer-prompt.md](references/synthesizer-prompt.md)** and
**[references/plan-doc-template.md](references/plan-doc-template.md)**. It writes the plan
at P's plan-doc location, structured to the **four `plan-review` areas** (which are both
the plan's sections and the self-check):

1. **Completeness vs criteria** — every `[hard]` criterion addressed; no silent scope
   narrowing.
2. **Technical soundness + architectural fit** — grounded in Phase 4 findings, citing
   real files; reuses the right primitives; no stale assumptions.
3. **Decomposition + dependencies** — a **near-ready sketch** (decision 4): intermediate
   groupings sized to the work (not a flat list), each group **independently landable**
   (no broken intermediate state — flag any "remove X" ordered before its consumers
   migrate), each item right-sized to one coherent change with **blast radius bounded**
   and ripple-handling stated, carrying its `type` tier, `[hard]` markers,
   `satisfies:REQ-NN` mapping, and dependency ordering. Items are **standalone-readable**
   but express relationships **through the graph, not prose** (no "Prerequisite:"
   narration, no ordinals).
4. **Risks + actionability** — every open question carries a mitigation or a decision; an
   engineer can execute each item without re-deriving the design.

Plus a first-class **Decisions** section (the Phase 2 log: each decision, the chosen
option, and the rejected ones with reasons). Decisions are **provisional** — if Phase 4
or 6 invalidates a premise, surface it to the owner and re-decide; do not silently
override or stubbornly keep a broken one.

---

## Phase 6 — Adversarial self-review (dispatched)

Before running the real gate, dispatch a `general-purpose` reviewer with
**[references/reviewer-prompt.md](references/reviewer-prompt.md)**. It reads the **actual
code** and the plan and tries to **fail** it against the four areas, returning blocking
findings. **Address every finding** before Phase 7. A pre-gate pass is *not* predictive
of the gate (a prior run saw pre-review PASS then the gate FAIL with deeper findings), so
this reduces but never replaces gate rounds.

---

## Phase 7 — Plan-review gate loop (main loop, owner-run)

1. **Link the plan to P:** `jit doc add <P> <plan-doc-path> --doc-type design`. The gate
   fails if the plan is missing or unlinked.
2. **Run the gate yourself** (Invariant 7): `jit gate pass <P> plan-review`.
3. **Read the recorded verdict** — `jit gate check <P> plan-review` (`.status`). The exit
   code is now meaningful (0 = pass, 4 = FAIL, 10 = runner error), but trust the recorded
   status over any outer wrapper/notification signal. On FAIL, read the findings from the
   recorded run.
4. **Iterate as a bounded convergent loop with an escalation checkpoint.** Expect several
   rounds — the reviewer is a genuine adversary. **Address ALL blocking findings, no
   cherry-picking** (including ones in secondary sections), revise, re-run. Distinguish
   **convergence** (findings shrink each round) from **pathology** (a *new class* of
   finding every round); at ~3–4 rounds without genuine convergence, **escalate** rather
   than grind. "Passed" is only the recorded status.

> Repo-wide validation coupling: `plan-review` and the later P-transition validate the
> whole repo. If an *unrelated* bracket's broken plan-doc blocks you, surface it (offer a
> minimal stub) — it is not this plan's bug.

---

## Phase 8 — Handoff to breakdown, then stop

On a recorded `plan-review` **PASS**, make the plan→breakdown handoff an **explicit phase
transition**:

1. **Mark P done** — `jit issue update <P> --state done`. P blocks B, so this releases
   breakdown. (If the transition is rejected by unrelated repo-wide validation, handle it
   per the Phase 7 coupling note, not as your own defect.) Commit JIT state.
2. **Stop.** The near-ready decomposition sketch lives in the plan doc for `jit-breakdown`
   to consume (it will splice the spine `C → impl → B → P` and create the children). **Do
   not create children or run breakdown here** (Invariant 9). Tell the user the plan is
   approved and breakdown is unblocked.
3. **Recurse one level at a time.** After `jit-breakdown` fans out and produces a
   next-level breakable container, re-enter this skill on that container. High- vs
   low-level requirements are the recursion axis — not a role split.

---

## Summary report

End with: the container (`C`) and its `[hard]` criteria count; `P` (`plan-review`:
passed, in N rounds); the plan-doc path; the number of sketch items and which criteria
each covers; open assumptions/escalations; and the explicit next step ("breakdown of `C`
is unblocked — run jit-breakdown").
