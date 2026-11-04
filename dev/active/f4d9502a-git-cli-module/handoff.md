# Handoff — Reach git through one storage-owned CLI module instead of libgit2 (f4d9502a) — session 1

**Date:** 2026-10-24
**Session number:** 1
**Prior handoffs:** None. The coordination snapshot [handoff-2026-10-23-codex.md](../9db27a3a-v1-0/handoff-2026-10-23-codex.md) sets the shared build, merge and commit constraints.

## Current state

- Epic: `f4d9502a`, state backlog. Assigned to `agent:f4d9502a-lead`; the lease is released.
- Wave in progress: wave 3 of 10 (the layered plan in `progress.json`). External blockers make the waves partial, so dispatch whatever jit reports ready.
- Children summary: 7 of the 53 interior tasks plus 5046408a are done, 1 is in progress, and the rest are backlog. Done:
  - 7aafec83: shared git fixtures
  - 810e32d4: server image ships git
  - 3d93442b: hooks install from the layout
  - cb01a945: pinned-document test off libgit2
  - aec50003: pinned-read counting on the stand-in
  - 28ccf9a9 and b0d0cdbd: the bracket nodes
- In progress: 81664a6a (git module foundation). It is merged to main (54a683471, rework c3ad7fda0) but not closed:
  - cargo-ci last failed only on main's unrelated red test (see Traps), run 9e908b95.
  - code-review last failed on R1 F1, which the rework c3ad7fda0 addresses; it has not been re-run on it.
- Unmerged branch: `worktree-agent-4ba18af4` at f97d6eb3a (head lookup), in worktree `.agents/worktrees/agent-4ba18af4`. Reviewed by the lead against REQ-01..04; Codex ran fmt, clippy, focused tests and doctests clean. Not merged, because main is red and its dependency 81664a6a is not done. Issue state: backlog.
- Active claims: none. All task leases are released; assignments are kept on 81664a6a (in_progress) and the epic.
- Open escalations: none.
- Progress file: `progress.json` in this directory, with the wave plan, statuses, rework counts, the dispatch log (vendor, model, tier, reason), the dispatch policy and `surfaced_pitfalls`.

## What just happened

- Intake: claimed the epic and recorded a 10-layer wave plan with external blockers per task (e40f24348).
- Reconciled 5046408a (configurable worktree root, created after the breakdown), recorded as DEC-23 in the plan's "Post-breakdown reconciliation" (2d08a65c3).
  - Edges: 5046408a → 3f8da8c4, which implies the whole layout chain; 5046408a → a807e699 (df129d9e's bootstrap move); 34851181 (surface trim) → 5046408a.
  - It carries no REQ credit; the epic reaches it through `git-operations`.
- 810e32d4 (haiku): one pass. Merged d21ed426b, done c669cb099.
- 3d93442b (sonnet): one pass. Merged 7ef7e8dc9, done a11f0fa48.
- 7aafec83 (opus): code-review R1 failed (real-git path could be relative); rework by the same worker passed. Merged 2c9575022, then 9de4fe221; done 8c5b5fc65.
- cb01a945 (Codex gpt-6-luna xhigh): one pass. Lead commit f7786e737, merged 3a14c2e06, done ee462f405.
- aec50003 (Codex gpt-6-luna xhigh): one pass. Lead commit c6f524273, merged 83ccd5ac4, done 04f57dd4a.
- 81664a6a (opus): code-review R1 F1 failed because layout detection bypasses the new probe. The rework names it a tracked `@/inv/convention-convergence` exception until 07663984 (bdbbbc2b6). The re-gate's cargo-ci failed on main's red test (43882248c).
- 4ba18af4 (Codex gpt-6-luna xhigh): diff reviewed and committed on its branch (f97d6eb3a). It also adds `CanonicalCommitOid::parse` (reused by `ArtifactVersion::pinned`) and a `test-support`-gated `GitCheckout::for_test`.
- Wind-down: 46a89061 and 07663984 had worktrees created but were never dispatched. Their worktrees and empty branches are removed and their leases released.

## What to do next

- [ ] Once just-in-time-05 confirms main is green: re-run 81664a6a's gates, cargo-ci first and then code-review (`jit gate evaluate 81664a6a cargo-ci|code-review` under the CI locks). Close it on a pass.
- [ ] Then merge `worktree-agent-4ba18af4`. Merge main into the branch in its worktree first, then gate (cargo-ci, code-review) and close.
- [ ] Dispatch in this order, alternating Claude and Codex:
  1. 46a89061, pinned reads into the module behind the bridge (Claude opus; sets two contracts).
  2. 07663984, layout detection on the probe (Claude sonnet or Codex).
  3. The module primitives, all independent and all touching `storage/git.rs` and `git_module_tests.rs`: 437907da worktree listing, 09974ad1 base comparison, 3a3b86cf dirty paths, 24d3b460 user name, bb5040a8 origin URL, cc9f213e path history.
  4. After 4ba18af4: 732f7afb and 9de4c09c. After 07663984: c5062e21. After 46a89061: c16c834c and ec9e812c.
- [ ] Lead notes for the 46a89061 and 07663984 briefs. The session scratchpad that held them is gone, so restate these:
  - Each bridge method builds a `GitCheckout` and delegates; no logic stays in `git_revision.rs`.
  - Make `GitRevisionError` an alias of the module error, keeping its variant names.
  - Move `read_blob_batch` as it is.
  - Leave storage's migration to c16c834c.
  - 07663984 keeps `WorktreePaths`' fields and entry points, maps the probe's `Err` to an anyhow error, and removes the tracked-exception comments that 81664a6a added in `worktree_paths.rs` and `git.rs`.
- [ ] External blockers, none started at handoff:
  - semantic-types 2195c98f: 0102769f and e1a6c8c2 (block d7cce445 and through it d68150e2 and everything after); 417be079 and 79aa3aa7 (3f8da8c4, fb5e8e8d); 855d027d (83c8632f, 9ecde705, c140921c, 1acfa592); f8f80eea (d9ba0029).
  - history-independent-reads f8a6b298: 614a6f97 (f6eaba42, f649c144); c3e3e2ff (e3c38fea); 5e1ba005 (d9ba0029).
  - rust-code-quality df129d9e: 22189592 (a273667c); a807e699 (5046408a).
  - Roughly 25 tasks are reachable without these.

## Traps — do not repeat these

- **Codex workers cannot build with sccache.** `scripts/cargo-ci.sh` exports sccache, and the sandbox denies it ("Operation not permitted"). The cb01a945 and aec50003 workers could not verify anything. Put `CARGO_CI_NO_SCCACHE=1` (with `CARGO_NET_OFFLINE=true`, `CARGO_INCREMENTAL=0`) in every Codex brief's cargo command; 4ba18af4 then verified cleanly.
- **Codex cannot commit or merge.** `.git` is read-only in the sandbox. The lead reviews the diff, commits in the worker's worktree (crediting both co-authors), and merges main into a stale Codex branch in that worktree before merging. Codex results so far: 3 of 3 passed review in one pass. Its reports omit nothing, but the lead must run the builds when the sandbox blocks them. Claude results: 810e32d4 and 3d93442b passed first time; 7aafec83 and 81664a6a (opus, contract-setting) each needed one same-tier rework.
- **A code reviewer flags a new primitive whose old twin still runs.** 81664a6a R1 failed because detection still spawned three processes beside `Git::probe`. Its later migration was planned (07663984), but the old path was not named as an exception. When a primitive lands before its consumer migrates, tell the worker up front to name the old path as an `@/inv/convention-convergence` exception with its convergence issue.
- **The clean check in the stock dispatch script refuses main.** Other leads keep `.jit/index.json` and their own issue files dirty. Use a copy whose `git diff --quiet` excludes `.jit` and `dev/active`. Never commit those foreign files; stage named paths only. Committing `.jit/events.jsonl` with foreign hunks is harmless.
- **Zsh-specific expansions trip the harness's safety check.** Constructs like `${(f)...}` and loops over `$(ls -td ...)` were refused as unverifiable. Collect gate-run directories with a small Python helper instead; the helper filtered `git status` for untracked `.jit/gate-runs/*`, matched on `result.json`'s `issue_id`, and staged them.
- **Main went red mid-session from another epic.** `cli_gate gate_cli_integration_test::test_issue_show_errors_on_corrupt_gate_run` broke with df129d9e's 46a747fc2. Check `git log` before diagnosing a cargo-ci failure, and preserve the failed run.
- **`jit issue claim` on a backlog task fails**, and `jit issue assign` needs a lease. For a task whose dependency is merged but not closed, take only `jit claim acquire`, and claim after the dependency closes.
- **Gating serializes merges.** Every cargo-ci holds the exclusive full-CI lock over main's tree for about 10 minutes, so merge the next branch only after the previous gate ends.
- Traps in `dev/active/df129d9e-rust-code-quality/handoff-2.md` through `handoff-9.md` also apply to this epic: lock forms, merge-tree pre-check, doctests before merge, lease renewal before gating, and rework routed to the worker, never the lead.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show f4d9502a`
- Plan and contracts: [plan.md](plan.md) (DEC-23 under "Post-breakdown reconciliation"), [breakdown.json](breakdown.json), [investigation.md](investigation.md)
- Progress: [progress.json](progress.json)
- Shared constraints: [handoff-2026-10-23-codex.md](../9db27a3a-v1-0/handoff-2026-10-23-codex.md)
- Dispatch scripts: `.agents/skills/jit-execution-lead/scripts/`; Codex procedure: user skill `codex-subagents`
