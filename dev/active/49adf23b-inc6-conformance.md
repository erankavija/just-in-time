# Increment 6 conformance evidence — 49adf23b (canonical profile consumers)

Recorded by the execution lead from worker-49adf23b-f's completion report, for the
package-completion review of issue `49adf23b`. Commits: `6c7ca9d2` (Step A),
`b0e5beb9` (Step B), `3d4af231` (Step C). Final bar at `3d4af231`: clippy
`--workspace --all-targets` warning-free; fmt clean; full jit suite 3538 passed /
0 failed (incl. 64 doctests).

Lead ratifications (2026-07-20): synthetic-valid parity exclusion; finalizer-owned
unsafe-occupant rejection replacing the planner precheck; snapshot/file_mode
deletion pulled forward into Step B; sibling `derive_profile_materializations`
entry (rejection-arm text verified still accurate).

## Plan §2 "Canonical profile consumers" contract — per-clause conformance

| Clause | Where implemented |
|---|---|
| profile::package retains manifest/source validation but exports semantic contributions as declaration edits + assets/regions as canonical claims (canonical VirtualPath, entry mode, bytes) | profile/apply_claims.rs `build_profile_claims` → `ProfileClaims{registries (toml_edit-merged bytes+FileMode), assets (VirtualPath→bytes,mode), regions (VirtualPath, ManagedDocumentClaim)}` |
| profile::drift + its filesystem traversal disappear; drift = compare_materializations over the layout-keyed image | profile/drift.rs deleted (`2ef6e21b`); live-tree drift re-expressed over the claim path in profile/dogfood.rs `test_live_assets_match_every_declared_source_tree_consumer` (`3d4af231`) |
| profile::dogfood keeps embedded package loader + package-authored declaration queries; live assets/regions flow through the same claims | profile/dogfood.rs `jit_dogfood_gate` reads gate inventory from manifest contributions directly; `jit_dogfood_live_projection`/`preserve_nested_region` deleted (`3d4af231`) |
| profile::application keeps public provenance/response envelopes; imports no profile mode or final-byte type | profile/application.rs unchanged shapes; `ProfileTargetChange::new` takes `repository_state::FileMode` (was `ProjectedFileMode`) (`b0e5beb9`) |
| profile::preset no longer owns projections, asset modes, or a compatibility inventory | profile/preset.rs deleted (`3d4af231`) |
| commands::{profile,init} use only the recovered layout-aware state-store session | commands/profile.rs (list/show/plan/prepare/apply) + commands/init.rs `compute_profile_contribution` capture via `capture_proposed_base` + `open_mutation_session`; no snapshot/planner/view (`b0e5beb9`) |

## Deletion inventory — disposition

| Symbol/type | Disposition |
|---|---|
| PackageProjection, ProjectedFile, ProjectedFileMode, project_package, write_projection_tree, render_managed_region, render.rs ProjectionError | profile/render.rs deleted whole (`3d4af231`) |
| RepositorySnapshot, SnapshotEntry, SnapshotFile, SnapshotError | profile/snapshot.rs deleted whole (`3d4af231`) |
| capture_profile_snapshot, capture_profile_path, capture_profile_entry, profile_file_is_executable, repository_root_for_storage | storage/json.rs — deleted (pulled into `b0e5beb9`; only the temp parity oracle used capture_profile_snapshot) |
| jit_dogfood_live_projection, preserve_nested_region | profile/dogfood.rs deleted (`3d4af231`); drift test re-expressed over claim path (kept, not dropped) |
| PresetProjection, PresetInventory(+Finding/Kind), derive_preset_projection, compare_preset_inventory | profile/preset.rs deleted whole (`3d4af231`) |
| ProfileApplicationPlan, PlannedTarget, PlannedTargetAction, PlanIdentity, plan_profile_application(_against), ProfilePlanError | profile/planner.rs deleted whole (`3d4af231`); main.rs ProfilePlanError downcasts removed |
| RepositoryView, FilesystemRepositoryView, OverlayRepositoryView, render_projections, projection_targets (+ dead validate_relative) | validation/repository.rs deleted (`3d4af231`); the view-based shared-target test removed (coverage lives in materialize.rs compose_configured_projections tests) |
| commands::file_mode (ProjectedFileMode adapter) | commands/mod.rs deleted (`b0e5beb9`) |

## Byte-parity evidence (Step A, `6c7ca9d2`)

A temporary oracle (profile/apply_claims.rs tests, deleted in `3d4af231`) ran the
predecessor `plan_profile_application_against` (over RepositorySnapshot +
RepositoryView) and the new `build_profile_claims` + `derive_profile_materializations`
(over a captured RepositoryImage) on the same seeded repo, asserting the full
target SET plus per-target BYTES and MODE match. Fixtures: planner-asset-only
(pure-asset path) and the shipped jit-dogfood package (complete pipeline — every
contribution kind, executable assets, a region projection (invariants→AGENTS.md)
and a separate-file projection (rules-and-gates.md), and the nested
dogfood-guidance/invariants regions). Parity holds because the proposed image's
occupant equals the composed targets, so compose_configured_projections yields
the rendered projection bytes whether or not it emits an action. synthetic-valid
was excluded (its rule/template contributions are semantically incomplete — the
predecessor errors on it too; no valid full-plan oracle), documented in-code.

## Absence-scan summary (Step C, `3d4af231`)

rg over every deleted symbol across `*.rs`: clean. Only non-hits were the
unrelated local variable `projection_targets` in apply_claims.rs and the
test-local `SnapshotEntry` enum in profile_acceptance_tests.rs (independent).
Two stale doc comments in repository_state/initialize.rs that named the deleted
planner/view were swept in the same commit.

## Item (d) addendum (increment 7, worker-49adf23b-f, `6565df41`)

ProfileApplied audit events compose through the mutation finalizer:
`finalize_audit_append` (resets frozen allocation, IdAuthority id + single
timestamp, torn-tail certification) + inline `profile_applied_event` (empty id +
sentinel_time). `finalize_initialization`/`finalize_profile_application` take the
operation's single `&MutationContext` (created once before the retry loop).
Retired: `append_profile_event_image` (+ export + test),
`commands::profile::has_malformed_unterminated_event_tail`,
`Event::new_profile_applied` on the finalizer path (kept as test-only seed
constructor), dead `WritePolicy::IfChanged` + `file_matches`. Full jit suite
3537/0; clippy/fmt clean.
