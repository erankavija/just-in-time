# Handoff — Compute one validation report and derive every validation view from it (9876a23a) — session 1

**Date:** 2026-11-04
**Session number:** 1 (lead session just-in-time-a4; successor just-in-time-b9)
**Prior handoffs:** none. The planning record is [plan.md](plan.md), the audit that amended it is [audit.md](audit.md), and the state of record is [progress.json](progress.json).

## Current state

- Epic `9876a23a`: backlog, assigned `agent:9876a23a-lead`. The invoker is just-in-time-6a, relaying the user; 6a also rules as steward while the user is away.
- Waves 1–6 are closed. Wave 7 has one issue open:
  - 612ee9d4 (report selection) is in its landing pipeline. Its merge conflict was resolved by its worker (1bb189565); then branch CI on `/dev/shm`, landing, cargo-ci and code-review.
  - 03c59566 is done in substance but shows `gated`: a third gate, `mcp-ci`, was added after its close. Its evaluation is running.
- Children: 25 done, 2 rejected (145b4d57, e16493fc, both superseded), 1 in progress (612ee9d4), 1 gated (03c59566), 2 ready (stories 420bc076, 1f24e3e9), 16 backlog.
- Also filed by this session outside the epic (no epic label, under milestone 9db27a3a):
  - 2a10917d: code-review context states commit ancestry. Open.
  - 14f49a28: M8 docs check assembles packages outside the repo. Done.
- Active claims and leases: the lead's lease on 612ee9d4 and 03c59566 while their pipelines run. Release them when done.
- Open escalations: none.
- Progress file: [progress.json](progress.json). Its `traps`, `surfaced_pitfalls` and `line_target` are current.

## What just happened

- **Audit, decisions, amendment.**
  - The plan was audited against 120402067 ([audit.md](audit.md)).
  - The user approved decisions 1–6 and the c92f4011 option A2.
  - The amendment ran through jit-planning-lead and is recorded in plan.md. plan-review then passed (0 findings), and breakdown-review passed in round 2 after restoring ea843d26 → de9a7557.
- **Done by wave:**

| Wave | Issues |
|---|---|
| 1 | d40d6f74, 7eb443b2, bd2534bd, 6100b960 |
| 2 | 70ac98fc, cf49da17, c92f4011 |
| 3 | 977d01d8, ea843d26, b3a768b3, f968dfe1 |
| 4 | 5d5a1794, 8b56c215, ccd6e1fb |
| 5 | 03c59566 (mcp-ci pending), 001a9c46 |
| 6 | 6ba4f623 |
| 7 | 3acaa5a3, 2b5f452c, c1af05ac, e2082ba9 (bug found by c1af05ac; regression from 001a9c46) |

- **Steward rulings**, each recorded as an issue `## Decisions` item and in plan.md:
  - c92f4011 D-01 (A2): default rules are always in force; `severity = "off"` on the row disables one; `enforce = false` does not.
  - 5d5a1794: REQ-02 narrowed to mutation paths. D-01: `issue create --orphan` removed. f368ce3d's orphan criterion was deleted (cross-epic).
  - 03c59566: criteria trimmed to one renderer, with findings only in the `--json` response.
  - 3acaa5a3 D-01: `jit invariant check` reads declarations through lock-free storage loads (`@/charter/D-22`). b697a3f7's 578ed10d gained REQ-08, which retires those loads onto the read driver.
  - 2b5f452c D-01: `--fix` repair planning decodes nothing; the other `--fix` steps converge in 1f24e3e9, whose REQ-01 was tightened. `jit recover` no longer depends on issue records.
  - 2b5f452c D-02: a repeated key is rejected exactly where the typed decode rejected it (repeated issue field with position; unknown or map key loads).
  - 70ac98fc: one authorised code-review rerun with an evidence note ([evidence-70ac98fc-ci-ancestry.md](evidence-70ac98fc-ci-ancestry.md)).
- **Cross-epic graph changes:**
  - The edge was swapped to 725af076 → cf49da17 (cf49da17 first; 3e's side done).
  - Added 578ed10d → f22147ec (reduction dropped two implied b697a3f7 edges).
  - The DEC-09 claims-index bridge was dropped: f8a6b298's 2f563673 deleted the check first.
- **`.gates.lock`** was removed in full inside 5d5a1794, agreed with just-in-time-3e.
- **Net production lines per wave** (`scripts/production-lines.py --test-support-as-test`, measured on each landing merge's first-parent diff):

| Wave | Net lines |
|---|---|
| 1 | +3 |
| 2 | −1 |
| 3 | −42 |
| 4 | +35 |
| 5 | +55 |
| 6 | −32 |
| 7 (so far, without 612ee9d4) | +178 |
| **Total** | **+196** |

  Wave 7 by issue: 2b5f452c +149, 3acaa5a3 +9, e2082ba9 +20, c1af05ac 0. The audit estimates about −1,050 at epic end. The deletions are still ahead: the views and checkers story (2c8c13b2, 208edf4c, 86f7e6eb, a106d1be, 6ac41880, f22147ec). Per-issue figures are in progress.json `line_target`.

## What to do next

- [ ] Collect 612ee9d4's pipeline result. Its log goes to `.agents/worktrees/ci-612ee9d4.log`; on success it lands and evaluates cargo-ci and code-review. On pass: close it, then reclaim its worktree with `.agents/skills/jit-execution-lead/scripts/reclaim-worker-worktree.sh 612ee9d4`. On fail: lead review and rework (count 1 so far).
- [ ] Collect 03c59566's `mcp-ci` result. On pass, set it done.
- [ ] Close wave 7. Record 612ee9d4's line delta in progress.json `line_target`.
- [ ] Wave 8 (ready once 612ee9d4 is done): 2c8c13b2 (per-issue and explain views), 86f7e6eb (rule view), 208edf4c (scope view), 728ecfb0 (selection property test). Check `jit issue search "" --label epic:one-validation-report --state ready` first.
  - Use a Claude worker for 2c8c13b2 and 208edf4c (they rewrite `commands/validate.rs` views); Codex gpt-6.1-sol or gpt-6-luna for 86f7e6eb and 728ecfb0.
  - Each view must read no event-log history or gate-run listing (DEC-17).
- [ ] Before any 2c8c13b2 or `--fix` work, check df129d9e's 06ee9b67 (validate handler) status with just-in-time-88. The second lander adapts.
- [ ] 1f24e3e9 (`--fix` in one mutation) is ready. Its REQ-01 now requires the one capture's parsed records.
- [ ] 612ee9d4 pitfall: 6ba4f623's report adds the `derived-materialization validation pass:` prefix only when history is supplied. The selection work should let callers choose that wording; see `surfaced_pitfalls`.
- [ ] Standalone 2a10917d (code-review commit ancestry) is unassigned. Dispatch it only if the invoker asks; it isn't part of this epic.

## Traps — do not repeat these

All entries in progress.json `traps` stay in force. The ones that cost most:

- **`dispatch-worker-worktree.sh` refuses a dirty main**, including other leads' tracker files. Commit a `chore(jit): sync tracker` right before dispatch, and check the `[ok] main at` line before writing briefs. Twice a failed dispatch went unnoticed.
- **Never leave an untracked or modified non-`.jit` file in main's checkout.** It blocks every lead's `land.sh` ("uncommitted changes outside .jit"). Edit plans in scratch, then copy and commit in one command.
- **Brief generation:** use `<<'EOF'` for any text containing backticks. An unquoted heredoc command-substituted the contract names (6ba4f623 brief). A `printf` header with an empty `$M` shifted its arguments (wave 7). Check each brief's header before dispatch.
- **Never wrap `land.sh` in `jit worktree exec`.** It takes the repository lock itself, so nesting deadlocks.
- **Authoritative runs** (branch CI and cargo-ci gates) are fixed by steward rule:
  - Run `TMPDIR=/dev/shm/jit-ci/9876a23a` (created before, removed after; never `/dev/shm/sccache-tmp`), under `flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock`.
  - Use sccache only if `ps -C sccache` finds a daemon; otherwise set `CARGO_CI_NO_SCCACHE=1` and tell 6a.
  - The load wait is capped at 30 minutes.
  - All of this is implemented in [land-pipeline.zsh](land-pipeline.zsh): `zsh land-pipeline.zsh <id> [gates…]`, `FORCE=--force` for a forced gate. An ENOSPC writes a stop file that halts the chain, and you then tell 6a.
- **`/tmp` ENOSPC is inode exhaustion** (1.0M inodes, about 473K of them in leaked `/tmp/.tmp*` test directories), not bytes. Cleanup is the user's decision.
- **Worker runs are advisory.** They use `TMPDIR=/mnt/nvme/jit/tmp/9876-workers` with `CARGO_INCREMENTAL=0`. There, `test_check_gate_classifies_named_run_drift_but_not_unrelated_run` fails (ext4 directory order; fixed by 944edb32) and fsync-heavy profile, lease and interruption tests time out. Workers report these and don't rework.
- **A cargo-ci gate that fails instantly** with `exit_code: null` and empty output is reused from an earlier failed run (`origin.derivation: reused`). `--force` is allowed only when the reused run failed for a host fault (ENOSPC, TIMEOUT-only, the drift flake) and the code under test is the right one. Name the reused run id in plan.md (row "Forced gate re-evaluations"). Never force past a content failure.
- **A nextest TIMEOUT on a millisecond test** is the host pool runner: rerun, never rework.
- **Codex workers never run a git write** (`merge`, `pull`, `commit`): git metadata is read-only in their sandbox. The lead merges main and commits their diff with `git add -A crates`.
- **A whole-workspace nextest run is required** for any behaviour change. c92f4011 passed its focused suites and failed two fixtures in the full run.
- **Review verdicts are final.** Don't rerun a review unchanged. A wrong finding goes to 6a with facts. Each change must be justified on its own merit, never to pre-empt a reviewer (user principle).
- **Criteria the plan wrote can be unmeetable** (5d5a1794 REQ-02, 3acaa5a3 REQ-01 against D-22, 03c59566 after 5d5a1794). Check every criterion against current main before dispatch, and take changes to 6a.

## Open questions needing invoker input

None pending. Waiting on the user, through 6a: `/tmp` cleanup (inodes), and the format-4 cutover window (a fresh stop request will come; stop all tracker writes when it does).

## Reference artefacts

- Epic: `jit issue show 9876a23a`; plan [plan.md](plan.md); manifest [breakdown.json](breakdown.json); audit [audit.md](audit.md); investigation [investigation.md](investigation.md).
- Progress: [progress.json](progress.json) (waves, rework counts, `line_target`, `traps`, `surfaced_pitfalls`).
- Landing script: [land-pipeline.zsh](land-pipeline.zsh). Worker brief common sections were kept in the session scratchpad and are not durable. Rebuild them from the skill templates plus the traps above.
- Neighbour leads: just-in-time-88 (df129d9e: dispatch layering; owns 06ee9b67 and 3f816619) and just-in-time-3e (f8a6b298: storage; owns the format 4 cutover, 725af076 and 944edb32).
