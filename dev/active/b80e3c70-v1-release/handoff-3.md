# Handoff — Version 1.0 release (`b80e3c70`) — session 3

**Date:** 2026-07-29

**Session number:** 3

**Prior handoffs:** `dev/active/b80e3c70-v1-release/handoff.md`

## Current state

- Epic: `b80e3c70` — state: backlog
- Wave in progress: wave 1a of 7; later wave-1 sub-waves remain undispatched because wave discipline requires f04 to settle first.
- Children summary: 7 done, 1 in_progress/escalated, 15 backlog or ready, 0 rejected.
- Active claims: `f04f7888` remains assigned to `agent:worker`, but no worker or issue worktree is active; its exceptional branch is merged and preserved.
- Open escalations: `f04f7888` exceeded two normal reworks plus the explicitly approved exceptional third round. A final stale-changelog finding awaits invoker direction.
- Progress file: `progress.json` in this directory reflects the above.

## What just happened

- Completed `c7f8ebc7`: exceptional commit `7327d1c0` merged as `3cc41bca`; cargo-ci passed with 4,088 tests and code-review passed with zero findings; issue transitioned to Done.
- Filed and wired `625cc07f`, **Strip the development root from container archive destinations**, after reviewing its standalone description against `.jit/reference/content-standards.md`.
- Expanded `625cc07f` REQ-06 after a scope scan found 297 stored archive files and 20 live non-history references using the duplicated development-root shape; a helper-only correction is explicitly insufficient.
- Merged f04 exceptional commit `abc54594`; exact-commit verification passed. Updated stored REQ-02 to the invoker-approved single application-owned deadline contract.
- First exceptional code-review rejected only spaced attribution `jit: f04f7888`. Amended the unpushed merge message to literal `jit:f04f7888`; old `32757166` and amended `f7ca2d03` have identical tree `35fe99260ea81ebd6ca6f5bd2a3e71e8ab1e1e16`; amended commit verification passed.
- A concurrent policy author committed `cea1815c`, adding the `convention-convergence` invariant and its AGENTS projection. It is preserved as a separate commit and was not absorbed into f04 state.
- On current main `cea1815c`, f04 cargo-ci passed with 4,089 tests, zero failures, clean provenance/budget/incremental checks. Code-review accepted attribution and behavior but failed on stale `CHANGELOG.md` wording that still assigns the timeout to axum-server. `jit-validate` remains pending.

## What to do next

- [ ] Obtain the invoker's decision on the f04 post-exceptional escalation.
- [ ] If authorized, change only the f04 changelog paragraph to describe JIT's sole five-second boundary, `graceful_shutdown(None)`, survivor sampling, and `shutdown()` force-close; commit with literal `jit:f04f7888`.
- [ ] Reinstall through `scripts/install-jit.sh`, run exact-commit verification, then run f04 cargo-ci, code-review, and jit-validate sequentially from main. Complete f04 only when all pass.
- [ ] Commit f04 gate/completion state separately, verify `jit validate`, then dispatch wave 1b (`f9e42a43` and `625cc07f`) using the progress-file conflict notes.
- [ ] Continue `625cc07f → ef118aea`, then run whole-tree `a122b9b3` alone before waves 2–7.

## Traps — do not repeat these

- **Do not pop `stash@{0}` expecting the temporary policy stash.** The temporary `convention-convergence` stash was consumed by the concurrent policy author and committed as `cea1815c`; current `stash@{0}` is an unrelated “deletion bug investigation” belonging to someone else.
- **Do not use spaced issue attribution in worker commits.** Review attribution requires literal `jit:<short-id>`; `jit: <short-id>` excluded the shipping patch even though the prose convention looked permissive.
- **Do not describe axum-server as owning the timeout.** The accepted implementation calls `graceful_shutdown(None)` and JIT owns the sole five-second boundary; at expiry JIT samples survivors and calls `shutdown()` only when the count is nonzero.
- **Do not bypass or accept the stale changelog as advisory.** Code-review classifies it as blocking under `@/charter/D-13` and `@/invariant/single-source-prose`.
- **Do not dispatch wave 1b while f04 remains escalated.** The execution-lead wave invariant requires every issue in the current wave to be Done or Rejected before the next wave.
- **Do not rerun cargo-ci with non-empty `target/debug/incremental`.** Focused diagnostics can create it; remove exactly that regenerable cache only after confirming it is the sole postcheck failure.

## Open questions needing invoker input

- Question: May the lead perform one final docs-only repair after the already approved exceptional third f04 rework?
  - Context: shutdown code, deterministic tests, attribution, and cargo-ci pass; only one stale changelog sentence remains, but any further repair exceeds the configured rework allowance.
  - Options: authorize the narrow repair and full gate rerun; provide alternate guidance or take over; reject/defer f04 from v1.0.
  - Recommendation: authorize the narrow repair because it aligns published prose with already accepted shipping behavior and avoids discarding a fully passing shutdown implementation.

## Reference artefacts

- Epic: `jit issue show b80e3c70`
- Escalated issue: `jit issue show f04f7888`
- Archive-layout issue: `jit issue show 625cc07f`
- Progress: `dev/active/b80e3c70-v1-release/progress.json`
- Prior handoff: `dev/active/b80e3c70-v1-release/handoff.md`
- f04 REQ-05 evidence: `dev/active/f04f7888/req-05-verification.md`
