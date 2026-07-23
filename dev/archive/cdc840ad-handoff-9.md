# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 9

Status: archived after epic completion on 2026-07-23.

**Date:** 2026-07-20T00:15:00+03:00
**Session number:** 9
**Prior handoffs:** `dev/active/cdc840ad-handoff.md` … `dev/active/cdc840ad-handoff-8.md`

## Current state

- Epic: `cdc840ad` — state: backlog (deps unmet by design), claimed by `agent:jit-execution-lead`
- Wave in progress: wave 5 of 9 (waves 1–4 done: cbc3a7e5, bacf2cd4, a6a9b964, 44d318ab)
- Children summary: 4 done, 1 in_progress (49adf23b), rest pending per progress file
- Active claims: `49adf23b` → `agent:worker`. NO worker is active — worker E was stopped by invoker order after landing one green increment-6 commit (see Addendum); the next lead dispatches the increment-6 core fresh.
- Open escalations: none
- Repository HEAD: `2e03f2d8`, tree clean. Installed jit `0.2.1 (commit 916606ee)` — STALE; reinstall from `.agents/worktrees/lead-install-clean` at exact HEAD before any gate run.
- Progress file: `dev/active/cdc840ad-progress.json` (current; this session fixed a pre-existing missing-comma JSON error in `escalations`)

## What just happened

- Resumed at `c6b1aed8`; dispatched fresh Opus worker C at increment 4 per handoff-8.
- Worker C stopped pre-code on a boundary conflict; lead ruled option (c): `validate_repository` → `&RepositoryImage` with all six callers migrated in inc 4 (only no-dual-path shape); overlay-over-image is the PERMANENT proposed-state primitive (plan §2 line 28); issue-mutating validate fixes = inc-7 scope (plan lines 40, 59).
- Increment 4 landed green (3469/0): `cb77db9a` apply_overlay + evidence accessors; `145ed8bf` validate_capture_closure; `709ff432` validate over image, six callers, no-dual-path table in commit; `3acc532b` `--fix` derived-state repair through the session.
- Lead accepted 4 worker-flagged decisions: tombstone-test drop (coverage debt → inc 7); live-repo view-test drop (covered via CLI/gate path); pinned-evidence message collapse (within plan's Git-unavailable/not-found/read-failed taxonomy); best-effort conservative closure on malformed registry.
- Worker C context-exhausted (~86%), checkpointed cleanly; worker D dispatched at increment 5.
- Worker D Q1/Q2/Q3 ruled: (A) constrained producer-intent route (`MaterializationIntent::InitializeRepository`/`ApplyProfile`) — a generic base+overrides diff planner would recreate command-local publication behind a typed API (plan §2 closed-call-graph clause); ordinary re-init/config publishers = inc 7; profile event/asset helpers carried transiently into the session delta, finalizer routing in inc 7, no new call sites.
- Increment 5 landed green in 3 commits (3740/0, clippy/fmt/doc clean): `2b7c697b` init through the recovered session (producer `repository_state/initialize.rs`; `capture_proposed_base` under ONE held guard; InitScaffold/FreshProfilePlan/publish_init_scaffold/prepare_fresh_profile/fresh-init kernel seam + duplicate render/index helpers deleted); `bdc34997` profile apply through the session (transaction_path/unix_mode/apply_embedded_profile_with_kernel/open_repository_capability deleted); `2e03f2d8` `.gitattributes` claimed inside the init transaction (typed Git evidence, 4-state report, typed preflight aborts; setup_gitattributes writer + main.rs merge deleted). Worker D's full inc-5 spec-clause conformance table is in its final report (session log).
- Two correctness finds pinned by tests: the delta needs explicit CreateDirectory for EVERY ancestor of a written file (both backends verify parents); init/profile validation must overlay the FINALIZED DELTA via probe capture, not the desired-files set (an IfAbsent file's neutral default shadows live declarations).
- Mid-slice-C the invoker declared worker D exhausted; lead released D and dispatched takeover worker E onto the then-dirty tree — but D's slice-C completion had crossed the stand-down in flight (committed `2e03f2d8`, green). E was HALTed before damage; its only edit (trivial test-import reorder in init.rs) discarded; invoker then directed E to CONTINUE — re-vectored to increment 6 from clean `2e03f2d8` (its dispatch brief already contains increments 6–8 and the debt ledger).

## What to do next

- [ ] Manage worker E through increment 6 (canonical profile consumers: delete profile/render.rs, snapshot, drift traversal, preset inventories, plan_profile_application_against/ProfileApplicationPlan; RepositoryView + Filesystem/Overlay views and render_projections/projection_targets die HERE; TargetClaim flow; drift = compare_materializations). Then increments 7 and 8 per its brief.
- [ ] Enforce the increment-7 tracked-debt ledger (all six in its conformance table): (a) apply_overlay proposed-ABSENCE vocabulary + tombstone-equivalent coverage; (b) ambient `resolve_plan_content` (validate.rs ~734, evaluate_graph_rules/scoped-validation/coverage surface) → image-projected, deletion in inc-8 scans (plan line 25); (c) ordinary re-init + config publishers (seed_project_config/scaffold_default_rules/config set/executor.init()) migrate; pin final re-init gitattributes behavior with a test — the transitional no-assertion gap is LIVE on main since `2e03f2d8`; (d) repository-owned events incl. ProfileApplied through the finalizer's ordered intents + IdAuthority; (e) issue-mutating validate fixes migrate; (f) no new predecessor-helper call sites.
- [ ] Increment 8: full predecessor deletion per issue inventory + live-tree absence scans (REQ-02: clean NOW); IssueStore read/query-only.
- [ ] On package completion: six-tier lead review (`references/lead-review-protocol.md` in full), reinstall jit from `.agents/worktrees/lead-install-clean` at exact HEAD (verify commit+dirty=false), then gates SEQUENTIALLY: cargo-ci, code-review, mcp-ci, docs-mechanical.
- [ ] Then waves 6–9 per progress file (661d6be2; a3a788f3 + b1621508 + ed3e773c; c73c7618; 7f0ac1b0).
- Binding lead rulings ledger (accumulated; do NOT relitigate): (1) option-A byte image, no typed caches; (2) seeded/unseeded memory-store contract; (3) render selectivity as declaration scope; (4) with_layout permanent seam; (5) apply_overlay permanent primitive; (6) issue-mutating validate fixes = inc 7; (7) producer-intent route for init/profile, no generic bytes-in-delta seam; (8) ordinary re-init/config = inc 7; (9) transitional re-init gitattributes gap approved with 3 conditions (documented in both conformance tables; inc-7 test pins final behavior; no silent coverage deletion).

## Traps — do not repeat these

- **Do NOT dispatch a takeover worker onto a dirty tree from a stale premise.** This session's near-miss: invoker+lead declared worker D exhausted mid-slice-C and dispatched replacement E to triage a "partial diff" — D's completion (`2e03f2d8`) had crossed the stand-down in flight, so E started editing under an expired premise. Caught by HALT; only a trivial edit to discard. Before any takeover dispatch: message the incumbent, wait one beat for a final report, and re-run `git log`/`status` IMMEDIATELY before spawning; write the brief's tree-state as "as of <time>, re-verify first", never as fact.
- **Do NOT treat stand-down/nudge crossings as anomalies — they are the norm.** Three crossings this session (nudge×2 vs. reports, stand-down vs. completion). Evidence-check (`git log` + `status`) before every reaction to an idle ping remains mandatory; a released worker may still deliver one final, valid, committed increment.
- **False LSP diagnostics recurred at cargo-clean commits** (module-not-found for files that exist or were legitimately deleted; stdlib E0308/E0603 cascades) — cargo remains the only arbiter (re-confirmed from handoff-8).
- **Do NOT let a generic "bytes-in-delta" helper into repository_state.** Worker D's option (B) (plan_from_overlay diffing command-computed bytes) was rejected: it recreates command-local publication behind a typed API and violates plan §2's closed-producer-call-graph clause. Any future "just diff base→desired" proposal gets the same answer.
- **Do NOT validate init/profile proposals by overlaying the desired-files set.** Overlay the FINALIZED DELTA via probe capture: an unwritten IfAbsent file's neutral default shadows live on-disk declarations and their closure (concretely: re-init's default gates.toml/rules.toml hiding dogfood declarations referencing jit-content-standards.json). Pinned by the re-init idempotence unit test.
- **Do NOT omit CreateDirectory ancestors from a delta.** Both backends verify parents (unlike the old kernel); a written file without its ancestor chain fails apply.
- Prior traps remain in force (handoff-8 §Traps and its chain), especially: provenance-matched install from the clean worktree before gates; sequential gate evaluates with explicit cwd; no /tmp builds; CARGO_INCREMENTAL=0 and clear `target/*/incremental` before cargo-ci; claim/serve suites need repo-lock writes + loopback outside the restricted sandbox; frozen predecessor APIs (swap-and-delete).

## Addendum (same session, after the handoff commit)

- Invoker ordered worker E stopped. Before the stop landed, E committed `2ef6e21b` (green: deletes dead `profile/drift.rs` + re-exports; full workspace 17 suites / 0 failed, clippy/fmt clean) and self-checkpointed. HEAD is now `2ef6e21b`, tree clean, NO active writer.
- **Increment-6 scoping finding (worker E, verified against code):** the brief's "delete the superseded profile inventory" understates the core. `profile/planner.rs` (44 private fns) is the profile's FULL parallel materialization engine — interpolation validation; target-layout/symlink validation; `merge_semantic_contributions` (map-entry/set-string/keyed-array toml_edit merges into config/gates/templates/rules); configured-projection re-rendering via the view triad; asset + region bytes; action classification/plan_hash. `finalize_profile_application` (inc 5) writes only OPAQUE final bytes, and `derive_materializations` explicitly REJECTS `MaterializationIntent::ApplyProfile` (repository_state/mod.rs:142-152). The shipped dogfood profile exercises the whole pipeline. Deleting the planner therefore requires NEW infrastructure (~1000+ lines + rewiring commands/{profile,init}.rs), not a rewire.
- **Banked decomposition for the increment-6 core (execute as its own focused run, each step green):**
  - Step A (hard 60%, additive): an ApplyProfile-aware derivation (`derive_materializations` extension or sibling `derive_profile_materializations`) consuming captured declarations overlaid by the profile's semantic contributions + asset `TargetClaim`s + `ManagedDocumentClaim` regions (existing `compose_managed_documents` replaces `render_managed_region`; manifest `[[region]]` maps 1:1 to `Region{owner,region_id,begin,end,content,placement=AppendIfAbsent}` with `<!-- jit:{id}:begin/end -->`). Prove byte-parity with `plan_profile_application_against` on the existing fixtures (planner-asset-only, synthetic-valid, dogfood).
  - Step B (rewire): `profile::package` exports declaration-overlay edits + asset/region claims (plan §2 "Canonical profile consumers"); repoint commands/profile.rs (plan/prepare/apply) + commands/init.rs `compute_profile_contribution`; drop their snapshot/planner/view usage. `ProfilePlanResult`: no test pins the plan_hash VALUE, only that the plan JSON carries a plan_hash key (profile_acceptance_tests.rs:629) — a fresh deterministic hash is fine.
  - Step C (delete + absence scans): planner.rs, render.rs, snapshot.rs, json.rs capture_profile_snapshot/path/entry, the view triad + render_projections/projection_targets in validation/repository.rs, ProjectedFileMode + its commands/mod.rs adapter, `jit_dogfood_live_projection`/`preserve_nested_region` (re-express `test_live_projection_matches_every_declared_source_tree_consumer` — dogfood.rs:494, real drift coverage — over the claim path; do not silently drop), and preset.rs AFTER moving `jit_dogfood_gate` off `derive_preset_projection`.
  - Correctness constraints: profile may import repository_state, repository_state must NOT import profile (claims produced in profile, compose/compare in repository_state, glue in commands); the ProfileApplied event/provenance path (append_profile_event_image, Event::new_profile_applied, torn-tail) stays untouched in inc 6 — it is inc-7 scope.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show cdc840ad --json`; active issue: `jit issue show 49adf23b --json`
- Plan: `dev/archive/cdc840ad-plan.md` (§2 contracts; §3 package ownership; the coverage map)
- Progress: `dev/active/cdc840ad-progress.json`
- Worker E dispatch brief (increments 6–8 + debt ledger + traps): session-bea3ce09 log; worker name `worker-49adf23b-e`
- Worker D final conformance table (increment 5): session-bea3ce09 log
- Obsolete: `scratchpad/worker-d-slice-c-partial.patch` (superseded by committed `2e03f2d8`)
- Gate evidence: `.jit/gate-runs/` (latest passing: 44d318ab both gates at `916606ee`)
- Installer worktree: `.agents/worktrees/lead-install-clean` (at `916606ee`, needs advancing to HEAD before gates)
- External references: none
