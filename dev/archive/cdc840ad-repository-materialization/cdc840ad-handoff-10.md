# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 10

Status: archived after epic completion on 2026-07-23.

**Date:** 2026-07-20T10:20:00+03:00
**Session number:** 10
**Prior handoffs:** `dev/active/cdc840ad-handoff.md` … `dev/active/cdc840ad-handoff-9.md`

## Current state

- Epic: `cdc840ad` — state: backlog (deps unmet by design), claimed by `agent:jit-execution-lead`
- Wave in progress: wave 5 of 9 (waves 1–4 done: cbc3a7e5, bacf2cd4, a6a9b964, 44d318ab)
- Active issue `49adf23b`: increments 1–7 DONE, **increment 8 is the only remaining work**
- Active claims: `49adf23b` → `agent:worker`. NO worker is active — workers F and G both stood down cleanly.
- Open escalations: none. The two decisions this session raised were both resolved (see below).
- Repository HEAD: `2eccabfd`, tree clean. Installed jit is STALE — reinstall from `.agents/worktrees/lead-install-clean` at exact HEAD before any gate run.
- Progress file: `dev/active/cdc840ad-progress.json` (current; carries per-increment status on wave 5)

## What just happened

Two workers, both stood down cleanly with no work in flight.

**Worker F — increment 6 (canonical profile consumers) + increment 7 item (d):**
- `6c7ca9d2` Step A: `repository_state::derive_profile_materializations` + `profile::build_profile_claims`, additive, with the predecessor planner retained as a byte-parity oracle.
- `b0e5beb9` Step B: `commands/{profile,init}` rewired onto the derivation; snapshot/planner/view usage dropped.
- `3d4af231` Step C: deleted `planner.rs`, `render.rs`, `snapshot.rs`, `preset.rs`, the `RepositoryView` triad + `render_projections`/`projection_targets`, `capture_profile_snapshot` (+helpers), `jit_dogfood_live_projection`/`preserve_nested_region`, `file_mode`. Absence scans clean.
- `6565df41` increment 7 item (d): repository-owned events compose through the finalizer (`finalize_audit_append`, inline `profile_applied_event`); retired `append_profile_event_image`, the command-side torn-tail helper, and dead `WritePolicy::IfChanged`.
- Full conformance evidence persisted at `dev/active/49adf23b-inc6-conformance.md` (`b81b215a`) — per-clause table, deletion dispositions, byte-parity method, absence scans.

**Worker G — increment 7 items (b), (c), (e):**
- `9153f0e5` (b): `evaluate_graph_rules`/`validate_scope`/`enforce_transition_graph_rules` project plan content from the captured image via `image_plan_content`; per-method `RepositoryStateStore` bound cascaded across the transition/rules/gate-check chains. Also normalized the Git-unavailable pinned-evidence reason to be control-free.
- `d3619a13` (e): `MutationIntent::UpdateIssue` (finalizer stamps `updated_at`, reconciles `created_at`/`first_ready_at` from the preimage, `MissingIssue` when absent, allocates no id); `apply_type_fix` and `fix_transitive_reduction` migrated via a reusable `publish_issue_mutation`. Required modelling ancestor `Data(...)` dirs in the memory backend and a synthetic `TestHarness` layout.
- `e4ac6863` (c): `pub finalize_config_edit` in `repository_state` (parse-before-plan, config.toml-only, SemanticMutation, complete producer set); `set_config`'s repo path publishes through the session.
- `ddb574b0` (c): plain re-init unified onto `run_initialization` — **closes the re-init `.gitattributes` gap live on main since `2e03f2d8`**, seeds config/rules/schemas through one transaction, runs the index format guard for existing roots. The transitional comment is replaced with final-behavior prose and pinned by `test_init_json_reinit_reclaims_deleted_gitattributes`.

**Lead-side repair — `02c50c02`:** `jit validate` was failing at HEAD on this repo. Root cause was stale repository DATA, not a capture defect: 25 document links on old done issues pointed through `.claude/skills/...`, a user-local convenience symlink that had been git-tracked by mistake. All 25 repointed to canonical `.agents/skills/...` (metadata preserved via `jit doc remove`/`add`); `.claude/skills` untracked; `.claude/` gitignored. Verified no linked doc path traverses a symlink and every target exists.

## What to do next

Increment 8 is the whole remaining scope of `49adf23b`. Worker G's dispatch-ready work list, entry points verified at `ddb574b0`:

- [ ] **A. `resolve_plan_content` elimination.** Definition `commands/validate.rs:1368`; sole live caller is the fallback branch at `validate.rs:1318` inside `image_plan_content`. Blocked on two mechanical fixture problems plus one decision that is now RULED (below): (i) in-memory — `tests/cli_query_graph/scope_validation_tests.rs` writes external plan docs to `storage().root().parent()` on the real filesystem and expects in-memory resolution; fix by seeding plan docs into the aggregate image or moving those cases into the `file_backed_external_plan` module already in that file; (ii) 10+ file-backed test fixtures build executors with no `.with_layout` (`fast_rules/{cli_warnings_integration,config_loading,default_rules_registry_derivation,label_membership_validation}`, `fast_docs_templates/{template_apply,template_apply_atomicity,artifact_discovery,artifact_mutation_storage,planning_preset}`, `cli_item_validate/{decision_kind,decision_risk_story,risk_kind}`) — mechanical.
- [ ] **B. `save_issue` → `UpdateIssue` sweep.** Copy the landed pattern: `publish_issue_mutation` captures `issues/{id}.json` + `events.jsonl`, creates ONE `MutationContext` before the retry loop, finalize + apply, `RetryableConflict` → continue. Finalizer pass 2b in `repository_state/mutation.rs`. Each adopting method needs a per-method `where S: RepositoryStateStore`; cargo drives the cascade. Already unblocked: memory models ancestor dirs (`memory.rs ensure_ancestor_dirs`), `TestHarness` carries a synthetic layout. The bulk chokepoint is `apply_state_transition` (`commands/mod.rs`), still on `IssueStore`.
- [ ] **C. `IssueStore` read/query-only.** Delete `save_issue`, `restore_issue_verbatim`, `delete_issue`, `append_event`, `save_gate_run_result` (plan §2). Blocked on B.
- [ ] **D. Config-publisher residue.** `seed_project_config` (callers now only `profile/dogfood.rs:514`, `commands/profile.rs:530`); `scaffold_default_rules` (`commands/mod.rs:1327`) plus its `refresh_default_schema_projections`/`sync_default_rule_membership` calls (~`mod.rs:1349-1350`), remaining callers are fast_rules tests; `user_config_store` split — `config_store::save_config_document` is now used ONLY by `set_config`'s global branch, split to user-global-only per plan §2 "Explicit retained primitives".
- [ ] **E. Absence scans.** REQ-02 requires the live tree clean NOW, not deferred to integration or assurance.
- [ ] On package completion: six-tier lead review (`references/lead-review-protocol.md` in full), reinstall jit from `.agents/worktrees/lead-install-clean` at exact HEAD (verify commit + `dirty=false`), then gates SEQUENTIALLY: cargo-ci, code-review, mcp-ci, docs-mechanical.
- [ ] Then waves 6–9 per the progress file (661d6be2; a3a788f3 + b1621508 + ed3e773c; c73c7618; 7f0ac1b0).

**Scope warning from worker G:** increment 8 is bigger than "~40 `save_issue` sites". Raw call-site counts (includes in-file `#[cfg(test)]` modules, so production is lower but still well above 40): `issue.rs` 47, `gate_check.rs` 32, `bulk_update.rs` 20, `snapshot.rs` 12, `gate.rs` 11, `archive.rs` 11, `dependency.rs` 8, `mod.rs` 5, `item.rs` 4, `validate.rs` 3. Budget 2+ workers; dispatch fresh per slice.

**Recommended ordering (worker G): D → B → A → C → E.** The ruling A depends on is already made (ruling 16). **B and D are independent and can run in parallel by two workers.** D is cheap and dependency-free; C is strictly after B; E gates everything and runs last.

**Correction to a lead hypothesis, load-bearing:** `InMemoryStorage` *can* supply a layout — worker G proved it in `d3619a13` by giving `TestHarness` a synthetic `RepositoryLayout::new(RepositoryRootEvidence::new(...), ...)` (both `pub`); the memory session accepts any valid layout and never touches those paths on disk. So "in-memory cannot supply a layout" is NOT a blocker for item A. The real blocker is fixture-shaped: an in-memory session captures the aggregate in-memory map, so plan docs written to the real filesystem under `storage.root().parent()` are invisible to it. Mechanical to fix.

## Binding lead rulings ledger (accumulated; do NOT relitigate)

Rulings 1–9 from handoff-9 remain in force. Added this session:

10. **Sibling profile derivation.** `derive_profile_materializations` feeding `finalize_profile_application` is the sanctioned route; `derive_materializations` legitimately still rejects `ApplyProfile` (its arm is the declaration-graph entry, and its wording — "finalized by their dedicated entries" — was verified still accurate).
11. **Finalizer-owned unsafe-occupant rejection** replaces the planner's explicit symlink/unsupported-target precheck. Rejection is preserved before publication; the error type changed (greenfield). If code-review flags this as a contract regression, escalate gate-vs-plan — do not revert unilaterally.
12. **synthetic-valid excluded from the profile parity oracle** — its rule/template contributions are semantically incomplete and the predecessor errors on it too. Documented in-code.
13. **`MutationIntent::UpdateIssue { issue }` is the sole new variant** for issue updates: full-issue semantic update of an issue present in the captured image; finalizer stamps `updated_at`; `created_at`/`first_ready_at` reconcile from the preimage; absent target is a typed error, never an implicit create; allocates no id (frozen order preserved). New variants are added only when their caller migrates.
14. **No new MutationIntent record class for config.** Config/registry mutation follows the declaration-edit route: edited authored bytes → session capture → declaration-derived producer graph invoking the COMPLETE producer set → one recoverable transaction. `finalize_config_edit` is constrained to config.toml, parses before planning, and validates proposals by overlaying the FINALIZED delta via probe capture. It is NOT the rejected generic bytes-in-delta seam.
15. **Capture's no-follow invariant stands unchanged** (invoker ruling, 2026-07-20). `.agents` is canonical; `.claude` is user-local and does not belong in git. The symlink-closure failure was stale data, repaired at `02c50c02`. No plan amendment.
16. **A discoverable repository layout is MANDATORY** for validate/transition/rules command paths (invoker ruling, 2026-07-20). Delete the ambient fallback; absence becomes a clear typed wiring error. The server constructs its layout from the repository mount at startup or fails there — no layout-less read-only mode survives. Evidence that this does not violate `@/charter/D-4`: `WorktreePaths::detect_from` falls back to the cwd as worktree root when there is no git repo, so requiring a layout does not require git. Delete the best-effort `None` branches at `main.rs:1883-1891` and `crates/server/src/main.rs:75-76`.

## Traps — do not repeat these

- **Do NOT diagnose a capture rejection as a capture defect before checking the DATA.** This session's headline near-miss: `jit validate` failed at HEAD with "unsafe repository mutation target", and the proposed fix was to change plan §2's no-follow capture invariant — a safety invariant — with a fallback plan of pinning assets or deferring re-init. The actual cause was 25 stale document links on old done issues pointing through a git-tracked user-local symlink. The fix was a data repair (`02c50c02`) and zero code. Before proposing any capture-semantics change: enumerate what concrete repository data enters the closure and check whether IT is wrong.
- **Do NOT let a "fallback for the layout-less case" survive as a kindness.** Worker G's `image_plan_content` retained the ambient reader as an in-memory/layout-less fallback — which is precisely the "dual path" the issue's constraints forbid. Caught at the increment boundary, ruled in ruling 16. Any future "just keep the old path for the case that can't do X" gets the same answer: solve X or make it a typed error.
- **Do NOT treat a worker's idle ping as a stall, and do NOT re-dispatch on a stale premise.** Three crossings this session (worker F's corrected handoff crossed the rulings it asked for; worker G's "(e) is blocked" crossed its own landed `d3619a13`; two idle pings arrived mid-decision). Protocol that worked every time: check `git log` AND `git status` before reacting, then nudge once with a resend summary. Nothing was re-dispatched and no work was lost.
- **Do trust a worker's own capacity read.** Both F and G self-assessed as near-exhausted and proposed bounded slices rather than starting large interdependent work; both reads were correct and both stood down with clean trees and precise residue lists. Workers C and D were lost mid-increment to the opposite pattern. Ask for a capacity read before dispatching a large increment onto a worker that just landed one.
- **A commit message claiming N tests must actually add N tests.** `ddb574b0`'s message says "two tests"; only one new `fn` was added — the other was an existing test whose stale TRANSITIONAL comment was rewritten to final-behavior prose (legitimate, and the right sweep per the stale-text rule, but the message overstates it). Verify test claims against the diff, not the message.
- **False LSP diagnostics recurred again** at every deletion/module-move commit (`E0277` bound cascades, `E0425`, `E0283`, module-not-found for legitimately deleted files). `cargo check`/`clippy` were clean at each. cargo remains the only arbiter.
### Mechanism traps from increments 6–7, not in the issue inventory (worker G)

These are concrete and will recur during the increment-8 sweep:

1. **Use PER-METHOD `where S: RepositoryStateStore`, never an impl-level bound.** An impl-level bound over-constrains `IssueStore`-only siblings and breaks the three mock stores. Let cargo drive the cascade iteratively; it converges in ~6 rounds.
2. **Expect non-obvious cascade edges.** Adding the bound to `remove_dependency` pulls in `apply_template_with`, because `commit_delta → remove_dependency → check_auto_transitions → apply_state_transition`. That is what forced `FaultyStore`/`StallingStore` to get delegating `RepositoryStateStore` impls (and `FaultyStore`'s generic bound widened). Expect more of these in item B.
3. **The memory backend modeled NO directory entries** until `d3619a13`; the first in-memory session write under `issues/` failed with "missing target parent". Fixed by `memory.rs ensure_ancestor_dirs` — any increment-8 site newly writing a nested `Data(...)` path in-memory depends on that fix being present.
4. **`declarations_from_image` REQUIRES `.jit/config.toml` in the image.** On first-time config creation the base has it absent, so declarations must be built from the config-OVERLAID image, not the base. This bit `set_config`.
5. **Whole-repo proposal validation gates on PRE-EXISTING repo validity.** Re-init and `set_config` now refuse on a repo that already fails validation, so test fixtures must be genuinely valid. Expect fixture fallout wherever increment 8 adds proposal validation.
6. **`jit init` bypasses the non-init startup `validate()` gate.** Unifying re-init therefore required explicitly re-adding the index format guard; without it the too-new-format test silently regresses from exit 10 to exit 1.
7. **`JsonFileStorage::new(temp)` often makes the DATA root the temp dir ITSELF**, so a layout needs the worktree to be its PARENT — `discover_repository_layout` rejects equal roots. Several fixtures trip on this.
8. **Pinned Git-unavailable evidence reasons must be control-free.** A multi-line git stderr produced invalid `PinnedDocumentEvidence`. Fixed, but the evidence contract rejects raw diagnostics generally.
9. **Stale FIXTURE data bites like stale live data.** `scratch_build` checks out an ANCESTOR commit, so the `.claude` repair at HEAD did not reach it. Any increment-8 work touching old-commit checkouts should assume pre-repair data.

- Prior traps remain in force (handoff-9 §Traps and its chain), especially: no generic bytes-in-delta helper in `repository_state`; validate init/profile proposals by overlaying the FINALIZED delta, never the desired-files set; every written file needs explicit `CreateDirectory` ancestors; ONE `MutationContext` created before the retry loop (init/profile finalize twice per attempt, retry 8×); provenance-matched install from the clean worktree before gates; sequential gate evaluates with explicit cwd; no `/tmp` builds; `CARGO_INCREMENTAL=0` and clear `target/*/incremental` before cargo-ci; claim/serve suites need repo-lock writes + loopback outside the restricted sandbox.

## Open questions needing invoker input

None. Both decisions raised this session were resolved (rulings 15 and 16).

## Reference artefacts

- Epic: `jit issue show cdc840ad --json`; active issue: `jit issue show 49adf23b --json`
- Plan: `dev/archive/cdc840ad-plan.md` (§2 contracts; §3 package ownership; the coverage map)
- Progress: `dev/active/cdc840ad-progress.json` (wave-5 per-increment status)
- Increment-6 conformance evidence: `dev/active/49adf23b-inc6-conformance.md`
- Worker G's increment-8 dispatch-ready work list: reproduced in full under "What to do next" above
- Gate evidence: `.jit/gate-runs/` (latest passing: 44d318ab both gates at `916606ee`)
- Installer worktree: `.agents/worktrees/lead-install-clean` (needs advancing to HEAD before gates)
- External references: none
