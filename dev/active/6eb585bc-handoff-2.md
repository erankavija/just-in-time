# Handoff — Core maintenance (6eb585bc) — session 2

**Date:** 2026-07-15
**Session number:** 2 (the 2026-07-14/15 core-maintenance batch; prior handoff: dev/active/6eb585bc-handoff.md)
**Prior handoffs:** dev/active/6eb585bc-handoff.md (its Traps remain in force except where superseded below)

## Current state

- Epic: `6eb585bc` — in_progress. **Standing change (@/charter/D-14, commit 57ec9eb7): the epic is a finite v1.0 prerequisite — close it when this scope completes, before the production-readiness source freeze. The living-container decision is superseded.**
- **Story 73482aa1 (rust-build-efficiency): DONE and closed** — all 9 children, story gates green. Report: dev/active/73482aa1-completion-report.md. Headline: clean test-compile −87.3%, rebuild −94.6%, 144→11 test targets, target dir 19.6→3.7 GiB, executables 13.8→0.94 GiB, `@/invariant/bounded-rust-build-footprint` enforced by cargo-ci.
- Epic interleaves DONE this session: ef0065ad (scoped bracket validation — unblocked production planning per @/charter/D-15), 3c4d6fe8 (secret-like label example; also produced the invoker-approved secret-detection gate rescope to scripts/secret-scan.sh).
- **Wave 3 (CLI contracts): `f40f1b0a` DONE** (invoker extended the session to finish it) — all four gates green on dc8df01e after 6 review rounds; see the epic progress note for the final scope (unified gates array, per-command schema declarations incl. mutation echoes and bare query, phantom `query ready` purged, three projection rules, doc-consuming schema cross-check test).
- Wave 3 NOT started: 6f881a85, 0daba57d, 8917c558. Waves 4–6 pending (incl. 45a140ae's required invoker interview). Two carryover decisions from handoff-1 remain open (c505031a slot; archived-resurrection question).
- Active claims: none (f40f1b0a done; assignment record retained on the issue). All workers relieved; their worktrees remain under .agents/worktrees/ (agent-8d4f7084/57d0eb79/3398bc19/ef0065ad/3c4d6fe8/83efbcb4/3f73423b/26f97dc2/362e3fec/f40f1b0a — all merged, safe to prune).
- Progress files: dev/active/6eb585bc-progress.json (epic; wave 2.5/3 statuses current), dev/active/73482aa1-progress.json (story; COMPLETE).

## What to do next

- [ ] Continue wave 3 serially (main.rs cluster): 6f881a85 (stdout panic; sonnet), 0daba57d (refused delete exit code; sonnet), 8917c558 (doc add idempotence; sonnet).
- [ ] Waves 4–6 per dev/active/6eb585bc-progress.json; interview the invoker on 45a140ae's semantic model BEFORE dispatching it; also settle c505031a's slot and the archived-resurrection question (handoff-1 Open questions).
- [ ] At epic completion: batch completion report, epic gates, **close the epic** (D-14), archive progress/handoff docs, link via jit doc add.

## Traps — do not repeat these

- **The install-race treadmill.** The parallel project-lead session commits to main and reinstalls ~/.cargo/bin/jit from ITS worktrees continuously. Every `jit gate evaluate` chain must be: commit pending .jit records FIRST → `./scripts/install-jit.sh` → evaluate IMMEDIATELY (one chained command). A refused run writes .jit records that dirty the NEXT install (dirty=true provenance → refusal loop). Cost this session: ~8 refusal cycles.
- **Reviewers enumerate gate-bearing surfaces one batch per round (f40f1b0a).** Round 1: list/search aliases. Round 2: create + 6 mutation echoes + help text. Round 3: query blocked --full + multi-ID envelopes. Round 4: bare `jit query` + cli-commands membership sentence. If any output-contract work remains, enumerate EVERY `--json` emission in main.rs FIRST (the worker's rework-2 table is the model) and close the whole set in one round.
- **Interactive cargo runs poison the next gate run.** The 57d0eb79 incremental-state step fails on non-empty `incremental` dirs from any plain `cargo build/test` (manifest sets incremental=true interactively). Clear with `find target -type d -name incremental -exec rm -rf {} +` — NOT just target/debug/incremental: the stale-binary child tests recreate a nested `target/jit-stale-child-test-cache/debug/incremental`.
- **Worker sessions stall silently between pipeline steps** (worst: 26f97dc2, five stalls). An idle ping + unchanged worktree + no processes = stalled; nudge with the exact remaining step list. An idle ping + running cargo/harness processes = working; do NOT nudge. Check `ps` + worktree status before deciding. If a worker stalls twice at the same step, announce takeover explicitly and do it lead-direct — but NEVER edit a script file while a worker's process is executing it (bash reads incrementally; a mid-run edit invalidated a full benchmark sampling run), and never work in a worker's worktree without messaging the takeover first (two collisions this session).
- **My shell cwd drifts into worktrees.** Gate runs executed from a worktree record into THAT worktree's .jit (wrong place) and the guard compares against the worktree HEAD. Prefix every gate/install chain with `cd /home/vkaskivuo/Projects/just-in-time`.
- **`jit issue status <id>` may omit gates the issue carries** — 26f97dc2's status line showed 2 gates while it carried 3 (doc-review discovered only via `gate status-all` after `--state done` diverted to gated). Tier-1 review MUST read `jit gate status-all`, not the status one-liner.
- **/tmp is quota-bound.** An isolated CARGO_TARGET_DIR under the scratchpad (3.3 GiB) exhausted it and broke harness output capture globally (empty tool results, exit 1). Stage big dirs under `${XDG_CACHE_HOME:-$HOME/.cache}`, and clean them immediately.
- **Background Bash tasks cap at 10 min.** Long benchmark/sampling runs must be `setsid nohup … &` detached with a log file, then polled (the 35-min baseline re-collection died at the cap first).
- **The `.jit` "leak" that isn't.** check-leak-into-main.sh flags the parallel session's legitimate issue-filing in main (ef0065ad, 3c4d6fe8 arrived this way mid-wave). Inspect the flagged files before reverting anything; issue JSON + events with sensible content = adopt, don't revert. Reverse-contamination also appears in `git diff main..HEAD` from worker worktrees after main moves — check `git log HEAD..main` and per-commit stats before accusing a worker.
- **Baseline/optimized symmetry is a review target.** Any measurement-protocol fix applied to one side only (the web-dist stub) fails review; re-collect the other side at its recorded revision with the same corrected harness (done once — baseline.json carries a re_collection provenance field).
- **zsh `===` heredoc/echo tokens break command chains** (`(eval): == not found`) — use `-----` or plain words as separators.
- Unresolved from handoff-1: all its traps stand; the `gate define --checker-command` trap is 1d59070d (wave 4), the delete-exit-0 trap is 0daba57d (wave 3).

## Open questions needing invoker input

- 45a140ae (Archived lifecycle semantics): interview required before dispatch (escalation policy 6) — carried from batch plan.
- c505031a slot + archived-resurrection loophole: carried from handoff-1.

## Reference artefacts

- Epic: `jit issue show 6eb585bc`; progress dev/active/6eb585bc-progress.json; escalation log inside it.
- Story: dev/active/73482aa1-completion-report.md, dev/active/73482aa1-progress.json, dev/benchmarks/rust-build-efficiency/report.md.
- Gate-run history: `jit gate status <id> --gate <g> --all`; structured findings under .jit/gate-runs/<run>/result.json (`.findings.findings[]`).
- Charter decisions D-14/D-15: dev/vision/9db27a3a-charter.md (parallel session also runs; coordinate via commits, expect main to move constantly).
