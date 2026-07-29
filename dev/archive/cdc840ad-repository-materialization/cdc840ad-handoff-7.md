# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 7

Status: archived after epic completion on 2026-07-23.

**Date:** 2026-07-19T17:22:37+03:00
**Session number:** 7
**Prior handoffs:** `dev/active/cdc840ad-handoff.md`, `dev/active/cdc840ad-handoff-2.md`, `dev/active/cdc840ad-handoff-3.md`, `dev/active/cdc840ad-handoff-4.md`, `dev/active/cdc840ad-handoff-5.md`, `dev/active/cdc840ad-handoff-6.md`

## Current state

- Epic: `cdc840ad` — state: backlog, assigned to `agent:jit-execution-lead`
- Wave in progress: wave 3 of 9 (waves 1–2 done)
- Active issue: `a6a9b964` — in progress, assigned to `agent:worker`
- Implementation commit: `e17bda300884361b73ad00be20fb507ec2a369cb` (`fix: resolve persisted lease identities canonically (jit:a6a9b964)`); repository HEAD additionally contains this state-only handoff commit.
- Source working tree: clean. Only JIT gate evidence plus this handoff/progress update are uncommitted.
- Gate state at HEAD: `cargo-ci` passed; `code-review` failed with one medium blocking test-fixture finding.
- Rework count: `a6a9b964 = 2`; a new open `rework-exceeded` escalation is recorded in `dev/active/cdc840ad-progress.json`.
- Installed JIT: `jit 0.2.1 (commit e17bda30, dirty=false, profile release)` from clean worktree `.agents/worktrees/lead-install-clean`.

## What happened in session 7

- The invoker authorized the minimal owned-image correction and explicitly required the issue gates afterward.
- Commit `1e273ff3` made `MaterializationPlan` own its captured `RepositoryImage` and reduced `RepositoryMutationSession::apply` to one plan argument. Independent final review passed; the full JIT library suite passed.
- The first formal code-review run then identified a real lease-identity bug: prefix-only matching could conflate two UUIDs sharing the first eight characters. It also reported a stale integration-branch constraint already superseded by the invoker-approved direct-to-main wave model; the issue description was reconciled accordingly in `6413baf5`.
- The lease correction now resolves persisted candidate IDs through `IssueStore::resolve_issue_id`, uses canonical equality as the identity decision, and keeps the lease/claim command format unchanged. It also always replays the append-only claims log under the coordinator lock, so a syntactically valid but stale index cannot duplicate a lease after an append-before-index-publication crash.
- New regressions cover unique legacy short-ID convergence, relevant ambiguity with zero mutation, unrelated ambiguity availability, colliding full UUIDs, expired unresolvable leases, and valid-but-stale index recovery.
- Independent adversarial review initially found the stale-index and unrelated-ambiguity gaps; both were fixed and the same reviewer returned PASS. Lead verification passed 1,992 library tests with 4 ignored.
- Commit `e17bda30` contains the reviewed correction. A provenance-matched JIT was installed from the clean installer worktree.
- Formal `cargo-ci` run `91c99cf4-e801-4de5-9f9d-b80f0fdfcef7` passed at `e17bda30`: fmt, Clippy, 3,714 tests, provenance, footprint budget, and incremental-state checks all passed.
- Formal `code-review` run `cbee4805-53b0-400d-891d-7ded5ffcec3e` failed on one new medium finding: `create_test_issue_with_id` in `crates/jit/src/commands/claim.rs` introduced fresh `Issue::new` and `save_issue` callers despite the scope amendment freezing those predecessor APIs until wave 5.
- The invoker requested an immediate handoff before that final fixture-only correction was attempted.

## Exact uncommitted state

- `.jit/events.jsonl` — formal gate events
- `.jit/issues/a6a9b964-d944-4d7f-a7e7-7fb88dcee13b.json` — current gate statuses
- `.jit/gate-runs/91c99cf4-e801-4de5-9f9d-b80f0fdfcef7/` — passing cargo-ci evidence
- `.jit/gate-runs/cbee4805-53b0-400d-891d-7ded5ffcec3e/` — failing code-review evidence and structured F1
- `dev/active/cdc840ad-progress.json` — rework count raised to 2 and open escalation recorded
- `dev/active/cdc840ad-handoff-7.md` — this handoff

Do not discard or overwrite the gate evidence. No source files are currently modified.

## What to do next

- [ ] Ask the invoker to resolve the open `rework-exceeded` escalation before changing source. Recommended resolution: authorize one narrow test-fixture-only correction.
- [ ] Remove the new predecessor caller while preserving all collision regressions. The smallest likely approach is to extend/reuse the pre-existing `create_test_issue` fixture boundary so the textual/API caller count for `Issue::new` and `save_issue` does not increase; using the typed finalizer is also valid if it stays fixture-only and proportionate.
- [ ] Run the claim coordinator and claim command focused suites, formatting, Clippy, diff check, then one independent read-only review against code-review F1.
- [ ] Commit the fixture correction with `jit:a6a9b964` traceability.
- [ ] Advance `.agents/worktrees/lead-install-clean` to the new commit, reinstall with `./scripts/install-jit.sh --force`, and verify `jit --version` matches the exact clean HEAD.
- [ ] Rerun **both** configured issue gates, even though cargo-ci passed at `e17bda30`, because the correction creates a new commit. Inspect structured findings rather than overriding them.
- [ ] If both gates pass, complete `a6a9b964`, commit JIT/gate state separately, update wave 3 to done/current wave to 4, run downstream/status/validate checks, and continue the epic in strict dependency waves.

## Cumulative review-finding ledger for wave 3

- Semantic plan hash omitted the mutation context seed/intents/evidence — closed before `1e273ff3`.
- Public plan fields allowed stale delta/hash pairing — closed before `1e273ff3`.
- Direct `HashMap` serialization made equivalent retries nondeterministic — closed before `1e273ff3`.
- Image and plan were separately supplied to `apply` — closed by owned-image one-argument apply in `1e273ff3`.
- Stale integration-branch constraint contradicted the invoker-approved direct-to-main model — closed by issue amendment recorded in `6413baf5`.
- Prefix-only persisted lease matching conflated colliding IDs — closed in `e17bda30`.
- Valid-but-stale claims index could miss a durable log lease — closed in `e17bda30` by authoritative log replay.
- Resolving every active legacy lease could let unrelated ambiguity block the repository — closed in `e17bda30` by candidate shortlisting plus canonical resolution.
- New collision-test helper adds frozen predecessor callers — **open**, code-review F1 in run `cbee4805-53b0-400d-891d-7ded5ffcec3e`.

## Traps — do not repeat these

- Do not describe the image/plan defect as attacker forgery. It was an ordinary accidental API-pairing correctness problem and is already fixed structurally.
- Do not weaken or override code-review F1. The finding follows the invoker-approved wave-3 scope amendment; fix only the new test fixture caller.
- Do not expand wave 3 into the wave-5 predecessor deletion. `49adf23b` owns migration and deletion of all remaining callers; wave 3 must merely add none.
- Do not trust `verify_index_consistency` to detect a valid-but-stale claims index; synchronized acquire intentionally rebuilds from the append-only log.
- Do not resolve every persisted lease on every claim. Prefixes shortlist plausible candidates only; repository resolution and canonical equality decide identity.
- Do not build in `/tmp`, do not run concurrent Cargo jobs against one target, and clear only generated `target/debug/incremental` state before cargo-ci if it is non-empty.
- Do not run formal gates with a stale installed JIT. Use the clean installer worktree and verify embedded commit/dirty provenance first.
- Full claim/serve tests need repository lock writes and loopback sockets; run the full suite outside the restricted sandbox when required.

## Open escalation needing invoker input

- Category: `rework-exceeded`
- Issue: `a6a9b964`
- Situation: the second gate/rework cycle reached the configured limit after code-review found one new fixture-only predecessor caller.
- Recommendation: authorize one narrow correction that removes `create_test_issue_with_id` as a new `Issue::new`/`save_issue` caller, preserves the collision coverage, and reruns focused review plus both issue gates.

## Reference artefacts

- Epic: `jit issue show cdc840ad --json`
- Active issue and scope amendment: `jit issue show a6a9b964 --json`
- Progress: `dev/active/cdc840ad-progress.json`
- Plan: `dev/archive/cdc840ad-plan.md`
- Passing cargo-ci: `jit gate status a6a9b964 cargo-ci --all --json`
- Open finding: `jit gate status a6a9b964 code-review --findings --json`
- Source correction commit: `e17bda300884361b73ad00be20fb507ec2a369cb`
- External references: none
