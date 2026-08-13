# Handoff — Complete profile lifecycle, composition, and upgrades (c639cfb5) — session 13

**Date:** 2026-08-13T17:15:00+03:00
**Session number:** 13
**Prior handoffs:** `handoff.md`, `handoff-2.md` … `handoff-12.md` in this directory

## Current state

- Epic: `c639cfb5` — state: in_progress
- Wave in progress: wave 16 of 22 (waves renumbered this session; see progress.json)
- Children summary: 20 done, 2 in_progress (the epic itself and `5a3ecba0`), 1 ready (`cb26de35`, held), 8 backlog, 0 rejected (31 members)
- Active claims: `5a3ecba0` claimed `agent:worker` (mid-rework, see next steps); epic claimed `agent:lead-c639cfb5`
- Open escalations: none — all four of this session's owner questions were answered (see progress.json → escalations)
- Progress file: `dev/active/c639cfb5-jit-profiles-complete/progress.json`
- Main head: `5eca81456`, working tree clean apart from this handoff
- Branch `worktree-agent-5a3ecba0` at `b8572db93` in `.agents/worktrees/agent-5a3ecba0`: clean, resumable, carries rounds 1–2 rework, committed gate evidence, and both owner rulings merged

## What just happened

- **`0708d692` (out-of-epic gate-determinism bug) driven to done.** Owner folded the budget re-derivation in as REQ-06; worker re-derived all three constants from fresh measurements (all confirmed at current values; canonical doc `dev/benchmarks/rust-build-budgets/README.md`). Code-review round 1 FAILED on false incremental attribution (`comm -13` blamed concurrent rust-analyzer writes on the gate); rework moved the verdict to a runtime `CARGO_INCREMENTAL=0` assertion with additions reported as named environmental observations. Both gates passed on branch and merged tree; merged at `2103a5e7a`; done. Cold margin (26,832 ms vs 30,000 ms) accepted-and-recorded by the owner.
- **Every remaining wave now gates through the repaired script** — the incremental-preflight and cold-verdict traps are structurally retired.
- **`5a3ecba0` dispatched (wave 16) and implemented**: contribution refresh on capture, 15 tests, docs, CHANGELOG. Owner ratified the REQ-01 ownership-scoping (record as ownership evidence; unclaimed declarations report `unowned`) — recorded on the issue.
- **Three code-review rounds, three findings, each a REQ-07 budget variant**: R1 arbitrary `CLAIM_MODEL_BUDGETS=32` (fixed: budgets scale with the repository's recorded-profile count; `RECORD_CAPTURE_BUDGET` removed; depth derived from construction); R2 record read skipped the shared `validate_applied_record_path` (fixed: fifth call site of the one validator, `MisplacedRecord` error, regression test); R3 structural — model-derived byte bounds cannot cover applied artifacts (adopter-supplied variable values, template-expanded targets). **Owner ruled after R3: unbound named-path payload reads, bound shape only** (no-discovery argument, like the listing capture). Ruling appended to the issue and merged onto the branch at `b8572db93`. **The R3 rework itself was NOT dispatched — session ended on the owner's instruction.**
- **`cb26de35` held** behind `5a3ecba0` per the owner's standing serialization; the owner's "max parallelism" instruction this session did not name it, so the hold stands.
- **Wave plan rewritten**: `5a3ecba0` slotted as wave 16, `cb26de35`+`acf49914` wave 17, `f6a17e0d` checkpoint moved out of stale wave 14 to wave 18, old waves 17–20 renumbered 19–22.
- **Filed `d2b7d19b`** (out-of-epic, `epic:v1-release`, high): fixture-receipt test TempDir race flakes ~1/3 of `cargo test --workspace` runs; reproduced on an untouched base commit; gates added.
- Five merged worktrees could not be pruned (permission classifier denied `git worktree remove`): `agent-25faa21d`, `agent-76ecc11f`, `agent-96268a98`, `agent-cf42d08b`, `agent-f2389b18`.

## What to do next

- [ ] **Dispatch the R3 rework on `5a3ecba0`** into the existing worktree (`.agents/worktrees/agent-5a3ecba0`, branch at `b8572db93`). Fix shape is the owner ruling in the issue description ("Owner ruling on REQ-07's derivation sources"): remove byte ceilings from the record and claimed-target reads (`recorded_profile_bytes` / `record_capture_budget` / `claim_capture_budget` in `crates/jit/src/commands/profile.rs`), bound shape only (one listing, `applied_record_depth()`, declared-path membership), document each with the no-discovery argument `record_listing_capture_budget` already carries, and update the budget tests to pin shape bounds rather than byte multiples. fmt/clippy/workspace tests.
- [ ] **Round-4 gates in the worktree**: `jit gate evaluate 5a3ecba0 cargo-ci --force`, then `code-review --force`. The reviewer will see both owner rulings in the issue description — they are rulings, not worker interpretations.
- [ ] **Merge to main**: commit the worktree's `.jit` gate evidence on the branch FIRST (see traps), then merge; expect a conflict on the issue JSON (keep branch gate evidence, keep the newest description) and union-merge on events. Then `check-leak-into-main.sh`, merged-tree `cargo-ci --force`, `jit issue update 5a3ecba0 --state done`, commit.
- [ ] **Dispatch wave 17**: `cb26de35` (presentation contract — now unblocked) and `acf49914` (end-to-end edit-and-refresh regression test) in parallel worktrees; both were sized in session 12. Watch for output-shape coupling: `acf49914` must assert semantics, not presentation, or `cb26de35` will break it.
- [ ] Wave 18: `f6a17e0d` story checkpoint — reconcile `surfaced_pitfalls` against its criteria first (lead-review-protocol container rule). Then waves 19–22: {`b3d92595`, `6574f951`, `3dcce8a8`} → `d7558ef6` → `8021d507` → `dd268d0f`.
- [ ] Optional: ask the owner to allow `git worktree remove` and prune the five merged worktrees.

## Traps — do not repeat these

- **The cwd trap bit BOTH the lead and a worker this session, in a new form: a failed `cd X && cmd` does NOT persist the cd.** The lead's verdict commit `c81f656c2` landed on the worker branch because the previous command's `cd` to main had failed partway (its python heredoc errored) and later no-cd commands silently ran in the worktree; a worker's greps read main's copy of a file and convinced it briefly that its own function didn't exist. Prefix every state-changing command with `cd <target> &&` AND verify with a leading `pwd &&` when it matters. Handoff-12's cwd trap remains fully in force.
- **Gate records evaluated in a worktree are uncommitted dirt; merging the branch does not carry them.** `0708d692`'s branch code-review pass was never committed, main showed the gate `pending` after the merge, and the AI review had to be re-run on main (~5 min). Remedy applied for `5a3ecba0` at `b996e002d`: `git add .jit && git commit` the gate evidence on the branch before merging. Do this before every branch→main merge.
- **The code-review reviewer reads the issue description of the tree it reviews.** An owner ruling recorded only on main is invisible to a worktree gate run. Merge main into the branch (or commit the ruling in the worktree's `.jit`) BEFORE re-running the gate, or the reviewer re-flags the settled question. Both rulings for `5a3ecba0` are already on its branch.
- **Do not reintroduce any model-derived byte multiple into the profile-agreement capture.** Three review rounds each found one: a count constant, a flat record budget, then the 1× package-byte factor itself — applied records carry adopter-supplied variable values and template-expanded targets the package model cannot bound in principle. The owner's ruling (on the issue) is shape-only bounds with unbounded named-path payload reads. A byte ceiling proposed anywhere in this flow is the settled defect returning.
- **Pre-sweep every `CaptureBudget` site before a gate run on budget-adjacent work.** The reviewer finds one stated constant per round, at ~10 minutes per round. `PACKAGE_TREE_MODEL_BUDGETS = 8` (publication path) survives deliberately: REQ-07's noun does not reach publication, and it carries a documented headroom rationale — do not "fix" it without an owner ruling, and do not let a worker cite it as precedent for new constants.
- **`git merge` refuses to start over uncommitted `.jit` changes in the worktree** ("would be overwritten"), and the issue JSON conflicts field-wise: resolve by keeping the branch's gate evidence and the newest description (`git show :2:` / `:3:` + field-level merge, as at `b8572db93`). `events.jsonl` has a union merge driver and self-resolves.
- **The fixture-receipt flake is real and pre-existing** (`d2b7d19b`): `test_profiled_repository_fixture_receipt_binds_setup_and_consumer_to_shared_runtime_target` fails ~1/3 of `cargo test --workspace` runs on a TempDir race. A workspace-suite failure naming "Could not locate working directory" at `test_utils.rs:1745` is the flake, not the change. Nextest gate exposure not established.
- **`git worktree remove` is blocked by the permission classifier in this session's mode** (both with and without `--force`). Don't burn calls retrying; ask the owner or leave the trees.
- **Idle notifications from workers are not reports.** Workers idle after finishing AND after merely processing a message; twice this session a report-less idle just meant the report was still coming, and once the messages crossed mid-instruction. Nudge once via SendMessage, then verify state directly in the worktree (`git log`) rather than re-instructing.
- Prior handoffs' traps otherwise remain in force. **Retired by `0708d692`'s merge:** handoff-12's `incremental-preflight`/`rm -rf` trap and the cold-worktree suite-duration trap (the fix landed; cold margin accepted and recorded in `dev/benchmarks/rust-build-budgets/README.md`).

## Open questions needing invoker input

None. All four of this session's questions were answered by the owner and are recorded in `progress.json` → `escalations`: the budget fold into `0708d692`, the cold-margin acceptance, the REQ-01 ownership scoping, and the REQ-07 no-discovery ruling.

## Reference artefacts

- Epic: `jit issue show c639cfb5`
- In-flight issue: `jit issue show 5a3ecba0` (both owner rulings are in its description); branch `worktree-agent-5a3ecba0` at `b8572db93`
- Next wave: `jit issue show cb26de35`, `jit issue show acf49914`
- Planning docs in this directory: `plan.md`, `breakdown.json`, `progress.json`, `investigation.md`, `c639cfb5-research.md`
- Budget derivation (canonical): `dev/benchmarks/rust-build-budgets/README.md`; gate-determinism evidence: `dev/benchmarks/cold-warm-verdict-0708d692/README.md`
- Filed bugs: `jit issue show d2b7d19b` (fixture-receipt flake, out-of-epic)
- Threat model binding `b3d92595`: `dev/active/15cc28c5/15cc28c5-threat-model.md`
- Charter: `dev/vision/9db27a3a-charter.md`
