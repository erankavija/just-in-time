# Completion Report: Plan-before-fan-out bracket (2fbd2a82)

**Started:** 2026-06-16
**Completed:** 2026-06-18
**Assignee:** agent:project-lead

### Summary

Delivered the plan-before-fan-out bracket: a breakable container `C` is now bracketed by a gated planning node `P` and breakdown node `B`, with the implementation subgraph spliced as a reduced `C → impl → B → P` spine. Planning is reviewed (plan-review gate) and coverage is checked (coverage-preview gate) *before* the implementation fans out — the front-end counterpart to epic 94d26c42's done-transition coverage enforcement.

### Metrics

| Metric | Value |
|---|---|
| Children completed | 12 / 12 tasks (4 / 4 stories) |
| Waves executed | 6 |
| Rework cycles | ~13 (across all tasks; T10 took 5) |
| Escalations | 2 (both on T10) |
| Sub-agent dispatches | ~27 (including reworks) |
| Issues created during execution | 0 |

### Success Criteria

- [x] **Breakable container scaffolds to `C→P` (plan-quality gate on P); breakdown splices the impl subgraph + B into a reduced `C→impl→B→P` spine, correctly typed** — T5 (`jit plan` / `--with-planning` scaffolds P, applies plan-review preset, moves upstream deps) + T10 (bracket breakdown creates B, wires source/sink spine, reduced form).
- [x] **Breakdown gate blocks when drafted children leave a `[hard]` criterion uncovered, transitively over the impl interior (exit 4)** — T2 (`jit validate --scope`, exit 4) + T3 (transitive coverage) + T6 (coverage-preview preset + preview rule) + T10 (runs the gate via the standard runner) + T12 (steering scenario `bracket-coverage-gap`).
- [x] **Coverage traversal is transitive by default; `child-type-exclude` excludes bracket nodes and halts descent at them** — T3 (`children_of` closure walk + `child-type-exclude` boundary + `container-from-label` indirection).
- [x] **`jit plan <existing>` retrofits a container, moving upstream deps onto P** — T5 (retrofit path).
- [x] **Demonstrated on SDD (epic) AND research (goal, no epic) with no methodology strings in Rust** — T7 (SDD example) + T8 (research example, no epic/milestone); domain-agnosticism enforced throughout T1/T2/T3 (type names read from the flat planning-config block, later superseded by `.jit/templates.toml` in epic 9ac9fdac; production-Rust grep clean).
- [x] **jit-breakdown and project-lead drive plan→review→breakdown→coverage→implement; steering suite regression-tests it** — T10 (jit-breakdown re-sequenced) + T11 (project-lead re-sequenced, config-driven) + T12 (3 steering scenarios: plan rejected, coverage gap, clean path).

### Wave Execution Log

- **Wave 1** (3, parallel worktrees) — engine primitives: T1 flat planning-config block + R5 schema regen, T2 `validate --scope`, T3 transitive coverage + `child-type-exclude` + container indirection.
- **Wave 2** (2, parallel worktrees) — T4 plan-doc location resolver (boundary I/O, pure engine), T6 plan-review + coverage-preview presets + preview rule.
- **Wave 3** (3) — T5 scaffolding command, T7 SDD example, T8 research example.
- **Wave 4** (2) — T10 jit-breakdown re-sequence (bracket builder + skill), T9 concept + how-to docs.
- **Wave 5** (1) — T11 project-lead re-sequence (lead-direct).
- **Wave 6** (1) — T12 steering scenarios (data-only fixtures).

### Key Decisions

- **Gate backfill.** The pre-existing breakdown left all 12 tasks gate-less; attached `cargo-ci` + `code-review` to each (matching the analogous back-end epic 94d26c42). The user subsequently codified per-task gate assignment into the jit-breakdown skill.
- **Worktree isolation** for parallel Rust waves; serial-on-main for the (many) gate-driven reworks.
- **T10 coverage-gate architecture (escalated, Option 2 chosen):** the `bracket_breakdown` helper stays a pure bracket-builder that *attaches* the coverage-preview gate; the gate is *run by the standard gate runner* as a skill step — never faked/stamped inside command code. Cleanest separation of concerns.
- **cargo-ci concise-output wrapper (user-directed, modeled on ../gf2):** replaced the inline `cargo fmt && clippy && test` with `scripts/cargo-ci.sh`, which emits one-line per-step summaries on success and full diagnostics only on failure (and bakes in the disk-backed TMPDIR + single-threaded run the suite needs). The raw test output was flooding the code-review reviewer's context.

### Escalations

1. **T10 "run the coverage-preview gate" (architecture).** After 3 rounds of the same finding (the reviewer wanted a real persisted gate run, not a fabricated status), escalated rather than dispatch a 4th attempt. User chose Option 2 (helper attaches; standard runner runs the gate) — implemented, accepted.
2. **T10 code-review wedged on pre-existing env-sensitive tests.** The code-review reviewer re-ran 5 unrelated tests (TCP-port binding, claim-coordinator concurrency) that fail only in its no-network sandbox. Surfaced as a shared-infra/test fragility; the gf2-style concise cargo-ci wrapper was put in place (user-directed) and the user passed the gate.

### Issues Discovered During Execution

No additional JIT issues were created. Notable pre-existing fragilities surfaced and worked around (flagged for follow-up): a load-sensitive `claim_coordinator` concurrency proptest and 5 environment-sensitive `serve.rs`/`issue.rs` tests that fail under restricted (no-network/parallel) sandboxes; the cargo-ci log-flooding (fixed via the wrapper).

### Holistic Quality Notes

- **Domain-agnosticism held end-to-end** — repeatedly grep-verified that no `epic`/`planning`/`breakdown` literals leaked into production Rust; all type/preset names flow from the flat planning-config block (later superseded by `.jit/templates.toml`, epic 9ac9fdac). The two example rulesets (epic-based SDD, goal-based research) prove agnosticism.
- **Consistent terminology** (`P`/`B`/spine/plan-review/coverage-preview) across engine, the jit-breakdown and project-lead skills, the concept/how-to docs, and the example rulesets.
- **Clean layer separation** — filesystem I/O confined to `commands/` boundaries; `validation/graph.rs` and `domain/` stayed pure (the doc-resolver and bracket-builder reworks were both about restoring this).
- **Side deliverable:** `scripts/cargo-ci.sh` — a project-wide gate-hygiene improvement beyond the epic's scope.
