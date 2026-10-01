# Handoff — Mutations commit through a redo record and never scale with project history (f1dc843a) — session 6

**Date:** 2026-09-21
**Session number:** 6 (session 5 left no handoff; its record is the wave 8–10 entries in `progress.json` and the gate-run records it left uncommitted)
**Prior handoffs:** `handoff.md` (session 1), `handoff-2.md` (session 2), `handoff-3.md` (session 4)

## Current state

- Epic: `f1dc843a` — state: backlog (assignee `agent:jit-execution-lead`)
- Wave in progress: **wave 16 of 23**, not yet dispatched. Waves 10–15 closed this session, including story checkpoint `fbd6c859` (Record publication cutover).
- Children summary (all `epic:redo-mutations` issues, epic included): **34 done, 4 ready, 12 backlog, 0 rejected**
- Active claims: none (`jit claim list` reports 0 leases)
- Open escalations: none
- Progress file: `progress.json` in `dev/active/f1dc843a-redo-mutations/`; `current_wave` is 16
- `main` is clean. `cargo-ci`, `docs-mechanical` and `jit validate` green at close.

## What just happened

- Resumed without a session-5 handoff; committed the orphaned `code-review` failure on `4319256e`.
- **Wave 10 `4319256e`** — F1: delta directories were created ahead of the commit point. ESC-09 amended OD-7: the record gained a make-directory operation. One rework round; done. ESC-09 follow-up: epic Notes synced, and `816df12a` gained REQ-07 for the charter's D-17 body.
- **Wave 11 `10ba2e35`** — init publisher. The lead found `apply_uncommitted` (a worktree-only delta over an absent root published with no commit point) and directed a typed refusal on both backends, per epic REQ-05. One rework round; done. 307 production lines against the 250 note, 97 of them REQ-07's allowances.
- **Wave 12 `b0ee2dd6`** — journaled kernel retired, net **−4367** lines, zero rework.
- **Wave 13** — `e6e25f9a`, `79252d23`, `ead8f86d`, `ccea63de`, plus new `b7c99dcb` (POL-01, `.jit-bootstrap` reservation; ESC-10 narrowed its REQ-01 to its own surface). The lead's dispatch prompts described a positioned event append that production does not run (PF-20), which cost reworks on three pages. doc-review then repeatedly claimed the opposite; ESC-11 and ESC-12: the owner chose to drop event-log write mechanics from the storage reference, recorded as `@/issue/ccea63de/decision/D-1`.
- **Wave 14** — `4b765a1e`, new `434a60b7` (POL-01, false "skips the residue" rustdoc), and `42478e5d`. ESC-13: `jit recover` now reports a failed leftover record as `recovery_failed` (owner chose code over docs-only). The troubleshooting guide carries the unreadable-record procedure (SP-25 closed).
- **Wave 15 `fbd6c859`** — all four checkpoint gates passed with 0 findings after the pitfall reconciliation (`checkpoint_reconciliation_fbd6c859`).
- Surfaced-pitfall IDs this session collided with session 5's SP-08..SP-10; renumbered to **SP-17..SP-26**.
- No issue reached `in_progress` until the owner noticed (PF-22); fixed from wave 14 on.

## What to do next

- [ ] **Wave 16, four-way fan-out: `3d9150d7`, `51eb1aa6`, `b4483ad6`, `c88822a5`.** Before each dispatch: link `plan.md`, run `jit issue claim <id> agent:jit-execution-lead`, confirm `in_progress`.
- [ ] Brief **`51eb1aa6`** with SP-26: delete the unreachable "repository absence restored" branch of `jit recover` (`main.rs`, `!storage.root().exists() && transactions_recovered > 0`, and its twin arm near the recovery dispatch) rather than renaming it. Its REQ-05 also re-edits the output example at `docs/reference/worktree-validate.md` (~:480), which now shows "Transactions recovered".
- [ ] Brief **`b4483ad6`**: REQ-06 ("adopter pages stating how the event log grows") now covers `docs/concepts/guarantees.md` (~:100) and not `docs/reference/storage-format.md`, which states no event-log mechanics by owner decision `@/issue/ccea63de/decision/D-1`. Do not add mechanics back to the storage reference.
- [ ] Brief **`c88822a5`** with its three implied consequences (from `handoff-3.md`): per-path read mode in `CaptureSpec`, a captured entry carrying length and tail, and `expected_of` no longer deriving the log preimage from a whole-log digest. These carry epic REQ-02.
- [ ] Carry **ER-01** into `722878d4` (wave 21). Add `b7c99dcb` and `434a60b7` to the exclusion list alongside `ce053478`, `19e7298a`, `2d4fda72`, `48e4f65c`, `7226f00b` and `1fce64b78`.
- [ ] `816df12a` (wave 18) owns the charter's D-17 body (its REQ-07, SP-17); `be13da79` (wave 19) owns the architecture note (SP-08, SP-21).
- [ ] Carry POL-04 (per-wave `cargo-ci` timings); this session's warm suite-clock held at 21.8–22.6 s.

## Traps — do not repeat these

Traps in `handoff.md`, `handoff-2.md` and `handoff-3.md` remain in force. New this session:

- **Do NOT point a documentation worker at the code that consumes a thing.** The lead described `redo_apply.rs`'s positioned event append as production behaviour. `redo_record_for` (`repository_state_store.rs`) passes no event append, and three pages documented a mechanism that does not run (PF-20). Point workers at the constructor, and verify the lead's own paraphrase as you would a worker's claim.
- **Do NOT treat a passed gate as proof of accuracy.** `ccea63de` and `ead8f86d` passed doc-review with the false positioned-append text; the lead's own check caught it.
- **Do NOT dispatch parallel doc tasks that share a canonical home without naming the owning page.** `ccea63de` and `79252d23` wrote the same contract into two pages (PF-21). Name the canonical page in every prompt so the others cite it.
- **When doc-review contradicts verified code repeatedly, do not reword the page to satisfy it.** Get the owner's decision and record it on the issue as a `## Decisions` item (`D-N`). Doc-review and code-review treat a recorded decision as authoritative and report a contradicted premise as advisory.
- **Do NOT dispatch with `jit claim acquire` alone.** A lease does not move an issue to `in_progress`; `jit issue claim <id> agent:jit-execution-lead` does (PF-22). Leases are still needed for `jit gate evaluate` and `jit issue update`.
- **Do NOT re-run a failed gate without `--force` on an unchanged tree.** `jit gate evaluate` returns the memoized verdict at 0 ms (PF-19). Every first `cargo-ci` in a fresh target fails the suite-duration budget with all tests passing; re-run warm with `--force`.
- **Do NOT run gate evaluations concurrently.** A `docs-mechanical` run timed out on the bootstrap lock while `cargo-ci` held it.
- **Do NOT write tracked files while a gate runs, and commit `.jit/` between sequential gates** (PF-23). Otherwise the later run records `Tree: dirty`.
- **Do NOT loop over space-separated pairs with `set -- $pair` in this shell.** zsh does not word-split unquoted variables; use a function taking two arguments.
- **Read the full `surfaced_pitfalls` contents at resume, and check ID uniqueness before adding one.** Session 6 missed session 5's SP-09 until the checkpoint and reused its IDs.
- **When a lead-created task's criterion reaches files owned elsewhere, it is unsatisfiable in its wave** (ESC-10). Scope POL-01 criteria to the issue's own surface.
- **Resolve a `.jit/issues/<id>.json` merge conflict as a union of the two sides, not by re-running `jit doc add`.** The auto-merged `events.jsonl` already carries the branch's events, and re-adding duplicates them.
- **A worker's `.jit/` lags main.** Its pre-commit audit cannot see gate runs the lead recorded after its anchor unless it merges main first.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show f1dc843a`
- Plan: `dev/active/f1dc843a-redo-mutations/plan.md` (OD-7 amended under ESC-09)
- Investigation (REQ-05 construct map; grep, do not read whole): `dev/active/f1dc843a-redo-mutations/investigation.md`
- Progress, findings, escalations: `dev/active/f1dc843a-redo-mutations/progress.json` (ESC-01..ESC-13, PF-19..PF-23, SP-01..SP-26)
- Footprint method for REQ-06: `dev/active/f1dc843a-redo-mutations/footprint.md`, `footprint.py`
- Canonical publication contract: `docs/reference/storage-format.md#publication-and-recovery`
- Unreadable-record procedure: `docs/how-to/troubleshooting.md` (Unreadable Leftover Record)
- Owner decision on the storage reference: `jit item show @/issue/ccea63de/decision/D-1`
