# Handoff — Give each test property one owning test on shared fixtures and contracts (c780514e) — sessions 3–4

**Date:** 2026-10-28 (Europe/Helsinki)
**Session number:** 4. Session 3 ended without a handoff; its work is in git (2026-10-27 02:00–03:20) and `progress.json`.
**Prior handoffs:** `dev/active/c780514e-test-topology/handoff.md` (session 1), `handoff-2.md` (session 2)

## Current state

- Epic `c780514e`: the tracker shows `backlog`, a container rollup state. Work is under way.
- Priority this session: the upstream of 2195c98f's blocked tasks 0af95f14 and 5fac48b8. Wave order was relaxed for it (recorded in `surfaced_pitfalls`).
- Children at last survey: about 100 done, 24 ready, 40 backlog, 0 rejected. Query with `jit issue list --label epic:test-topology`.
- In flight at writing (two worktrees, both reused):
  - **Slot A** `.agents/worktrees/agent-22355a56`:
    - Codex luna is working branch `worktree-agent-w4-fastseed` for 2d342676 (fast_rules), 3ca153c7 (fast_issue) and f0bee761 (fast_docs_templates). Codex cannot commit; the lead commits per suite directory.
    - Branch `worktree-agent-w4-workflow` (173d98d5, lead-reworked as 7985a3918) was landing with cargo-ci and code-review. Check its gates.
  - **Slot B** `.agents/worktrees/agent-de880d9c`: Claude Sonnet is on branch `worktree-agent-w4-serverfile` for 4b5b8f4a (server file-backed constructors). The server tests bind sockets, which is why it is not on Codex.
- Executed by peer `just-in-time-c2` (2195c98f lead, now paused), with user approval: e2eb8862 → 2c9badcd. Leave them alone. 2a6bc44c and 55043636 are done by c2.
- Leases: finite, `agent:jit-execution-lead`. It is the same identity the 2195c98f lead uses; leave its leases alone. Set `JIT_AGENT_ID=agent:jit-execution-lead` for jit writes.
- Open escalations: none. Session 4's user decisions are recorded in `progress.json` `escalations`:
  - 870e8e46: code-review re-run approved.
  - 529f621d: REQ-01 amended to a whole-graph decision.
  - b7078f0d: shared fixture extended (`MemoryDeclarations`).
  - Guard wording: epic REQ-09, 3f8da8c4 REQ-03 and orthogonal-tests now read "tests never change process-wide state".
- Progress file: `progress.json` here. Updated through 5f6a7b182 except the in-flight items above.

## What just happened

- Closed:
  - e21ca76d, 8e69776c, ad1e6d0e, 07f4b1d4, b93687b3, 717f9c2f, 870e8e46;
  - 3be2238f, d49d53db, 077937f5, b469f36d, b7078f0d, 50b800bb, 2d537b7c, 529f621d;
  - 155d8c3c, 64f04da6, 44b40ce1;
  - 14d95c74, 28373fb7, 37c169ad, ea5ffaba (new), 3f15b95c, 8da3f7f5;
  - 5a93e443, 9e411075, ca5eb7ee, 2cbe3ed8.
- ad1e6d0e plus b7078f0d rework 2: `commands/test_helpers.rs` `MemoryDeclarations { config, rules, templates, invariants, gates, after_initialization }`. This is the one way to declare repository state for in-memory tests, including declarations that initialisation rejects (`after_initialization: true`).
- 07f4b1d4 broke main: the `scratch_build` compiled-module walk did not follow `#[path]` (build.rs). Fixed in 2a6802d8b.
- /tmp ran out of inodes from about 163k leaked `/tmp/jit-test-<uuid>` directories. With user approval, directories older than 30 minutes were deleted. Fix 022a46af6: `InMemoryStorage::new()` owns its minted root and removes it on last drop. About 8 directories still leak per lib run (stores `rooted_at` a borrowed minted root that outlive it). There is no task for that residue yet.
- The sccache server died host-wide: it had been started with a gate's scratch TMPDIR that was later removed. Restarted with `TMPDIR=/tmp`. Failed cargo-ci runs from that window were force-rerun.
- The user dropped the load guard from `land.sh`; the disk guard of 60 GiB or more stays.
- Created ea5ffaba: `query all` composition moved into the domain layer, with owners. 3f15b95c depends on it.
- 529f621d: the transition, delete and dependency-removal cascades all apply `domain::queries::issues_becoming_ready`.

## What to do next

- [ ] Slot A, fast_* batch:
  - When Codex exits, read `scratchpad/codex-F.txt`.
  - Review the diff, especially changed literals and paths (Codex has rewritten them twice; see traps).
  - Run `cargo nextest run -p jit --test fast_rules --test fast_issue --test fast_docs_templates` outside the sandbox.
  - Commit per suite, then `land.sh worktree-agent-w4-fastseed … 2d342676 3ca153c7 f0bee761` and `review.sh`.
- [ ] 173d98d5: check gates (`jit gate status-all 173d98d5`); close if both passed. Its rework count is 1.
- [ ] Slot B, 4b5b8f4a: review, land and gate. Then 5fac48b8's upstream is only 0af95f14 (c2's 2c9badcd). Message `just-in-time-c2` (or `ListAgents` for its successor) when 0af95f14's and 5fac48b8's upstreams are terminal.
- [ ] Remaining ready work, grouped by footprint:
  - "read created ids from JSON output": 5001b556, 6a5de806, 6bd02fa4, 7208e568, a057a4f5, b0fa5e5d, b305e55e, c211f14c, c6cd250d. Group by integration target.
  - Spawn migration 26cbc32e, then 2d3c886f (cli_issue suite root).
  - Shared constructors: aadae7cd, d7c26016, 955ee89e.
  - Others: 20ab27e8, 2f63adec, 510daa14, 53d582ab, f1c1acc8.
  - Respect the overlap groups in `handoff-2.md`.
- [ ] Before the epic gate: reconcile `surfaced_pitfalls` against the epic criteria, then do the Section 10 completion.

## Traps — do not repeat these

Session 1 and 2 traps stay in force, except the load guard, which the user dropped (disk guard stays).

- **Codex edits wander outside the retired helpers.**
  - b7078f0d: it rewrote a `JsonFileStorage` config path (`storage.root()` is already the data root), which broke main until 74d338c74.
  - b469f36d: it duplicated an owner-list entry (code-review d36bb7b2).
  - Diff-review every changed literal before landing.
- **Codex cannot run socket tests.** Its nextest stops early in the sandbox. Run the lib and target suites outside the sandbox before committing Codex work, and never dispatch server or serve work to Codex.
- **Do not tell a worker to follow a target file's existing conventions when that file is unmigrated.** The 3f15b95c worker copied `cargo_bin!` and `setup_repo` into the claim suite (code-review 37c2f3d8).
- **A build.rs `#[path]` module must be visible to `scratch_build`'s compiled-module walk.** Run `cargo nextest run -p jit --test scratch_build` before landing module-layout or build.rs changes.
- **"Initialization rejects it" does not justify a post-construction declaration write.** Code review failed b7078f0d twice on that. Use `MemoryDeclarations { after_initialization: true, .. }`.
- **`land.sh` dirt rules.**
  - It refuses dirt other than issue files, events and gate runs, even when the branch is already merged.
  - `jit issue create` rewrites `.jit/index.json`; commit it first.
  - Keep `progress.json` clean on main for the whole run.
- **Do not write wait loops on `pgrep -f`.** A hook blocks them. `land.sh` already waits on the exclusive lock.
- **Run jit writes from the main checkout root, with `JIT_AGENT_ID` set.** `jit issue claim` refuses backlog issues; use `jit issue assign`, then claim once the issue is ready.
- **Plain `cargo nextest run -p jit --lib` failed on the nextest setup script.** 2a6bc44c removed that script on main; older branches still have it. Use `cargo nextest run --workspace -E 'package(jit) & kind(lib)'` there.
- **A gate failure caused by the environment is reused** for unchanged inputs. After fixing the environment, rerun with `jit gate evaluate <id> cargo-ci --force`, under the `-x` lock.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show c780514e`; plan `plan.md`, manifest `breakdown.json`, progress `progress.json`, investigation `investigation.md` (this directory).
- Session-4 lead tooling: `/tmp/claude-1000/-home-vkaskivuo-Projects-just-in-time/f67d3bcf-f717-4ac4-a081-99d75521e616/scratchpad/`. Lost on reboot; regenerate from `handoff-2.md`.
  - `land.sh` (no load guard), `review.sh`, `close.sh`.
  - `brief-adopt-common.md` (in-memory/file-backed adoption template), `batch-template.md` (CLI spawn migrations).
- Peer handoff: `dev/active/2195c98f-semantic-types/handoff-3.md`.
- Shared fixtures: `crates/jit/src/commands/test_helpers.rs`, `crates/jit/src/test_utils.rs`, `crates/jit/src/test_cli.rs`, `crates/jit/src/test_taxonomy.rs`, `dev/TESTING.md`.

## Addendum (later in session 4)

- Closed since the handoff above: 173d98d5, 4b5b8f4a, 5a93e443, 9e411075, ca5eb7ee, 2cbe3ed8, 6bd02fa4, c211f14c, c6cd250d, b305e55e, 6a5de806, a057a4f5, b0fa5e5d, 26cbc32e, aadae7cd, 2d342676, 3ca153c7, f0bee761, 5001b556, d7c26016.
- Seven of the "read created ids from JSON output" tasks were already satisfied by earlier spawn migrations. Gates passed with no change; close those on gates when criteria verify.
- The shared fixture now also has `MemoryDeclarations.absent: &[VirtualPath]` and `memory_executor_wrapping(storage, declarations, wrap)` for failure-injection wrapper stores (user decision).
- c2 completed 0af95f14 and is running 5fac48b8 plus its retyping wave (13190465, b45a2865, b73b3e5a, 19d9bd7d). Its remaining waits on this epic:
  - 53d582ab (for 13190465);
  - b58ea1e2 and ce097844 (for b45a2865);
  - 3f01cb72, 6a13eeac, 8f87e366 (for 19d9bd7d), which wait on 168a9c16, 966d0627, e5166159, f1c1acc8, 56eec47b and c5c933f2.
  Message `just-in-time-c2` as each completes.
- In flight at this addendum:
  - Re-gating 7208e568, 53d582ab and f1c1acc8 after lead fixes. 7208e568 is at rework 2 of 2, so another failure escalates.
  - Slot A: Claude Sonnet on `worktree-agent-w4-rulesowners` (b58ea1e2, ce097844).
  - Slot B: free. Next for it: 56eec47b and c5c933f2 (ready; upstream of 966d0627 → 6a13eeac).
- Traps added:
  - Code review counts `setup_test_repo_with_config` wrapped in `JitCli::with_repository` as in-process setup in CLI spawn migrations. Use `JitCli::with_repository(declaration_dir(..))` plus `cli.run(["init", "--json"])`.
  - A test left asserting only what holds before its action fails review. Delete a re-proof whose sole distinct property moved elsewhere.
