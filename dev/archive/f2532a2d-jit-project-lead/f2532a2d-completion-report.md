# Epic Complete: jit-project-lead skill — milestone-level vision steward (f2532a2d)

**Started:** 2026-06-23
**Completed:** 2026-07-03
**Assignee:** agent:jit-execution-lead

## Summary

Delivered the `jit-project-lead` skill: an autonomous strategic-tier steward one level above `jit-execution-lead`. It owns the project vision, derives its tiers from config, routes an opening request into one of four modes, dispatches a subordinate execution lead per sub-strategic container in topological waves with cross-container coherence review, resolves subordinate escalations against a durable vision/charter, and runs a project-wide content-standards sweep. The epic also promoted the content standards to a canonical doc and renamed the two prior leads (`project-lead` → `jit-execution-lead`, `jit-plan` → `jit-planning-lead`).

## Metrics

| Metric | Value |
|---|---|
| Children completed | 24 / 24 (0 rejected) |
| Waves executed | 6 (over 5 sessions) |
| Rework cycles | ~22 across all issues (concentrated in the eval-verification and skill-prose issues) |
| Escalations | 3 (all resolved by the invoker) |
| Sub-agent dispatches | ~30+ including reworks and eval runs |
| Issues discovered during execution | 3 bugs (9067c337, 8689e255, 16402e14) |

## Success Criteria

- [x] REQ-01 — thin-orchestrator `SKILL.md` (<500 lines) + `references/` role prompts, config-derived tiers with zero hardcoded domain type names — delivered by **e8b1cee3** (skeleton + config-derived tier derivation), **02a2bbb9** (activation + tier-derivation scenario evals). Verified: SKILL.md 225 lines, zero `milestone|epic|goal` literals in control-flow (REQ-02 config-derivation fix applied to the standards scanner too).
- [x] REQ-02 — four invocation modes with request routing — delivered by **6c5f70ad** (four-mode front door + `references/mode-routing.md`, routing evals 4/4), **206bd960** (standards-sweep mode body), **304f6d94** (sub-strategic dispatch mode), **3c192f5e** (project-wide sweep mode, REQ-06 satisfier).
- [x] REQ-03 — durable vision/charter (vision + decision log of chosen/rejected/why) + resumable progress file, both config-located and session-surviving — delivered by **eff48a6e** (`references/vision-charter.md`, `progress-artifact.md`, template; concrete `docs/9db27a3a-charter.md` + `dev/active/9db27a3a-progress.json` instantiated and linked to the v1.0 milestone).
- [x] REQ-04 — dispatch the epic lead per container in topological waves + cross-container coherence review before acceptance — delivered by **304f6d94**, **7a9eb806** (`references/coherence-review.md`), **28731a53** (wave layering), **e7d41080** (`references/container-dispatch.md`).
- [x] REQ-05 — parent-escalation policy (exactly three human-forward categories) + context-aware epic-lead escalation target — delivered by **634b2382** (`references/parent-escalation.md`), **b46e13d9** (context-aware escalation target for jit-execution-lead).
- [x] REQ-06 — canonical content-standards doc consumed by both leads + standards-sweep audit (auto-fix mechanical, surface judgment) — delivered by **faed8ffa** (canonical `docs/reference/jit-content-standards.md`), **66aeee5f** (scanner), **0b7e864d** (mechanical fixer), **206bd960** (sweep mode + report), **3c192f5e**.
- [x] REQ-07 — renames (`project-lead`→`jit-execution-lead`, `jit-plan`→`jit-planning-lead`) with cross-references swept; both skills activate from their descriptions and pass evals — delivered by **c6325c5b** (residual-reference sweep), **a5b04c9f**/**6662f738**/**41aa1b75**/**c23dfe71**/**02a2bbb9** (adjudication method + scenario + trigger evals; 17/17 triggers, all scenarios PASS).

## Wave Execution Log

- **Wave 1** (3) — canonical content-standards doc, context-aware escalation target, residual-reference sweep (serialized on shared skill files).
- **Wave 2** (3) — jit-project-lead skeleton shell, scenario-eval adjudication method + jit-execution-lead evals, activation-trigger evals for both leads.
- **Wave 3** (4) — content-standards scanner, config-derived strategic-tier derivation, sub-strategic wave layering, jit-planning-lead scenario evals.
- **Wave 4** (4) — mechanical violation auto-fixer, per-container dispatch prose, project-lead skeleton verification, runnable eval baseline rollup.
- **Wave 5** (3) — standards-sweep-mode workflow, cross-container coherence review, linchpin skeleton (config-derived tiers).
- **Wave 6** (5) — durable vision/charter + progress artifacts, parent-escalation policy, sub-strategic dispatch mode, project-wide sweep mode, four-mode front door.

## Key Decisions

- **Bracketed the epic** (planning `8f5de6bc` + breakdown `f59d1b6b`) via `jit apply plan`; plan doc records decisions D1–D9. Impl waves dispatched only after coverage-preview passed.
- **cargo-ci-before-code-review** standing process (invoker-directed): flaky parallel Rust tests block code-review non-deterministically, so every issue gets a passed cargo-ci in its run-history before code-review, and flaky tests are not "fixed" (invoker's call). A new flaky test (`test_worktree_list_excludes_expired_leases`, timing/lease-expiry) surfaced this session and cleared on re-run.
- **REQ-02 whole-skill literal-cleanliness**: the config-derivation requirement was enforced across the whole skill package, not just SKILL.md — the standards scanner's label-namespace `case` was moved from a hardcoded `epic|story|milestone` to config `[type_hierarchy.label_associations]`.
- **eff48a6e required concrete artifacts, not templates**: the reviewer read REQ-01..03 literally (the charter/progress must exist and be linked to the strategic container), so a real v1.0 charter with a genuine 5-entry decision log and a 22-epic progress file were instantiated and linked to milestone 9db27a3a.
- **jit-planning-lead criteria-format fix**: the planning skill was emitting GitHub-checkbox-prefixed criteria that the start-anchored coverage parser read as zero criteria (coverage-preview passing vacuously). Fixed the skill to mandate canonical `- [hard] REQ-NN:` and re-ran the plan-from-import eval (8/8, non-vacuous coverage proven by the strip-one-label test). The orthogonal engine-parser leniency is tracked in bug 16402e14.

## Escalations

1. **c6325c5b — criteria conflict (escalation-policy entry 4).** REQ-01 (zero old-name hits across live scope) and REQ-03 (touch only the named files) were jointly unsatisfiable because backlog-attached breakdown specs quoted old names verbatim. After three unanswered `AskUserQuestion` attempts, the lead amended REQ-03 to its plan-stated intent (Done-attached records untouched; live-scope docs corrected). **⚠ FLAGGED FOR USER REVIEW:** this criteria amendment was applied under the plan-as-authoritative-contract rule after the interactive ask timed out; confirm the amendment is acceptable.
2. **82e04ab0 — rework exceeded MAX_REWORK_ATTEMPTS (entry 5).** Three failed rounds on the tier-derivation fallback. Invoker approved lead take-over; the lead rewrote the fallback to gather-then-stop-for-confirmation; gate passed.
3. **6c5f70ad — cross-epic blocker (entries 2/7).** The four-mode front door depended on `eed6750c` (the jit-planning-lead skill, owned by epic 2fbd2a82), which was authored+dogfooded but not gated/done. Invoker directed: "finish eed6750c and proceed." The lead completed eed6750c (cross-epic, invoker-authorized: canonical-criteria fix + eval re-run + gates), which unblocked and closed 6c5f70ad.

## Issues Discovered During Execution

- **9067c337** — `jit doc add/remove` appended no event (violated INV-EVENT-LOG); blocked faed8ffa's gate. Fixed, done.
- **8689e255** — flaky test-binary spawn (`current_exe`-derived path); use `CARGO_BIN_EXE_jit`. Fixed, done.
- **16402e14** — `coverage-preview` passes vacuously on checkbox-prefixed `[hard]` criteria (start-anchored parser at `crates/jit/src/validation/`). Filed during execution; the **skill side** was fixed under eed6750c; the **engine-parser** leniency remains open, tracked under milestone v1.0 (`ready`, not an f2532a2d deliverable).

## Holistic Quality Notes

- The final skill is coherent one tier up from `jit-execution-lead`: consistent "strategic tier / sub-strategic container / steward anchor" vocabulary, config-derived throughout, and each mode routes to a body that already existed (no duplication). `SKILL.md` stayed a thin 225-line orchestrator with all detail in `references/`.
- Cross-references between the new references reconcile cleanly: `mode-routing.md` → `sub-strategic dispatch` / `jit-planning-lead` / `standards sweep mode`; `parent-escalation.md` → `vision-charter.md` decision-log format; `progress-artifact.md` → `wave-layering.md` + `container-dispatch.md`.
- The skeleton eval suite's terminal-stop narrative was reconciled when routing landed (generic prompts now stop-and-ask for a mode rather than "routing pending"), with tier-derivation assertions preserved verbatim.
- One convention tension worth noting: the content standards list "no em-dashes," yet parts of the pre-existing skill corpus use them; new references were kept em-dash-free where authored fresh.
