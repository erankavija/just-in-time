# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 4

**Date:** 2026-07-19T01:43:46+03:00
**Session number:** 4
**Prior handoffs:** `dev/active/cdc840ad-handoff.md`, `dev/active/cdc840ad-handoff-2.md`, `dev/active/cdc840ad-handoff-3.md`

## Current state

- Epic: `cdc840ad` — state: backlog
- Wave in progress: wave 2 of 9
- Children summary: 1 implementation child done (`cbc3a7e5`), 1 in_progress (`bacf2cd4`), 9 backlog; planning and breakdown bracket nodes are done
- Active claims: `cdc840ad` assigned to `agent:jit-execution-lead` since 2026-07-18T16:22:26Z (~6h21m at handoff); `bacf2cd4` assigned to `agent:worker` since 2026-07-18T21:14:25Z (~1h29m at handoff)
- Open escalations: None. `bacf2cd4` records rework count 2 because the final retry was dispatched, but the invoker requested this handoff before that retry edited or tested anything; resume that same retry without incrementing the counter.
- Progress file: `dev/active/cdc840ad-progress.json` (reflects the above)

## What just happened

- Ran JIT pre-flight and recovered one stale `.git/jit/locks/claims.lock`; discovered the configured nine-wave bracketed execution plan and resumed wave 2.
- Claimed `bacf2cd4`, committed the claim as `7589b814`, recorded dispatch as `f6b9e187`, and reinstalled the provenance-matched `jit` binary from main.
- Dispatched the first high-complexity worker for `bacf2cd4`; it produced a 2,365-line ambient-path repository transaction engine plus focused tests. Focused tests, 1,925 library tests, 64 doctests, formatting, and Clippy passed.
- Stopped expansion after the invoker warned against over-engineering. Two independent reviews rejected the first attempt for capability-confinement, journal traversal/progress, DeleteFile recovery, lock-order, durability, no-replace, exact-preimage, backend-conformance, and failure-matrix defects.
- Recorded rework attempt 1 as `1bb140bf`; dispatched a constrained rework that deleted the ambient publication engine and extended the existing capability-based `FileTransactionKernel`.
- Rework 1 passed 9 repository-store tests, 25 pure repository-state tests, 327 storage tests (1 ignored), 1,924 library tests (4 ignored), 64 doctests, formatting, workspace Clippy with `-D warnings`, and diff checks.
- Two independent re-reviews still returned FAIL. The current uncommitted diff is 9 files, 3,149 insertions, 18 deletions; gates remain pending and no implementation commit exists.
- Recorded the final retry as `3aca78da`; dispatched rework attempt 2, then stopped it immediately for this handoff. It performed only read-only intake/audits and made no intentional workspace changes. Its nested session-binding helper was interrupted before completion; the diff stat remained unchanged.
- Removed the dedicated rework build output (about 1.5 GiB). No generated build directory remains from the workers.

## What to do next

- [ ] Resume wave 2 by continuing `bacf2cd4` rework attempt 2 without incrementing `rework_counts.bacf2cd4`; the attempt was opened but no implementation work occurred.
- [ ] Before editing, inspect the complete combined workspace with `git diff HEAD` and `git status --short`; the first attempt left staged entries and rework 1 left unstaged corrections, so plain `git diff` or `git diff --cached` alone is incomplete.
- [ ] Fix root/session binding: acquire bootstrap serialization before capability discovery, rediscover/reopen after external recovery, enforce same-layout retained reentry, bind the absent-root parent identity, and revalidate current path-to-capability identities immediately before journal creation.
- [ ] Give Worktree and Data actions same-filesystem staging/backup authorities so mixed deltas work when an existing disjoint data root is on another filesystem.
- [ ] Reverify every staged Data action immediately before the absent-root no-replace rename and verify every final action afterward.
- [ ] Validate decoded journals before recovery: bounded role-specific control names, containing-directory ID equality, unique actions/paths, layout/identity binding, and cleanup only through the validated directory ID.
- [ ] Make partial control/journal creation recoverable, including missing `stages`/`backups`, complete `journal.next`, and data-stage ownership persisted or deterministically derived before stage creation.
- [ ] Implement D6 literally: prepare and sync verified backups before delete/replace, use same-handle identity checks for replacement-sensitive operations and `SetMode`, never remove a post-crash occupant, and treat dangling symlinks as occupants.
- [ ] Treat missing target ancestors as captured absence; reject cross-root physical aliases; record true Git blob OIDs for HEAD fallback; map unsupported no-replace to the typed filesystem error.
- [ ] Replace backend-specific happy-path tests with one JSON/memory conformance matrix covering identical canonical semantics, action order/hash, nested/disjoint existing/absent roots, all action kinds, aliases, every declared failure edge, rollback/no-swap, forward recovery/cleanup, and occupied-root races.
- [ ] Narrow storage-only visibility and remove inert or contradictory helpers; do not add downstream mutation/materialization/consumer work.
- [ ] Re-run focused, storage, full library, doctest, formatting, workspace Clippy, and diff checks; then perform an independent lead review before staging or committing implementation.
- [ ] If rework attempt 2 still fails review after actual implementation, escalate under MAX_REWORK_ATTEMPTS rather than dispatching another retry.

## Traps — do not repeat these

- **Do not implement a second ambient-path transaction engine.** The first attempt passed all mechanical checks but duplicated `FileTransactionKernel` and retained symlink/TOCTOU, journal-traversal, and recovery hazards. Rework 1 correctly moved publication into the existing capability kernel; preserve that direction.
- **Do not treat green tests as evidence that REQ-01/REQ-02 are met.** Both attempts passed their authored suites, yet independent review found concrete counterexamples: cross-filesystem mixed deltas use Data-hosted hard links into Worktree, root capabilities can detach, initial journal residue can wedge recovery, and staged root contents are not reverified before commit.
- **Do not patch one recovery finding at a time without a shared failure matrix.** Rework 1 closed durable progress and several identity checks but left untested durability edges and backend divergence. Write the cross-backend/crash matrix first, then make the smallest implementation corrections it demands.
- **Do not broaden the final retry into later waves.** Typed mutation/audit/claim synchronization belongs to `a6a9b964`; materializers/drift/repair to `44d318ab`; consumer migration/deletion to `49adf23b`. Existing init/profile consumers may remain on their predecessor kernel until wave 5; add no new old-path use or wrapper.
- **Do not accept raw control names from journals.** `ControlName` removed traversal, but current recovery still fails to require `journal.transaction_id == containing directory id`; cleanup can target a different valid directory. Validate before any recovery action.
- **Do not form DeleteFile backups only during publication.** The current rework checks a path and then renames by name, leaving an occupant-swap race and violating D6's prepared synchronized-backup contract. Prepare and verify the backup before publication, and use same-handle identity checks.
- **Do not inspect only one side of the index/worktree split.** Current source changes are staged-and-unstaged (`MM`/`AM`). Use `git diff HEAD`; do not commit until the final retry is reviewed and the complete desired tree is staged intentionally.
- **Do not count the handoff-stopped final retry as an exhausted implementation attempt.** Rework count is already 2 because dispatch increments it, but attempt 2 made no edits. Resume it once; escalate only if that actual correction fails review.
- Prior traps remain in force: `dev/active/cdc840ad-handoff.md`, `dev/active/cdc840ad-handoff-2.md`, and `dev/active/cdc840ad-handoff-3.md`, especially stale installed JIT binaries, no full builds in `/tmp`, explicit doctests after ownership moves, and branch-on-main rather than the retired integration branch.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show cdc840ad`
- Active issue: `jit issue show bacf2cd4`
- Design docs: `dev/active/cdc840ad-plan.md`
- Planning docs: `dev/active/cdc840ad-research.md`, `dev/active/cdc840ad-investigation.md`
- Progress: `dev/active/cdc840ad-progress.json`
- Benchmark/result artefacts: no `bacf2cd4` gate runs yet; worker validation results are summarized above
- Relevant state commits: `7589b814`, `f6b9e187`, `1bb140bf`, `3aca78da`
- External references: None.
