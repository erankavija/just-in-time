# Handoff — Mutations commit through a redo record and never scale with project history (f1dc843a) — session 7

**Date:** 2026-10-01
**Session number:** 7
**Prior handoffs:** `handoff.md` (session 1), `handoff-2.md` (session 2), `handoff-3.md` (session 4), `handoff-4.md` (session 6)

## Current state

- Epic: `f1dc843a` — state: backlog (assignee `agent:jit-execution-lead`)
- Wave in progress: **wave 16 of 23**, with wave 17 partly dispatched early (see below)
- Children summary (`epic:redo-mutations`, epic included): **37 done, 3 in_progress, 10 backlog, 0 rejected**
- In progress, each with a worker that was **still running when this handoff was written**:
  - `c88822a5` (wave 16) — rework **2 of 2** (the last before escalation), worker in `.agents/worktrees/agent-c88822a5`
  - `ec72359b` (wave 17, Sonnet) — first dispatch, `.agents/worktrees/agent-ec72359b`, anchored `027d013a8`
  - `e346d387` (wave 17, Opus) — first dispatch, `.agents/worktrees/agent-e346d387`, anchored `027d013a8`
- Open escalations: none (ESC-14, ESC-15 resolved this session)
- Progress file: `progress.json` (`session` 7; `wave_16_dispatch`, `wave_16_observations`, `rework_counts`, ESC-14/15, PF-24, SP-27/28, POL-05)

## What just happened

- Pruned build artifacts before dispatch: the disk was 93 % full; the cache pool held 71 GB of stale rustc incremental sessions hardlinked into every new worktree. Freed to 145 GB. The user's instruction is recorded as **POL-05** (watch disk, keep throughput).
- Wave 16 dispatched as a 4-way fan-out. `b4483ad6` and `c88822a5` overlap, so `c88822a5` built the phase collapse first, waited (`WAITING-FOR-b4483ad6`), then merged main.
- **`51eb1aa6` done.** Zero rework. Recover key is `redo_records_applied`; the unreachable SP-26 branch was deleted. The lead removed a stale test comment raised as advisory.
- **`3d9150d7` done.** Two rework rounds:
  - R1: `src/bin` targets were missed; roots now come from `cargo metadata`.
  - R2: helper writers escaped the per-function scan; the guard is now inverted, so every production filesystem writer is a publisher, a classified non-repository target, or a confined helper.
- **`b4483ad6` done.** One rework round: doc-review found pages saying every file is replaced. ESC-14: the owner recorded `@/issue/ccea63de/decision/D-2` (one clause on the canonical page), superseding D-1.
- **`c88822a5`** is merged on main through rework 1 (`5c7f7e261`).
  - R1-F1, a long unterminated final line misread as torn, is fixed.
  - Rework 1's fix read the log whole; code-review R2-F1 failed that against REQ-03.
  - ESC-15: the owner chose a tail that grows to the final line's start, recorded as `@/issue/c88822a5/decision/D-1`.
  - Rework 2 was dispatched with that decision.
  - `cargo-ci` on the rework-1 merge failed only the six known cold-run timeouts.
- The user directed the use of `BUSY_HOST_OVERRIDE=1` (`scripts/rust-build-budget.sh:170`) on this contended host, **explicitly not as a standing policy**. Every gate run this session used it from that point on.
- Wave 17 was started early on the user's max-throughput instruction, because `ec72359b` (dep `51eb1aa6`) and `e346d387` (dep `b4483ad6`) were already unblocked. `33f97287` and `8454236c` wait on `c88822a5`.

## What to do next

- [ ] **Check the three workers before anything else.** Use `ListAgents` and `git -C .agents/worktrees/agent-<id> status` and `log`.
  - If a worker died with uncommitted edits, make a lead-preserve commit per `worktree-dispatch-protocol.md`, but only once it is certainly not running (PF-16).
  - If a report arrived after this handoff, review it normally.
- [ ] **`c88822a5` rework 2 review.** Run the Tier 1.5 table:
  - R1-F1 must stay closed: four tests, including `test_create_issue_appends_after_a_valid_final_line_longer_than_the_tail_window`.
  - R2-F1 must be closed: no whole-log read in `capture_proposed_base_inner` (`commands/validate.rs` ~:388), and the long-line case reads only the final line, per D-1.
  - REQ-07 net removal must still hold, with no further tests deleted for the budget (PF-24).
  - If code-review fails a third time, **escalate**: the rework cap is reached.
- [ ] Then gate `c88822a5` (`cargo-ci`, `code-review`), close it, and dispatch `33f97287` and `8454236c` (both depend only on it). Link `plan.md`, run `jit issue claim`, and create worktrees first.
- [ ] Review `ec72359b` (plan hash retirement, REQ-05 net removal) and `e346d387` when they report.
  - `e346d387` was told to **report, not reinterpret**, two live publication-sense terms: init's "staging" (`storage/init_publish.rs`) and `ExpectedPreimage` in the conflict check.
  - REQ-04 may be unsatisfiable as written. Expect an owner question (escalation category 4).
- [ ] When wave 16/17 closes, reclaim worktrees: harvest **one**, remove the rest (PF-03), then delete `.agents/cache-pool/target/debug/incremental` (POL-05). Worktrees to reclaim: 51eb1aa6, 3d9150d7, b4483ad6, plus c88822a5, ec72359b and e346d387 once merged.
- [ ] Carry **ER-01** into `722878d4` (wave 21). The exclusion list is unchanged from `handoff-4.md`.
- [ ] Carry SP-28 to the completion report: `repo_lock::acquire` and `worktree_identity` can create an absent `.jit/`. Both predate the epic and are outside `3d9150d7`'s surface; they are a candidate follow-up.

## Traps — do not repeat these

Traps in `handoff.md`, `handoff-2.md`, `handoff-3.md` and `handoff-4.md` remain in force. New this session:

- **Do NOT guess a nextest profile name.** The lead "warmed" the suite with `--profile ci`, which does not exist (`profile ci not found`). The warm-up did nothing, and the next gate run was cold. The gate's own command is `cargo nextest run --workspace` (`scripts/cargo-ci.sh:452`).
- **Do NOT let stale incremental caches ride the cache pool.** `reclaim-worker-worktree.sh` merges `target/debug/incremental` newest-wins, and `dispatch-worker-worktree.sh` hardlinks the pool into every new worktree. The pool had grown to 71 GB of dead sessions. Delete the pool's and fresh worktrees' `target/debug/incremental` before workers build.
- **The zsh shell here aliases `ls` (eza) and breaks `tail -N` on multiple files.** `ls -t` failed as an eza `--time` flag, and `tail -1 a b` failed with "option used in invalid context". Use `command ls` and one `tail` per file.
- **Do NOT tell a worker to keep a page untouched under an owner decision whose premise its own change invalidates.** The `b4483ad6` dispatch forbade editing `storage-format.md` under D-1, and D-1 was premised on whole-file replacement, which that task removed. doc-review then failed the page. Read a decision's stated premise before citing it as a constraint.
- **A rework brief must name the criteria that bound the fix (PF-24).** `c88822a5` rework 1 fixed R1-F1 by a whole-log read, which broke REQ-03, and it also deleted three tests "to keep REQ-07".
- **Do NOT treat a passing guard as complete when its own rustdoc states a limit.** `3d9150d7`'s rustdoc said a root handed to a helper escapes the scan, and code-review failed exactly that. A stated limitation against a criterion with no exception is an open finding.
- **The host is shared with an unrelated `gf2` project's builds.** Load ran 15–25 with only two jit workers, so suite-clock exceeded 30 s on nearly every run. Every cold run after a merge timed out the same six tests: 4 `fast_rules` cli_warnings and 2 `test_utils` profiled fixtures. Re-run once; a different signature is real.

## Open questions needing invoker input

- Question: **Should `BUSY_HOST_OVERRIDE=1` continue next session?**
  - Context: the user authorized it for this session's contended runs and explicitly declined to make it a standing policy.
  - Options: (A) ask the user at session start; (B) run without it and re-run on a quiet host.
  - Recommendation: **A.**

## Reference artefacts

- Epic: `jit issue show f1dc843a`
- Plan: `dev/active/f1dc843a-redo-mutations/plan.md`
- Progress: `dev/active/f1dc843a-redo-mutations/progress.json`
- Owner decisions this session: `@/issue/ccea63de/decision/D-2` (event-log clause on the storage reference), `@/issue/c88822a5/decision/D-1` (tail bounded by the final line)
- Canonical publication contract: `docs/reference/storage-format.md#publication-and-recovery`
- Source guard: `crates/jit/tests/scratch_build/repository_state_cutover_tests.rs` (`NON_REPOSITORY_WRITES`, `scan_filesystem_writers`)
- Footprint method: `dev/active/f1dc843a-redo-mutations/footprint.md`, `footprint.py` (wave 16: `b4483ad6` +5, `51eb1aa6` −65, `c88822a5` −90 before rework 2)
- REQ-02 measurement: `dev/active/7226f00b/whole-log-read-cost.md`
