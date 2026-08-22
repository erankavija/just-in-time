# Handoff — Complete profile lifecycle, composition, and upgrades (c639cfb5) — session 3

**Date:** 2026-08-09
**Session number:** 3
**Prior handoffs:** `dev/active/c639cfb5-jit-profiles-complete/handoff.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-2.md`

## Current state

- Epic: `c639cfb5` — state: in_progress
- Wave in progress: wave 4 of 18
- Children summary: 4 done, 1 in_progress, 19 backlog/ready, 0 rejected
- Active claims: `fc47a7bf` claimed by `agent:worker-fc47a7bf`; its final isolated branch commit is `573e6474`
- Open escalations: `fc47a7bf` exceeded two rework retries after the final lead stale-narrative sweep found one remaining neighbouring phrase at `docs/reference/storage-format.md:67`
- Progress file: `progress.json` in the epic's artifact directory, `jit doc dir c639cfb5 dev/active` (reflects the above)

## What just happened

- Completed and gated `474a90a8`, then completed checkpoint story `f73d54a2`; Wave 4 advanced to `fc47a7bf`.
- Dispatched initial `fc47a7bf` implementation to Luna xhigh; commit `6b51f0be` failed cargo-ci on a stale schema acceptance test and failed lead semantic review on six variable-resolution boundaries.
- Dispatched rework attempt 1 to Luna xhigh; commit `5a8bfaa6` passed exact cargo-ci but failed code-review run `84ebaa76-5623-4770-b605-e2d11f3d2d01` because validation/repair rebuilt with default inputs.
- Dispatched final rework attempt 2 to Sol xhigh because persistence, validation, record provenance, and audit behavior were semantically coupled; commit `573e6474` persisted typed public variable provenance and made validation/repair re-resolve from the record.
- Verified `573e6474` remained scoped despite Sol: one canonical record field and coupled plumbing; no migration framework or unrelated refactor. Worker-reported full Rust and MCP suites pass.
- Installed exact final commit at `/tmp/jit-fc47a7bf-final-install/bin/jit` with embedded clean provenance.
- Lead Tier 1.5 confirmed the prior code-review finding closed at `commands/validate.rs:924-932` and `profile/apply_claims.rs:39-50`.
- Lead Tier 2.5 found one stale neighbouring phrase: `docs/reference/storage-format.md:67` still says “minimal provenance record”; the canonical record now contains resolved public variables and source kinds.
- Stopped before a third repair as required by `MAX_REWORK_ATTEMPTS = 2`; invoker guidance is pending.

## What to do next

- [ ] Apply the invoker's decision for `fc47a7bf`. Recommended: authorize one narrowly targeted retry, reset its rework counter, and change only `docs/reference/storage-format.md:67` plus any exact identical stale matches.
- [ ] If authorized, use Luna xhigh through `codex exec` for the well-specified correction; do not let the worker touch `.jit/**` or broaden the record design.
- [ ] Re-read the lead review protocol in full, rerun all stale-narrative and deferred-item sweeps, and inspect the final diff.
- [ ] Install the new exact worker commit with `./scripts/install-jit.sh`; rerun `cargo-ci` and `code-review` gates from the exact binary. Preserve all failed and passed gate history.
- [ ] Commit lead-owned `.jit` gate evidence on the worker branch, merge the reviewed branch into current `main` (which includes lead progress/escalation commits), install exact merged main, and rerun cargo-ci plus MCP.
- [ ] Complete `fc47a7bf`, update Wave 4 to done and `current_wave` to 5, remove the worktree and exact disposable targets, then dispatch `cbcd9318`.

## Traps — do not repeat these

- **Do not reconstruct applied variables from declaration defaults or the current environment.** Code-review run `84ebaa76` proved this makes validation/repair disagree with successfully published `--set` content. Use the canonical resolved values stored in `AppliedProfileRecord`.
- **Do not accept stored target hashes as the expected hashes.** The earlier `expected_record(package, layout, &actual.target_hashes)` was self-confirming. Final commit `573e6474` re-resolves from stored public values and independently recomputes hashes.
- **Do not over-engineer the final repair.** The remaining defect is one stale phrase at `docs/reference/storage-format.md:67`; do not add record migrations, integrity frameworks, or lifecycle abstractions.
- **Do not edit generated event prose alone.** `docs/reference/events.md` is generated from `crates/jit/src/domain/event_catalog.rs`; `573e6474` correctly changed the source and projection together.
- **Do not chase the `web/dist/index.html not found` server build warning.** It is a pre-existing worktree advisory and unrelated to `fc47a7bf`.
- **Do not commit or delete the worker worktree's `.jit` evidence.** It contains lead-owned cargo/code-review runs, including failures required for transparency.
- **Do not apply `stash@{0}`.** It preserves separately owned pre-Wave-2 profile documentation and remains recoverable; current docs are a newer superset.
- **Do not run cargo-ci with a reused incremental target.** Install the exact commit and use a fresh disposable target so the stale-binary and build-footprint guards produce authoritative evidence.

## Open questions needing invoker input

- Question: May the lead perform one further narrowly targeted correction for `fc47a7bf` after two rework retries?
  - Context: The implementation and focused/full suites pass; the only current lead-review failure is the stale phrase `docs/reference/storage-format.md:67`.
  - Options: authorize the targeted retry and reset the counter; take over the one-line correction manually; or reject `fc47a7bf` and stop this dependency chain.
  - Recommendation: authorize the targeted retry because it is well-specified, low-risk, and necessary for the mandatory stale-narrative gate.

## Reference artefacts

- Epic: `jit issue show c639cfb5`
- Current issue: `jit issue show fc47a7bf`
- Planning docs: `dev/active/c639cfb5-jit-profiles-complete/plan.md`, `dev/active/c639cfb5-jit-profiles-complete/breakdown.json`, `dev/active/c639cfb5-jit-profiles-complete/progress.json`
- Research: `dev/active/c639cfb5-jit-profiles-complete/c639cfb5-research.md`
- Worker branch: `worktree-agent-fc47a7bf`, final commit `573e6474`
- Exact final binary: `/tmp/jit-fc47a7bf-final-install/bin/jit`
- Gate evidence: `.jit/gate-runs/84ebaa76-5623-4770-b605-e2d11f3d2d01/` and the other `fc47a7bf` runs in the worktree

