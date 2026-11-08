# Handoff — Dispatch layering, executor surface, and build hygiene in crates/jit (df129d9e) — session 10

**Date:** 2026-10-24
**Session number:** 10
**Prior handoffs:** [handoff.md](handoff.md), [handoff-2.md](handoff-2.md) through [handoff-9.md](handoff-9.md)

## Current state

- Epic: `df129d9e` — state: backlog (children carry the work; the epic is not claimed).
- Wave in progress: wave 3 of 13 is complete. Wave 4 has not started.
- Children summary: 18 of 78 implementation tasks done (plus planning 0ba00a6f and breakdown 27387b92); 0 in progress; the rest backlog or ready
- Active claims: none. Every lease this session took is released. Assignments stay with `agent:df129d9e-lead`.
- Open escalations: none.
- Progress file: [progress.json](progress.json). It carries per-task status, `rework_counts`, `model_tier`/`vendor`/`tier_reason` per dispatch, and `line_target`.
- Main: green at 1d998db27 (5d0a9a90 cargo-ci passed there: full suite, doctests and budgets)
- Worktrees: none owned by this epic remain; `agent-5d0a9a90` was reclaimed after merge. The other `.agents/worktrees/*` belong to other streams. Do not touch them.

## What just happened

- **9cc3056a** (validation renderer) closed in 9e9defa92. Both gates had already passed on merge 145091232 (session 9).
- **d4373d07** (foreground serve operation): merged 8cf19175a, then rework 8c6d518bc; closed 94a990276.
  - REQ-02's "no `std::process::exit`" clause could not be met: `main` has no exit path for a raw child status. The user chose to amend REQ-02 (9f201b611). 8c46fc85 REQ-03/04 remove the exit under plan D-23.
  - Code-review R1 failed on stale `start_server`/`run_foreground` rustdoc; R2 passed.
- **2e7e1348** (archive execution renderer): merged 10fde232e, closed 63f3c94b4. Both gates passed on the first attempt.
- **a807e699** (library repository bootstrap `commands::open_repository`): merged 853d1cfc8, then rework b834fcbae; closed af0b1bc9a.
  - R1 failed: the `jit recover` leftover-record mapping was still in main.rs.
  - The rework moved it into the library as a typed `RecoveryFailedError`; R2 passed.
- **5d0a9a90** (empty stderr for `--json` failures): Codex Luna implemented it; the lead committed and merged it (46a747fc2).
  - cargo-ci and code-review then failed on `cli_gate::test_issue_show_errors_on_corrupt_gate_run`, which still read stderr. Main was red from that merge until the fix landed.
  - Fix: Codex attempts 3–4 (8ceb670e1), merged 1d998db27 after full `scripts/cargo-ci.sh` passed on the merged branch (4,605 tests, 25 doctests). Gates: cargo-ci and code-review passed; closed.
- **Line target added** (user decision relayed by just-in-time-05; 4ac79e8ff).
  - Rule: main.rs + dispatch/ + output must end below 8,700 default-mode production lines (baseline 9,804 at dc2ea8706).
  - Amended: epic REQ-10 and completion-report a08c4b38 REQ-04. Plan D-26 derives the 1,104 margin.
  - f9977efd is unchanged: its ceiling covers main.rs only.
- **Line trend**, as the per-merge delta of main+output+dispatch measured over the merge's first parent:
  - Sessions 1–9 total: -115. The renderer tasks netted +115.
  - This session: d4373d07 -24, 2e7e1348 -4, a807e699 -158, 5d0a9a90 +8.
  - Epic total: -293. At HEAD the modules hold 9,031 (main 6,772 + output 2,259; dispatch/ not yet created); 332 lines must still go (target: below 8,700).
- **Dispatch policy changes from just-in-time-05 (user):**
  - Choose the tier by difficulty, with no Fable.
  - Alternate Claude and Codex per implementation dispatch.
  - Raise a tier only when a failure shows the task exceeds the model; slips get a same-tier re-dispatch with a sharpened brief.
  - Codex procedure: the user skill `codex-subagents`.
- **Alternation results:**

  | Task | Vendor and tier | Attempts | Notes |
  |---|---|---|---|
  | d4373d07 | Sonnet, then an Opus rework | — | Pre-refinement rule: that escalation would now be a same-tier re-dispatch. |
  | 2e7e1348 | Haiku | first try | Clean. |
  | a807e699 | Opus | 2 | Missed criterion; the same worker passed on rework. |
  | 5d0a9a90 | Codex Luna xhigh | 4 | Every miss traced to the lead's brief (see Traps). Codex code quality was good. |

## What to do next

- [ ] Wave 4 (facades; executor-facade-issues 27f0a575 set the shape). Ready: d84a7bfa, 5962427b, 6f49f3b3, 7a7d0bfe, fd2073b7, 93aaba24, f79d2510, a24b11aa, 985c2b15, dbd05242, 1002524a, a015dcbb. 083c9a91 waits on 614a6f97 (outside this epic). Run them one worktree at a time, alternating Claude and Codex. The pattern is fixed, so the user's tier rule calls these mechanical: start with haiku or Luna.
- [ ] Then waves 5–13 per progress.json.
- [ ] After each merge, record the line delta in `progress.json` `line_target.epic_merge_deltas`. As soon as the trend cannot reach 8,700, report a blocking concern to just-in-time-05; do not loosen the target. Handler waves (7+) add dispatch/ boilerplate, so brief workers to delete duplicated per-arm JSON/exit code, not just move it.

## Traps — do not repeat these

- **Merge evidence is the full suite, never focused targets.** 5d0a9a90 merged on four hand-picked test targets plus doctests. `cli_gate` was not among them, and its stale stderr assertion turned main red for every lead (gate runs 3cabb615 and 9e908b95). Before every merge, run `./scripts/cargo-ci.sh` in the merged worktree under `flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock`. A worker's runs, especially sandboxed Codex runs, do not count (just-in-time-05 ruling).
- **A contract change needs a whole-tree sweep in the brief.** For an output-stream or envelope change, have the worker run `cargo test -p jit --tests --no-fail-fast` (Codex) and classify every failure. A list of targets misses suites.
- **The Codex sandbox denies socket binding.** About 22 serve, TLS and listener tests fail there with `PermissionDenied`/"Failed to bind". Codex cannot commit (`.git` is read-only). Run those suites and all commits from the lead session.
- **Do not put new requirements into a rework brief by accident.** The attempt-3 brief told Codex to keep "the offending path" in a test the original never checked. The worker added an assertion production cannot satisfy and reported it as a blocker, costing a fourth dispatch. Quote the original assertion instead of paraphrasing its intent.
- **Read the issue's exit-code path before dispatching any "no `process::exit`" criterion.** `main` maps only the closed `ExitCode` enum, so a raw passthrough status has no route out. d4373d07 needed a user amendment; 8c46fc85 owns the D-23 typed-error route.
- **A criterion that lists steps "the function performs" includes error mappings.** a807e699 R1 failed because the recover mapping stayed in main.rs. Say so in the brief when the issue enumerates the steps.
- **A failure in the leak check right after your own gate run is your own gate evidence, not a worker leak.** `check-leak-into-main.sh` diffs against the dispatch snapshot, so `.jit/gate-runs/*` and other leads' dirty files show up. Attribute each entry with `jq -r .issue_id` before acting.
- **`jit claim acquire` on an unassigned issue assigns it.** Amending a08c4b38 assigned it to the lead. Unassign after the edit (`jit issue unassign`, under a lease).
- **The old lead scripts in `scratchpad/df129-lead/` no longer exist** (the scratchpad is per-session). This session wrote prompts from the skill templates; its Codex briefs are in this session's scratchpad and are not durable.
- Traps in [handoff-2.md](handoff-2.md) through [handoff-9.md](handoff-9.md) remain in force, except handoff-2's "at most 1 live worktree", which still applies as a coordination cap.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show df129d9e`
- Plan: [plan.md](plan.md); decisions D-23 (foreground serve exit) and D-26 (line target)
- Breakdown: [breakdown.json](breakdown.json); investigation: [investigation.md](investigation.md)
- Line baseline: [line-count-baseline.md](line-count-baseline.md); counter: `scripts/production-lines.py`
- Coordination: `dev/active/9db27a3a-v1-0/handoff-2026-10-23-codex.md`; user skill `codex-subagents`
