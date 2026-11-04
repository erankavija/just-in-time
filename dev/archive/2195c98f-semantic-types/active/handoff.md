# Handoff — Semantic types (2195c98f) — session 2

**Date:** 2026-10-26
**Session number:** 2 (session 1 ended without a handoff; its state was reconstructed from git and `progress.json`)
**Prior handoffs:** none

## Current state

- Epic: `2195c98f` — state: in_progress
- Wave in progress: wave 4 of 11 (waves are partial; dispatch whatever is ready in the lowest open waves)
- Children summary: 2/6 stories done; interior tasks: 51 done, 6 rejected (absorbed into 91081bd3), 3 in flight, 20 pending
- Active claims: 91081bd3, e98ae5a4, f265e748 (lead and `agent:worker`); leases lapse — re-acquire with `JIT_AGENT_ID=agent:jit-execution-lead jit claim acquire <id>` before every gate evaluation or state change
- Open escalations: none
- External blocker: test-topology epic c780514e (paused) blocks 0af95f14 → 13190465, 19d9bd7d, b45a2865, b73b3e5a, 5fac48b8 → c797b59b → 7abcc22d (the newtype flip) and 641bd14e. The epic cannot finish until c780514e progresses.
- Progress file: `progress.json` in `jit doc dir 2195c98f dev/active`

In-flight branches (all committed, worktrees clean):

| Issue | Branch head | Gates | Next |
|---|---|---|---|
| e98ae5a4 event tags via `wire_vocabulary!` | 7c4c85519 | code-review PASS, cargo-ci PASS | merge to main |
| f265e748 optional event issue id | 0b9770767 | code-review FAIL ×3 (last fixed by lead) | re-run code-review, then cargo-ci |
| 91081bd3 CLI boundary + executor-family retype | 9ed0f9caf | code-review FAIL (R6 fixed by Codex rework 2) | re-run code-review, then cargo-ci |

## What just happened

- Reconciled session-1 leftovers: committed the uncommitted 91081bd3 R2 gate records; found b01208cb (event-log canonicalization) mid-flight.
- b01208cb: lead rewrote `.jit/events.jsonl` under the repo-write and event locks (8602 lines re-serialized, 8 retired-tag lines kept, second pass identity). Done.
- d72bf093 (Event envelope): code-review R1 failed on retired-tag lines; invoker amended REQ-03 to scope the round trip to current tags and pin the skipped lines; Claude sonnet rewrote the test. Merged to main (1ecace042), merged-main cargo-ci green, done.
- 91081bd3: R2 rework (Claude opus) passed cargo-ci; code-review R3 failed because executor methods re-parse resolved ids (sibling scope). Invoker decisions: absorb 75951558, 804a8605, 8e82fa49, cd8adfc1, d50d6976, c1c86312 into 91081bd3 (rejected as absorbed; downstream 1c5da41d, 641bd14e, 8acca4f4 now depend on 91081bd3; c797b59b now gates 7abcc22d; rejected tasks linked from story e2651d77 for membership); drop the c797b59b dependency; `issue delete` resolves before its confirmation check with a seeded failure-lever arm. Description now REQ-01..14 (rework counter reset). Codex sol implemented (34 files), then two reworks. Last commit 9ed0f9caf not yet reviewed.
- f265e748: Claude sonnet implemented; lead review + two code-review rounds forced reworks; invoker approved a lead fix after the limit (0b9770767). Not yet reviewed.
- e98ae5a4: Codex luna implemented; both gates green; not merged.
- 804a8605 was amended (invoker) before being absorbed; d72bf093 REQ-03 amended (invoker).

## What to do next

- [ ] Merge `worktree-agent-e98ae5a4` into main (`git merge --no-ff`), resolve any `.jit/issues/*.json` conflict by keeping main's fields plus the branch's `gates_status`, run cargo-ci on main alone, then `jit issue update e98ae5a4 --state done`.
- [ ] Re-run `jit gate evaluate f265e748 code-review` in its worktree. It overlaps e98ae5a4 in `repository_state/mutation.rs` (`PendingEvent`) and `commands/events.rs`; after e98ae5a4 lands, merge main into the f265e748 branch first, re-run cargo-ci there, then merge.
- [ ] Re-run `jit gate evaluate 91081bd3 code-review` in its worktree (rework counter is at 2 of 2 — a further failure escalates). Then cargo-ci alone, merge to main, cargo-ci on main.
- [ ] After 91081bd3 lands: 1c5da41d and 8acca4f4 become ready (641bd14e still waits on 5fac48b8).
- [ ] Reclaim finished worktrees with `.agents/skills/jit-execution-lead/scripts/reclaim-worker-worktree.sh <id>...` (b01208cb needs `LEAD_RECLAIM_FORCE=1`: it holds only an untracked scratch tool). Disk is at ~100 GiB free; the dispatch policy needs ≥60 GiB per new worktree.

## Traps — do not repeat these

- **Do NOT run cargo-ci gates concurrently with builds, code-reviews or other projects' gate runs.** The 30 s suite-time budget failed at 34–58 s under load (gf2 sessions run their own gate evaluations; load 8–17) and passes alone at ~15 s. Run each cargo-ci alone, gated on `/proc/loadavg` < 1.5.
- **Do NOT re-run a failed gate without `--force`.** `jit gate evaluate` reuses the prior verdict when inputs are unchanged, so a load-induced failure "re-runs" as the same failure in 0 ms.
- **Do NOT let the shared sccache server outlive its TMPDIR.** It was started under an ephemeral `/tmp/tmp.*/scratch` and every compile then failed in cargo-ci setup. Fix: `sccache --stop-server`.
- **Do NOT edit an issue description without reading `.jit/reference/content-standards.md`.** `###` subheadings inside `## Success Criteria` made every REQ unaddressable (code-review failed on it), and sibling-issue references are forbidden.
- **Do NOT trust "compiles under the alias" for REQ "compiles unchanged once IssueId is the validated newtype".** Reviewers found one more instance per round (non-UUID literals, `to_string()` ids, `as_deref()`/Deref reliance, `unwrap_or_default()`). Before review, flip `pub type IssueId = String;` to `ValidatedIssueId` in `crates/jit/src/domain/identity.rs`, `cargo check -p jit --lib --tests`, fix errors on lines the task changed, restore the line. Also grep for literal ids assigned to still-`String` domain fields; the compiler cannot see those.
- **Do NOT hand clippy-incompatible reviewer one-liners in verbatim.** `.as_ref().map(|id| id.as_str())` trips `option_as_ref_deref` under the alias; return `Option<&IssueId>` instead.
- **Do NOT `jit dep add` around rejected issues without removing their edges first.** Rejected deps no longer block, but paths through them still count for transitive reduction; and rejected issues left without edges fail `jit validate` (isolated) and membership backing.
- **State-changing jit commands need `JIT_AGENT_ID=agent:jit-execution-lead`** (otherwise the lease belongs to a different identity) and an acquired lease.
- **Editing an issue on main while its worker branch records gates conflicts on `.jit/issues/<id>.json`.** Resolve by taking main's file and the branch's `gates_status`.
- **A killed Codex wrapper shell (exit 144) does not kill Codex.** Wait on the `-o` file instead of the wrapper.

## Open questions needing invoker input

- Question: How should the epic proceed past the paused test-topology epic c780514e?
  - Context: 7abcc22d (the newtype flip) and everything after it wait on c797b59b, which waits on c780514e's tasks.
  - Options: resume c780514e; re-plan c797b59b's suite tasks to not depend on 0af95f14; accept the epic stalling at that point.
  - Recommendation: no strong preference; decide before the issue-identity chain reaches 3ee3a4d1.

## Reference artefacts

- Epic: `jit issue show 2195c98f`
- Planning docs: `plan.md`, `investigation.md`, `breakdown.json` in this directory
- Worker briefs and Codex reports from this session were in the session scratchpad and are not durable; the 91081bd3 alias-flip audit (221 later-layer diagnostics) is reproducible with the flip check above.
