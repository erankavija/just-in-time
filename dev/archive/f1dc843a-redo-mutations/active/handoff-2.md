# Handoff — Mutations commit through a redo record and never scale with project history (f1dc843a) — session 2

**Date:** 2026-09-20
**Session number:** 2
**Prior handoffs:** `dev/active/f1dc843a-redo-mutations/handoff.md` (session 1)

## Current state

- Epic: `f1dc843a` — state: backlog (assignee now `agent:jit-execution-lead`)
- Wave in progress: **wave 7 of 23** — the first story checkpoint `21e976d1` is **done**, all three gates green
- Children summary: **16 done, 31 backlog** of 47
- Active claims: none. `21e976d1`'s indefinite checkpoint lease was released.
- Open escalations: **none**. All eight are closed; four were decided by the user this session (ESC-05..ESC-08).
- Progress file: `progress.json` in `dev/active/f1dc843a-redo-mutations/` — waves, `surfaced_pitfalls` SP-01..SP-07, `verifications` V-01..V-11, `process_findings` PF-01..PF-10, `escalations` ESC-01..ESC-08, `standing_policies` POL-01/POL-02, `evidence_requirements` ER-01/ER-02, and `checkpoint_reconciliation`.

## What just happened

- Resolved both session-1 blockers. **SP-05**: user chose to amend `c88822a5` REQ-03 to length-and-tail event-log capture (ESC-05); epic REQ-02 untouched; corrected two false premises in that issue's Background and linked the spike as its evidence. **V-04**: user chose to hold the injector (ESC-06), resetting `35544121`'s rework counter.
- `35544121` **done**. Held-injector rework merged; re-running `code-review` against the new commits then found a nested-put durability defect three prior rounds had missed (`create_dir_all` syncing only the deepest directory). Fixed via `open_or_create_capability_parent`. Two rework rounds.
- `4e818d10` **done**. Worker correctly refused the lead's instruction to delete the memory kernel modelling (PF-06). User approved a `## Non-goals` section (ESC-07). One rework round for a cross-backend divergence: memory overwrote a directory entry where the filesystem fails `EISDIR`.
- **`8559f529` created and done** — new task, record-level overlap validation, wired ahead of `d591a1c8`. Two rework rounds: a checked constructor beside `pub` fields is bypassable (V-08), then a lost no-rollback property (V-09).
- `d591a1c8` **done**. One conformance suite over both backends; both per-backend crash-matrix proptests deleted as subsumed. One rework round: REQ-02's pre-durability property was implied by two assertions but owned by no test (V-10).
- Amended `21e976d1` REQ-06 (ESC-08): the phase total binds (1137 of 1500), per-task figures stay estimates reported against. Created `footprint.md`, linked to `21e976d1`.
- Closed SP-06 (stale `#[allow(unused_imports)]` in `storage/mod.rs`) inside `d591a1c8`'s last rework rather than as a separate cycle.
- Performed the checkpoint pitfall reconciliation **before** the gate (recorded under `checkpoint_reconciliation`). It surfaced two items that would otherwise have failed the gate; both are closed.
- **`21e976d1` done — wave 6 closed.** `cargo-ci` and `doc-review` passed first time. `holistic-review` failed once, on the lead's own evidence artifact rather than any worker's code: REQ-06's figures were self-attested with no reproducible command (V-11). Fixed by committing `footprint.py`, quoting its verbatim output, and re-running both AI gates — `holistic-review` then passed with zero findings.

## What to do next

- [ ] **Open wave 7** — `08b8c0ef`, `581bddcf`, `7658bf81`, `f05a7965`. All four depend only on `21e976d1`, which is done, so all four are dispatchable at once. This is the **first genuine 4-worker fan-out**; waves 1–6 were serial or paired, so the conflict heuristics and the cache-pool trap (one harvest per wave — see session 1's traps) get their first real exercise here.
- [ ] Wave 8 (`8d7f4cec`) depends on `581bddcf` **and** `7658bf81`; wave 9 (`0938d99e`) on `8d7f4cec`; wave 10 (`4319256e`) on `08b8c0ef` **and** `0938d99e`. So the spine narrows again immediately after wave 7 — dispatch all four now rather than staging them.
- [ ] Carry **ER-01** into `722878d4` (wave 21): report the REQ-06 balance attributed, excluding `ce053478`, `19e7298a`, `2d4fda72`, `48e4f65c`, `7226f00b`. **`8559f529` is NOT excluded** — it is redo-record work and counts in the subtotal. Reuse `footprint.md`'s method; do not re-derive it.
- [ ] `c88822a5` (wave 16) now carries three implied consequences in its Notes: a per-path read mode in `CaptureSpec`, a captured entry carrying length+tail (`EventLogTail` is private today), and `expected_of` no longer deriving the log's preimage from a whole-log digest. Verify all three at review; they are the load-bearing part of epic REQ-02.
- [ ] `8b6a9e34` (wave 22) still owns SP-01's live half: `dev/TESTING.md:471` cites an archived benchmark path plus the `transaction-benchmark-evidence` contract that LD-4 retires.
- [ ] `d591a1c8` scoped SP-07 (`ELOOP` vs `Unaddressable`) rather than closing it. It is unclosable by design — the memory backend has no symlinks. Leave it; it surfaces in the completion report.

## Traps — do not repeat these

Session 1's trap section remains in force in full (`handoff.md`). Every one of its eleven traps still applies; three were re-confirmed this session (reviewers have no memory across rounds; LD-7's cold-run budget trip, hit once at 31.8 s then 22.4 s warm; criteria drafted broader than intent). New traps:

- **A `passed` gate result does not say which tree it judged.** `jit issue show` reports `gates[].status` with `last_run_at` and nothing tying it to HEAD. `35544121`'s `code-review` read `passed` from a run 2 hours stale, after the module it reviewed had been rewritten — 130 lines in the exact file its only finding pointed at. Accepting it would have closed the issue on a review of code that no longer existed. **Re-evaluate every AI-review gate after every merge, and check the run's `Attribution:` line covers the new commits.** (PF-05)
- **A checked constructor beside `pub` fields is not a constructor, it is a suggestion.** `8559f529` first delivered `RedoRecord::new` with validation while the fields stayed public, so a struct literal still built the record the task existed to forbid — and its rustdoc asserted the invariant as fact. Contrast `835bae3f`'s root-class rejection, which is unbypassable because it lives in `VirtualPath`'s constructor. **Matching a check's placement is not matching its unbypassability.** (V-08)
- **A green build does not prove no bypassing construction site remains.** When a test both constructs a type by struct literal and later reads a field, rustc reports only the later `E0616` and silently drops the `E0451` for the construction — reproduced in an isolated crate. Fixing every reported error and building clean proves nothing. **Grep the construction form.** (PF-08)
- **Do not instruct a worker to delete work the plan assigns to a later task.** The lead's `4e818d10` dispatch said to delete the memory kernel modelling, citing the issue's Notes line "against roughly 340 lines of kernel modelling this replaces" and `@/inv/canonical-cutover`. Both misread: the Notes line frames the **epic's** net balance, and plan decision **LD-11** is itself the named versioned boundary that invariant wants, naming `b0ee2dd6` as the removal condition. Following it would have broken `21e976d1` REQ-03. **Read the parent story's criteria and grep the plan's decision table before composing a dispatch.** (PF-06)
- **A task's Notes footprint estimate is not a budget, and a worker will size to it.** `8559f529`'s "roughly 30 production lines" came from an analysis that costed the comparison logic and not the encapsulation; the worker delivered exactly that and produced the bypassable check above. **State the property, not the line count — and if you state a number, say it is descriptive.** (V-08, ESC-08)
- **Deleting a test can silently drop a property its replacement does not carry.** `8559f529` made two records unconstructible and redesigned their tests correctly, but the replacements were single-put records, so "a refusal leaves the record's earlier operations applied" lost its only owner. A backend that rolled back on refusal would have passed every test in the tree. **Diff the test inventory before and after, then read each deleted body assertion by assertion.** (V-09)
- **A mutation experiment leaves artifacts beyond the source edit.** A deliberate mutation that fails a proptest makes proptest write a regression seed, which survives reverting the source. `d591a1c8` left one; a later reader would have hunted a defect that never existed in the tree. **Check for and remove harness-recorded state when reverting a mutation.**
- **Evidence for a criterion must be reproducible, not asserted.** `holistic-review` failed the checkpoint on `footprint.md`: the lead had written its measurement method into prose and quoted figures from an ad-hoc script it never committed, so the numbers were self-attested. The amended REQ-06 requires each footprint to be *recorded* against its estimate, and a figure nobody can regenerate is an assertion. **Commit the generator beside any measurement a criterion rests on, quote its verbatim output, and parameterise it for the later consumer** — `footprint.py` now takes `--tasks` so `722878d4` gets ER-01's attributed subtotal without re-deriving anything. `bench.sh` already set this precedent for epic REQ-01/REQ-02. (V-11, PF-10)
- **A naive production-line count is wrong in both directions at once.** Cutting each file at its first `#[cfg(test)]` counts `redo_conformance.rs` whole as production (it is gated by `#[cfg(test)] mod` on its **declaration** in `mod.rs`, not inside the file) and truncates `memory.rs` at line 146, where two `#[cfg(test)]` function attributes sit above 200-odd production lines. It reports 1428 where the answer is 1137. Cumulative additions across a task's commits measure churn, not footprint — `35544121` added 413 and removed 93 of its own, delivering 320. **Brace-match each `#[cfg(test)]` item's extent and report net per task; the method is written up in `footprint.md`.** (PF-09)
- **The leak check reports the lead's own uncommitted gate-run records as suspected worker leaks.** Its snapshot is taken at dispatch; `jit gate evaluate` then writes `.jit/gate-runs/*/result.json`, `events.jsonl` and the issue file on main. They are not leaks. **Commit tracker state immediately after each gate evaluation** so the next wave's check has a clean baseline. Also: do not write `git status --porcelain && echo TREE-CLEAN` — the echo is unconditional and prints a clean verdict over a dirty tree. (Lead error this session.)
- **`jit claim renew` does not take an issue id or `--ttl`.** It takes a **lease id** and `--extension`, unlike `acquire`/`release`, which take an issue id. `jit claim renew 21e976d1 --ttl 3600` fails with only "For more information, try '--help'".
- **Without `JIT_AGENT_ID`, jit behaves three different ways.** `claim acquire` refuses outright; `doc add` writes an event with `updated_by: "doc-add"` (a command name in a field whose other values are actor identities); `claim release` silently falls back to the git user. **Export `JIT_AGENT_ID=agent:jit-execution-lead` in every shell** — it does not persist between tool calls.
- **Do not let a worker write `.jit/` at all.** `write_policy = "allow"` in `.jit/config.toml` means worker worktrees can mutate the tracker, which collided in session 1. This session's dispatches forbid `jit doc add` and every other `.jit/` write; workers report artifact paths and the lead links them from main. Keep that.

## Open questions needing invoker input

None. The `21e976d1` checkpoint closed cleanly, all eight escalations are resolved, and wave 7 is dispatchable without any further decision.

## Reference artefacts

- Epic: `jit issue show f1dc843a`
- Plan: `dev/active/f1dc843a-redo-mutations/plan.md` (contracts, OD-1..OD-10, LD-1..LD-11 — **LD-11 places kernel retirement in the cutover phase**)
- Investigation: `dev/active/f1dc843a-redo-mutations/investigation.md` (long — grep, do not read whole)
- Breakdown manifest: `dev/active/f1dc843a-redo-mutations/breakdown.json`
- Progress + all findings: `dev/active/f1dc843a-redo-mutations/progress.json`
- **Footprint method and REQ-06 evidence:** `dev/active/f1dc843a-redo-mutations/footprint.md`
- **REQ-02 measurement (SP-05):** `dev/active/7226f00b/whole-log-read-cost.md`
- Capture-ceiling survey: `dev/active/48e4f65c/event-log-capture-ceilings.md`
- Prose relocation evidence: `dev/active/2d4fda72/relocation-evidence.md`
- Benchmark harness: `dev/active/f1dc843a-redo-mutations/bench.sh` (pre-epic baseline: label update 124 ms at 1x, 372 ms at 10x)
- Conformance suite: `crates/jit/src/storage/redo_conformance.rs`
- REQ-06 baseline: `git diff --shortstat 34d37f8be -- '*.rs'`
