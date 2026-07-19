# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 5

**Date:** 2026-07-19T14:55:00+03:00
**Session number:** 5
**Prior handoffs:** `dev/active/cdc840ad-handoff.md`, `dev/active/cdc840ad-handoff-2.md`, `dev/active/cdc840ad-handoff-3.md`, `dev/active/cdc840ad-handoff-4.md`

## Current state

- Epic: `cdc840ad` — state: backlog, assigned to `agent:jit-execution-lead`
- Wave in progress: wave 3 of 9 (waves 1–2 done)
- Children summary: 2 implementation children done (`cbc3a7e5`, `bacf2cd4`), 1 in_progress (`a6a9b964`), 8 backlog; bracket nodes done
- Active claims: `cdc840ad` → `agent:jit-execution-lead`; `a6a9b964` → `agent:worker`
- Open escalations: None. Wave 3 has one open code-review gate finding (run `af64bdba`, seed-not-in-plan-hash) awaiting a rework round; rework count 0 of 2 — no invoker input needed.
- Progress file: `dev/active/cdc840ad-progress.json` (reflects the above)

## What just happened

- Resumed wave 2, dispatched rework attempt 2 for `bacf2cd4`; worker closed 9/10 findings, flagged cross-filesystem staging (finding 2); lead directed an in-constraint companion design (inside `Worktree(.jit-bootstrap/transactions/{id})`, marker-distinguished, no new `.jit` literal); worker implemented it.
- Two independent reviews: closure review PASS (all 10 findings verified closed); crash-consistency review FAIL with 4 lead-verified findings (multi-data-root bootstrap-lock gap with orphan-sweep data loss; committed-recovery worktree wedge; memory failure-point parity; tautological held-FD revalidation). Invoker authorized a guided correction with counter reset; worker fixed all 4 (worktree-keyed lock registry, owner-digest scoping, committed-arm worktree skip, full failure-point modeling, fresh-path revalidation).
- Crash reviewer re-review PASS with 2 new LOWs (lexical-path owner-digest residue leak; untimed in-process owner-wait AB-BA hang); worker fixed both (canonicalized digests, timed owner-wait).
- Committed `bacf2cd4` implementation as `ceb5ed1d`. cargo-ci passed (after clearing leftover `incremental/` dirs). code-review then ran six rounds: R1 consumer-adoption → invoker-approved scope amendment (`438a83f6`); R2 delta cross-root physical-alias gap → fixed `e52ab07c`; R3 journal-recovery alias recheck → fixed `ea7fe9e1` (+ invoker-authorized spec-clause sweep, counter reset); R4 worktree-only absent-root parity → fixed `b8c05942`; R5 second parity sibling → invoker chose a batch conformance audit; R6 zero findings, PASS.
- Batch audit: first run on Fable stopped by the invoker for cost; salvaged the completed parity auditor (5 findings) from the journal; re-ran 6 remaining footprints on Opus with Sonnet verifiers. The workflow `args` arrived as a string so the salvaged-verification branch silently no-oped; verified the 5 salvaged findings with direct Sonnet agents instead. Net: 9 confirmed findings + round-5 gate finding + 1 refuted; worker fixed all 10 in one pass (`1aa0cd4e`); both gates passed; `bacf2cd4` done, wave 2 complete (`172d3e88`).
- AGENTS.md incident: an unexplained working-tree edit ("greenfield, no backward compatibility") was reverted on suspicion of agent tampering; the invoker later confirmed it was their own edit; restored and committed as `a0de7468`.
- Wave 3: claimed and dispatched `a6a9b964` on an Opus worker with wave-2 lessons baked in. Worker asked the producer-vs-consumer boundary question; lead ruled (wave 5's "Deletion happens here" is decisive) and the invoker pre-approved a scope amendment (`46894584`): wave 3 = finalizer machinery + claim-acquire migration only; producer migration and `Issue::new`/`Event::new_*` deletion = wave 5.
- Worker built `repository_state::mutation` (MutationContext, IdAuthority/MutationClock, frozen allocation order, canonical audit append with torn-tail certification, deterministic index membership) and `storage::guard_order`, migrated claim acquire (coordinator-held, idempotent, canonical-id, index-vs-log reconcile). Opus adversarial review PASS with findings; worker's pre-gate pass closed the latent markerless-torn-tail append corruption, deleted dead `seed_fact`, added 2 finalize-driven parity tests.
- Committed as `59f535a3` (1976 lib tests, 64 doctests, clippy clean). cargo-ci PASSED. code-review FAILED with one high finding (run `af64bdba`): the mutation-context seed does not enter the semantic plan hash — `finalize` returns only a `RepositoryDelta` and the store hashes only the serialized delta (`repository_state_store.rs:894`); the established full plan-hash API (accepting captured evidence + seed) is never invoked by the mutation path. Session ended here per invoker instruction (handoff after gates).

## What to do next

- [ ] Dispatch the wave-3 gate rework (rework 1 of 2, counter is 0): route the mutation path's transaction identity through the established full plan-hash API so the context seed and captured evidence enter the semantic plan hash — the gate finding names the API and cites `mutation.rs:614` / `repository_state_store.rs:894`. The prior worker deferred this to wave 4 believing `plan_hash(image, seed, intent, delta)` is derive-path machinery; the gate rejected that reading — comply, do not re-argue. Keep cross-backend hash equality (extend the parity tests to assert it explicitly).
- [ ] Re-run both gates after the fix (reinstall jit from a clean worktree at the new HEAD first).
- [ ] On PASS: complete `a6a9b964` (state done), commit JIT state, advance the progress file to wave 4 (`44d318ab`).
- [ ] Wave-5 carry-items (record when reaching `49adf23b`): (a) reviewer finding — guard-order rejection covers only the acquire path; renew/heartbeat/release/force-evict take `claims.lock` unguarded (defense-in-depth); (b) the deferred finalizer record classes (issue delete, registry declarations, completion/gate-state/first-ready timestamp wiring) land with their consumers; (c) `Issue::new`/`Issue::new_with_labels`/`Event::new_*`/independent publishers are frozen deletion debt — wave 5 deletes them as a pure caller-swap; (d) the shared `create_test_issue` helper migrates with them.
- [ ] Wave-4 carry-item: fold the context seed into the derive-path plan hash if the wave-3 gate fix has not already unified them.

## Traps — do not repeat these

- **Do not defer a literal spec clause on an architectural reading the gate has not accepted.** "Every allocated identity and the context seed enter the semantic plan hash" was deferred to wave 4 with a reasoned note; the independent reviewer flagged it LOW and the gate failed it HIGH the same day. Evidence: gate run `af64bdba`. If a clause cannot be met in-wave, get an invoker-approved amendment BEFORE the gate, as was done twice for wave boundaries (`438a83f6`, `46894584`).
- **Do not pass workflow args as anything but pure JSON values, and verify the branch consumed them.** The audit workflow's salvaged findings arrived as a JSON string; `args.salvaged` was undefined and the verification branch no-oped silently (agent count exposed it: 12 = 6 auditors + 6 fresh verifiers, zero salvage verifiers). Check the run's journal.jsonl agent count against expectation before trusting an empty branch.
- **Do not treat teammate idle pings as completion or as stalls.** Idle notifications raced with SendMessage deliveries all session. Protocol that worked: check the worktree for concrete evidence (`git status`, targeted `rg`) before nudging; nudge once with a resend summary; workers also idle after every report — those pings need no action. One nudge fired on a stale pre-commit view of the tree (work was already committed at HEAD) — check `git log` too, not just the diff.
- **Do not revert unexplained working-tree edits without asking the invoker first.** The AGENTS.md "greenfield" line looked like sub-agent review-tampering (undeclared, mid-worker-window, governance file) and was reverted; it was the invoker's own edit. Surfacing + exclusion from commits would have sufficed. It is now committed (`a0de7468`) and is general guidance, not a wave-scope directive.
- **Do not verify a claimed fix only through the reporting worker's tests.** Wave 2's six gate rounds each surfaced a real, distinct, spec-cited gap after green worker reports. The converging remedies were: spec-clause conformance tables (made mandatory in dispatches), memory-mirrors-the-kernel parity discipline, and the batch audit with adversarial verification. Wave-3 dispatches already bake these in — keep doing so.
- **Do not run `jit gate evaluate` from a deleted cwd or with a stale binary.** One evaluate failed with ENOENT after `git worktree remove` of the cwd (use a subshell for the install cd); the provenance guard requires the installed binary's commit == HEAD and dirty=false — the main tree is permanently dirty-adjacent, so always install from a clean temp worktree at HEAD (pattern: `git worktree add /home/vkaskivuo/.cache/jit-install-worktree <HEAD>`, install inside a subshell, remove, then evaluate). Also `rm -rf target/debug/incremental` before cargo-ci (its incremental-state check fails on leftovers; one provenance-suite failure was transient — re-run before diagnosing).
- **Do not sample IDs/time in constructors on the finalizer path.** The inline-variant + `assign_identity` + `sentinel_time()` pattern replaced Event::new_* there precisely so no-op discipline is provable with PanicClock. New intents must follow it.
- **`RepositoryDelta::new` sorts actions by path with Worktree < Data.** Index-keyed failure points hit different actions than authoring order suggests; a wave-2 parity gap hid behind exactly this. Any index-keyed reasoning or test must account for the sort.
- Prior traps remain in force (handoff.md through handoff-4.md), especially: no `/tmp` builds (disk-backed `CARGO_TARGET_DIR=/home/vkaskivuo/.cache/jit-cdc840ad-verify`, `CARGO_INCREMENTAL=0`); explicit `cargo test -p jit --doc` after ownership moves; no concurrent cargo on one target; waves land on main; batch-audit non-convergent reviews instead of round-by-round fixing.

## Open questions needing invoker input

None. The wave-3 gate finding has a concrete, gate-named fix and rework budget (0 of 2 used).

## Reference artefacts

- Epic: `jit issue show cdc840ad`
- Active issue: `jit issue show a6a9b964` (includes the wave-3 scope amendment)
- Design docs: `dev/active/cdc840ad-plan.md` (§2 binding)
- Planning docs: `dev/active/cdc840ad-research.md`, `dev/active/cdc840ad-investigation.md`
- Progress: `dev/active/cdc840ad-progress.json`
- Gate runs: wave-2 final passes cargo-ci + code-review at `1aa0cd4e`; wave-3 cargo-ci PASS and code-review FAIL `af64bdba` at `59f535a3`
- Audit: salvaged parity findings `/tmp/.../scratchpad/salvaged-parity-findings.json` (session-local; confirmed set summarized in progress escalations and the batch-fix commit `1aa0cd4e` message)
- Key commits this session: wave-2 `ceb5ed1d`, `438a83f6`, `e52ab07c`, `ea7fe9e1`, `b8c05942`, `1aa0cd4e`, `172d3e88`; wave-3 `ea917ffa`, `46894584`, `59f535a3`, `a0de7468`, `9db1dfb8`
- External references: None.
