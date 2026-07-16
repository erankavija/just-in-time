# Handoff — Core maintenance (6eb585bc) — session 3

**Date:** 2026-07-16
**Session number:** 3 (max-parallelism wave A batch)
**Prior handoffs:** dev/active/6eb585bc-handoff.md, dev/active/6eb585bc-handoff-2.md (their Traps remain in force except where superseded below)

## Current state

- Epic: `6eb585bc` — in_progress; finite v1.0 prerequisite (@/charter/D-14).
- Wave in progress: wave A (max-parallelism replan, invoker-directed 2026-07-16; supersedes one-wave-at-a-time). 11 issues dispatched in parallel worktrees + 1 hotfix (46657f6f).
- Children summary: 21 done (prior sessions), 15 open of which 11 dispatched this session, plus hotfix child 46657f6f (dispatched). Wave B pending: c505031a, 554ad07f, 450db193, 0283ce74.
- Active claims: epic (agent:jit-execution-lead); agent:worker on 6f881a85 0daba57d 8917c558 894337e2 8d7fc762 16402e14 1d59070d 52665a07 3e12ffbd 45a140ae d74a9ed1 46657f6f.
- Open escalations: none awaiting input (45a140ae semantics and 46657f6f ownership both resolved by invoker this session; recorded in progress-file escalations).
- Progress file: `dev/active/6eb585bc-progress.json` (current).

### Per-worker snapshot (worktrees under .agents/worktrees/agent-<id>, branches worktree-agent-<id>)

| Issue | State | Evidence |
|---|---|---|
| 8d7fc762 | MERGED (0a468803); gates mcp-ci/cargo-ci/code-review/doc-review PASSED; docs-mechanical BLOCKED by 46657f6f | desk review clean; report received |
| 894337e2 | MERGED (9934f89d, build-verified); cargo-ci + code-review evaluating (background task at session end) | desk review clean; report received |
| 46657f6f | **HOTFIX IN FLIGHT — COMPLETE THIS FIRST** (invoker: "the fix must be completed immediately") | worker-46657f6f dispatched (opus), worktree at anchor 6ddf4fe3 reading code at session end |
| 8917c558 | committed e7782142, clean tree; awaiting cargo-ci confirmation + report (nudged 2x) | doc add idempotence fix |
| 16402e14 | committed 35e13e21 (chose TOLERATE), clean tree; nudged for cargo-ci + report | checkbox criteria fix |
| 52665a07 | committed 4a950394, clean; cargo-ci was running at session end | preset removal |
| 1d59070d | committed 3bb9c937 + 22 dirty files — mid --by-required sweep per lead ruling (see Traps) | gate define mode |
| 6f881a85 | 7 dirty files, cargo running — mid-work | broken-pipe fix |
| 0daba57d | 9 dirty files (full expected footprint incl. exit-codes.md), cargo-ci run launched | delete exit code |
| 3e12ffbd | 13 dirty files, mid-work | batch export |
| 45a140ae | committed c15b26eb (decision record + domain terminality done, tasks 3–8 of its plan pending) | Archived semantics |
| d74a9ed1 | committed design note b876f3b7 + 3 dirty | rules.toml write-through |

## What just happened

- Invoker directive: maximize parallelism. Replanned remaining 15 children into wave A (11 parallel, disjoint footprints) + wave B (4 contended: c505031a, 554ad07f, 450db193, 0283ce74). Recorded in progress file.
- Pruned 10 merged session-2 worktrees (~55 GB reclaimed).
- Dispatched 9 wave-A workers (anchor 8901b649), then 45a140ae + d74a9ed1 (anchor 60ef7bbc) after the invoker interview.
- Invoker interview pinned 45a140ae's Archived model: terminality-preserving overlay; coupled retirement (archive execution requires Done/Rejected, success → Archived); revive restores exact pre-archive state (closes handoff-1's resurrection loophole). Pinned into the issue description.
- Lead ruling for 1d59070d REQ-03 (worker escalated): bare `jit gate evaluate` on a MANUAL gate becomes exit-2 usage error hinting `--by <attestor>`; attested evaluate records the attestation. Rationale: evaluate IS the manual pass mechanism (949cd9d0 D-1); a new verb would collide with c505031a. Call-site sweep (~20 test/doc files) ruled in scope.
- 8d7fc762 reviewed + merged; 4/5 gates green after one cargo-ci retry (lock flake, see Traps).
- **Discovered eceffc17 regression** (codex session's merge ec8e6ebe): docs-mechanical unpassable via gate evaluate — see Traps. Escalated; invoker chose "file + fix now". Filed 46657f6f (high, under epic, gates cargo-ci + code-review), dispatched opus fix worker with env-handshake design.
- 894337e2 reviewed + merged (9934f89d); gates evaluating at session end (task bjp11g7j1).

## What to do next

- [ ] **FIRST: complete 46657f6f immediately (standing invoker directive).** If worker-46657f6f left work in its worktree, review/finish it lead-direct; else re-dispatch from the issue description (it contains the full analysis + suggested env-handshake design). Then: merge, verify-commit-builds, evaluate cargo-ci + code-review, and live-verify `jit gate evaluate 46657f6f`-style docs-mechanical passes (REQ-04 is live in-repo verification).
- [ ] After the fix lands: `jit gate evaluate 8d7fc762 docs-mechanical` (its last gate) → complete 8d7fc762 (state done). Check task output for 894337e2's cargo-ci/code-review (or re-evaluate) → complete 894337e2.
- [ ] Collect remaining wave-A workers (session-bound agents die with the session — inspect each worktree per the snapshot table; completed-looking worktrees: review per lead-review-protocol, merge sequentially with verify-commit-builds after EACH merge, evaluate gates post-merge. Mid-work worktrees: re-dispatch a continuation worker into the SAME worktree with the original issue prompt + "continue from worktree state").
- [ ] Merge-order caution: 6f881a85 and 0daba57d both touch main.rs; 1d59070d's sweep touches many test/doc files that 52665a07's doc sweep may also touch. Merge smallest-first, resolve conflicts lead-side, rerun gates per merged commit.
- [ ] Dispatch wave B after A merges: c505031a (after 1d59070d + 8d7fc762), 554ad07f (after 1d59070d), 450db193 (after 52665a07), 0283ce74 (after 0daba57d).
- [ ] Epic completion per Section 10: batch completion report, epic gates, close 6eb585bc (@/charter/D-14), archive progress/handoffs, jit doc add.

## Traps — do not repeat these

- **docs-mechanical CANNOT pass via `jit gate evaluate` until 46657f6f lands.** eceffc17 (merge ec8e6ebe, codex session) holds an exclusive `.jit-bootstrap.lock` RepoWriteGuard for a mutating command's entire execution; reentrancy is in-process only, and gate checkers are child processes, so docs-mechanical's M5 `jit invariant render` times out after 5s — every time (3/3 verified), while `./scripts/docs-mechanical.sh` standalone passes. Do NOT retry-loop the gate or debug per-issue; land 46657f6f first. Evidence: crates/jit/src/storage/recovery_coordinator.rs (RecoverySession holds `_bootstrap_guard`), cli.rs `requires_recovery_dispatch` (Gate + Invariant commands both true).
- **cargo-ci schema_v2 "Lock timeout ... jit-cargo-ci-tmp/.jit-bootstrap.lock" failures are the same-family flake, not a work defect.** Under parallel worker load, test-spawned jit processes contend on the shared cache tmp bootstrap lock. Signature: test_index_v2_save_and_load / test_delete_issue_adds_to_deleted_ids failing with "Lock timeout". Retry the gate once before suspecting the diff (8d7fc762's retry passed clean).
- **verify-commit-builds may verify the WRONG commit when the codex session commits mid-chain.** My 8d7fc762 merge produced 0a468803, but codex's ec8e6ebe landed before `verify-commit-builds.sh` ran, so the verifier reported ec8e6ebe. Capture `git rev-parse HEAD` immediately after `git merge` and compare against what the verifier prints; re-run against your merge SHA if they differ.
- **Workers idle-ping while queued on the global build lock — that is not a stall.** cargo-ci.sh serializes on `flock /tmp/jit-cargo-ci.lock` across ALL worktrees; with 11 workers the queue is minutes long. Before nudging: map cargo/flock PIDs to worktrees via `readlink /proc/<pid>/cwd`. Idle ping + flock-waiting or running process = working; idle ping + clean-or-dirty tree + NO processes = stalled (nudge with the exact remaining step list; both nudges this session resumed cleanly).
- **zsh `=====` separator trap re-hit this session** (handoff-2 trap stands): `echo =====DOCS=====` inside a chained command aborted the chain (`not found`). Use plain words.
- **`jit issue children --json` envelope is `{container, count, issues}`** — `.issues[]`, not `.children[]`; short_id projection there may be null, use full `.id`.
- Unresolved from handoff-2: install-race treadmill (commit .jit → install → evaluate as ONE chain, from repo root), reviewer batch-enumeration on output contracts, /tmp quota, background-Bash 10-min cap, stale `jit issue status` gate lines (use `gate status-all`).

## Open questions needing invoker input

None. (45a140ae model and 46657f6f ownership were both settled by invoker this session; c505031a's slot is wave B per the replan.)

## Reference artefacts

- Epic: `jit issue show 6eb585bc`; progress: dev/active/6eb585bc-progress.json (wave A/B replan note, escalation log, rework counts).
- Hotfix issue: `jit issue show 46657f6f` (contains full root-cause analysis + suggested fix design); eceffc17 commit b89f414f / merge ec8e6ebe.
- Worker reports received this session: 8d7fc762 and 894337e2 (in-session only — key facts folded into the snapshot table above).
- 45a140ae decision record (worker-produced): dev/active/45a140ae-archived-semantics.md (committed in its worktree at c15b26eb; not yet on main).
- Gate-run history: `jit gate status <id> --gate <g> --all`; structured findings under .jit/gate-runs/<run>/result.json.
- Dispatch/leak/build-verify protocol: ~/.claude/skills/jit-execution-lead/references/worktree-dispatch-protocol.md (scripts in the skill's scripts/).
