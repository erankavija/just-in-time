# Handoff — The workspace test suite runs in under 30 seconds (4b7c06d0) — session 2

**Date:** 2026-08-12
**Session number:** 2
**Prior handoffs:** `dev/active/4b7c06d0-test-suite-performance/handoff.md`

## Current state

- Epic: `4b7c06d0` — state: backlog
- Wave in progress: wave 5 of 8
- Children summary: all work through wave 4 done; `6d10e5d4` ready; `94d85bf1`, `25faa21d`, and `76ecc11f` backlog
- Active claims: none
- Open escalations: approval to change shared nextest configuration and shared test fixture support for `6d10e5d4`
- Progress file: `progress.json` in the epic's artifact directory, `jit doc dir <epic-id> dev/active` (reflects the above)

## What just happened

- Completed waves 2-4, including journal recovery coverage, transaction evidence, pinned-nextest cargo-ci cutover, per-step/suite-clock reporting, and a 4,504-identity warm profile.
- Dispatched `94d85bf1`; the hard worker stopped clean because the unchanged checker rejects the observed 60,562-113,272 ms suite clock against 30,000 ms.
- Independently audited the profile: 883,556 aggregate test-ms yields a 42,139 ms ideal lower bound after doctests on 24 logical CPUs; at least a one-third aggregate reduction is mandatory.
- Created and wired `6d10e5d4` as the new prerequisite, demoted and unassigned clean `94d85bf1`, and preserved the transitive-reduced DAG.
- Reclaimed the clean `94d85bf1` worktree; no worker changes or leak entered main.

## What to do next

- [ ] Resolve the open shared-infrastructure escalation from the invoker response.
- [ ] If approved, split execution into two hard subproblems under `6d10e5d4`: prepare the stale-binary artifact inside nextest without occupying six test slots, and share verified immutable profile/repair baselines while cloning isolated mutable state.
- [ ] Require the unchanged default nextest plus doctest span to stay below 29,000 ms in three consecutive warm runs and regenerate the complete profile before closing `6d10e5d4`.
- [ ] Resume wave 6: dispatch `94d85bf1` for its unchanged one-line live-clock wiring, then complete waves 7-8 and the epic gates.

## Traps — do not repeat these

- **Do not wire the live suite clock now.** The worker reproduced 60,562 ms warm and the authoritative merged gate recorded 113,272 ms; `scripts/rust-build-budget.sh --test-suite-ms 60562` correctly exits 1.
- **Do not treat host load as the root cause.** The aggregate profile divided by 24 logical CPUs plus doctests is already 42,139 ms under perfect packing; reduce at least 291,332 aggregate test-ms before expecting the fixed budget to be feasible.
- **Do not optimize only the six stale-binary tests.** Their 95,249 aggregate ms is material but insufficient; profile lifecycle and derived-state repair fixtures must also converge on reusable immutable setup.
- **Do not move setup outside the named suite clock or share mutable repositories across tests.** The epic fixes the clock as nextest plus doctests and requires the same semantic properties, not cheaper accounting.
- Prior handoff traps remain in force; re-read `handoff.md` before dispatching.

## Open questions needing invoker input

- Question: May the lead implement the required project-wide test-infrastructure changes inside this epic?
  - Context: Meeting the fixed 30-second contract now requires nextest-level stale-fixture preparation and shared immutable profile/repair fixture support, beyond the originally dispatched one-file enforcement leaf.
  - Options: approve both scoped hard prerequisites; or decline the shared changes and leave the epic blocked because the unchanged checker deterministically fails.
  - Recommendation: Approve both; they preserve the exact test identity set and assertions while addressing the measured aggregate lower bound.

## Reference artefacts

- Epic: `jit issue show 4b7c06d0`
- Planning docs: `dev/active/4b7c06d0-test-suite-performance/progress.json`
- Benchmark/result artefacts: `dev/benchmarks/suite-profile.json`, `dev/benchmarks/transaction-model-limit.json`
- Prior handoff: `dev/active/4b7c06d0-test-suite-performance/handoff.md`
