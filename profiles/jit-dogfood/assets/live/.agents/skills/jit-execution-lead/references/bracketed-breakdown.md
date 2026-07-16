# Bracketed breakdown (plan → review → breakdown → coverage → implement)

A breakable container `C` (the epic) is bracketed by a planning node `P` and a
breakdown node `B`, with the implementation subgraph spliced as a spine
`C → impl → B → P` (precedence **P > B > impl > C**). Drive the bracket strictly
in order — each gate blocks the next step. Section numbers refer to the skill's
`SKILL.md`.

1. **Scaffold the bracket.** Run `jit apply plan <epic-id>`. It creates BOTH `P`
   (planning-role type, plan-quality gate) and `B` (breakdown-role type, coverage
   + breakdown-review gates), wires `B → P` and the anchor edge `C → B`, and moves
   `C`'s pre-existing upstream deps onto `P`. Node roles, types, gates, and `P`'s
   plan-doc location come from `.jit/templates.toml`. Commit JIT state.

2. **Produce the plan on `P`.** The plan is the spec for the decomposition. Use
   the epic's design doc (Section 2) as `P`'s plan-doc; otherwise dispatch an
   architect agent (Section 6, `design` classification) to author it at `P`'s
   configured plan-doc location. Make it concrete enough to decompose and to
   judge coverage of `C`'s `[hard]` success criteria.

3. **Pass `P`'s plan-quality gate — it BLOCKS breakdown.** Run the gate the
   template declares on the planning node (`plan-review` in the default
   rulesets). Do not begin breakdown until the recorded status is `passed`; on
   failure revise the plan and re-run. Never bypass the gate.

4. **Break down behind the approved plan.** Delegate to jit-breakdown
   (`.agents/skills/jit-breakdown/SKILL.md`). Its bracket path consumes the
   pre-created `B` (already typed, labeled `brackets:<C-short-id>`, gated, and
   depending on `P`), drafts the impl children in Backlog with their
   `satisfies:<criterion-id>` coverage labels, and splices the interior spine
   (sources → `B`, `C` → sinks; transitive reduction drops the scaffold's
   `C → B` edge). Apply gate inheritance + per-task quality gates to the drafted
   children (Section 3B bullets). Self-approve the decomposition; escalate only
   if it introduces stories or higher-level types
   (`references/escalation-policy.md`).

5. **Pass `B`'s coverage gate — it BLOCKS the implementation fan-out.** Run the
   coverage gate the template declares on the breakdown node (`coverage-preview`
   in the default rulesets) via the standard runner; it is deterministic
   (`jit validate --scope <C>`) and blocks (exit 4) while any `[hard]` criterion
   is uncovered. Do not dispatch implementation waves until the recorded status
   is `passed`; on failure add or relabel children to cover the gap and re-run.

6. Commit JIT state in batch, then run Section 4 (Wave Planning) over the
   **impl interior only** — `P` and `B` are bracket infrastructure, not
   implementation waves; exclude them from the wave plan.
