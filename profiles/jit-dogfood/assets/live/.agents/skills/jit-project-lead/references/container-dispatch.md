# Sub-strategic container dispatch

Drive one wave of sub-strategic containers through two explicit, sequential
delegations per container: `jit-planning-lead` produces and gates the complete
breakdown, then a fresh `jit-execution-lead` executes that already-planned tree.
This is the one-tier-up analogue of issue-level dispatch and reuses the **same**
worktree-isolation and leak-detection scripts one tier up.

This reference consumes the ordered wave list from `references/wave-layering.md`
and runs the current wave only. It does **not** plan, break down, or execute a
container itself. The planning lead owns the plan and complete recursive
breakdown; after that lead stops, the execution lead owns implementation. One
agent never silently crosses both roles.

## Inputs

- **WAVES** — the ordered wave list from `references/wave-layering.md`. Each wave
  is a set of sub-strategic container ids; waves run in order, one at a time.
- **`current_wave`** — the index of the wave to dispatch now, read from the
  container-level progress file (the steward's analogue of
  `jit-execution-lead`'s `progress.json`).
- The steward's derived tiers (anchor, boundary) held from tier derivation, used
  only to confirm every id in the wave is a delegation-boundary container.

## Preconditions

- The steward is on `main` with a clean working tree. A dispatch that isolates
  concurrent leads (below) reuses `dispatch-worker-worktree.sh`, which
  refuses a dirty or non-`main` tree; commit or stash the steward's own state
  first. If it cannot be made clean, stop (see Stop and escalate).
- Every prior wave's container results are already on `main`. The next wave's
  worktrees anchor to `main` HEAD, so a dependent container that cannot see its
  predecessor's work would dispatch onto a stale base (the TRAP 1 failure in
  `../../jit-execution-lead/references/worktree-dispatch-protocol.md`). Landing and
  reconciling each completed container branch onto `main` — including merges of
  the append-only `.jit/` logs — is owned by the integration story; this
  reference requires that boundary reached before it dispatches, and does not
  define its mechanics. The cross-container coherence review that spans the
  wave's accepted containers is defined in `references/coherence-review.md` and
  runs at the step-8 → step-9 seam below.

## Procedure

Read `../../jit-execution-lead/references/worktree-dispatch-protocol.md` in full
before the first concurrent dispatch. The scripts named below live in
`../../jit-execution-lead/scripts/` and are project-agnostic; **invoke them in
place, never copy or fork them** — the canonical copy lives with the
execution-lead skill so improvements propagate. The steward runs from the
repository root, so it invokes them by their repo-relative path
(`.agents/skills/jit-execution-lead/scripts/<script>`), not a skill-local `scripts/`
path.

1. **Take the current wave.** `W = WAVES[current_wave]`, the set of container
   ids to dispatch now. If `W` is empty, the wave plan is corrupt; stop and
   escalate. Confirm each id is a delegation-boundary container carrying the
   parent's membership label (it already passed `wave-layering.md`'s roster
   filter; a mismatch here means the plan drifted — stop and escalate).

2. **Choose the isolation mode by wave width.**
   - **One container in `W`** — no concurrent writer exists. Dispatch the single
     lead directly (step 4); worktree isolation is optional.
   - **Two or more containers in `W`** — concurrent leads would collide on
     `main`'s checkout and on `.jit/` state. Isolate every container in the wave
     (step 3). This is mandatory, not a judgment call.

3. **Isolate concurrent leads (≥2 containers).** From `main`, run once for the
   whole wave:

   ```bash
   .agents/skills/jit-execution-lead/scripts/dispatch-worker-worktree.sh \
       <container-short-id-1> <container-short-id-2> ...
   ```

   passing every container short-id in `W`. The script snapshots `main`'s
   `git status -uall`, creates one worktree per container at
   `.agents/worktrees/agent-<short-id>` anchored to `main` HEAD with SHA
   verification, and emits a prompt-header block per container. Do **not** use
   Agent's `isolation: "worktree"` parameter — it has been observed to branch
   from a stale ancestor.

4. **Plan every unplanned container.** Inspect each container's live bracket.
   A container is planning-complete only when its planning and breakdown nodes
   are both `done`, their configured gates passed, and the recursive breakdown
   has no unplanned breakable frontier. Never infer this from the container's
   prose or from the mere presence of bracket nodes.

   For every incomplete container, dispatch a `general-purpose` subagent with
   the container's step-3 prompt header and instruct it to invoke
   `jit-planning-lead` in `plan-from-existing` mode. Its contract is planning and
   breakdown only: mandatory investigation, synthesis, adversarial review,
   plan gate, `jit-breakdown`, recursion, breakdown gates, then stop. Name the
   steward as invoker. Use no Agent `isolation` parameter after manual worktree
   creation. Dispatch the whole planning phase in background when the wave has
   two or more containers.

   Await every planning lead. Verify the bracket and recursive frontier again
   from jit state and record the container `planned`. A failed gate, incomplete
   frontier, or planning escalation blocks execution; return it to the planning
   lead or escalate. An already planning-complete container skips this phase,
   not its verification.

5. **Dispatch fresh execution leads.** For each verified `planned` container,
   dispatch a new `general-purpose` subagent in the same isolated worktree (or
   directly for a solo non-isolated container). Prefix the same prompt header,
   instruct it to invoke `jit-execution-lead`, and state that the container is
   already fully planned. The execution lead consumes the existing tree: it
   plans implementation waves, dispatches workers, reviews, enforces gates, and
   completes the container. It must not recreate or revise the planning bracket
   except through an explicit escalation back to the steward and a new planning
   pass. Name the steward as invoker and use no Agent isolation parameter.

6. **Await execution.** Wave discipline: every execution lead in `W` must
   finish (its container `done`, or the lead escalated/stopped) before the next
   wave starts. Do not dispatch `WAVES[current_wave + 1]` while any planning or
   execution lead in `W` is still running.

7. **Leak check (after any wave that used worktrees).** Once every lead in `W`
   has returned, run:

   ```bash
   .agents/skills/jit-execution-lead/scripts/check-leak-into-main.sh
   ```

   It compares `main`'s current `git status -uall` against the pre-dispatch
   snapshot. Any entry present now but not in the snapshot is a likely leak — a
   lead that wrote into `main` instead of its worktree. Recover per the script's
   printed options and re-run until clean **before** any commit on `main`. This
   is the only guard against leak-into-`main`; eyeballing `git status` is what
   missed it originally.

8. **Gather each container's own result (in scope).** For each container in `W`,
   read the returned lead's completion report and confirm the container's own
   acceptance against jit: `jit issue show <container-id> --json` shows it
   `done`, and `jit gate check-all <container-id>` (or the report's recorded
   gate results) shows its gates `passed` and its `[hard]` success criteria met.
   Record the per-container result in the progress file. A container that
   finished without its own gates passing or criteria met is not accepted — send
   it back to its lead (rework) or escalate; do not advance the wave over it.
   The coherence check spanning multiple containers does not run inline inside
   one container's acceptance; it runs across the wave's accepted containers
   between this step and step 9, per `references/coherence-review.md`.

9. **Advance.** Before advancing, run the cross-container coherence review over
   every container accepted in `W` this wave, per
   `references/coherence-review.md`; a FAIL blocks acceptance and the wave does
   not advance until every finding is resolved and the review re-runs to PASS.
   When every container in `W` is accepted and the coherence review passes, cross
   the wave boundary (predecessor results onto `main`, per Preconditions — owned
   by the integration story), set `current_wave += 1`, commit the progress
   file, and repeat from step 1 for the next wave. When no wave remains, the
   steward's sub-strategic delegation for this strategic container is complete.

## What this reference does not do (REQ-03 boundary)

The dispatched planning lead owns the plan and recursive breakdown. The later
execution lead owns implementation inside the planned container. This reference
never:

- investigates, authors, reviews, or gates the plan itself,
- breaks a container into stories or tasks,
- plans the execution lead's internal implementation waves,
- classifies, dispatches, reviews, or reworks implementation issues,
- pre-creates either lead's interior worker worktrees or runs their interior
  dispatch scripts.

It hands over the container id and isolated worktree to one role at a time, then
verifies the durable jit boundary before dispatching the next role. The scripts
it reuses are invoked unmodified one tier up.

## Stop and escalate

Stop and report to the invoker (the human when the steward runs standalone) when:

- The wave list is empty or a wave contains an id that is not a
  delegation-boundary container carrying the parent's membership label (the plan
  is corrupt or drifted).
- `main` cannot be brought to a clean state for `dispatch-worker-worktree.sh`.
- The dispatch script fails its pre-flight (not on `main`, dirty tree, branch or
  worktree-path collision, or a worktree HEAD that does not equal `main` HEAD).
- The leak check reports leaks that cannot be reconciled to a clean `main`.
- A dispatched lead escalates something the steward cannot resolve against the
  vision, or exhausts rework on its container.
- A prior wave's results are not on `main` and cannot be landed before the
  dependent wave (the integration boundary is unmet).

## Red flags

- Copying, forking, or editing `dispatch-worker-worktree.sh` or
  `check-leak-into-main.sh` instead of invoking the execution-lead's canonical
  copies in place. Verbatim reuse is the requirement.
- Re-implementing breakdown, wave planning, or per-issue execution at the
  steward tier. Hand planning to `jit-planning-lead` and implementation to
  `jit-execution-lead`; do not do either role's interior work.
- Dispatching `jit-execution-lead` while the planning or breakdown node is open,
  a planning gate is not passed, or a recursive breakable frontier remains.
- Letting the planning agent continue into execution. Even in the same worktree,
  execution receives a fresh agent and an explicit `jit-execution-lead` prompt.
- Dispatching ≥2 concurrent leads without worktree isolation, or adding Agent's
  `isolation: "worktree"` on top of a manually created worktree (double,
  stale-base worktree — TRAP 1).
- Skipping the post-wave leak check because the wave looked clean by eye. It is
  one shell line and the only guard against leak-into-`main` (TRAP 6).
- Starting the next wave while any lead in the current wave is still running.
  Wave discipline is strict; a dependent container dispatched early sees a stale
  base.
- Accepting a container whose own gates did not pass or whose `[hard]` criteria
  are unmet. Per-container acceptance is a hard gate on advancing the wave.
- Folding the cross-container coherence check into step 8 (one container's own
  acceptance). The review is real and required, but it runs across the wave's
  accepted containers between step 8 and step 9, per
  `references/coherence-review.md` — not inside a single container's gate check.
