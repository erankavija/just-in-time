---
name: jit-planning-lead
description: >
  Turn a vague idea, imported document, or existing container into a reviewed
  concise design plus an authoritative, worker-sized JIT issue manifest. Planning
  and breakdown only; jit-execution-lead executes the resulting graph.
---

# JIT Planning Lead

Deliver two linked artifacts for each breakable container `C`, inside the
directory `jit doc dir <C> dev/active` resolves:

- `plan.md`: shared design, decisions, risks, and a generated overview.
- `breakdown.json`: the complete authoritative issue graph, directly consumable by `jit issue batch-create`.

Planning is complete only when `P` and `B` are done, both artifacts are linked,
every manifest issue/edge was created exactly, and every finest-tier issue is
worker-sized. Never implement the planned work.

## 1. Pre-flight and intake

1. Run `jit recover` and `jit validate`; read `.jit/config.toml`,
   `.jit/templates.toml`, `.jit/rules.toml`, gate presets, and
   `.jit/reference/content-standards.md`. Derive all types, labels, criterion
   syntax, gates, and paths from live configuration.
2. Select `research-and-plan`, `plan-from-existing`, or `plan-from-import`.
   Read inputs before interviewing. Resolve owner decisions one at a time and
   record chosen and rejected options. Autonomous callers may choose conservative
   defaults but must record them.
3. Create or reconcile `C`. Its description must have atomic, observable marked
   criteria such as `[hard] REQ-NN`. Verify input claims against the live system.
4. Run `jit apply plan <C>` once. Record planning node `P`, breakdown node `B`,
   their configured gates, and P's plan path. Commit JIT state.

Read [interview-protocol.md](references/interview-protocol.md) when eliciting intent.

## 2. Investigate

Dispatch the investigator with
[investigator-prompt.md](references/investigator-prompt.md). It writes and links
`investigation.md`, inside the directory `jit doc dir <C> dev/active` resolves, to
`P`. Consumer and file inventories live there; the plan only cites them. Dispatch
[researcher-prompt.md](references/researcher-prompt.md) only for external
dependencies, real option selection, or unfamiliar architectural work.

## 3. Author manifest first

Dispatch the synthesizer with
[synthesizer-prompt.md](references/synthesizer-prompt.md) and
[plan-doc-template.md](references/plan-doc-template.md). It writes the manifest
first, then the concise plan. The manifest contract is
[breakdown-manifest.schema.json](references/breakdown-manifest.schema.json); use
the deterministic helper rather than retyping its rules:

```bash
DIR="$(jit doc dir <C> dev/active)"
.agents/skills/jit-planning-lead/scripts/breakdown_manifest.py validate \
  "$DIR/breakdown.json" --config .jit/config.toml \
  --plan "$DIR/plan.md" --known-source <every-valid-source-id> ... \
  --required-source <mandatory-source-id> ... \
  --required-criterion <criterion-id> ... --deny-warnings
.agents/skills/jit-planning-lead/scripts/breakdown_manifest.py render \
  "$DIR/breakdown.json" "$DIR/plan.md" --write
.agents/skills/jit-planning-lead/scripts/breakdown_manifest.py render \
  "$DIR/breakdown.json" "$DIR/plan.md" --check
jit issue batch-create --from-json "$DIR/breakdown.json" --dry-run --json
```

Link both artifacts to `P`. The plan contains only outcome/criterion approach,
named shared contracts, the generated overview, material risks, owner decisions,
and investigation links. Never copy issue bodies, exhaustive inventories,
acceptance criteria, DAG prose, or review history into it.

### Terminal invariant

Iteratively split oversized finest-tier work into horizontal siblings until each
terminal issue has exactly one primary outcome, one bounded consumer family, one
observable test boundary, and work one agent can implement and review in one
focused cycle without another decomposition. A terminal must not combine
independently testable foundation, migration, deletion, documentation, or release
deliverables. It must disclose non-empty footprint `creates`/`touches`, or concrete
uncertainty. A shared `landing_group` is integration metadata, never permission to
merge work. No finer configured type is required to split an oversized task.

Mark every plan contract `plan-fixed` or `implementation-produced`. The latter
has exactly one `produces_contracts` owner transitively reachable from every
consumer; a plan-fixed contract has none.

Every non-finest manifest entry must depend transitively on a strictly finer
entry; relabeling executable work never suppresses terminal checks. Sizing
heuristics warn on broad quantifiers, independent verbs, multiple consumer
families, three or more acceptance clusters, mixed deliverable classes, and
implementation plus release. Split the task or add a reason under that warning's
stable `warning_overrides` code. Overrides remain visible and require reviewer
approval; `worker_sized_reason` is not an escape hatch.

## 4. Review and approve P

Dispatch [reviewer-prompt.md](references/reviewer-prompt.md). It checks both
artifacts, current code, readability, deterministic validation, and simulates one
assignment per leaf. Fix every finding by replacing the defective contract/task,
removing superseded prose, regenerating the plan, and rerunning all four commands.
Append-only correction sections fail.

Audit upstream dependencies moved from `C` to `P`. Keep planning-required edges.
For implementation-consumed edges, remove them from `P` and record the external
dependency plus its target manifest key in Decisions for breakdown to re-home.

Run the configured plan-review gate. Count recorded failures; after the third
failure stop and escalate. On pass, mark `P` done and commit.

## 5. Instantiate and approve B

Invoke `jit-breakdown` on `C`. Bracketed work must consume the linked manifest
directly; a Markdown-only plan is an actionable hard failure with no fallback or
backfill. Plain breakdown analysis produces the same manifest before creation.

After creation, require exact manifest-to-issue and manifest-to-edge fidelity,
correct bracket/external wiring, and passing coverage and breakdown-review gates.
Repeat the terminal assignment simulation. Correct the manifest first, reconcile
the graph, regenerate, and re-review; a shared-contract change also reruns
plan-review. Escalate after the third recorded breakdown-review failure.

## 6. Report

Report `C`, criterion count, both artifact paths, manifest issue/edge/terminal
counts, P/B gate rounds, re-homed dependencies, and any escalations. State that
the container is ready for `jit-execution-lead`.
