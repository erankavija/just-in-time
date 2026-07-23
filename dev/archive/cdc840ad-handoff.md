# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 1

Status: archived after epic completion on 2026-07-23.

**Date:** 2026-07-18T20:43:55+03:00
**Session number:** 1
**Prior handoffs:** None.

## Current state

- Epic: `cdc840ad` — state: backlog, assigned to `agent:jit-execution-lead`
- Wave in progress: wave 1 of 9
- Children summary: 0 implementation children done, 1 in_progress (`cbc3a7e5`), 10 backlog; planning and breakdown bracket nodes are done
- Active claims: `cbc3a7e5` assigned to `agent:terra`; epic `cdc840ad` assigned-only to `agent:jit-execution-lead`
- Open escalations: `cbc3a7e5` exceeded two rework cycles; invoker decision required before another retry
- Progress file: `dev/active/cdc840ad-progress.json` (reflects the above)

## What just happened

- Reconciled the approved bracket and persisted a nine-wave topological execution plan.
- Claimed `cdc840ad` and wave-1 issue `cbc3a7e5`; created dedicated worktree `/tmp/jit-cdc840ad-integration` on `integration/cdc840ad`.
- Dispatched Terra for the wave-1 declarations/repository-state foundation; committed implementation as `aef6dd26`.
- Rework 1 fixed two stale rustdoc/module links after Cargo CI found the doctest failure; committed as `57e50a8c`.
- First AI review found uncaptured-map and public-path construction/deserialization bypasses; rework 2 hardened capture, path, serde, identity, evidence, seed, claim, and delta invariants; committed as `7443fd0e`.
- Final Cargo CI passed in run `d17e2460-d512-47d1-aece-7fa2eb937b74`.
- Final AI review run `acd90fcb-ef67-4070-97cc-8bd9bb13f9cd` failed on one new high-severity finding: `compare_materializations` converts `RepositoryImage::entry` errors, including `UndiscoveredRepositoryPath`, into drift via `matches!` instead of propagating them.
- Committed all integration-branch gate evidence separately through `bc3cfac5`; no product package has merged to main.
- Pruned generated `/tmp` targets twice (9.3 GiB and 4.4 GiB); only the active source worktree remains under `/tmp`.

## What to do next

- [ ] Resolve the open escalation for `cbc3a7e5`: guidance/reset, manual takeover, or rejection.
- [ ] If guidance/reset is chosen, dispatch a targeted retry that replaces error-masking comparisons with error propagation and sweeps every pure `RepositoryImage` consumer for the same root cause.
- [ ] Re-run focused tests, Cargo CI, cumulative prior-finding audit, and code-review; no prior F1/F2 regression is acceptable.
- [ ] On PASS, complete `cbc3a7e5`, propagate its JIT-only state to main and back to the integration branch, prune generated caches, and begin wave 2 (`bacf2cd4`) with Terra.

## Traps — do not repeat these

- **Do not create worker worktrees under `.agents/` in this sandbox.** Terra's first edit failed because `.agents/worktrees/...` is read-only; the writable checkout is `/tmp/jit-cdc840ad-integration` on the same branch.
- **Do not retain Cargo build output in `/tmp`.** The first install/gate hit tmpfs quota with an 8+ GiB target. Use the disk-backed gate target under the repository, then prune it after the gate; run `cargo clean` in the active temporary checkout after worker verification.
- **Do not run a gate with a stale installed JIT.** The repository's guard rejects any binary whose embedded commit differs from the worktree HEAD. Reinstall with `scripts/install-jit.sh` after each code or gate-evidence commit before the next gate.
- **Do not trust workspace tests to cover rustdoc.** Terra's first full workspace run passed, but Cargo CI caught a stale rustdoc import. Run `cargo test -p jit --doc` explicitly after public ownership moves.
- **Do not treat closed capture as only a constructor property.** The second review found a pure consumer masking `UndiscoveredRepositoryPath` after constructor hardening. Sweep all consumers for `matches!`, `ok()`, `unwrap_or`, or defaulting over image read errors.
- **Do not touch the unrelated dirty main-checkout changes.** At handoff, `.jit/events.jsonl`, issues `0735879a`/`13c69884`, and `crates/jit/src/graph/hierarchy.rs` have external uncommitted edits; stage only this handoff and progress file.

## Open questions needing invoker input

- Question: How should `cbc3a7e5` proceed after exceeding the two-retry limit?
  - Context: Prior high-severity path/capture findings are closed and Cargo CI passes, but the final review found one local error-propagation defect in `compare_materializations`.
  - Options: authorize guidance and reset the retry counter; take over the fix manually; reject `cbc3a7e5` and stop this dependency chain.
  - Recommendation: authorize one targeted guided retry because the remaining finding is concrete and local, while rejection blocks every later epic wave.

## Reference artefacts

- Epic: `jit issue show cdc840ad`
- Design docs: `dev/archive/cdc840ad-plan.md`
- Planning docs: `dev/active/cdc840ad-research.md`, `dev/active/cdc840ad-investigation.md`
- Benchmark/result artefacts: Cargo CI gate run `d17e2460-d512-47d1-aece-7fa2eb937b74`; code-review runs `1b332a0e-5060-448a-9417-089b168b16c3` and `acd90fcb-ef67-4070-97cc-8bd9bb13f9cd`
- External references: None.
