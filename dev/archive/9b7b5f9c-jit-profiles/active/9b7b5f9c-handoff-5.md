# Handoff — Versioned repository profiles and portable JIT dogfood setup (9b7b5f9c) — session 6

**Date:** 2026-07-16T11:09:24+03:00
**Session number:** 6
**Prior handoffs:** `dev/active/9b7b5f9c-handoff.md`, `dev/active/9b7b5f9c-handoff-2.md`, `dev/active/9b7b5f9c-handoff-3.md`, `dev/active/9b7b5f9c-handoff-4.md`

## Current state

- Epic: `9b7b5f9c` — state: backlog
- Wave in progress: wave 1 of 9
- Children summary: 1 done, 2 in_progress, 9 backlog, 0 rejected; planning and breakdown bracket nodes are also Done outside implementation waves
- Active claims: `866a5bbd` and `92039d9c` by `agent:worker`; prerequisite `d3709cc6` by `agent:worker`
- Open escalations: adding `cargo-ci-features` to `d3709cc6` requires explicit invoker approval because it changes that issue's gate scope
- Progress file: `dev/active/9b7b5f9c-progress.json` (reflects the above)

## What just happened

- Invoker approved external `codex exec` export for every configured code/doc review in this epic.
- `d3709cc6` code-review run `eaf0c6f1-becf-4258-af6f-f2ccb0969a1c` failed: its optional `quick-xml` upgrade requires canonical `cargo-ci-features` gate evidence. Existing cargo-ci and jit-validate remain passed.
- `be542b98` code-review run `8a1f5f75-e2d3-4ec8-8f84-35458a62ffe0` found one missing-index fallback in `validate_silent`.
- Fixed that caller in `dfe6b0c9` by selecting the repository-view pipeline through `IssueStore::is_file_backed()` and added a missing-index regression.
- Focused regression, formatting, cargo-ci, repo-validate, and final code-review run `54c31a0b-2979-4229-8513-ec16768fb148` all passed.
- Completed all six lead-review tiers for `be542b98`: all five prior findings remain closed, both success criteria are met, stale-narrative sweep is clean, no linked-doc deferrals exist, and the issue is coherent with wave-1 outputs.
- Marked `be542b98` Done and validated the repository; completion state is committed at `91f5d9d6`.
- Stopped before beginning another issue per the invoker's host-restart request.

## What to do next

- [ ] After host restart, run `jit recover`, verify clean `main` at `91f5d9d6` plus this handoff commit, and reinstall with `./scripts/install-jit.sh`.
- [ ] Ask the invoker to explicitly approve adding required gate `cargo-ci-features` to `d3709cc6`.
- [ ] On approval: `jit gate add d3709cc6 cargo-ci-features`, commit JIT state, reinstall, evaluate the new gate, commit evidence, reinstall, and rerun `d3709cc6` code-review.
- [ ] Perform the full six-tier review for `d3709cc6`; if PASS, mark it Done and commit JIT state.
- [ ] `866a5bbd` and `92039d9c` already have all gates passed. Reconfirm status, then mark both Done to finish wave 1.
- [ ] Advance progress to wave 2 and dispatch `eceffc17`; do not start it before every wave-1 issue is Done.

## Traps — do not repeat these

- Prior handoff traps remain in force.
- **Do not argue that the manually run all-feature matrix substitutes for `cargo-ci-features`.** Review run `eaf0c6f1-becf-4258-af6f-f2ccb0969a1c` requires the canonical configured gate record for the optional `quick-xml` change.
- **Do not add `cargo-ci-features` without explicit approval.** Gate changes are scope changes under the execution-lead escalation policy, even when a reviewer requires the gate.
- **Do not infer file-backed storage from `index.json` existence anywhere.** `dfe6b0c9` fixed the remaining `validate_silent` caller; use `IssueStore::is_file_backed()` so a missing index is judged by the repository-view pipeline.
- **Do not rerun a gate before reinstalling at the current clean HEAD.** The installed JIT binary predates completion commit `91f5d9d6` and this handoff commit.
- **Keep all-feature doctests serialized in this environment.** Use `cargo test -p jit --all-features --doc -j 1 -- --test-threads=1`; default doctest concurrency previously exhausted `/tmp`.

## Open questions needing invoker input

- Question: May the lead add the configured `cargo-ci-features` gate to `d3709cc6`?
  - Context: code review requires canonical feature-enabled clippy/test evidence because the issue upgrades optional `quick-xml`; adding a gate changes issue scope.
  - Options: approve adding and passing the gate; decline, leaving `d3709cc6` and wave 1 blocked.
  - Recommendation: Approve; the gate precisely matches the existing REQ-02 all-feature criterion and repository policy.

## Reference artefacts

- Epic: `jit issue show 9b7b5f9c`
- Plan: `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-plan.md`
- Progress: `dev/active/9b7b5f9c-progress.json`
- Previous handoff: `dev/active/9b7b5f9c-handoff-4.md`
- Completed overlay issue: `jit issue show be542b98`
- Overlay rework: `dfe6b0c9`; completion state: `91f5d9d6`
- Overlay failed/final reviews: `.jit/gate-runs/8a1f5f75-e2d3-4ec8-8f84-35458a62ffe0/result.json`, `.jit/gate-runs/54c31a0b-2979-4229-8513-ec16768fb148/result.json`
- Dependency prerequisite review: `.jit/gate-runs/eaf0c6f1-becf-4258-af6f-f2ccb0969a1c/result.json`
