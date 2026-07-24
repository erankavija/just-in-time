# Handoff — Repository-state quality hardening (1cc809de) — session 1

**Date:** 2026-07-24
**Session number:** 1
**Prior handoffs:** None.

## Current state

- Epic: `1cc809de` — state: **in_progress** (claimed `agent:jit-execution-lead`).
- Wave in progress: **wave 1 of 8** (dispatched; workers running in background, NOT yet reviewed or merged — this session did no reviews by instruction).
- Children summary: 35 impl-interior issues + 5 story checkpoints. Bracket nodes done (planning `02dc4bac`, breakdown `24bab642`). All impl issues backlog/ready except the 6 wave-1 tasks now claimed+dispatched.
- Active claims (`agent:worker`, claimed this session, all base commit `1c2281ce`):
  - `4b2005fe` retry-combinator-foundation → worktree `.agents/worktrees/agent-4b2005fe`, branch `worktree-agent-4b2005fe`
  - `9fda1f86` total-journal-action-extraction → `agent-9fda1f86`
  - `eefbfe84` producer-error-family → `agent-eefbfe84`
  - `fee3c528` plan-identity-tail (REQ-04) → `agent-fee3c528`
  - `73981310` benchmark-harness → `agent-73981310`
  - `fc744df6` lock-hygiene-sidecar → `agent-fc744df6`
- Open escalations: None.
- Progress file: `dev/active/1cc809de-progress.json` (full 8-wave plan; wave-1 issues marked `dispatched` with worker agent IDs).
- Git: current `main` HEAD `9ed43066` ("wave-1 claims"). Dogfood binary reinstalled to HEAD this session (was stale at `276b0674`).

## What just happened

- Discovery: epic already broken down; plan `dev/active/1cc809de-plan.md` complete, all bracket gates passed. Confirmed no unresolved design questions (PD-1 lock-hygiene=eliminate sidecar; PD-2 vendor deck assets; REQ-04 has recorded-exemption exit path owned by `fee3c528`).
- Extracted full dependency DAG; built 8-wave plan over the impl interior (excludes bracket nodes P/B). Fan-out order per plan: **S1+S3 parallel → S2+S4 after S1 story → S5 last**.
- Reinstalled jit from clean main (`./scripts/install-jit.sh`) → provenance now `commit 77329379 dirty=false`, then advanced by two lead commits.
- Claimed epic, set `in_progress`, wrote progress file (commit `1c2281ce`).
- Created 6 wave-1 worktrees via `dispatch-worker-worktree.sh` (all verified anchored at `1c2281ce`). Claimed 6 issues, committed claims (`9ed43066`).
- Dispatched 6 background `general-purpose` workers with full verbatim specs + shared-contract context. All still running at session end.

## What to do next

- [ ] **Wait for the 6 wave-1 workers to finish**, then for EACH: run Section 7 six-tier lead review (`references/lead-review-protocol.md`). This is the first review of each issue → skip Tier 1.5 only, run all other tiers.
- [ ] Review sequence to reduce merge pain: merge S1 producer/plan-identity pair carefully (see Trap on `mod.rs` overlap). Suggested merge order: `9fda1f86`, `73981310`, `fc744df6` (disjoint files) first; then `4b2005fe`; then `eefbfe84` and `fee3c528` (resolve `repository_state/mod.rs` overlap on the second).
- [ ] Per merge: `git merge --no-ff worktree-agent-<id>` then `scripts/verify-commit-builds.sh` BEFORE the next merge (Step 5 of worktree-dispatch-protocol). Then run gates: `jit gate evaluate <id> cargo-ci` and `jit gate evaluate <id> code-review` (sequential, explicit cwd = repo root — see Traps).
- [ ] After all wave-1 merges land on main, **reinstall the binary** (`./scripts/install-jit.sh` from clean main) before evaluating any gate — merged Rust changes make the installed binary stale.
- [ ] Run `.agents/skills/jit-execution-lead/scripts/check-leak-into-main.sh` after the wave completes and before committing on main (leak snapshot: `/tmp/lead-pre-dispatch-latest.txt`, taken at dispatch).
- [ ] Complete passing issues (`jit issue update <id> --state done`), advance progress file to wave 2, dispatch wave 2 (`a4e5ca3d`, `a6ee4e23`).

## Traps — do not repeat these

- **`eefbfe84` and `fee3c528` both edit `crates/jit/src/repository_state/mod.rs`.** Both workers were told to keep edits localized (error-type defs + `Producer` variant + sink-helper deletions for eefbfe84; `MaterializationRequest`/plan-identity routing region for fee3c528), but a merge conflict on `mod.rs` is still likely. Merge one, build-verify, then merge the second and resolve — do NOT dispatch a third worker to "fix" it.
- **Do NOT evaluate gates with a stale installed binary.** The stale-binary guard requires `commit == HEAD, dirty=false`. Every wave that lands Rust changes invalidates the installed binary — reinstall from clean main before `jit gate evaluate`. (This session already hit this: binary was at `276b0674` vs HEAD `77329379`; reinstalled.)
- **Do NOT run `jit gate evaluate` in parallel or with a drifting cwd.** Parallel evaluations lose results to per-issue locks; a non-repo-root cwd once wrote gate evidence into an install worktree's `.jit`. Evaluate sequentially, always from the repo root.
- **Worker worktrees are anchored at `1c2281ce`, one commit behind current `main` (`9ed43066`).** That one commit is only the wave-1 claims (`.jit` issue JSON), which workers do not touch — merges are non-conflicting on that commit. Do NOT mistake the missing claims commit for a TRAP-1 stale-base problem; the base is intentional and recent.
- **Many stale `agent-*` worktrees/branches exist** from the planning/breakdown sessions (dozens, some with `-r1/-r2` rework suffixes). Only the six at base `1c2281ce` (`agent-4b2005fe/9fda1f86/eefbfe84/fee3c528/73981310/fc744df6`) belong to this wave. Ignore the rest; do not merge them. Consider `git worktree prune` + branch cleanup as pre-existing housekeeping, but verify none are mid-flight from another session first.
- **REQ-04 (`fee3c528`) exemption is plan-amendment-only.** If the worker reports Initialize/ApplyProfile genuinely cannot fold into the plan-identity tail, that is an escalation for a formal plan amendment — NOT an in-task exemption and NOT a lead decision to wave through.
- **`28254964` label/parent mismatch (structural, not blocking).** It carries `satisfies:REQ-12` (an S4/hygiene criterion) but is DAG-parented under story `2958105e` (S2). Ordering is correct (shared foundation for S2 cutover guard + S4 demotions). Recorded in progress file `surfaced_pitfalls`; surface in the epic completion report, do not "fix" mid-flight.

## Open questions needing invoker input

None. (The invoker directed: do the handoff now, let workers complete, no reviews this session.)

## Reference artefacts

- Epic: `jit issue show 1cc809de`
- Plan/design: `dev/active/1cc809de-plan.md`
- Breakdown manifest: `dev/active/1cc809de-breakdown.json` (+ `-keymap.json`)
- Progress file: `dev/active/1cc809de-progress.json`
- Audit (finding-level source of truth): `dev/studies/cdc840ad-audit-2026-07-23.md`
- Session-cost template artifact: `dev/studies/perf/session-cost-27ffbd2d.json`
- Dispatch snapshot for leak check: `/tmp/lead-pre-dispatch-latest.txt`
- Protocols: `references/worktree-dispatch-protocol.md`, `references/lead-review-protocol.md`, `references/escalation-policy.md` (MAX_REWORK_ATTEMPTS=2)
