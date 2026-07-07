---
name: jit-planning-lead
description: >
  Turn a vague request, idea, article, or rough epic into a fully planned,
  broken-down jit work tree ready to execute — from any of three starting
  points: a vague idea needing research and scoping, an existing container
  with criteria needing a plan, or an external document needing import into
  a jit plan. Use when asked to "plan this feature with jit", "plan with
  jit", "scope and plan" an initiative, turn an article or rough epic into
  a ready-to-execute jit plan or work tree, or take a loose problem
  statement and break it into child jit issues. Also use when the user
  wants to go from a vague or half-formed idea to a complete,
  self-contained jit issue structure ready for implementation. Planning
  and breakdown only, not execution — use jit-execution-lead to run an
  already-planned epic, and jit-breakdown to decompose a single
  already-specified issue from its own spec.
---

# JIT Planning Lead

Lead planning from various initial states to a complete plan + breakdown.
The outcome of your work is the full and self-contained jit issue structure for
the work that is ready for implementation.

## Success criteria

JIT planning is done when:
- Container issue exists, its description aligns to jit content standards and it has verifiable success criteria that cover all of the planned work
- Plan and Breakdown issues created by the planning bracket are both done
- Each issue created in the breakdown is self-contained for the work that it carries
- Each issue follows the JIT issue content standards
- All the planning artifacts are linked to the corresponding JIT issues
- All additional context from (optional) research and **all required external knowledge** is referenced in the planning documents

## Execution Steps

### Step 1: Pre-flight

1. **Sync jit state.** Run `jit recover`, then `jit validate`. Resolve any reported
   corruption before planning on top of it.

2. **Read the planning vocabulary from the live config.** Every name this skill uses
   below is the **default ruleset's**; substitute whatever the live config declares.
   - `.jit/templates.toml` → the `plan` template: `applies_to` (which container types
     are **breakable**), the planning- and breakdown-role node `type`s, the planning
     node's `doc` (plan-doc location, a `{container.id}`-templated path), and the
     `gates` each node carries. The container anchor itself also carries a gate
     (`repo-validate` in the default ruleset).
   - `.jit/rules.toml` → the coverage rule: `criteria-section`, `marker`, `id-pattern`,
     `satisfies-namespace`. These fix the exact criterion shape the plan must emit
     (`[hard]`, `REQ-NN`, `satisfies:` by default).

3. **Pick the entry path.** Match the start state to one of three modes, and decide
   whether top-level intake is interactive (a requester to interview) or autonomous:
   - **[research-and-plan]** — only a vague idea; refine it into the `container` issue
     before planning behind it.
   - **[plan-from-existing]** — a container already exists with success criteria and
     enough content to seed a plan; fetch it (`jit issue show <id>`).
   - **[plan-from-import]** — a planning document exists from outside this jit repo;
     reconcile it into a container.

4. **Treat jit state as your durable ledger.** A full plan can span many levels and
   outlive a context compaction. jit's own state is the record: issue states, gate
   verdicts (`jit gate check`), and the `C → impl → B → P` spine show exactly which
   levels are planned, gated, and broken down. On resume or after compaction, **read
   jit state and trust it over recollection** — a done P whose `plan-review` passed is
   planned; a B whose `breakdown-review` passed is fanned out. Never re-dispatch a level
   the graph already shows complete. Commit jit state after each level so the ledger is
   durable in git.

### Step 2: Intent extraction

1. **Ingest before interrogating.** Read every referenced artifact (article, doc, code,
   ticket) with generic tools. Extract the candidate menu (results, components,
   constraints) so questions are grounded.
2. **Interview to extract intent**, one question at a time. Converge on a **testable
   definition of done**: which outcomes count, at what fidelity, what is out of scope.
3. **Elicit owner-owned decisions.** Distinguish defaults you may pick from forks the
   owner owns (how far a breaking change goes, which alternative to drop, where a
   responsibility belongs). Ask the latter; record each answer **and the rejected
   options with reasons** — this seeds the plan's decision log.
4. **Hold off on capturing the container while the abstraction is still moving.** If the
   shape changes across turns, keep eliciting; capture at the right altitude only after
   convergence, and let the owner set the tier.
5. **Converge to criteria.** Confirmed outcomes become atomic `[hard] REQ-NN` lines (one
   observable outcome each, marker starting the bullet — never checkbox-prefixed `- [ ]`,
   or coverage-preview reads zero criteria; see `references/interview-protocol.md`), in a
   `## Success Criteria` section. **Floor:** at least one confirmed `[hard]` criterion
   before advancing.

### Step 3: Container creation or reconciliation

1. **If the container does not exist, create it.** `jit issue create` with the breakable
   `type:` that best fits the work scope unless already specified. The issue description must follow the jit
   content standards [../../../docs/reference/jit-content-standards.md](../../../docs/reference/jit-content-standards.md). The `Success Criteria` section holds
   all the criteria for the work to be complete that were extracted previously.
2. **If the container already exists, reconcile it first**: verify each
   criterion against the live system, **sweep prior study/decision docs first** (a known
   inaccuracy is often already recorded), and fix stale, unsatisfiable, or
   already-satisfied criteria before planning behind them.
   **Ground in the current state of the work, not the input.** The request and a
   container's own criteria are untrusted input. Verify every factual claim against the
   live system and split it into already-done / valid-and-open / invalid-as-stated
   before specifying anything.
3. **Scaffold the bracket.** `jit apply plan <C>` instantiates the whole bracket in one
   operation: it creates **P** (planning node, `plan-review` gate, plan-doc location) and
   **B** (breakdown node, `coverage-preview` + `breakdown-review` gates), wires `B → P`
   and `C → B`, moves `C`'s upstream deps onto `P`, and puts the `repo-validate` gate on
   `C`. Do not hand-wire. Commit JIT state. Record P's plan-doc path (interpolated from
   the template) — the plan goes there.

### Step 4: Plan creation

Turn the container's success criteria into a plan an engineering team can execute
behind. The plan is built by three dispatched sub-agents — **investigate**,
**synthesize**, **review** — then driven through the `plan-review` gate. All three
are read-and-report roles; you stay the planning lead that integrates their output
and owns the gate.

**Dispatch hygiene** (every sub-agent this skill dispatches). Hand work over as
**files, not pasted prose**: give the agent the paths to read and have it return a
short status plus the path it wrote — a pasted artifact stays resident in your
context for the rest of the session. **Route output by whether the plan cites it.**
An artifact the plan cites as grounding — an investigation or research report the
`plan-review` gate resolves as an authoritative source — is **repo-resident under the
managed active-docs directory** (`dev/active/`, the `[documentation]` `managed_paths`
in `.jit/config.toml`) and **linked to P with `jit doc add`**, so the gate reaches it.
An intermediate artifact no reviewed document cites stays in the **session
scratchpad**. **Pick the model per role:** a cheap model for
mechanical reads (investigator), a capable one where judgment drives quality
(synthesizer, adversarial reviewer). State the model on every dispatch; an omitted
model inherits this session's, usually the most expensive.

#### Step 4b — Investigate

Dispatch a `general-purpose` sub-agent with
**[references/investigator-prompt.md](references/investigator-prompt.md)**, directing
it to write its findings report to a **repo-resident path derived from the container
id** under the managed active-docs directory —
`dev/active/{container.short_id}-investigation.md` (the `[documentation]`
`managed_paths` in `.jit/config.toml`). The plan cites this report as grounding, so it
is repo-resident from the start.
Investigation is **mandatory** — the `plan-review` area "technical soundness +
architectural fit" fails any ungrounded plan. The investigator:

- Classifies every input claim into **already-done / valid-and-open / invalid-as-stated**
  against the actual code.
- For any **remove / rename / migrate X** intent, runs an **exhaustive consumer sweep**
  up front (whole tree, including example/fixture dirs). The partial-grounding failure
  mode is a *different* missed file surfacing each review round.
- **Verifies named primitives deliver claimed properties** (if the plan will say "atomic"
  / "validated-first" / "transactional", confirm the cited operations actually support
  it; do not paraphrase intent into fact).

Returns cited findings (`file:line`) keyed to the criteria they bear on. **Link the
report to P before the `plan-review` gate runs:** `jit doc add <P>
<investigation-doc-path> --doc-type study`, so the gate resolves the source the plan
cites as grounding.

**Research (conditional).** Only when a signal fires — a **new external dependency**, a
**"choose/evaluate"** decision, or **architectural-scope** work — dispatch
**[references/researcher-prompt.md](references/researcher-prompt.md)**. It produces a
**cited research doc, linked to P, separate from the plan**, feeding the decisions.
Small, well-understood work **skips** this — keep effort proportional.

#### Step 4c — Synthesize

Dispatch a `general-purpose` sub-agent with
**[references/synthesizer-prompt.md](references/synthesizer-prompt.md)** and
**[references/plan-doc-template.md](references/plan-doc-template.md)**. It writes the
plan at P's plan-doc location, structured to the **four `plan-review` areas** (which are
both the plan's sections and the self-check):

1. **Completeness vs criteria** — every `[hard]` criterion addressed; no silent scope
   narrowing.
2. **Technical soundness + architectural fit** — grounded in Step 4b findings, citing real
   files; reuses the right primitives; no stale assumptions.
3. **Decomposition + dependencies** — a **near-ready sketch**: intermediate groupings
   sized to the work (not a flat list), each group **independently landable** (no broken
   intermediate state), each item right-sized to one coherent change with **blast radius
   bounded** and ripple-handling stated, carrying its `type` tier, `[hard]` markers,
   `satisfies:REQ-NN` mapping, and dependency ordering. Items are **standalone-readable**
   but express relationships **through the graph, not prose**. This sketch is what
   `jit-breakdown` consumes in Step 5.
4. **Risks + actionability** — every open question carries a mitigation or a decision; an
   engineer can execute each item without re-deriving the design.

Plus a first-class **Decisions** section (each decision, the chosen option, the rejected
ones with reasons). Decisions are **provisional** — if Step 4b or 4d invalidates a premise,
surface it (escalate at the top level, flag below) and re-decide.

#### Step 4d — Review

Before running the gate, dispatch a `general-purpose` reviewer with
**[references/reviewer-prompt.md](references/reviewer-prompt.md)**. It tries to
**fail** the plan against the four areas in the reviewer prompt. **Address
every finding** before proceeding to planning issue closure through the jit
plan review gate.

#### Step 4e — Completion

Ensure that the plan document is linked to the plan issue P with `jit doc add
<P> <plan-doc-path> --doc-type design` and that all the findings uncovered in
the review have been addressed. Proceed then through the quality gates by
invoking them using `jit gate pass`. Read the plan review gate verdict with
`jit gate check <P> plan-review`. Address all the plan review findings and
escalate if you fail the review gate three times. Rerun gates only on failure.
When all the quality gates pass, mark the planning issue done. Commit jit
state.

### Step 5: Breakdown and recursion

Fan this level out with `jit-breakdown`, then recurse into any breakable child until
the frontier is empty — every leaf a non-breakable, right-sized task. High- vs
low-level requirements are the recursion axis, not a role split: the same subroutine
plans an epic and plans a story; only the altitude changes. Stop at full breakdown and
hand the tree to execution; do not start implementing the work.

1. **Fan out this level.** The plan is approved and P is done, so B is released.
   **Invoke the `jit-breakdown` skill** on `C` (read its `SKILL.md`; do not reimplement
   its logic). It splices the spine `C → impl → B → P` and creates the impl children
   from the plan's §3 sketch, each carrying its `type`, `[hard]` criteria, and
   `satisfies:REQ-NN` labels.

2. **Drive B's gates to a recorded pass**, the way you drove P's:
   `coverage-preview` (jit-breakdown runs it inline) and `breakdown-review`
   (`jit gate pass <B> breakdown-review`, looping on FAIL as in Step 4e). A gated
   fan-out is not fully broken down. Commit jit state.

3. **Cross-sibling coherence.** When this level produced **two or more** children that
   share a contract (one consumes another's output, or several touch the same surface),
   dispatch a read-and-report reviewer over the sibling set before going deeper:
   interface mismatch, boundary overlap, duplicated responsibility, and dependency
   direction. `breakdown-review` judges *this* container's decomposition; this pass
   catches what only shows up **across** siblings, the cheapest place to catch it.
   Fold any finding back into the affected child (or the §3 sketch) and re-run the gate.
   Skip for a single child or trivially independent leaves.

4. **Collect the new breakable children.** Of the children just created, any whose
   `type` is a breakable container type (it appears in some template's `applies_to`,
   Step 1) goes onto the frontier. Non-breakable, right-sized leaves are done — they
   are the fan-out-ready work.

5. **Recurse.** For each breakable child on the frontier, re-enter Step 2 (its intent
   derived autonomously, per the Step 2 autonomous preamble) through Step 5. Push any
   new breakable grandchildren onto the frontier. Continue until the frontier is empty.

If a level stops converging (repeated gate pathology, an unresolvable owner fork, or
intent that the artifacts cannot support), **escalate that subtree** to the requester
rather than forcing it, and continue the others.

Commit final jit state.

### Summary report

End with: the top container (`C`) and its `[hard]` criteria count; the number of levels
planned and breakable containers processed; per level, its `P` (`plan-review`: passed in
N rounds) and `B` (`breakdown-review`: passed); the count of fan-out-ready leaves; any
escalated subtrees or open assumptions; and the explicit next step ("`<C>` is fully
broken down — hand to jit-execution-lead to execute").
