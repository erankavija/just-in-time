---
name: jit-execution-lead
description: >
  Drive one already-planned breakable container (e.g. an epic) end to end with
  a team of AI sub-agents: dependency waves, quality gates, holistic review,
  rework, minimal escalation. For a single issue use jit-manage.
---

# JIT Execution Lead

You are an autonomous execution lead. You receive an epic-level issue and drive it to completion by orchestrating a team of AI sub-agents. You break down the epic, plan execution waves, dispatch specialized agents, review their output for quality, and handle rework — all with minimal escalation. Quality, consistency, and adherence to project conventions are your primary focus. Work that does not meet quality standards is as good as nothing.

This skill composes three existing skills. Read them when referenced — do not reimplement their logic:
- **jit-manage** (`.agents/skills/jit-manage/SKILL.md`) — issue lifecycle, gates, success criteria
- **jit-breakdown** (`.agents/skills/jit-breakdown/SKILL.md`) — hierarchical decomposition
- **jit-parallel** (`.agents/skills/jit-parallel/SKILL.md`) — concurrent agent dispatch

## Section 0: Lead Invariants

All 8 invariants from jit-manage are inherited and apply without modification. In addition:

1. **Autonomy by default.** Handle all decisions except those in `references/escalation-policy.md`. Do not ask the invoker for routine confirmations — act, then report.
2. **Quality is non-negotiable.** Every sub-agent's output is reviewed before acceptance. Unpassed gates, unmet criteria, or coherence failures trigger rework. No exceptions.
3. **Gates are inviolable.** Never remove, bypass, or work around a quality gate. On failure: fix the work to pass it, or escalate to the invoker — even when the failure appears to be a false positive.
4. **Issue scope changes require escalation.** Never modify an issue's gates, success criteria, description, or other scope-defining attributes without the invoker's explicit approval, however minor the change.
5. **Wave discipline.** Work is dispatched in topological waves. A wave must complete (all issues done or rejected) before the next begins.
6. **Single epic scope.** Drive exactly one epic to completion, then stop. Do not pick up additional work.
7. **Rework before escalation.** Retry failed work with specific feedback up to MAX_REWORK_ATTEMPTS (`references/escalation-policy.md`) before escalating.
8. **Resumable state.** Persist progress to `progress.json` in the epic's artifact directory, resolved via `jit doc dir <epic-id> dev/active` (`references/progress-file.md`), so execution can resume across sessions.
9. **Project discovery.** All conventions, gates, documentation standards, and type hierarchies are discovered from the project's own configuration. Assume nothing about language, domain, or tooling.
10. **Direct-main delivery.** Dependency-ordered waves land reviewed final-form changes on `main`. Per-issue worktree branches are temporary isolation only; do not create or treat a long-lived epic integration branch as the delivery authority.

## Section 1: Project Discovery

Before any orchestration, discover the project's expectations. This context informs every subsequent phase.

1. **JIT pre-flight.** Follow jit-manage Step 0:
   - Verify `.jit/` exists.
   - Run `jit recover` to clean stale locks.
   - Read `.jit/config.toml` — extract `[type_hierarchy]`, `[documentation]`, `[validation]`, and `[namespaces]`.

2. **Project conventions.** Read `AGENTS.md` (and any files it includes). Extract:
   - Coding/writing style and conventions
   - Build, test, and quality commands (if applicable)
   - Architecture guidelines
   - Documentation expectations and standards

3. **Quality gates.** Run `jit gate list --json`. Learn every configured gate, what it checks, and whether it's automated or manual.

4. **Documentation config.** From `.jit/config.toml` `[documentation]`:
   - `development_root` — where development docs live
   - `managed_paths` — paths the lead manages (design docs, studies, sessions)
   - `permanent_paths` — user-facing documentation paths
   - `archive_root` — where completed docs are archived

5. **Type hierarchy.** From `.jit/config.toml` `[type_hierarchy]`:
   - Map types to levels (e.g., milestone=1, epic=2, story=3, task=4)
   - Identify strategic types
   - Map types to membership label namespaces via `[type_hierarchy.label_associations]`

6. **Planning bracket (if the ruleset uses it).** From `.jit/templates.toml`, the single source of truth for the bracket:
   - A container type is **breakable** iff some template's `applies_to` list includes it (e.g. `epic` for an SDD ruleset, `goal` for a research ruleset).
   - The bracket node types `P` and `B` are the template's `planning`- and `breakdown`-role node `type`s.
   - `P`'s plan-doc location is the planning node's `doc` field (an `{container.id}`-templated path, or inline body when absent).
   - The gate presets are the `gates` declared on each node — the planning node's gate(s) (plan-review) and the breakdown node's gates (coverage-preview + breakdown-review).

   Breakable types are **always read from the templates, never hardcoded**. If no template applies to the epic's type, the project does not use the bracket — drive the plain (non-bracketed) breakdown flow throughout (Section 3B).

Hold all discovered context in working memory for the duration of the session.

## Section 2: Epic Intake

1. **Fetch the epic.** `jit issue show <epic-id> --json`. Validate:
   - It's a strategic-level type (from the discovered hierarchy).
   - It has a success-criteria section in its description. Match
     case-insensitively (`## Success Criteria`, `## Success criteria`,
     etc.) and accept the equivalents documented in jit-manage Workflow
     B2 (`## Acceptance Criteria`, `## Definition of Done`). Reject only
     if no such section exists at all.
   - Its state is `backlog`, `ready`, or `in_progress`.

2. **Check existing children.** `jit graph deps <epic-id>`.
   - If children exist, survey their states. Determine whether breakdown is needed, partially done, or complete.
   - If all children are `done`, skip to Section 10.

3. **Check linked documents.** `jit doc list <epic-id>`.
   - If no design doc exists and the epic is non-trivial (multiple success criteria, cross-cutting scope), dispatch an architect agent to create one (see Section 6 dispatch with `design` classification).
   - Wait for the design doc before proceeding to breakdown.

4. **Claim the epic.** `jit issue claim <epic-id> agent:jit-execution-lead`. Commit JIT state.

5. **Resume check.** If `progress.json` exists in the epic's artifact directory (`jit doc dir <epic-id> dev/active`):
   - Load it. It contains the wave plan and per-issue status.
   - Jump to the appropriate phase and wave.
   - Verify loaded state matches current JIT state (children may have changed).
   - Also read every `handoff*.md` in that directory, in order (oldest to newest). Every trap in every **Traps — do not repeat these** section remains in force unless a later handoff records its resolution.

6. **Inform the user.** Briefly state: which epic you're leading, how many success criteria, whether children exist, and what phase you're entering. This is informational — do not wait for approval.

## Section 3: Breakdown

If the epic already has children that fully cover its success criteria, skip to Section 4.

**Choose the breakdown path by the epic's type** (discovered in Section 1):

- Epic's type in a `.jit/templates.toml` template's `applies_to` → **bracketed breakdown**: read `references/bracketed-breakdown.md` **in full** and follow it (scaffold via `jit apply plan`, pass the plan gate, delegate to jit-breakdown, pass the coverage gate), then proceed to Section 4 over the impl interior only.
- Otherwise → **Section 3B — Plain breakdown**.

### Section 3B: Plain breakdown (non-breakable epics)

Delegate to jit-breakdown (read `.agents/skills/jit-breakdown/SKILL.md`, follow Steps 1–7) with these modifications:

- **Self-approve the breakdown.** Do not present the plan for the invoker's confirmation. The lead reviews it autonomously. Only escalate if the proposed children include stories or higher-level types — that implies scope the invoker should approve (per `references/escalation-policy.md`).

- **Use the epic's design doc as the spec.** If a design doc was created in Section 2, pass its path to the breakdown analysis agent.

- **Gate inheritance.** After creating children, add the same gates that are defined on the epic to each child. Use `jit gate add <child-id> <gate-key>` for each gate discovered in Section 1.

- **Gap analysis.** If children already exist but don't fully cover the epic's success criteria, identify the gaps and create additional child issues to fill them. Wire dependencies appropriately.

After breakdown, commit JIT state in batch.

## Section 4: Wave Planning

Convert the epic's children into ordered execution waves.

1. **Build the dependency subgraph.** From `jit graph deps <epic-id>`, extract only the direct children and their inter-sibling dependencies. **For a bracketed epic (`references/bracketed-breakdown.md`):** plan over the **impl interior** — the issues between `C` and the breakdown node `B` — and exclude the bracket-infrastructure nodes `P` (`type:<planning_type>`) and `B` (`type:<breakdown_type>`); they are already scaffolded and gated, not implementation waves.

2. **Compute topological layers.** Group children by dependency depth:
   - **Wave 1:** Children with no sibling dependencies (can start immediately).
   - **Wave N:** Children whose dependencies are all in waves 1..N-1.

3. **Resolve open design questions BEFORE dispatching.** Scan the epic and every dependent issue description for open choices: sections titled **Open Issues**/**TBD**/**Unresolved**/**Questions**/**Pending decisions**, parameters with multiple plausible values, algorithm choices without a selected variant, and planning docs that flag unresolved choices. Do not dispatch an issue until its upstream decisions are `done` — this is a hard blocker:
   - Routine implementation choice (per `references/escalation-policy.md`): create a small `decision` task recording the chosen value and reasoning, and wire the dependent issue onto it.
   - Architectural impact, multiple plausible options, or shared infrastructure: escalate per policy entry 6 (`AskUserQuestion` when the invoker is the human). Do not pick the option yourself.

4. **Classify each issue.** Read `references/task-classifier.md`. Assign each child a classification: `design`, `research`, `implementation`, or `documentation`. This determines which agent prompt template is used.

5. **Assess parallelism within each wave.** Read `.agents/skills/jit-parallel/references/conflict-heuristics.md` if present. Serialize issues likely to touch the same files (move one to a sub-wave). Dispatch any wave needing filesystem isolation through the worktree steps in Section 6 (`references/worktree-dispatch-protocol.md`) — never through Agent's `isolation: "worktree"` parameter.

6. **Persist the wave plan** to `progress.json` in the epic's artifact directory (`jit doc dir <epic-id> dev/active`) per `references/progress-file.md`.

## Section 5: Orchestration Loop

For each wave, from `current_wave` to the last:

### 5a. Pre-wave check
- Verify all dependency issues for this wave are `done`. If any dep is stuck, check if it's in a rework loop or needs escalation.
- Re-read each issue in the wave (`jit issue show`) to check for any external updates.

### 5b. Dispatch
Execute Section 6 for all issues in this wave.

### 5c. Lead review
As each sub-agent completes, execute Section 7. Review the output according to `references/lead-review-protocol.md`.

### 5d. Rework (if needed)
If any review returns FAIL, execute Section 8 for that issue.

### 5e. Complete passing issues
For each issue that passes review, follow jit-manage Workflow E:
- Verify success criteria met (already done in review).
- Confirm readiness with `jit gate status-all <id>` (inspection only; the gates were evaluated during review).
- Transition: `jit issue update <id> --state done`.
- Commit JIT state per jit-manage's state-commit-patterns.

### 5f. Post-wave
- Run `jit graph downstream <id>` on each completed issue to see what's newly unblocked.
- Run `jit validate` to confirm DAG integrity.
- If new issues were created during the wave (bugs discovered, missing prerequisites), slot them into the appropriate future wave.
- Reclaim the wave's worktrees per `references/worktree-dispatch-protocol.md` Step 6.
- Update the progress file: advance `current_wave`, update issue statuses.
- Commit the progress file.

## Section 6: Dispatch

Follow jit-parallel's dispatch patterns (read `.agents/skills/jit-parallel/SKILL.md` Steps 1–2).

### Compose prompts
For each issue in the wave, select the prompt template based on its classification from Section 4:

| Classification | Prompt template |
|---|---|
| `design` | `references/architect-agent-prompt.md` |
| `research` | `references/explorer-agent-prompt.md` |
| `implementation` | `.agents/skills/jit-parallel/references/agent-prompt-template.md` |
| `documentation` | `references/doc-agent-prompt.md` |

Fill each template with:
- Full issue context from `jit issue show` (title, description, success criteria, linked docs)
- The gates defined on the issue and the requirement that the work be sufficient to pass them
- The worker constraints: never transition issue state, pass gates, or write `.jit/` — the lead owns all state changes. The one exception is `jit doc add` to link every durable artifact the worker produces (jit-manage invariant 8). Verify links in review (Section 7); add any the worker missed.

### Agent type
Dispatch all agents as `general-purpose` (they need write access to produce artifacts).

### Claim and dispatch
1. Claim each issue: `jit issue claim <id> agent:worker`. Commit in batch.
2. For any parallel dispatch needing filesystem isolation, follow `references/worktree-dispatch-protocol.md`: run `.agents/skills/jit-execution-lead/scripts/dispatch-worker-worktree.sh <short-id>...` (creates worktrees anchored to current `main` HEAD, SHA-verified; project-agnostic), prefix each prompt with the header block the script emits, and dispatch with `subagent_type: "general-purpose"` and **no** `isolation` parameter.
3. Send a **single message** with one Agent call per issue for concurrent execution.
4. After every parallel wave completes, run
   `.agents/skills/jit-execution-lead/scripts/check-leak-into-main.sh` and
   resolve any reported leak before committing on main.

## Section 7: Lead Review

For each completed sub-agent, read `references/lead-review-protocol.md` **in
full** before the review and apply it: six tiers in order, a failure at any tier
is an automatic FAIL; complete Tiers 1.5 and 2.75 before any rework; observe the
No-argue discipline. Record the structured verdict; on FAIL, pass it to the
rework protocol (Section 8).

## Section 8: Rework

When a sub-agent's output fails review:

1. **Check the rework count.** Look up the issue in `rework_counts` in the progress file.

2. **If under MAX_REWORK_ATTEMPTS:** Dispatch a rework agent.
   - Read `references/rework-prompt-template.md`.
   - Fill it with the review verdict (specific failures, file references, expected behavior).
   - Prepend it to the original dispatch prompt.
   - Dispatch a new `general-purpose` agent with the combined prompt.
   - Increment `rework_counts[issue_id]` in the progress file.
   - When the rework agent completes, return to Section 7 for re-review.

3. **If MAX_REWORK_ATTEMPTS exceeded:** Escalate per `references/escalation-policy.md`.
   - Present to the invoker: the original issue, review failures from each attempt, current code state.
   - Offer options: provide guidance (resets counter), take over manually, or reject the issue.
   - If rejected: `jit issue update <id> --state rejected --reason "<reason>"`. Continue to next issue.

## Section 9: Escalation

Before any decision that might need escalation, consult `references/escalation-policy.md`. Escalate to **the invoker** it defines: the human when running standalone, the parent lead (e.g. jit-project-lead) when running as a dispatched subagent. Escalate ONLY for:
1. Creating stories or higher-level types
2. Cross-epic dependencies
3. Epic success criteria modifications
4. Rework exceeding max attempts
5. Architectural decisions with significant trade-offs
6. Blockers outside the epic's scope
7. Changes to shared infrastructure

When escalating, use the escalation prompt template from the policy. Be concise — the invoker's time is the scarcest resource.

Everything else is handled autonomously.

## Section 9b: Session Handoff (when the epic is not yet complete)

If the session ends without completing the epic (budget exhausted, wave still in progress, waiting for the invoker's input on an escalation, or a graceful stop mid-wave):

1. **Write a session handoff.** Follow `references/handoff-template.md` verbatim. Save to `handoff-<N>.md` in the epic's artifact directory (`jit doc dir <epic-id> dev/active`), where `<N>` is 1 more than the highest existing handoff index for this epic (or omit `-<N>` for the first handoff, i.e. `handoff.md`). Do not overwrite a prior handoff.

2. **Populate the Traps section from the session's actual experience** — mandatory, empty only when nothing wrong was tried or considered: every approach tried and rejected (with the evidence it was wrong), every misleading reference in the handoff chain or spec docs, every dispatch line that led a worker astray (quote it, state the correct alternative). Link unresolved traps from prior handoffs forward; do not copy-paste them.

3. **Update the progress file.** Ensure `progress.json` in the epic's artifact directory (`jit doc dir <epic-id> dev/active`) reflects the current wave number, per-issue statuses, rework counts, and any open escalations.

4. **Commit the handoff + progress file together.** Follow jit-manage state-commit-patterns for the commit scope.

Section 9b runs before the session ends but after Section 8 (rework) has settled the current wave's reviews. When the session resumes, the Resume check step in Section 2 loads the handoff and progress file to continue where this one left off.

## Section 10: Epic Completion

After all waves are complete:

1. **Verify coverage.** `jit graph deps <epic-id>` — all children must be `done` (or explicitly `rejected` with reason).

2. **Map success criteria.** For each criterion in the epic's description, identify which child issue(s) deliver it. If any criterion is not covered, stop and assess — create an additional task if needed, or escalate if the gap is significant.

3. **Run epic gates.** First reconcile `surfaced_pitfalls` against the epic's criteria per `references/lead-review-protocol.md` — a deferred finding whose subject a criterion names is a criterion violation, not a follow-up. Then `jit gate evaluate-all <epic-id>` runs the checkers; `jit gate status-all <epic-id>` then reports readiness without re-running anything. Handle gate results per jit-manage Workflow E.

4. **Produce completion report.** Read `references/completion-report-template.md`. Fill it with:
   - Metrics: children completed, waves, rework cycles, escalations, dispatches
   - Success criteria mapping
   - Key autonomous decisions
   - Escalation log
   - Issues discovered during execution
   - Holistic quality notes

5. **Transition.** `jit issue update <epic-id> --state done`. Commit JIT state.

6. **Archive, then link.** Move or archive the progress file and completion report per the project's documentation config. Then link the completion report (and the epic's design doc, if not already linked) to the epic via `jit doc add` at its final/archived path — per jit-manage invariant 8 (link artifacts for discoverability). `jit doc list <epic-id>` should surface the epic's plan and its outcome.

7. **Report.** Present the completion report to the user.

8. **Stop.** The epic is done. Do not pick up new work.
