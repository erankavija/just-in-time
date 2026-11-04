# Handoff — Dispatch layering, executor surface, and build hygiene in crates/jit (df129d9e) — session 12

**Date:** 2026-11-04
**Session number:** 12
**Prior handoffs:** [handoff.md](handoff.md), [handoff-2.md](handoff-2.md) through [handoff-11.md](handoff-11.md)

Outgoing lead: just-in-time-7c (`agent:df129d9e-lead`), which took over from just-in-time-88. Steward: just-in-time-6a, the user's channel. Send every scope change, ruling request and milestone ledger to 6a. The session ended on 6a's wind-down request: nothing new was dispatched after it, and in-flight work was finished or parked.

## Current state

- Epic: `df129d9e`, state backlog. Its children carry the work, and the epic itself is unclaimed.
- Children (label `epic:rust-code-quality`): 86 done. Recount with `jit query all --label epic:rust-code-quality --json`.
- Open children:
  - Stories: c06daf50, f35c86de, fe3ad1b0, 46fe7593, fb2dc5c3. They close when their tasks do.
  - Tasks still to run: 487f45a9, 0eca332a, fb38bf1f, 0546fc82, f9977efd, f487aeef (held), fba8d478 (after f487aeef), and a08c4b38 (completion report, last).
- b697a3f7 issues executed by this lead (they count on b697a3f7's own ledger, plan D-54):
  - Done: f56bad8d, f7a59f5f, 4575a03d, 1ade3e6f, ceaaf0ab, 5d09e74b, d58f80c9, 01a88d32, c517b88b, 945f9181, b674382e, eaca3f52.
  - 635f23c7 and 578ed10d: see the final status section at the end.
  - Track A remainder, all backlog: c2b3cefa → d2acb105 → fe9147fa → 693a2fdd. c2b3cefa is sequenced against gate writes.
  - Track B is finished apart from the archive tail (65aafaba, c0775a83, 4e2bed35, e73ad8bc, 30334066, 19404174), which is blocked on epic f4d9502a (git-cli-module). That epic has no lead.
- Active claims: none. Every lease was released at handoff. Re-acquire with `jit claim acquire <id> --ttl 86400`. Acquiring also assigns an unassigned issue, then run `jit issue update <id> --state in_progress` so the tracker shows dispatched work; 6a checks this.
- Open escalations: none awaiting a ruling. The items needing the user are below.
- Progress file: [progress.json](progress.json), holding per-issue status, vendor, rework counts, the incident log and both ledgers.

### Ledgers

- **df129d9e, REQ-10 (whole-crate net production lines against baseline dc2ea8706, ceiling +375, plan D-54):** **+188** over 98 first-parent merges; the crate is at 73,644 at b93170ced.
  - Per-merge deltas since 2026-11-04 are in `progress.json` → `line_target.whole_crate_ledger.since_2026_11_04`.
  - This session: f82a2e4a −60, 10fdad38 −221, 9be5b0de −23, 8fbf8c43 −31, 24a982b7 +27, f87f9b00 −114, 3f816619 −105, b23b1bf3 −53, 5687e281 −175, da30b92a −8, ae3f4f8b −13, 0bc448fb +31, 8b3cce62 −106 (first merge).
  - Measure a merge as `python3 scripts/production-lines.py <merge>^1` against `<merge>`; the TOTAL line is the figure.
  - 6acd7a05 (expected about +12) and 8b3cce62's rework (expected about −21) are recorded in the final status section.
- **b697a3f7 (own ledger):** 945f9181 +4, c517b88b +129 then +1 (rework), b674382e +21, eaca3f52 +248, 635f23c7 −99, 578ed10d +79. Recorded in `whole_crate_ledger.b697a3f7_merges`. Earlier b697a3f7 merges are in [ledger-2026-11-05.txt](ledger-2026-11-05.txt).

## What just happened

- **Took over from 88** (handoff-11).
- **Wave 7 batch and follow-ups** all landed and closed.
- **Net-reduction tasks:** N5 f82a2e4a, N7 10fdad38, N4 9be5b0de, N2 8fbf8c43, N1 ae3f4f8b and N6 8b3cce62.
- **Handler moves:** 24a982b7, f87f9b00, 3f816619 and b23b1bf3.
- **5687e281:** `main.rs` is 43 lines with one `std::process::exit`. `worktree exec` is a real leaf (`Outcome::Exited`), and the pass-through form of `command_group!` is removed.
- **da30b92a:** output schemas come only from the handler registry.
  - The first Codex pass stopped correctly: the path-to-type table was already gone, and `worktree exec` was unregistered. It was resequenced after 5687e281.
- **0bc448fb:** crate-private executor kernel. A syn-based policy test was added; syn was already in Cargo.lock, so it adds only features.
- **6acd7a05:** `validation_warnings` and `plan_warnings` replace payload `warnings`, and the schema test walks every variant.
  - Rework 1 removed a `path == "validate"` special case.
  - The lead fixed one stale CLI assertion that branch CI caught.
- **8b3cce62:** per-command exit-code rows are derived from `raises`/`findings` declarations in `command_group!`.
  - Code review F1: `admit` exempted universal codes.
  - Rework 1: findings must be declared, with no exemption. `gate evaluate-many` declares every code it can carry, and `gate preset apply` returns a lock timeout as an error (exit 11, unchanged).
- **fb38bf1f:** rescoped (plan D-59) to "commands import nothing from the output layer", with a line estimate of +120…+500 for the dropped relocation.
- **b697a3f7 work (track A and B):** listed under Current state.
  - c517b88b: the cargo-ci gate hit a link error from a stale sccache object. It passed on a 6a-approved rerun without sccache (incident in progress.json). Code review F1 (context not reset on retry) was fixed in rework 1.
  - 635f23c7 code review: F1 (rejection needs same-session proof) and F2 (batch draft needs `first_ready_at` and demotion-event assertions), both test-only reworks.
  - 578ed10d code review: F1, no `ReadSessionOpen` race test (REQ-04).
- **Rulings recorded:** see Decisions below.

### Decisions recorded this session

- **Plan [plan.md](plan.md):**
  - D-50: host-fault relaunch; no advisory runs beside authoritative ones.
  - D-51: net reduction (superseded by D-54).
  - D-52: net-reduction plan N1/N2/N4/N5.
  - D-53: N6/N7; bulk and archive held; N9 dropped.
  - D-54: ceiling +375; the lead executes b697a3f7's tracks.
  - D-55: line targets come from measurement.
  - D-56: cold-doctest budget overruns are host faults until f487aeef.
  - D-57: 5d09e74b REQ-04 trimmed.
  - D-58: uniform findings and warnings rules recorded once, as epic D-03/D-04.
  - D-59: response types stay where their result is defined; fb38bf1f rescoped.
- **Epic df129d9e:** D-01 (REQ-10 is whole-crate net), D-02 (ceiling +375), D-03 (uniform findings-to-stderr rule), D-04 (uniform warnings rule).
- **Issue decisions:**
  - 1a4f2ebd D-01 and 8e91126b D-01: command-owned response types are the intended design (amended per D-59).
  - fb38bf1f D-01: rescope.
  - 635f23c7 D-01: REQ-01 names `derive_field_update`. REQ-05's matrix is stated per creation path, with no terminal-only case.
  - 5687e281 D-01: REQ-08 removes the pass-through form.
  - 8b3cce62 D-01: exit-code reference content changes accepted.
  - 0eca332a D-01: retire the five unconstructed variants and the gate fallback codes.
  - a08c4b38 REQ-04: restated to match D-54.
  - 9be5b0de: target amended to at least 20.
  - 10fdad38 REQ-03 D-01: shared test helper.
- **Worker deviations accepted without a ruling:** these are recorded in commits and in progress.json notes.
  - 945f9181: rejection moved onto the shared readiness helper.
  - c517b88b: a no-op `config set` writes nothing.
  - eaca3f52: `read_history_log` placement, and a shared fixtures file.
  - 578ed10d: `read_history_log` moved to the read trait.
  - b674382e: with invalid declarations, readiness repairs are skipped under one combined message.

## What to do next

- [ ] Finish anything left open in the final status section at the end; it is authoritative over the bullets above.
- [ ] df129d9e, in order. fb38bf1f and 487f45a9 may run in parallel; both touch output and error codes, so the second to land merges main and adapts.
  - [ ] fb38bf1f: commands import nothing from output, plus a structural scan test.
  - [ ] 487f45a9: one requires-Git error.
  - [ ] 0eca332a: retire unproduced codes. It uses 8b3cce62's declarations as the census of live codes.
  - [ ] 0546fc82: the CLI output contract stated once.
  - [ ] f9977efd: a `main.rs` line ceiling. It waits on f487aeef in the graph; ask 6a whether that edge still matters now that `main.rs` is 43 lines.
  - [ ] a08c4b38: completion report, last.
  - [ ] Keep reporting per-merge whole-crate deltas to 6a. The projected end is +70…+250 against +375.
- [ ] b697a3f7 track A: c2b3cefa, then d2acb105, fe9147fa, 693a2fdd. Check each issue's criteria against current code before dispatch; every b697a3f7 issue so far had stale citations.
- [ ] f487aeef stays held until the user reports the cargo-pool-run fix; fba8d478 follows it. Don't touch the dotfiles or work around the hold.

## Traps — do not repeat these

- **Do NOT leave `progress.json` (or any non-.jit file) uncommitted on main while a landing runs.** `land.sh` refuses with "uncommitted changes outside .jit", which cost 635f23c7 and b674382e a full CI run each. Commit in the same command that edits it.
- **Do NOT assume a merged branch is done.** Code review runs after the merge. 8b3cce62, 635f23c7, 578ed10d and c517b88b all merged and then failed review. Rework goes on the same worktree branch after `git merge main`, then lands again as a second merge. Keep the worktree until the issue is closed.
- **Do NOT exempt a category from a literal criterion.** 8b3cce62's universal-code exemption and 6acd7a05's validate-only walk both failed: one in review, the other caught in lead review. Read "every"/"any" literally.
- **Do NOT treat "convert existing X" as satisfied when none exists.** 578ed10d's REQ-04 required a `ReadSessionOpen` race test. The worker found nothing to convert and added none; review failed it. Make the worker add the test the criterion names.
- **Do NOT trust a Codex worker's targeted test choice.** 6acd7a05 renamed a JSON field and ran only schema and archive tests, and branch CI caught a CLI assertion. Have Codex grep the field across `crates/jit/tests`, or accept that CI will catch it.
- **Do NOT let Codex write report files into the repository.** Two runs wrote `dev/active/<id>/report.md`; I deleted them before committing. The brief appendix now forbids it.
- **Do NOT quote `======` in zsh.** `echo ======` is an `=cmd` expansion and kills the chain.
- **Do NOT kill processes with kill/pkill or a `pgrep -f` loop.** Hooks block them. Use TaskStop on your own background tasks, and `ps -C <comm>` to inspect.
- **Do NOT move readiness fixes before reduction in `validate --fix`.** Removing a redundant edge can unblock an issue (a rejected middle blocker), so reduction must run first. The lead fixed this in b674382e after the Codex rework reordered it.
- **Do NOT use sccache for an authoritative run that fails to link.** A missing hidden drop-glue symbol on a tree that just linked is a cache fault. 6a's standing rule: one rerun with `CARGO_CI_NO_SCCACHE=1`. If a link error recurs and disappears without sccache, tell 6a, who would then make authoritative runs sccache-free.
- Traps from [handoff-11.md](handoff-11.md) and earlier remain in force: `flock -o` only, `/dev/shm` TMPDIR for authoritative runs, the sccache guard, the Codex git ban, no `cd` into worker worktrees, and no stopping a landing after `land.sh`.

## Host rules (standing)

- **Authoritative runs:** branch CI and cargo-ci gates.
  - Run them under `flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock` with `TMPDIR=/dev/shm/jit-ci/df129d9e` and `CARGO_INCREMENTAL=0`.
  - Use sccache only if a daemon already runs (`ps -C sccache`); otherwise set `CARGO_CI_NO_SCCACHE=1`. Never start the daemon from a run.
- **Advisory whole-suite runs:** take `flock -o /tmp/jit-ci-turnstile.lock flock -o -s /tmp/jit-full-ci.lock`. Never run them beside your own authoritative run (D-50). Targeted single-test runs need no lock.
- **Workers:**
  - Use `TMPDIR=/mnt/nvme/jit/tmp/df129d9e-workers/<id>` and `CARGO_INCREMENTAL=0`.
  - Worktrees and the cache pool live on /mnt/nvme behind symlinks, under the 60 GiB guard. Never delete or prune the cache pool.
- **Host faults:**
  - A landing retries a host fault once: exit 75, TIMEOUT-only failures, ENOSPC, or the D-56 cold-doctest overrun. A further host failure goes to 6a before any other attempt. A content failure is a verdict.
  - `--force` on a gate only over a reused host-fault run whose code and binary are right, with the run id named in the plan (D-47).
- **Codex workers:**
  - They run no git command that writes (`lead-tools/codex-git-rule.md`); the lead merges main into their branches and commits with "Worker: Codex. Committed by the lead after review."
  - Their sandbox cannot spawn the jit binary, so MCP tests fail there and `mcp-ci` covers them.
- **Review verdicts are final.** Bring a disputed finding to 6a; never rerun a review unchanged.
- **Scope changes go to 6a.** Record each ruling in the issue as a `## Decisions` item, amending the criterion text when a criterion changes (6a's 635f23c7 ruling).

## Proposed build-hygiene tasks (steward 6a, for the user to approve next session; not filed)

Context: on 2026-11-04 /mnt/nvme had 189 GB free. 21 done worktrees held hundreds of GB of `target/*/incremental`: 311 GB in agent-0fe46027 and 384 GB in agent-2e2c8ee3. The incremental state spread through the cache pool: the reclaim script harvested it, and dispatch seeded it into every new worktree (231 identical entries per worktree today). After deleting the incremental directories and reclaiming, 292 GB were free and the pool was 42 GB. Each task needs a measured target before filing.

1. **`reclaim-worker-worktree.sh` harvests no `*/incremental` and only the newest artifact per crate and target name.** Today the pool keeps every hashed generation, and seeding copies all of them; the user saw 13 generations of each test binary in one worktree. Target: pool size and seeded worktree size before and after.
2. **The test profile stops forcing incremental.** Remove `[profile.test] incremental = true` from `Cargo.toml`, so no worker depends on remembering `CARGO_INCREMENTAL=0`. Target: no `incremental` directory after a worker run without the variable.
3. **`split-debuginfo = "unpacked"` for the dev and test profiles.** About 75% of each roughly 115 MB test binary is debug info. For cli_issue that is `.debug_str` 25.7 MB, `.debug_info` 19.4 MB and `.debug_line` 18.3 MB, against 16.1 MB of code, duplicated into all 13 executables. Target: active test-executable bytes before and after, as reported by the budget step.
4. **Consolidate the nine `jit` integration test targets into one or two suites** (`@/inv/bounded-rust-build-footprint`), removing about seven linked copies per build. Target: integration-target count and executable bytes from the budget step.

## Open questions needing invoker input

- Question: has `cargo-pool-run` been fixed to stop waiting for jobserver tokens inside nextest's per-test timer?
  - Context: f487aeef (per-test limits) and fba8d478 stay held until the user reports the fix in their dotfiles. Leads must not touch the dotfiles or work around the hold.
  - Options: release f487aeef once fixed, or leave it held.
  - Recommendation: release it after the user confirms the fix.
- Question: who leads epic f4d9502a (git-cli-module)?
  - Context: b697a3f7's archive tail is blocked on it (65aafaba, c0775a83, 4e2bed35, e73ad8bc, 30334066, 19404174).
  - Options: assign a lead; let the df129d9e lead execute it as it did for b697a3f7; or defer.
  - Recommendation: no strong preference. It is outside df129d9e's ledger either way.

## Reference artefacts

- Epic: `jit issue show df129d9e`. b697a3f7 tracks: plan D-54.
- Plan and decisions: [plan.md](plan.md); D-50 to D-59 are this session's.
- Progress, ledgers and incidents: [progress.json](progress.json).
- Ledger snapshots: [ledger-2026-11-04.txt](ledger-2026-11-04.txt), [ledger-2026-11-05.txt](ledger-2026-11-05.txt).
- Lead tooling: [lead-tools/](lead-tools/), refreshed this session. Copy it to your scratchpad before use; the scripts resolve paths beside themselves.
  - `land-one.sh <id>`: serial landing under `flock -o land2.lock`; honours a `hold` file. Exit codes: 3 conflict, 5 CI failed, 6 not merged or dirty main, 7 gate failed.
  - `gate-one.sh <id>`: evaluates the gates of an already-landed issue.
  - `close-issue.sh <id>`: verifies gates, closes the issue and commits `.jit/gate-runs`.
  - `dispatch-tracker-tolerant.sh`: creates worktrees.
  - `mkbrief.zsh` with `brief-rules.md`: builds briefs.
  - `codex-git-rule.md`, `return.md`, `worker-tmpdir-rule.md`.
  - Codex invocation: `codex exec --ephemeral -s workspace-write -C <worktree> -m <gpt-6-luna|gpt-6.1-sol|gpt-6-astra> -c model_reasoning_effort=high -o <out> - < <brief>`.
- Tracker: format 4, and the install freeze is lifted. Gate runs are at `.jit/gate-runs/<issue-id>/<run>.json`; read a review's findings from the newest run's `stdout`.

## Final status at handoff

(Filled in at handoff commit time; this section supersedes any earlier line about these issues.)
