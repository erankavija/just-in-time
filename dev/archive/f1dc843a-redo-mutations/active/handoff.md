# Handoff — Mutations commit through a redo record and never scale with project history (f1dc843a) — session 1

**Date:** 2026-09-20
**Session number:** 1
**Prior handoffs:** None.

## Current state

- Epic: `f1dc843a` — state: backlog (assignee `agent:claude`; reassignment needs a lease, see Traps)
- Wave in progress: wave 3 of 23 in the plan's spine; 5 extra tasks were created and closed outside it
- Children summary: **11 done, 1 in_progress (blocked), 35 backlog** of 47 total
- Active claims: `35544121` claimed `agent:worker`, **blocked** — see V-04. All other leads' leases are 1 h TTL and will have lapsed.
- Open escalations: **SP-05** (epic REQ-02 unreachable) and **V-04** (`35544121` unaddressed finding). Both need the invoker.
- Progress file: `progress.json` in `dev/active/f1dc843a-redo-mutations/` — carries the wave plan, `surfaced_pitfalls` SP-01..SP-05, `verifications` V-01..V-04, `process_findings` PF-01..PF-04, `escalations` ESC-01..ESC-04, `standing_policies` POL-01/POL-02, and `evidence_requirements` ER-01.

## What just happened

- Repaired `main`, which had been **red for 3 commits** (`bbd53719e`, `ffac40f80`, `3d5e0e802` — all `docs(jit:f1dc843a)` planning-bracket commits). Created and closed `ce053478`. Root cause PF-01: the planning node carries only `plan-review`, so prose commits that change `AGENTS.md` and packaged gate content never run the Rust suite.
- Wave 1 done: `835bae3f` (redo record vocabulary, 248 prod lines), `fe6fa7e5` (benchmark harness 10x-log fixture).
- Wave 2 done: `555ff4be` (durable record write; **1 rework** — three findings incl. an `@/inv/atomic-writes` violation: truncating write on the fixed record path would destroy an unapplied commit point), `faa668c9` (write-set + event-log conflict check; **failed review for missing phasing context, passed with zero code change** after ESC-02).
- Wave 3: `35544121` (file-backend apply) — **NOT DONE**, see V-04.
- Extra tasks created and closed under POL-01: `19e7298a` (claim 16 MiB ceiling), `2d4fda72` (prose assertions moved to `docs-mechanical`), `48e4f65c` (document 64 MiB ceiling), `7226f00b` (REQ-02 spike).
- `7226f00b` measured that **epic REQ-02 is unreachable** as `c88822a5` is specified. SP-05. It is wired as a hard dependency of `c88822a5`, so wave 16 cannot start until this is resolved.

## What to do next

- [ ] **Resolve SP-05 first.** Read `dev/active/7226f00b/whole-log-read-cost.md`. Decide with the invoker whether to amend `c88822a5` REQ-03 (escalation cat. 4) and, if the amendment cannot reach <10 %, whether epic REQ-02 itself changes (cat. 3). Everything from wave 16 on depends on this.
- [ ] **Unblock `35544121` (V-04).** Its code-review R1 finding is open at HEAD: `redo_apply.rs:88` takes `injector: &dyn RedoInterruptionInjector` per call; REQ-05 requires "an injector it holds". R3 passed without re-checking R1. Either change the artifact or escalate. Do **not** complete it on R3's pass.
- [ ] Then resume the spine: `4e818d10` → `d591a1c8` → story checkpoint `21e976d1` (first `holistic-review`).
- [ ] Before `21e976d1`'s gate, reconcile `surfaced_pitfalls` against that story's criteria per `lead-review-protocol.md` — a pitfall whose subject a criterion names is a criterion violation, not a follow-up.
- [ ] Carry **ER-01** into `722878d4` (wave 21): report the REQ-06 balance attributed, excluding `ce053478`, `19e7298a`, `2d4fda72`, `48e4f65c`, `7226f00b`. `2d4fda72` alone removed 707 Rust lines of prose tests; uncorrected, that flatters the storage-layer claim.
- [ ] `8b6a9e34` (wave 22) owns SP-01's live half: `dev/TESTING.md:471` cites `dev/benchmarks/test-suite-performance-4b7c06d0.json`, which is archived, plus the `transaction-benchmark-evidence` contract that LD-4 retires.

## Traps — do not repeat these

- **A backgrounded gate's "exit code 0" can mean the gate FAILED.** The notification carried the exit status of the shell pipeline (`jit gate evaluate … | tail -3`), not the gate. Acting on it would have completed `35544121` with a blocking corruption finding. Read the gate-run record or re-query `jit issue show <id> --json`. (PF-04)
- **Reviewers have no memory across rounds.** `35544121` R1 found an unaddressed blocking issue; the lead dispatched rework for R2's finding only; R3 passed declaring all criteria met while R1 was still open. Always build the Tier 1.5 cumulative table and verify each prior finding **independently at HEAD**. (V-04)
- **Do NOT evaluate a `cargo-ci` gate while a worker is building.** Two false failures on `555ff4be`: one hit LD-7's 20 s per-test bound with 6 dogfood-fixture timeouts plus a 42 s suite-duration budget breach; the next exceeded the gate's own 900 s timeout and recorded `exit=None` with empty stdout. Wait for every dispatched worker to go idle. (PF-02)
- **Do NOT reclaim more than one worktree per wave into the cache pool.** `reclaim-worker-worktree.sh` harvests "newest file wins"; reclaiming two independently-built worktrees made the pool a merge of two inconsistent build trees, and `dispatch-worker-worktree.sh` then hardlinked it forward with `cp -al`. A worker hit `E0463` on unloadable `serde` artifacts and lost 68 s to `cargo clean`. Pool deleted. Harvest one, remove the rest with `git worktree remove`. (PF-03)
- **Do NOT dispatch a worker without linking the governing plan to its issue.** The `code-review` prompt reads linked documents (`contrib/gates/code-review-prompt.md:11,26`), and `faa668c9` failed a round purely because the plan was not linked — its `Policy sources:` line showed only `AGENTS.md`. Run `jit doc add <id> dev/active/f1dc843a-redo-mutations/plan.md --doc-type design` before dispatch.
- **Do NOT draft a criterion broader than its intent.** Two self-inflicted failures: `2d4fda72` REQ-01 ("No Rust test asserts the textual content of a markdown artifact") is literally false at HEAD because generated-markdown tests remain (V-02); `48e4f65c` REQ-02 caught six listing-expanding budgets the fix deliberately excludes, needing ESC-04 to narrow it. State the scope in the criterion.
- **Do NOT edit an issue's description while a worker may run `jit doc add` on it.** That collides in `.jit/issues/<id>.json` and produces a merge conflict (hit on `48e4f65c`). Resolve with `--ours` for the lead's description, then re-run `jit doc add` on `main`.
- **`jit dep add` refuses an edge that breaks transitive reduction.** Adding `f1dc843a → 48e4f65c` was rejected because `f1dc843a → 19e7298a` became redundant. Use `jit dep add <a> <b> --reduce`.
- **Gate evaluation and description edits require an active lease**, and 1 h TTLs lapse mid-wave. Re-acquire immediately before each `jit gate evaluate` / `jit issue update --description`, not at dispatch time.
- **Worker reports truncate.** Three reports were cut mid-sentence, each costing a round trip — one truncated exactly at a blocking finding. Cap dispatch prompts' report requests at ~500 words and put the most important facts first.
- **LD-7 is real and specific**: cold-build runs trip the 20 s per-test bound on dogfood-fixture tests, and the 30 s suite-duration budget is measured warm. Rerun once warm; do not "fix" it. Do not use it to excuse a failure whose signature differs.

## Open questions needing invoker input

- Question: **Can `c88822a5` REQ-03 stand, and if not, does epic REQ-02 survive the amendment?**
  - Context: measured whole-log read is 0.89 ms/MB against a 0.34 ms/MB budget; hardware floor is 0.81 ms/MB, so there is no implementation slack. Projected 10x label update is +26 % against REQ-02's required <10 %.
  - Options: (A) amend `c88822a5` REQ-03 to take the log as length + bounded tail, adding a per-path read mode to `CaptureSpec`; (B) keep REQ-03 and amend epic REQ-02's threshold; (C) accept REQ-02 as at-risk and re-measure at wave 21.
  - Recommendation: **A.** The spike shows no label-update consumer needs whole log bytes — the two that read them are conflict machinery `ensure_write_set_unchanged` already answers with length + 4 KiB tail. B changes the epic's contract on measurement grounds alone; C defers a known-failing criterion onto finished work.

- Question: **How to close `35544121`'s REQ-05 ("an injector it holds")?**
  - Context: `apply_record` takes the injector per call; no backend holds it. The worker's position is that the session becomes the holder at wiring time (`4319256e`), mirroring the kernel's `execute_repository_delta(…, &*self.injector)`.
  - Options: (A) make a backend hold the injector now; (B) amend REQ-05 to state that the caller supplies it until wiring; (C) accept the worker's reading.
  - Recommendation: **A or B, not C.** No-argue discipline forbids the lead softening REQ-05 by argument, which is what C amounts to without an amendment.

- Question: **SP-04 — do the five listing-expanding capture budgets need a bound redesign?**
  - Context: `gate.rs:248/1108`, `gate_check.rs:781`, `template.rs:33`, `validate.rs:1253` declare the event log with 256–512 MiB ceilings and `max_listings` 2–65536. Worker assessed exposure as implausible (this repo: 3.4 MiB log over 14,946 events).
  - Recommendation: no action this epic; recorded for v1.0 triage.

## Reference artefacts

- Epic: `jit issue show f1dc843a`
- Plan: `dev/active/f1dc843a-redo-mutations/plan.md` (contracts, decisions OD-1..OD-10, LD-1..LD-11)
- Investigation: `dev/active/f1dc843a-redo-mutations/investigation.md` (baseline HEAD `3d5e0e802`; long — grep, do not read whole)
- Breakdown manifest: `dev/active/f1dc843a-redo-mutations/breakdown.json`
- Progress + all findings: `dev/active/f1dc843a-redo-mutations/progress.json`
- **REQ-02 measurement:** `dev/active/7226f00b/whole-log-read-cost.md`
- Capture-ceiling survey: `dev/active/48e4f65c/event-log-capture-ceilings.md`
- Prose relocation evidence: `dev/active/2d4fda72/relocation-evidence.md`
- Benchmark harness: `dev/active/f1dc843a-redo-mutations/bench.sh` (pre-epic baseline: label update 124 ms at 1x, 372 ms at 10x)
- REQ-06 baseline: `git diff --shortstat 34d37f8be -- '*.rs'`
