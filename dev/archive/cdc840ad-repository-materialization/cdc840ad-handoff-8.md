# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 8

Status: archived after epic completion on 2026-07-23.

**Date:** 2026-07-19T18:10:00+03:00
**Session number:** 8
**Prior handoffs:** `dev/active/cdc840ad-handoff.md` … `dev/active/cdc840ad-handoff-7.md`

## Current state

- Epic: `cdc840ad` — state: backlog, claimed by `agent:jit-execution-lead`
- Wave in progress: wave 5 of 9 (waves 1–4 done: cbc3a7e5, bacf2cd4, a6a9b964, 44d318ab)
- Children summary: 4 done, 1 in_progress (49adf23b), rest pending per progress file
- Active claims: `49adf23b` → `agent:worker` (wave-5 package, mid-flight)
- Open escalations: none
- Repository HEAD: `087174d2`, source tree clean. Installed jit: `0.2.1 (commit 916606ee)` — STALE vs HEAD; reinstall from `.agents/worktrees/lead-install-clean` before any gate run.
- Progress file: `dev/active/cdc840ad-progress.json` (current)

## What just happened

- Session-7 escalation resolved: invoker authorized fixture-only correction + counter reset. `d1edcb1b` consolidated claim-test fixtures to one grandfathered call-site boundary.
- Wave 3 gates round: cargo-ci passed; code-review found REQ-01 sampling violation (IdAuthority::random drew two UUIDs; eager context sampling on no-op claim). Rework `d01298d2`: single-UUID SHA-256-expanded seed, lazy-once draw, plan hash omits undrawn seed, panic-clock no-op regression. Both gates passed → `a6a9b964` done (`1c040abd`).
- Wave 4 (`44d318ab`): delivery model reconciled (`9ecbe13b`); worker landed relocation (validation::{defaults,serialize,projection,project_render,rules_gates_projection} → repository_state; invariants → declarations), one-way pipeline (derive/compare), rules.toml span-ownership repair, obsolete-schema proven-ownership deletion — 8 commits `084e3302..f6df8a2a`.
- Wave-4 boundary escalation: issue text described live validate/--fix + render wiring, but plan §3 (line 90) assigns 44d318ab only the REQ-06 mechanism; invoker approved amending to the plan boundary (`d2685611`).
- Wave 4 gates: cargo-ci passed; code-review F1 = duplicate rule names collapse to name-keyed ownership → custom schema deletable. Rework `916606ee`: typed `AmbiguousOwnership` poisons the whole rules/schema pass on any duplicate; sweep found+fixed sibling defect (schema shared by default+custom rule was deletable; now requires exclusive default ownership). Both gates passed → `44d318ab` done.
- Wave 5 (`49adf23b`) claimed; delivery model reconciled (`9bef65a7`). Worker A landed: `baa9a749` (repo-file store unified onto session image), `efe78262` (seeded memory store = existing data root; unseeded = absent-root pinned). Worker A self-reported context fatigue before the core rewrite and was released at a clean checkpoint.
- Fresh worker B landed: `81113cd1` (option-A: memory typed maps deleted, all IssueStore reads/writes over the byte image, both-backend crossing regressions), `d1af6c9f` (render capture-closure planner), `42cc7473` (selective render as declaration scope), `087174d2` (jit project render through a recovered mutation session).
- Lead rulings issued this session (do not relitigate): (1) option A — byte image is the single persisted source, typed caches forbidden by issue text; (2) seeded-memory-store-models-existing-root with unseeded/absent regression + doc-stated contract; (3) render single-name selectivity kept, modeled as declaration scope (charter D-9 forbids removing the capability; plan completeness clause guards producer coupling, not target scoping); (4) `with_layout(...)` is the PERMANENT executor construction seam — increment 5 deepens internals to retained recovered-session without call-site churn.

## What to do next

- [ ] Worker B checkpointed OUT cleanly at `087174d2` after this handoff was first written: increments 1–3 verified whole-set green (full `cargo test --workspace` 17 suites 0 failures, fmt/clippy clean, mcp-server npm test 9/9; the analyzer dead-code warnings were stale — workspace clippy is clean). Tree clean, worker released. No predecessor deletions have happened yet (that is inc 8), so nothing is half-removed.
- [ ] Resume wave 5: dispatch a fresh Opus worker STARTING AT INCREMENT 4 (validate), reusing the session-8 dispatch brief pattern plus these worker-B notes: replace `validation/repository.rs`'s `RepositoryView`/`validate_projections`/`render_projections` with a captured-image validate closure (full plan-§2 D14 two-phase: issues listing+files, events, registries, schemas, projection sources/targets, plan docs — the plan-doc closure is the intricate part); `--fix` → `compare_materializations`/`derive_repair` through the session, revalidate. Open sub-question for the continuation worker to resolve against the plan: validate.rs's issue-mutating fixes (type/transitive-reduction via `save_issue`) may belong with inc 7 rather than inc 4.
- [ ] Then increments: 5) init/config/profile → recovered session (deepen the `with_layout` seam to retained-session, internals only) + permanent Git-optional attributes contract + inventory deletion; 6) canonical profile consumers (delete profile/render.rs, snapshot, drift traversal, preset inventories; TargetClaim flow); 7) remaining mutations + export classification; 8) predecessor deletion per issue list + live-tree absence scans + IssueStore read/query-only.
- [ ] Foundation available to the continuation worker (established, tested): `CommandExecutor::with_layout(...)` permanent seam wired in `main.rs` AND `crates/server/src/main.rs`; `commands::declarations_from_image(image)`; the proven two-phase caller-driven capture pattern (`phase_one` → parse → `discover_paths` → recapture, conflict retry); error-taxonomy mapping of `ProjectionError`/`ManagedDocumentError` to the validation exit code; the render fixture pattern (temp worktree → `discover_repository_layout` → `with_layout` → seed aggregate image).
- [ ] On package completion: six-tier lead review, reinstall from `.agents/worktrees/lead-install-clean` at exact HEAD (verify `jit --version` commit+dirty=false), then gates SEQUENTIALLY: cargo-ci, then code-review.
- [ ] Then waves 6–9 per progress file (661d6be2 integration leaf; a3a788f3 + b1621508 + ed3e773c; c73c7618; 7f0ac1b0).

## Traps — do not repeat these

- **Do NOT trust editor/LSP diagnostics after module moves.** Three separate false alarms this session (E0433 unresolved imports, E0599, E0061 arg-count) at commits where `cargo check`/clippy were clean. Verify with cargo only.
- **Do NOT pull wave-5 consumer wiring into wave 4 (or generally: read plan §3 package-ownership lines before directing workers).** The lead's own mid-wave directive to implement validate/render wiring in 44d318ab contradicted plan §3 line 90 and was reversed by invoker-approved amendment `d2685611`. The worker's push-back was correct.
- **Do NOT treat a worker idle-ping as either completion or stall.** Worker-44d318ab idled once without starting directed work (nudge sufficed); messages also cross in flight — check ping timestamps/summaries against your own last send before reacting.
- **Do NOT key ownership classification on non-unique keys.** Reviewer finding: name-keyed HashMap let a custom rule's schema be classed default-owned. Fixed in `916606ee` by whole-pass typed AmbiguousOwnership on duplicates + exclusive-ownership check for shared schema references. Any future classifier: collision ⇒ non-repairable, never silent ownership transfer.
- **Do NOT remove `jit project render <name>` selectivity to satisfy the plan's "complete operations" sentence.** Ruled: selection is declaration scope; completeness guards producer coupling. If code-review flags it, escalate gate-vs-plan to the invoker — do not weaken either side (no-argue discipline).
- **Do NOT reintroduce typed caches in InMemoryStorage.** The two-store split (repo_files + typed maps vs session entries) hid a real parity hole — wave-3 claim tests never crossed apply→load_issue. `baa9a749`/`81113cd1` unified; crossing regressions pin it. A reconciled cache = forbidden "duplicate source-of-truth inventory".
- **Do NOT infer worktree root from storage parent.** Plan §2 forbids storage-parent inference; the layout arrives via the `with_layout` executor seam from the Git-optional worktree root (@/charter/D-4).
- Prior traps remain in force (see handoff-7 §Traps): provenance-matched install from the clean worktree before gates; sequential gate evaluates with explicit cwd; no /tmp builds; clear `target/debug/incremental` before cargo-ci; claim/serve suites need repo-lock writes + loopback outside restricted sandbox; frozen predecessor APIs (now wave-5 deletion mandate — swap and delete, no interim callers).

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show cdc840ad --json`; active issue: `jit issue show 49adf23b --json` (delivery model reconciled in `9bef65a7`)
- Progress: `dev/active/cdc840ad-progress.json`
- Plan: `dev/archive/cdc840ad-plan.md` (§2 contracts incl. D14 two-phase capture; §3 package ownership; D16/D17/D18)
- Wave-5 worker A's scoped increment plan and blocker evidence: session-8 transcript (store split, closure-planner gap) — summarized in "What to do next"
- Gate evidence: `.jit/gate-runs/` (latest passing: 44d318ab cargo-ci + code-review at `916606ee`; a6a9b964 both at `d01298d2`)
- Installer worktree: `.agents/worktrees/lead-install-clean` (currently at `916606ee`)
- External references: none
