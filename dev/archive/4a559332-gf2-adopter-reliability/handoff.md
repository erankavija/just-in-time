# Handoff — gf2 adopter reliability: safe worktrees, durable gates, actionable profiles (4a559332) — session 1

**Date:** 2026-08-16T15:46:09+03:00
**Session number:** 1
**Prior handoffs:** None.

## Current state

- Epic: `4a559332` — state: backlog; assigned to `agent:jit-execution-lead`.
- Wave in progress: wave 3 of 9; wave 3 has not been claimed or dispatched.
- Children summary: 6 done, 0 in_progress, 5 ready, 8 backlog, 0 rejected.
- Active claims: epic `4a559332` only, claimed by `agent:jit-execution-lead` at `2026-08-16T08:53:26.755231837Z`; no child issue remains claimed.
- Open escalations: none. The exceptional third rework authorized for `9757ab31` is resolved and recorded in progress.
- Progress file: `progress.json` in the epic's artifact directory, `jit doc dir 4a559332 dev/active` (reflects the above).

## What just happened

- Loaded the approved plan, investigation, breakdown manifest, configured gates, and nine-wave execution plan.
- Completed wave 1 issue `9757ab31`: selected-data-root `WorktreePaths` is authoritative across storage, worktree identity, claims, lifecycle lease checks, validation, and branch stamping; cargo-ci and code-review pass.
- Reworked `9757ab31` three times. After the configured two-retry limit, escalated; the invoker authorized one exceptional, narrow third rework. The final review found zero findings.
- Completed wave 1 issue `9ea7f672`: all 23 production mutation-context construction sites route through one dispatch-scoped factory; direct construction is private; cargo-ci and code-review pass.
- Completed wave 1 issue `dd26c204`: profile conflicts/divergences carry typed serializable remedies and render prose from that same data; cargo-ci and code-review pass.
- Completed wave 2 issue `fc773c22`: storage exposes an explicit-root, read-only exact issue/event snapshot that bypasses aggregate/history/primary fallbacks; cargo-ci and code-review pass.
- Completed wave 2 issue `6a489764`: the actual `update_issue_state(..., Gated)` postcheck path propagates evaluation/persistence errors while ordinary failed checker verdicts remain successful recorded outcomes; cargo-ci and code-review pass.
- Completed wave 2 issue `45de0b2b`: `profile show <ID>` is accepted without changing sibling tagged-selector contracts; canonical CLI grammar/command references were updated; cargo-ci, code-review, doc-review, and docs-mechanical pass.
- Removed all clean, merged wave-1 and wave-2 worker worktrees and branches. Main owns every completed implementation and gate record.
- Advanced `progress.json` to wave 3 without claiming or dispatching it, per the invoker's handoff request.

## What to do next

- [ ] Resume wave 3: claim and dispatch `1c8f00bd`, `99ac9679`, and `7ca9a0dd` from current main according to `progress.json`.
- [ ] Use frontier/high reasoning for `99ac9679` (cross-cutting dispatch annotation and event semantics); use high reasoning for `1c8f00bd` (configuration plus docs) and `7ca9a0dd` (durability regression on production paths).
- [ ] Pre-flight overlaps before dispatch. Preserve the one `WorktreePaths` authority from `9757ab31` and the one mutation-context factory from `9ea7f672`; do not introduce local detection or construction variants.
- [ ] Serialize workers' full `./scripts/cargo-ci.sh` runs through `/tmp/cargo-ci.lock`; do not launch all three full suites together. Focused tests and read-only audits may still run in parallel.
- [ ] Merge one issue at a time, run every configured gate on merged main, perform all six lead-review tiers, then mark the issue done and commit gate/JIT/progress state.
- [ ] Continue waves 4 through 9 exactly as recorded in `progress.json`; do not skip the single-issue wave barriers.
- [ ] At epic completion, run `jit graph deps 4a559332`, map every epic criterion to completed children, run the epic gate, write the completion report, archive execution artefacts, and complete the epic.

## Traps — do not repeat these

- **Do not use a worker's bare relative `apply_patch` target from the shared checkout.** The initial `dd26c204` worker followed the dispatch line “Run every shell command from your worktree root,” but `apply_patch` had no workdir and placed two patches in main. The lead interrupted it and restored exactly those tracked files. For worker edits, resolve patch targets explicitly inside `.agents/worktrees/agent-<id>/`; immediately run the leak check after any uncertainty.
- **Do not stop the selected-root audit at storage and identity initialization.** The first two `9757ab31` reviews found that claim acquisition and lifecycle lease checks still re-detected from process CWD even though `main` already held authoritative `worktree_paths`. Audit complete end-to-end dispatch call chains, including Git branch/config commands, and pass injected `WorktreePaths` instead of excusing a local detect.
- **Do not change tests, thresholds, or build infrastructure for isolated suite-clock misses.** Cargo-ci repeatedly passed every test but varied between roughly 23.8 s and 31.2 s against the strict `<30,000 ms` budget. Record the failed gate, use a clean warm audit, measure changed tests, and rerun only when no inherent cost is present.
- **Do not launch all workers' full cargo-ci runs concurrently.** The wave-2 prompt line “Run ... full `./scripts/cargo-ci.sh`” was sent to all three workers; the shared lock serialized them while old per-worker caches consumed `/tmp`, producing disk-quota and proptest noise. Stagger full runs. The exact rebuildable caches `/tmp/jit-dd26c204-r1-focused` and `/tmp/jit-dd26-target` were removed; no repository data was deleted.
- **Do not bypass `/tmp/cargo-ci.lock`.** One `45de0b2b` retry described itself as lock-bypassed and produced no usable result. The post-merge configured run supplied valid cargo evidence and all four gates passed. Queue or poll the authoritative lock instead.
- **Do not assume `jit graph children` exists.** It is not a supported subcommand. Use `progress.json`, `jit graph deps`, `jit graph downstream`, and batched `jit issue status` instead.
- **Do not interpret a gate review's `tree_dirty: true` as a worker leak without checking status.** Review gates record their own `.jit` run evidence before returning; commit that evidence with the issue transition after the gate result.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show 4a559332`
- Design/planning: `dev/active/4a559332-gf2-adopter-reliability/plan.md`
- Investigation: `dev/active/4a559332-gf2-adopter-reliability/investigation.md`
- Authoritative breakdown: `dev/active/4a559332-gf2-adopter-reliability/breakdown.json`
- Execution progress: `dev/active/4a559332-gf2-adopter-reliability/progress.json`
- Session handoff: `dev/active/4a559332-gf2-adopter-reliability/handoff.md`
- Final wave-1 selected-root review evidence: `.jit/gate-runs/459214a7-f9ec-4f38-9577-c57be6ee6833/result.json`
- Final wave-2 gate evidence: `.jit/gate-runs/600df956-811a-4dde-b0f9-670461097f62/result.json`, `.jit/gate-runs/0e62a504-c069-4e09-ac04-32c9c3d935a8/result.json`, `.jit/gate-runs/ef6a06ab-15c1-4f87-a77c-f90c6d365cc0/result.json`
- External references: None.
