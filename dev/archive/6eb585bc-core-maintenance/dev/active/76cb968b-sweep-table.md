# Skill sweep table (76cb968b, phase 3 / REQ-01)

One row per repo-tracked skill file touched or inspected. Tier per the issue's
three-tier rule: **T1** projection (leave inline), **T2** gloss + address, **T3**
bare address / dispatch-resolve mechanism. Registry-owned content is invariant,
rule, and gate text; skill-owned workflow prose (naming a gate by key to say which
node carries it, lead-invariant sections) is the skill's own content and stays.

## Changed

| File | Tier(s) | Change |
|---|---|---|
| jit-project-lead/references/standards-fix.md | T2 | `INV-EVENT-LOG`, `INV-ATOMIC-WRITES` storage-boundary mentions → `@/inv/event-log`, `@/inv/atomic-writes`; the `temp-file + atomic-rename pattern` gloss kept, tagged `@/inv/atomic-writes`. |
| jit-project-lead/scripts/standards-fix.sh | T2 | Same two invariant citations in the header comment converted to `@/inv/event-log`, `@/inv/atomic-writes` (mirrors the reference doc). |
| jit-project-lead/references/wave-layering.md | T2 | "Invariant relied on" restatement of `INV-DAG-ACYCLIC` ("cycle detection runs before every dependency operation") reduced to the clause-length gloss "the graph stays acyclic" + `@/inv/dag-acyclic`; the removed detail is carried by the resolved address. |
| jit-project-lead/references/coherence-review.md | T2 + T3 | Tier C2 convention list: `INV-LABEL-FORMAT`, `INV-ASSIGNEE-FORMAT` → `@/inv/label-format`, `@/inv/assignee-format` (glosses "label format"/"assignee format" kept). Added the review resolve-check: cited `@/…` addresses in K's artifacts must resolve via `jit item show`; a dangling citation is a FAIL. |
| jit-execution-lead/references/lead-review-protocol.md | T3 | Added the review resolve-check to Tier 3 (Documentation narrative): every `@/<kind>/<self-id>` citation the issue's artifacts introduce must resolve via `jit item show`; a dangling citation is a FAIL. No registry text was restated in this file. |
| jit-parallel/references/agent-prompt-template.md | T3 | Added the one standing resolve instruction ("Addressable context" section) after the pasted issue description. No per-citation edits. |
| jit-execution-lead/references/architect-agent-prompt.md | T3 | Added the standing resolve instruction under Project Context. |
| jit-execution-lead/references/doc-agent-prompt.md | T3 | Added the standing resolve instruction under Project Context. |
| jit-execution-lead/references/explorer-agent-prompt.md | T3 | Added the standing resolve instruction under Project Context. |
| jit-execution-lead/references/rework-prompt-template.md | T3 | Added the standing resolve instruction after the pasted review verdict. |
| jit-breakdown/references/analysis-prompt.md | T2 + T3 | Coverage-preview restatement ("**coverage-preview** gate: every `[hard]` criterion below must be delivered…") → `@/gate/coverage-preview`. Info-preservation guard: `jit item show @/gate/coverage-preview` returns thinner mechanism text than the prose's operational detail, so the gloss is kept alongside the address (tier 2), not reduced to a bare address. Standing resolve instruction added before "Your task". |
| jit-manage/references/issue-extraction-prompt.md | T3 | Standing resolve instruction added before "Your task" (pasted plan/context may cite addresses). |
| jit-migrate/references/analysis-prompt.md | T3 | Standing resolve instruction added before "Your task". |
| jit-planning-lead/references/investigator-prompt.md | T3 | Standing resolve instruction added before "What to produce". |
| jit-planning-lead/references/researcher-prompt.md | T3 | Standing resolve instruction added before "What to produce". |
| jit-planning-lead/references/synthesizer-prompt.md | T3 | Standing resolve instruction added before "Structure". |
| jit-planning-lead/references/reviewer-prompt.md | T3 | Standing resolve instruction added; plus the review resolve-check in area 3 (Technical soundness): any `@/…` address the plan cites must resolve, a dangling citation is a blocking failure. This is the plan-review protocol. |

## Inspected, intentionally left

| File(s) | Reason |
|---|---|
| jit-breakdown/{SKILL.md, references/bracket-spine.md, references/plan-schema.md} | Gate keys (`coverage-preview`, `breakdown-review`, `plan-review`) appear only as workflow mechanics — which node carries which gate, run it, block on its status — not as restatements of the gate's definition text. Naming ≠ restating; left as prose. (analysis-prompt.md is a changed row above.) |
| jit-planning-lead/{SKILL.md, references/interview-protocol.md, references/plan-doc-template.md} | `plan-review`, `coverage-preview`, `breakdown-review` named as workflow mechanics; no invariant/rule/gate definition restated. These are not dispatch-prompt templates (the four prompt templates in this skill are changed rows above). |
| jit-execution-lead/SKILL.md, references/worktree-dispatch-protocol.md | Gate keys named as workflow mechanics (gate presets per node, re-run `cargo-ci` before integration). No definition restated; not dispatch-prompt templates. |
| jit-project-lead/references/{standards-scan.md, standards-sweep.md, container-dispatch.md, mode-routing.md, tier-derivation.md} | Enforce `docs/reference/jit-content-standards.md` STD-* rules, which are not `.jit/` registry items and have no `@/…` address; no invariant/rule/gate registry text restated; not dispatch-prompt templates. |
| jit-manage/references/{design-doc-template.md, state-commit-patterns.md}, jit-manage/SKILL.md, jit-migrate/{SKILL.md, references/plan-schema.md} | No invariant/rule/gate registry-text restatements; gate/rule mentions are workflow mechanics; not dispatch-prompt templates. (The dispatch-prompt templates jit-manage/references/issue-extraction-prompt.md and jit-migrate/references/analysis-prompt.md are changed rows above.) |
| jit-planning-lead/evals/results.md and all */evals/ transcripts | Historical eval records; left unchanged per the issue. |
