# Epic Complete: Agent-facing CLI ergonomics: gate verdicts, --json contract, and workflow primitives (53e3fa36)

**Started:** 2026-06-24 (execution session)
**Completed:** 2026-06-24
**Assignee:** agent:project-lead

### Summary

Delivered the agreed agent-ergonomics scope mined from 42 Claude Code sessions: a stable, hard-broken CLI `--json` contract for `issue show` (root `short_id`, always-array fields, a unified enriched `gates` array, field projection, multi-id arrays) plus gate-workflow and issue/claim primitives (exit-code taxonomy + verdict, skip-if-passed-at-HEAD, fail-fast `pass-all`, owner-bypass `claim release` by issue id, actionable claim-blocked errors + `--assign-only`, `search --label`, declarative `batch-create`, and four command/flag aliases). All changes are CLI-presentation-only; the domain model, server API, web UI, and storage format are unchanged.

### Metrics

| Metric | Value |
|---|---|
| Children completed | 11 / 11 implementation tasks (4 / 4 stories; planning + breakdown bracket nodes already done) |
| Waves executed | 3 dependency layers, run as one sequential in-main-tree order (11 tasks) |
| Rework cycles | 10 worker rework dispatches + 3 lead-direct trivial doc fixes |
| Escalations | 0 |
| Sub-agent dispatches | 21 (11 initial + 10 reworks) |
| Issues created during execution | 0 (all 11 tasks pre-existed from the approved breakdown) |

### Success Criteria

- [x] REQ-01: `issue show --json` includes root `short_id`; `labels`/`dependencies` always arrays — **870176ce**
- [x] REQ-02: unified `gates` array `[{key,status,last_run_at,exit_code}]` replacing `gates_required`/`gates_status`, enriched from the latest `GateRunResult` — **4899e4b6**
- [x] REQ-03: `--field`/`--fields` projection and multi-id `--json` array — **f646f7a5**
- [x] REQ-04: `gate pass` exit-code taxonomy (0/4/10 + pre-verdict 2/3) and `verdict` field — **65ef3741**
- [x] REQ-05: `gate pass` skips re-running when already passed at HEAD; `--force` overrides — **9bfcc474**
- [x] REQ-06: `gate pass-all <id>` fail-fast — **df7f934c**
- [x] REQ-07: `claim release <issue-id>` releases the active lease regardless of owner, with audit — **e9d71bf6**
- [x] REQ-08: claim-blocked error names `jit issue assign`; `--assign-only` — **7bb91384**
- [x] REQ-09: `issue search --label <ns:val>` with optional positional query — **4597743e**
- [x] REQ-10: `issue batch-create --from-json` with full pre-validation + DAG wiring, pure `{key:id}` map — **6ec1dd6f**
- [x] REQ-11: `dependency`/`document`/`issue list`/`--add-label` aliases — **ea717ef1**

### Wave Execution Log

The bracket (planning node `2937919f` plan-review, breakdown node `838483f3` coverage-preview + breakdown-review) was already complete on intake. The 11 impl tasks form 3 dependency layers; because nearly every task edits the monolithic `cli.rs`/`main.rs`, they were executed **sequentially in the main tree** (the project's own conflict heuristics: any two CLI additions collide on `main.rs`/`cli.rs`), one reviewed + gated + committed before the next.

**Layer 1 (roots):** REQ-01, REQ-04, REQ-07, REQ-08, REQ-09, REQ-10, REQ-11 — JSON-contract base, gate exit codes, claim/search/batch/alias primitives.
**Layer 2:** REQ-02 (←REQ-01), REQ-05 (←REQ-04) — enriched gates array, skip-if-passed.
**Layer 3:** REQ-03 (←REQ-02), REQ-06 (←REQ-05) — field projection, pass-all.

### Key Decisions

- **Sequential-in-main-tree over parallel worktrees.** The project's `conflict-heuristics.md` states any two CLI additions conflict on `main.rs`/`cli.rs`; nearly every task adds CLI surface, so worktree fan-out would merge-conflict. Serialized in dependency order; zero integration conflicts.
- **Kept the installed `jit` binary stale through the REQ-02 hard break.** The lead's orchestration reads `gates_status` from `issue show --json`; rebuilding would have broken that mid-epic. Verification relied on the gate (which builds from source) + tests instead.
- **Made 3 trivial `# Examples` doc fixes directly (REQ-03, REQ-07, REQ-10 final blockers) rather than escalating.** Each was the same well-established public-API doc rule, doc-only, zero logic risk; escalating "approve two doctests" would have wasted the user's time. All substantive findings in those issues were already resolved by the worker.
- **REQ-07 audit identity: error rather than fabricate.** When no `JIT_AGENT_ID` and no git `user.name` exist, `claim release` now errors with remediation instead of recording a non-attributable `human:unknown` — the literal reading of "the acting identity is recorded for audit".

### Escalations

No escalations were required. Every decision fell within the lead's autonomy (execution strategy, routine implementation choices, rework with specific feedback). No task exceeded MAX_REWORK_ATTEMPTS at the worker level (the three issues that hit a 3rd review round closed on a trivial lead-applied doc fix, not a stuck worker).

### Issues Discovered During Execution

No additional issues were discovered. The approved breakdown (11 tasks, each `satisfies:REQ-NN`) fully covered the epic; coverage-preview had already passed at plan time.

### Holistic Quality Notes

- **The `code-review` gate enforced a consistent contract** that drove almost all rework: every public API needs a `# Examples` doctest; fallible operations return typed `thiserror` errors (not bare `String`) and propagate with context; all file writes use atomic temp+rename; no duplication. From REQ-04 onward the lead baked this contract checklist into every worker prompt, and REQ-06/08/09/11 passed both gates on the first attempt.
- **The hard `--json` break stayed scoped to the presentation layer** across all CLI-output tasks: `IssueShowResponse`/`output.rs` and CLI tests/docs changed; the domain `Issue` (`gates_required`/`gates_status`), `crates/server`, `web/`, and `storage-format.md` were verified untouched throughout.
- **Storage robustness improved as a side effect of REQ-02:** gate-run result files are now written atomically and malformed runs surface as errors with context rather than being silently dropped.
- **Naming and exit-code semantics are coherent across the gate surface:** REQ-04's taxonomy, REQ-05's skip outcome, and REQ-06's fail-fast all route through one `pass_gate` path and one shared `render_gate_pass_error`.
