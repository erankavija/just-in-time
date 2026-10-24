# Codex recovery coordination handoff — 2026-10-23

The user requested handoff; do not dispatch further work until resumed. This session recovered interrupted Claude work and coordinated three Codex leads. Read their latest handoffs and live tracker before acting; this document is a dated coordination snapshot, not the dependency graph.

## Ownership and outcomes

| Stream | Session ID | Checkpoint |
|---|---|---|
| Greenfield cleanup `842b0ac6` | `01a164f8-f0b6-7fb1-8fcc-8af3e3435bfe` | Complete and archived; all 40 implementation tasks and two bracket nodes done. Both epic gates passed, holistic review had zero findings, post-archive validation had zero errors/warnings. |
| Profile/workflow simplification `3d9a0ce3` | `01a164fd-a47b-7d23-b92d-7fb1f95e0fbb` | `abe963dd` remains unmerged. User later explicitly told this lead to wait; preserve that pause. |
| Test topology `c780514e` | `01a16502-c6e3-7951-a88b-3f0feb4c70f3` | At handoff request: 22/148 leaf tasks done, two active. Lead requested to checkpoint workers and pause. |
| Coordinator | `01a164c8-d7ba-7812-a630-db8dddcee05b` | This handoff. |

Cleanup completion evidence: [completion report](../../archive/842b0ac6-greenfield-cutover/active/completion-report.md). Do not reopen the epic. Its final import cutover is on main. The contradictory drift-deletion criterion was reconciled with the already-approved plan's rename decision, with failed review and prior wording retained; a new holistic review passed.

Test-topology recovery completed all three interrupted tasks (`22355a56`, `de880d9c`, `4a82ed46`) plus the discovered gate-lock status bug `5c6c108a`. Subsequent completions include declared shell self-test gates, doctest consolidation, missing-Cargo failure handling, shared available-work queries, finalization-test ownership and host-independent lock-test preconditions. Use the live tracker for the full list and sequencing.

## Resume locations

- Test topology: [handoff](../c780514e-test-topology/handoff.md), [progress](../c780514e-test-topology/progress.json), reviewed plan and breakdown in the same directory. The physical checkout `agent-22355a56` carries clean `worktree-agent-df505379` at `ce23d3b19` (claimed, no implementation yet); `agent-de880d9c` carries clean `worktree-agent-fa9bf296` at `163e3a371` (registry-only WIP, projection and tests pending). Names of physical worktrees no longer identify their active issues. Both workers are paused, task leases released and assignments retained; no owned builds/reviews/locks remain according to the lead's checkpoint. Reacquire claims on resumption and preserve these commits.
- Profiles: the authoritative latest handoff is `dev/active/3d9a0ce3-lead-friction/handoff-4.md` on `worktree-agent-abe963dd` in `.agents/worktrees/agent-abe963dd`, committed with its progress update as `2cc3395bc`. It is not on main. The lead confirmed the worktree clean and paused, both workers completed, and no owned active processes or locks. Read that branch's progress rather than main's older copy.
- Rust quality `df129d9e` has no active lead. `9cc3056a` was merged and both gates passed, but remains in progress. Its interrupted gate evidence was preserved in `ee3fcbc84`; administrative closure still remains. Other work is not restarted.
- Mutation-path planning `b697a3f7` has no active lead. Plan/manifest rework is checkpointed at `0c4ec0ae6`; second review reportedly raised 23 new findings after resolving 38 earlier ones. Resume planning review, not implementation.

## Profile caveats

The `abe963dd` worker consolidated profile selection and fixed duplicate model validation for accepted path aliases. Existing parent-traversal rejection remains; tests must not broaden accepted paths. Earlier independent review passed with zero findings and a full Rust gate passed 4,596 tests and 58 doctests, but main subsequently changed substantially through test-topology work.

The refreshed gate later timed out after 900 seconds compiling the library test binary, before any test verdict. Preserve that infrastructure error (`d3a0da67`, committed in `e00b10914`). A read-only Luna audit found no obvious source-level cause. A standalone no-run prebuild subsequently succeeded in 34.19 seconds. The final gate and merge were NOT run after the user's later wait instruction. On explicit resume, reconcile latest main and run required checks; do not treat prebuild success as passing gates.

The user explicitly approved merging `abe963dd` in the coordinator conversation after an automatic approval rejection. That approval covers the reviewed task's main merge and gate/closure commits, but the subsequent wait instruction controls whether to act now. Test-topology separately received direct autonomous-work authorization in its session. If approval review rejects an operation, supply the actual user evidence through the normal reviewer; never bypass it.

## Operating constraints and lessons

- User model preference: **Luna xhigh is the workhorse**. Future bounded worker dispatches use `gpt-6-luna`, `reasoning_effort=xhigh`, `fork_turns=none`, with sufficient scoped context. Stronger tiers require task-difficulty justification. Keep independent review and configured gates.
- Use existing worktrees; test-topology lead has a two-active-worktree cap. Free space was about 66 GiB at handoff request. Recheck before dispatch/build; recorded new-worktree threshold is 60 GiB.
- Explicitly set `CARGO_INCREMENTAL=0` in every build/gate launcher. A generated topology launcher omitted it; a quiet-host suite-budget failure was preserved, the launcher corrected, and the unchanged authoritative gate passed. This mismatch was not proven to be the sole cause. Profile's compile timeout already had the variable set.
- Full gates/main source merges use `flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock`. Other builds use the shared (`-s`) full-CI lock after the turnstile. Coordinate windows between leads. The gate script acquires its own distinct internal `cargo-ci.lock`; do not acquire that twice.
- Before merging a source branch, integrate current main and pass the full workspace tests and doctests, plus required gates. Preserve failed evidence; do not relax budgets or repeatedly force retries without diagnosis.
- The gate-lock bug is fixed: child infrastructure exit 75 must not be mistaken for lock acquisition timeout. No lock-file cleanup was necessary.
- No broad cache/worktree cleanup. A one-time approved cleanup removed only main `target/debug/incremental` and `agent-22355a56/target/debug/incremental`, with verified canonical paths, no active compilers and exclusive locks. Actual reclaimed space was 4,741,713,920 bytes, less than logical directory sizes because sharing was likely. This is not general deletion permission.
- Keep main clean and commits scoped. Never stage all `.jit/` when concurrent leads may be recording evidence. Do not delete intended archive markers to satisfy clean-tree checks.

## Narrow reconciliations already authorized

- `df505379`: add only required `JsonSchema` derives/imports on existing `PresetInfo` and `Lease` in `gate_presets.rs` and `storage/claim_coordinator.rs`, besides the planned footprint. Preserve runtime/serde behavior and validate schemas; no duplicate adapters. The expansion is recorded in `26a67efba`.
- `9bb30f15`: remove unsupported `satisfies:c780514e/REQ-10` credit from issue/plan/manifest while retaining the no-ignored-tests invariant and unchanged criteria/gates. Correction is `66941bb60`; verify requirement coverage remains valid.
- `5c6c108a`: the additional gate-lock-status bug was explicitly accepted into test-topology and completed with shell regression coverage and full gates.

## Next coordinator actions after the user resumes

Read the final lead handoffs, inspect main and the three surviving worktrees, and check live process/lock holders before starting builds. Resume test-topology from its current two tasks. Keep profiles paused until the user resumes that stream; then use the existing approved merge scope and refresh checks. Cleanup needs no further work. Do not automatically recruit or restart the gf2 session: it was mistakenly contacted once, and that request was withdrawn.
