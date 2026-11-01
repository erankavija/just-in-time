# Handoff — Give every identity and closed vocabulary one semantic type (2195c98f) — session 5

**Date:** 2026-10-29
**Session number:** 5
**Prior handoffs:** `handoff.md` (session 2), `handoff-2.md` (session 3), `handoff-3.md` (session 4), all in this directory

## Current state

- Epic: `2195c98f`, state backlog (a container rollup; the work is under way).
- Children: 5 of 6 stories done. Only story e2651d77 (issue identity) remains.
- Story e2651d77 tasks: 12 done, 6 rejected, 1 ready (5fac48b8, blocked on an escalation), 13 backlog.
- Active worker worktree: `agent-5fac48b8` (branch `worktree-agent-5fac48b8`, head 4608d2843 plus lead gate commits; cargo-ci passed; code-review failed). Its build target was cleaned to free disk; the next build there is cold.
- Open escalations: one (see Open questions).
- External blocker: 19d9bd7d waits on c780514e tasks 3f01cb72 (after 2916a306 → 168a9c16) and 6a13eeac (after c5c933f2 → 966d0627). Both chains are active and not blocked on the invoker (4f, 2026-10-29): 2916a306 is finishing a user-approved shared `test_cli.rs` helper; c5c933f2 is in rework after one code-review failure. 4f messages this session or its successor when 3f01cb72 and 6a13eeac are terminal.
- Progress file: `progress.json` in this directory.
- Session rule from the invoker (recorded in `progress.json` `session_rules`): do not stop to ask; record escalations; write the handoff when only invoker input remains; check workers every 30 min.

## What just happened

- **Invoker approved taking e2eb8862 → 2c9badcd** from c780514e. just-in-time-4f agreed.
  - e2eb8862 (Codex luna): storage finalization re-proof deleted. Landed d273d0962.
  - 2c9badcd (Claude opus): one `IssueStore` conformance suite (`storage/issue_store_conformance.rs`, `Backend` trait + `conformance_suite!`). Rework 1 after code-review F1: the in-memory backend now serves working-tree path reads from its image. Landed 8b1d10d66.
- **0af95f14** (Codex sol): `test_utils::issue_id(seed: u128) -> IssueId`; 53 files of unit tests use it. Landed c66663dc8.
- **Suite retyping** after 4f's shared-seeding landing:
  - 13190465 fast_issue (Claude sonnet; rework 1 on direct id slices). Landed 1afa3fa0b.
  - b45a2865 fast_rules (Codex luna). Landed 520da32bc.
  - b73b3e5a fast_docs_templates (Codex luna; rework 1 on String-valued binding maps). Landed 42ee68606.
- **5fac48b8** (Claude opus): `Issue::draft(id, ..)`, `MutationContext::issue_id_at`, finalizer reserves one index per created issue. cargo-ci passed; code-review failed on REQ-01 (escalated).
- **Disk:** at 4f's request, `cargo clean` in idle worktrees agent-5fac48b8, agent-4ba18af4, agent-abe963dd, agent-123efe74 (build output only). Free space 57 → 69 GiB.

## What to do next

- [ ] Read the invoker's answer on 5fac48b8 REQ-01.
  - Option A (amend REQ-01): amend the issue, then re-run code-review on `worktree-agent-5fac48b8` (run `scratchpad/ci-only.sh 5fac48b8` first if main moved, since review wants fresh cargo-ci), then `land.sh 5fac48b8 ...`.
  - Option B (rework): send rework 1 to `w-5fac48b8` (SendMessage keeps its context) to make `Issue::draft` take `ValidatedIssueId`.
- [ ] After 5fac48b8: 641bd14e → 89e7a273 → 3ee3a4d1 → 7abcc22d (the flip) → 0102769f, 488eb3c1, ce2bdbb0, 417be079, 79aa3aa7, 855d027d, be18da9e. c797b59b gates 7abcc22d too.
- [ ] When 3f01cb72 and 6a13eeac are done: reclaim 19d9bd7d from `agent:planning-lead-c780514e`, dispatch it (brief: `scratchpad/brief-19d9bd7d.md`), then c797b59b (assigned `agent:planning-lead-2195c98f`; reclaim first).
- [ ] Then story e2651d77's holistic review, then epic completion (skill Section 10).
- [ ] Next alternation slot: Claude (the last dispatch was the Codex rework of b73b3e5a).
- [ ] Lead tooling lives in `/tmp/claude-1000/-home-vkaskivuo-Projects-just-in-time/2e183f77-4a70-40e9-a071-ae96e24a3ed1/scratchpad/` and does not survive a reboot:
  - `land.sh <id> <subject>` (merge main, cargo-ci in the exclusive window, merge into main, source-equality check);
  - `ci-only.sh <id>` (merge main and run cargo-ci on the branch, no landing);
  - `resolve-issue-conflicts.sh`;
  - briefs `brief-*.md`, `body-*.md`, `common-tail.md`, prompts `prompt-*.md`.

## Traps — do not repeat these

Traps in `handoff.md`, `handoff-2.md` and `handoff-3.md` stay in force.

- **Run cargo-ci on the branch before code-review.** With cargo-ci pending, code-review failed 2c9badcd on REQ-04 ("both backends pass each case") as unverified. Use `ci-only.sh <id>` first, then code-review, then `land.sh`.
- **Codex workers rewrite other epics' planning artifacts during a stale-prose sweep.** The e2eb8862 worker edited `dev/active/c780514e-test-topology/{breakdown.json,investigation.md}`; the lead reverted them. State in each brief that `dev/active/` is out of footprint.
- **Code-review reads REQ-05-style "ids only through as_str(), short_id(), ..." against every id use in the family, not only signatures.** It failed direct slices (`&a[..8]`, 13190465) and `id.as_str().to_string()` stored in a `BTreeMap<String, String>` that feeds a `BTreeMap<String, IssueId>` API (b73b3e5a). Brief the sweep over the whole directory, including ids bound from harness returns.
- **Code-review reads "no code path creates X" as a type-level guarantee.** Under the `IssueId = String` staging alias, 5fac48b8 REQ-01 cannot be guaranteed by the type, and review failed it although no path builds an empty id. Check staged criteria for this before dispatch and escalate wording first.
- **Worktree targets are hardlinked to `.agents/cache-pool`.** `du` of a single worktree overstates what `cargo clean` frees there; only unique files are released.
- **4f's jit state keeps main's tracked tree dirty.** The dispatch script refuses a dirty main. Wait with a loop on `git diff --quiet` (not on `pgrep -f`, which a hook blocks), or ask 4f to commit.
- **Closing an issue makes c780514e dependents ready and changes their files.** Commit those files with the close (94bbdbd6 after 2c9badcd).

## Open questions needing invoker input

- Question: How should 5fac48b8 REQ-01 ("`Issue::draft` takes the issue's `IssueId` as a parameter, and no code path creates an issue whose id is empty") be satisfied while `IssueId` is the staging alias `String`?
  - Context: code-review F1 [high, blocking] at `crates/jit/src/domain/identity.rs:15`: the alias lets `Issue::draft` accept an empty id, so the type cannot guarantee REQ-01. The plan keeps the alias until the flip 7abcc22d, which is downstream of 5fac48b8. No production or test path builds an empty id after the change; cargo-ci passed.
  - Options: A, amend REQ-01 to state that no code path creates an empty issue id, with the type-level guarantee delivered by the flip 7abcc22d, and re-run code-review. B, rework `Issue::draft` to take `ValidatedIssueId` now, departing from the alias-staging contract that staging entries compile unchanged at the flip. C, other guidance.
  - Recommendation: A. It matches the plan's staging (`alias-staging-bridge`) and keeps 7abcc22d as the single cutover.

## Reference artefacts

- Epic: `jit issue show 2195c98f`. Story: `jit issue show e2651d77`.
- Planning: `plan.md`, `breakdown.json`, `investigation.md` and `progress.json` in this directory.
- Parallel session: `just-in-time-4f` (c780514e lead). Coordinate by SendMessage; it uses the same agent identity and lock forms.
