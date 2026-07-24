# Handoff — Repository-state quality hardening (1cc809de) — session 4

**Date:** 2026-07-25
**Session number:** 4
**Prior handoffs:** `dev/active/1cc809de-handoff.md`, `dev/active/1cc809de-handoff-2.md`, `dev/active/1cc809de-handoff-3.md`

## Current state

- Epic: `1cc809de` — state: in_progress; epic gates `repo-validate` and `holistic-review` remain pending until the implementation interior is complete.
- Wave in progress: **wave 5 of 8 COMPLETE (12/12)** — `dd5b43b6` reported after the initial handoff commit and was merged, gated (cargo-ci + code-review ✓✓), reviewed PASS, and completed within session 4. `current_wave` is 6; wave 6 has NOT been dispatched.
- Children summary: waves 1–5 fully done (26 issues incl. stories S1 `d159f9d4` and S3 `d752293b`); waves 6–8 (6 issues incl. stories S2/S4/S5) pending.
- Active claims: none. All workers shut down gracefully.
- Open escalations: none awaiting input. Three rework-cap escalations were resolved by invoker guidance this session (412925b9 ×1, 531ce80d ×2 — see progress file `escalations`).
- Progress file: `dev/active/1cc809de-progress.json` (current).

## What just happened

- Wave 2: `a6ee4e23` PASS first round; `a4e5ca3d` needed 1 lead-review rework (claim.rs:212 misclassified as self-managed against the authoritative plan inventory) — both Done.
- Wave 3: `9298b912` PASS first round (lock ordering verified byte-identical). `58c2f0e9` PASS first round (spec-internal REQ-01-vs-REQ-05 tension resolved in favor of the explicit mapping; recorded in review). `412925b9` took 4 rework rounds — R1 non-session preview authorizing skips, R2 omitted Done-redirect gate check, R3 abort-on-verification-failure, R4 apply-less stale skips — final design: `is_provable_noop` nominates only; ONE shared verification session confirms via the authoritative `derive_field_update` (shared `CapturedFieldUpdate::bulk` constructor) and an empty-delta `SessionStep::Apply` for apply-instant atomicity; `.unwrap_or_default()` demotes all candidates on verification failure. Invoker amended REQ-01/02/04 (budget C*J+1) via AskUserQuestion; amendment carried into story `d752293b` REQ-10 citing `@/issue/412925b9/decision/D-1`.
- Wave 4: `a4b0fadf` PASS first round (bulk scenario `issue_update_bulk_mutation_k30_j15`, artifact `dev/studies/perf/session-cost-c488ef85.json`, doc-linked). Story S1 `d159f9d4` FAILED story-level review once (15 residual per-site `RetryableConflict` arms in `capture_*` helpers); arm-fold rework (`worker-d159f9d4`, commit `c493af1e`) routed them through `capture_or_retry`/`classify_apply`; PASS round 2. Story S3 `d752293b` PASS first round.
- Wave 5a (10 parallel workers): `eb4be05c`, `27c8256a` (107 MCP warnings → 0, resolver scoping + productive-recursion cycle handling), `5b121c3f` (sabotage-tested discriminating property), `9503c8b4` (−225 lines), `7ab27a9b` (Cow VirtualPath + 12 consts + ALL_KNOWN), `39e1c091` (395-line architecture doc + stale pointer sweep), `ab69a4ef`, `8becd4a1` (repair_target_paths authority), `e6d5e440` (3,900-line test split + 10 renames incl. 3 finding1 tests added by lead direction) — all PASS. `531ce80d` took 4 reworks across 6 review rounds: the reviewer walked the audit-threshold ladder (fail-open aggregation → `moderate` → `low` → ambient npm config → cargo-audit warning categories); final state: explicit `npm audit --audit-level=info` ×2 + `cargo audit --deny warnings` + whole-surface fail-closed sweep with source-level config-precedence proof.
- Wave 5b: `28254964` PASS first round, all 3 gates (clippy/cargo-ci/code-review). Worker corrected two spec assumptions with compiler evidence (deps never build with `cfg(test)` → `with_repository_state_failure_view` and `RepositoryIndex::mark_deleted` widened to `#[cfg(any(test, feature = "test-support"))]`).
- Wave 5c: `dd5b43b6` dispatched with anti-flake directives (barriers, invariant assertions, 10× loop proof); result NOT yet received.
- One cargo-ci flake diagnosed and re-run clean: `test_execute_waits_for_competing_repository_write_guard` timed out under 10-parallel-worker CPU load during `8becd4a1`'s first gate run; test pre-existing, untouched by that issue.
- `jit recover` cleaned a stale claims.lock twice after gate-evaluation shell timeouts.
- Net line count since `a95dd58a` reviewed with invoker: production Rust ≈ +3,250 (dominated by inline `#[cfg(test)]` tests and typed-error enums); dedup waves strongly negative (−146/−61/−32/−225).

## What to do next

- [ ] ~~Await `worker-dd5b43b6`~~ DONE within session 4: merged (`b468921c`), gates passed, completed (`c425c422`). Gate-chain pattern for wave 6: background script per issue — evaluate cargo-ci, commit evidence, reinstall via `./scripts/install-jit.sh`, `rm -rf target/debug/incremental`, evaluate code-review, commit, reinstall.
- [ ] Wave 6 (four tasks then two stories): `1781aec2` (VirtualPath const call-site migration; touch list in plan manifest — wide but mechanical), `67a77503` (visibility-enforced cutover guard), `73d9070f` (delete dead export + demote publics), `42ee0dd5` (gate rule-serialization publics behind test-support twin pattern per 28254964). Conflict analysis: 1781aec2 overlaps most repository_state/commands files — dispatch it SOLO first or in a worktree with the other three serialized after; 73d9070f/42ee0dd5 both touch visibility surfaces — check exact files before parallelizing. Then stories S2 `2958105e`, S4 `755cf453` (story-level gates; expect the story reviewer to read against EPIC criteria, not just task criteria — see Traps).
- [ ] Wave 7: `f87e3273` (successor deck; REQ-16 traceability — every falsifiable claim cites a repository artifact); wave 8: `cba48167` (tombstone), story S5 `9ee14023`, then Section 10 epic completion (map all 16 REQs, run `jit gate evaluate-all 1cc809de`, completion report, archive).
- [ ] Dispatch-prompt policy (worked, keep): workers verify fmt + clippy + targeted tests + `cargo test --workspace --no-run`, commit, report immediately; lead's cargo-ci gate is the authoritative full-suite run.
- [ ] Surface in the completion report: the `28254964` label/parent mismatch (progress `surfaced_pitfalls`), the cargo-audit `severity_threshold` tool limitation (no CLI override; machine-local `~/.cargo/audit.toml` could suppress low-CVSS advisories — flagged by worker, unfixable in-script), and the claim.rs single-vs-self-managed classification ambiguity now made structural by the combinator API.

## Traps — do not repeat these

- **All traps in handoffs 1–3 remain in force** (zsh word-splitting; never prune live worktrees; sequential gate evaluation from repo root; commit-exact reinstall before every evaluate; cargo-ci incremental-dir cleanup; historical docs are not stale narrative; worker `.jit` snapshots stale; no reuse of completed worktrees; attribute combined-tree regressions to their producer).
- **Do NOT pipe the dispatch script through a filter.** `dispatch-worker-worktree.sh ... | grep '^\[ok\]'` swallowed the script's clean-tree refusal (uncommitted failed-gate `.jit` evidence made main dirty); the worker then found no worktree. Evidence: `worker-d159f9d4`'s "worktree does not exist" report. Run it bare; commit ALL gate evidence (pass or fail) before dispatching.
- **Do NOT run `jit gate evaluate` in a foreground Bash call.** Two evaluations were SIGTERM-killed at the 10-min shell cap mid-run (one left a stale claims.lock; `jit recover` cleaned it). Run gate chains via `run_in_background` scripts; the chain must reinstall the binary after every evidence commit (see `gate-chain-*.sh` pattern in this session's scratchpad — recreate, scratchpad is session-scoped).
- **Do NOT merge or commit on main while a background gate chain is running.** The chain's reinstall makes binary==HEAD; any interleaved commit desyncs provenance and the next evaluate fails the stale-binary guard. Queue merges until CHAIN-COMPLETE.
- **Do NOT read a worker's "self-managed"/classification claim as authoritative over the plan.** `a4e5ca3d`'s worker misclassified claim.rs:212 against the plan's explicit inventory (plan lines 101/135). The plan binds; verify per-site classifications against it.
- **Do NOT patch single adversarial-review findings on policy-flavored issues.** `531ce80d` burned 4 review rounds as the reviewer walked one threshold vector per round. When a finding implies a CLASS (fail-open vectors, narration, visibility), fix the whole class in one rework with an exhaustiveness table (the whole-surface rework passed immediately). Same lesson as the doc-review memory.
- **Do NOT let workers run full `cargo test --workspace` as their last step.** Main stall vector: 15–40 min under contention; turn ends; finished work sits uncommitted (ab69a4ef: 2+ h, committed only on nudge). Targeted tests + `--no-run` compile proof worker-side; cargo-ci authoritative post-merge. Stall heuristic: idle ping + dirty worktree + newest mtime >15 min ⇒ nudge with a concrete directive; recent mtime ⇒ working, leave alone.
- **Do NOT ask a mid-report worker "where are you?"** Workers process queued inbox messages one turn behind; three crossed-timing echo rounds with `worker-412925b9` resulted. Send an authoritative state ledger ("commits X,Y merged and gated; only Z pending") instead.
- **Story-level reviews read EPIC/story criteria, not the union of task criteria.** `d159f9d4` failed on residual capture-helper conflict arms that every task-level review had passed (the plan even sanctioned them). Before running a story gate, re-read the story's REQ text literally and sweep for anything task scoping left behind (this is how the 3 `finding1` renames were caught pre-gate for e6d5e440).
- **A load-induced test timeout is not automatically the gated issue's defect.** `test_execute_waits_for_competing_repository_write_guard` failed once under 10-worker load and passed on a quiet re-run; the failed evidence commit stays in history (`d25db1a7`). Verify the test predates the issue and re-run quiet before attributing.

## Open questions needing invoker input

None. All three rework-cap escalations were resolved with invoker guidance this session; no scope decisions are pending.

## Reference artefacts

- Epic: `jit issue show 1cc809de`
- Plan (authoritative breakdown): `dev/active/1cc809de-plan.md`; manifest `dev/active/1cc809de-breakdown.json`
- Progress: `dev/active/1cc809de-progress.json`
- Prior handoffs: `dev/active/1cc809de-handoff{,-2,-3}.md`
- Audit (evidence source): `dev/studies/cdc840ad-audit-2026-07-23.md`
- New architecture doc: `dev/architecture/repository-state-materialization.md`
- Perf artifacts: `dev/studies/perf/session-cost-27ffbd2d.json` (baseline/schema), `dev/studies/perf/session-cost-c488ef85.json` (bulk scenario)
- 412925b9 amended criteria + D-1: `jit issue show 412925b9`
- Key completion commits this session: waves 2–4 (`8d67e7fe`, `c7476204`, `04f0cebc`, `9e4951ed`, `020042ec`, `8127aa66`, `045b9f2f`, `b7b91392`), wave 5a (`f8a057c5`, `5acb733b`, `4a516676`, `32260b01`, `8a286703`, plus e6d5e440 in `0832cb8c`), wave 5b (`848181ff`)
