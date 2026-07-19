# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 6

**Date:** 2026-07-19T16:18:29+03:00
**Session number:** 6
**Prior handoffs:** `dev/active/cdc840ad-handoff.md`, `dev/active/cdc840ad-handoff-2.md`, `dev/active/cdc840ad-handoff-3.md`, `dev/active/cdc840ad-handoff-4.md`, `dev/active/cdc840ad-handoff-5.md`

## Current state

- Epic: `cdc840ad` — state: backlog, assigned to `agent:jit-execution-lead`
- Wave in progress: wave 3 of 9 (waves 1–2 done)
- Children summary: 2 implementation children done, `a6a9b964` in progress, 8 backlog; bracket nodes done
- Active claims: `cdc840ad` → `agent:jit-execution-lead`; `a6a9b964` → `agent:worker`
- Open escalation: `a6a9b964` exhausted two rework attempts; one final image/plan binding correction needs invoker authorization
- Progress file: `dev/active/cdc840ad-progress.json`
- Working tree: five uncommitted source files contain the reviewed wave-3 correction; do not discard them

## What just happened

- Recovered one stale claim lock and reconciled the session-5 resume state.
- Rework 1 routed the mutation context seed, canonical intents, captured evidence, semantic intent, and delta through the established full plan hash; both backends and repository journals now carry that hash unchanged. It removed both delta-only hash recomputations.
- Independent review rejected rework 1 because public plan fields allowed accidental stale delta/hash pairing and direct `HashMap` serialization made retries non-deterministic.
- The invoker approved the proportionate response: private plan fields/read-only accessors plus local canonical JSON serialization, explicitly no threat-model or storage-verification framework.
- Rework 2 implemented those changes and added opposite-insertion-order issue/delta/hash tests across JSON and memory. Mutation, repository-store, claim, doctest, Clippy, formatting, and diff checks passed.
- Lead independently ran the full JIT library suite outside the restricted sandbox: 1,984 passed, 4 ignored. The first sandboxed run's claim-lock/socket failures were environmental only.
- Final contract review passed. Final adversarial review found one remaining pairing defect: `apply(image, plan)` accepts its coupled values separately, so image A's plan can accompany image B when action-target preimages match. Rework budget is exhausted; no source commit or gate rerun occurred.

## What to do next

- [ ] Resolve the open `a6a9b964` escalation.
- [ ] If authorized, reset the rework counter and make the minimal correction: `MaterializationPlan` owns the captured `RepositoryImage`; `RepositoryMutationSession::apply` accepts only `&MaterializationPlan` and revalidates/applies `plan.image()` with `plan.delta()` and `plan.hash()`.
- [ ] Add the exact two-image regression: same action-target preimages, different extra captured evidence; cross-pairing must be unrepresentable through the public API, while honest plans apply on both backends.
- [ ] Re-run focused suites, full library tests outside the sandbox, independent review, then commit the implementation with `jit:a6a9b964`.
- [ ] Install provenance-matched JIT from a clean temporary worktree, evaluate `cargo-ci` and `code-review`, complete `a6a9b964`, and advance to wave 4 only after both pass.

## Traps — do not repeat these

- **Do not frame ordinary API pairing as an attacker forgery threat.** The invoker correctly rejected that framing. The actual defect is accidental mismatch between separately supplied image and plan; solve it structurally and minimally.
- **Do not add storage-side rehashing or retain duplicate seed/intent verification state.** The approved correction is one owned captured image and a one-argument apply boundary.
- **Do not trust a full-suite failure caused by the restricted sandbox.** Claim tests need `.git/jit` lock writes and serve tests need loopback binds. This session's sandboxed suite failed only there; the approved rerun passed 1,984/1,984 non-ignored tests.
- **Do not discard the uncommitted five-file source diff.** It contains both reviewed rework attempts and is the base for the one remaining correction.
- Prior traps remain in force, especially provenance-matched JIT installation, no builds in `/tmp`, no concurrent Cargo against one target, explicit doctests, and no later-wave consumer migration in wave 3.

## Open questions needing invoker input

- Question: Authorize one guided correction after the two-attempt limit?
  - Context: All prior findings are closed; only the separately supplied image/plan pairing remains.
  - Options: authorize the minimal owned-image correction; take over manually; reject `a6a9b964` and block the remaining epic chain.
  - Recommendation: authorize the minimal correction because it removes the mismatch by construction without a verifier or new subsystem.

## Reference artefacts

- Epic: `jit issue show cdc840ad`
- Active issue: `jit issue show a6a9b964`
- Design docs: `dev/active/cdc840ad-plan.md` (§2 binding)
- Progress: `dev/active/cdc840ad-progress.json`
- Failed gate evidence being repaired: code-review run `27ea23d0-82ba-48ae-8377-b05cc909496a`
- Current uncommitted diff: `git diff HEAD -- crates/jit/src/commands/claim.rs crates/jit/src/repository_state/mod.rs crates/jit/src/repository_state/mutation.rs crates/jit/src/storage/file_transaction.rs crates/jit/src/storage/repository_state_store.rs`
- External references: None.
