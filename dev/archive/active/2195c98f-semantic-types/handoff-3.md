# Handoff — Give every identity and closed vocabulary one semantic type (2195c98f) — session 4

**Date:** 2026-10-28
**Session number:** 4
**Prior handoffs:** `dev/active/2195c98f-semantic-types/handoff.md` (session 2), `dev/active/2195c98f-semantic-types/handoff-2.md` (session 3)

## Current state

- Epic: `2195c98f`, state backlog (a container rollup; the work is under way).
- Wave in progress: wave 4 of 11. Waves are partial; dispatch whatever is ready in the lowest open wave.
- Children: 5 of 6 stories done (0461a6f7, 9537f74b, 5dd29767, 8c40d01b, 83d04069). Only story e2651d77 (issue identity) remains.
- Interior tasks: 67 done, 6 rejected, 20 backlog. None is ready: all 20 sit behind 0af95f14 or 5fac48b8.
- Active claims: none that matter to this epic.
  - The lease list shows `agent:jit-execution-lead` on 3d9a0ce3, 3f15b95c, 5a93e443, 8da3f7f5, 9e411075, ca5eb7ee. Those belong to the parallel c780514e lead, which uses the same identity.
  - The 55043636 lease is stale; the issue is done.
- Open escalations: one (see Open questions).
- External blocker: test-topology epic c780514e, led by session `just-in-time-4f`. That session is waiting on the invoker.
  - 0af95f14 waits only on 2c9badcd, which waits on e2eb8862 (ready, not dispatched).
  - 5fac48b8 waits on 0af95f14 plus 2cbe3ed8, 4b5b8f4a, 5a93e443, 8da3f7f5 and 9e411075. 529f621d is also on that path.
- Progress file: `progress.json` in this directory (`jit doc dir 2195c98f dev/active`).
- Session rule from the invoker (session 4; recorded in `progress.json` `session_rules`):
  - do not stop to ask questions;
  - record escalations for the next session;
  - write the handoff when only invoker input remains;
  - check workers every 30 min.

## What just happened

- **Escalations from session 3 resolved by the invoker:**
  - Story 8c40d01b REQ-03 amended to the current-type round trip. Holistic review passed and the story is closed.
  - 8acca4f4: the lead removed three `#[allow(unused_imports)]`. Code-review and cargo-ci passed; landed 76a3af872; closed.
- **10ae7776 (vocabulary sweep; Claude opus):**
  - Round 1 failed REQ-01 on multi-token lists in rustdoc and fixtures. Rework 1 closed the whole class (about 90 sites), added `ArtifactFormat` and `test_utils::toml_string_array`, and reworded the JsonSchema docs.
  - Invoker decisions: REQ-01 stays literal (the archive-plan golden token pins were removed); REQ-03 amended so `jit --schema` description prose may change.
  - Landed 631967da3; closed. Story 83d04069 holistic review and cargo-ci passed; closed.
- **Out-of-epic bug 55e3d3fd (v1.0 milestone; invoker-requested fix of jit problems reported by the gf2 session):**
  - Cause: recover, validate and mutation captures read every issue document. Codex gpt-6.1-sol scoped them by purpose; validate now checks each document in its own bounded capture.
  - Landed 7aa7ed622; closed.
  - gf2 rewrote its 13 pre-3f217dd2 `profile_lifecycle` events (invoker's choice; jit has no compatibility decoder).
  - The lead installed jit main (e371e3a16) to ~/.cargo/bin. gf2 confirmed recover and validate exit 0.
- **c780514e tasks taken over with invoker approval to shorten the 0af95f14 path:**
  - 2a6bc44c (Codex luna; invoker scoped REQ-02 to profiled-repository scripts): landed 34a960123; closed.
  - 55043636 (Claude opus; rework 1 after code-review F1 on a whole-environment read): no test mutates env or cwd any more, and `EnvGuard`/`CurrentDirGuard` are deleted. Landed e3be5cb24; closed. The c780514e scope items it raised were handed to just-in-time-4f.
- **Infrastructure:**
  - Reclaimed 9 merged worktrees; disk is at 70 GiB free.
  - Host-wide changes by other sessions: a cargo jobserver pool shim (`~/.dotfiles/bin/cargo`, with test runner and rustdoc wrappers); /tmp inode exhaustion fixed by 4f's in-memory root cleanup (022a46af6).

## What to do next

- [ ] Read the invoker's answer to the open question. If yes: claim e2eb8862 → 2c9badcd under `agent:jit-execution-lead`, tell just-in-time-4f, and dispatch e2eb8862. The alternation is next on Codex; the last dispatch was Claude for 55043636.
- [ ] When 2c9badcd is terminal, reclaim 0af95f14 (assigned to `agent:planning-lead-c780514e`) and dispatch it. Then dispatch 13190465, 19d9bd7d, b45a2865 and b73b3e5a in parallel.
- [ ] When 5fac48b8's upstream is terminal, run the chain: 5fac48b8 → c797b59b and 641bd14e → 89e7a273 → 3ee3a4d1 → 7abcc22d (the newtype flip) → 0102769f, 488eb3c1, ce2bdbb0, 417be079, 79aa3aa7, 855d027d, be18da9e. Then story e2651d77's holistic review, then epic completion (Section 10).
- [ ] Ask just-in-time-4f (or its successor; check ListAgents) for c780514e's status and whether it settled 3f8da8c4 REQ-03 and REQ-09's wording with the invoker.
- [ ] Lead tooling lives in `/tmp/claude-1000/-home-vkaskivuo-Projects-just-in-time/2e183f77-4a70-40e9-a071-ae96e24a3ed1/scratchpad/` and does not survive a reboot:
  - `land.sh <id> <subject>` (merge main, cargo-ci under the turnstile and `-x` lock with a load < 1.5 guard, merge into main, source-equality check);
  - `resolve-issue-conflicts.sh`;
  - `inv/scan2.py` (vocabulary co-occurrence scan);
  - the briefs.
  Regenerate `land.sh` from handoff-2's description if it is missing.

## Traps — do not repeat these

Traps in `handoff.md` and `handoff-2.md` stay in force: the alias-flip check, no concurrent cargo-ci, `--force` on re-runs, and the sccache TMPDIR.

- **Do NOT take `git checkout --ours <file>` to resolve a code conflict.** It drops main's non-conflicting changes to the whole file. The 8acca4f4 merge lost main's fixture repoint in `output.rs` this way. Recreate the conflict with `git checkout -m <file>`, then resolve only the conflict hunks.
- **Do NOT guess script flags.** `dispatch-worker-worktree.sh --header-only` treated the flag as an issue id and created a worktree and branch, which had to be removed. The script prints the header itself; read its usage first.
- **A "can't find crate for serde/uuid/axum_core" cargo-ci failure is a corrupt per-worktree target, not the code.** It came from builds during the broken-sccache window (empty `.rmeta` files). Plain `cargo check` passes because cargo-ci builds through `RUSTC_WRAPPER=sccache`. Fix with `cargo clean` in that worktree (its target is hardlinked, so the pool is safe), then re-run.
- **A suite-budget failure on a quiet start can still be load.** 8acca4f4 measured 63 s against 11.5 s alone; gf2 started load mid-run. Confirm with a nextest timing run before treating it as a regression.
- **Code-review applies REQ-01-style "no handwritten list" criteria to rustdoc, `//` comments, TOML or JSON inside string literals, and expected-output strings.** 10ae7776 round 1 exempted those and failed. Brief class sweeps with a co-occurrence scan script (`scratchpad/inv/scan2.py`).
- **rustdoc on `JsonSchema` types becomes `jit --schema` JSON.** Rewording it changes schema bytes, which collides with any "JSON byte-identical" criterion. 10ae7776 needed a REQ-03 amendment.
- **A whole-environment read (`std::env::vars`, a child `Command` inheriting env) counts as reading every variable some test sets.** 55043636 round 1 failed on `test_cli.rs` `program_command`.
- **Closing an issue on main makes dependent issues ready and changes their files.** The dispatch script then refuses a dirty main. Commit the dependents' issue files with the close (2f63adec after 2a6bc44c).
- **zsh does not word-split `$var`.** A multi-line list of paths passed to `git add $mine` becomes one pathspec. Use `${(f)mine}`, or a file plus `xargs`.
- **Old event logs written before a shape change stop parsing.** 3f217dd2 broke gf2's `profile_lifecycle` lines, and this repository's log has none of that type, so nothing here caught it. Before installing jit for another repository, run the new binary's `jit validate` on a clone of it.

## Open questions needing invoker input

- Question: Should this lead also take c780514e tasks e2eb8862 ("Storage tests stop re-proving mutation finalization") → 2c9badcd ("Shared IssueStore conformance suite runs against both backends"), the last two blockers of 0af95f14?
  - Context: e2eb8862 has been ready and undispatched since the c780514e lead (just-in-time-4f) went to waiting on the invoker. Your earlier approval covered only 2a6bc44c and 55043636.
  - Options: A, take them and tell 4f; B, leave them to 4f.
  - Recommendation: A. They are the whole remaining critical path to 0af95f14, which unblocks four tasks of this epic.

## Reference artefacts

- Epic: `jit issue show 2195c98f`.
- Planning: `plan.md`, `breakdown.json`, `investigation.md` and `progress.json` in this directory.
- Out-of-epic fix: `jit issue show 55e3d3fd`. The gf2 event rewrite script is `scratchpad/rewrite-lifecycle.py`; it was applied in gf2 2f604fc79.
- Parallel session: `just-in-time-4f` (c780514e lead). Coordinate by SendMessage; it uses the same lock forms and the same agent identity.
