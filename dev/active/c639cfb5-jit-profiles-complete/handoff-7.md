# Handoff — Complete profile lifecycle, composition, and upgrades (c639cfb5) — session 7

**Date:** 2026-08-10T00:40:30+03:00
**Session number:** 7
**Prior handoffs:** `dev/active/c639cfb5-jit-profiles-complete/handoff.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-2.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-3.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-4.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-5.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-6.md`

## Current state

- Epic: `c639cfb5` — state: backlog
- Wave in progress: wave 7 of 18
- Children summary: 7 done, 1 in_progress, 16 backlog/ready, 0 rejected
- Active claims: `9fad8581` — `agent:worker-9fad8581`, claimed at `2026-08-09T21:27:21Z`
- Open escalations: None
- Progress file: `progress.json` in the epic's artifact directory, `jit doc dir c639cfb5 dev/active` (reflects the above)

## What just happened

- Completed and handed off Wave 6 in `handoff-6.md`; main closed at `bba38a8a`, and Wave 7 became Ready.
- Re-read the execution-lead, issue-management, worktree-dispatch, repository, content, and Wave 7 planning contracts. Classified `9fad8581` as high-difficulty cross-cutting implementation rather than a well-specified Luna implementation.
- Installed exact clean base binary `/tmp/jit-wave7-base-install/bin/jit` at `bba38a8a`, ran recovery, claimed `9fad8581`, and committed the claim/progress transition as `dda209aa`.
- Created the isolated worker branch/worktree `worktree-agent-9fad8581` at `.agents/worktrees/agent-9fad8581`, exactly anchored to `dda209aa`.
- Dispatched Terra xhigh for implementation and Luna xhigh for a bounded read-only seam audit. Luna identified the direct-apply loop, per-package publisher, post-scaffold init loop, existing multi-event audit finalizer, failure injectors, no-Git fixtures, stale sequential prose, duplicate-selector result trap, migration-once constraint, and final-closure retry constraints.
- The first test patch leaked its test body into main because `apply_patch` ignored the worker's shell workdir. The dispatch snapshot exposed the leak immediately. The lead relocated the test to the worktree, restored main, and instructed all future patches to name the worktree path explicitly.
- Two fresh Cargo targets filled `/tmp` and prevented even the sandbox mount from starting. The lead removed only `/tmp/jit-wave7-red-target` and the first `/tmp/jit-wave7-target` (about 8 GB of reproducible cache), then resumed with one target path.
- Observed the focused production-path test red: `test_apply_profile_selection_publishes_multiple_packages_through_one_transaction` reports two `RepositoryBeforeControlCreation` publication boundaries for one two-package selection (`left: 2`, expected `1`). No production implementation has started.
- Interrupted the worker at the user's handoff request and preserved the test-only checkpoint as `fb58495e` (`wip(jit:9fad8581): preserve session-7 red transaction test`). The worktree and main are clean; no Cargo process remains.

## What to do next

- [ ] Resume `/root/wave7_aggregate_publication` with a follow-up task, or dispatch a Terra xhigh successor into the existing worktree. Start from `fb58495e`; do not create a new worktree or drop the red checkpoint.
- [ ] Require every `apply_patch` path to begin with the explicit Wave 7 worktree path. Run all shell commands from `.agents/worktrees/agent-9fad8581`.
- [ ] Use only `CARGO_TARGET_DIR=/tmp/jit-wave7-target` for all Wave 7 Cargo commands. The warmed target is about 3 GB and already compiles the focused red test.
- [ ] Replace the singular/per-package publication seam with one typed aggregate request over `Vec<ProfileApplicationInput>` or the equivalent single canonical collection. Retain the existing recovered mutation session and `MaterializationPlan`; do not add a transaction framework.
- [ ] Make direct selector application and profiled initialization derive one complete closure plan and call `session.apply` at most once. Initialization must include the neutral scaffold, every dependency/root target, every v2 or migrated record, and existing audit entries in the one delta.
- [ ] Combine current per-profile audit entries through existing `finalize_audit_append` in one action. Keep Wave 8's one-lifecycle-event and three-way decision work deferred.
- [ ] Preserve dependency-first unique writes, ordered duplicate root observations, shared-owner first-run completeness, one v1 authentication/conversion map, adopted retention, and final-closure retry semantics.
- [ ] Add focused aggregate no-op, mid-transaction target+record+audit atomicity/retry, profiled-init single-publication, duplicate-selector, no-Git apply/init, and structural no-bypass coverage in existing cohesive test targets.
- [ ] After the worker finishes, run `.agents/skills/jit-execution-lead/scripts/check-leak-into-main.sh` before touching main. Then perform the full six-tier lead review, bounded rework, exact gates, merge validation, issue completion, and Wave 7 handoff.

## Traps — do not repeat these

- **Do not trust `apply_patch`'s shell working directory for worktree isolation.** The first Wave 7 test body landed in main while its imports/helper landed in the worker tree. The dispatch snapshot caught `M crates/jit/src/commands/profile.rs`. Use an explicit `.agents/worktrees/agent-9fad8581/...` patch path every time and inspect both statuses after each patch batch.
- **Do not allocate one Cargo target per validation phase.** `/tmp/jit-wave7-red-target` reached 4.5 GB and `/tmp/jit-wave7-target` reached 3.5 GB, exhausting the 16 GB tmpfs and preventing sandbox startup. Reuse only `/tmp/jit-wave7-target`; serialize Cargo workloads.
- **Do not remove or recreate the Wave 7 worktree.** `fb58495e` is a lead-preserve commit containing the test helper and genuine red assertion. Resume that branch; it is intentionally not merge-ready.
- **Do not mistake the quota failure for the test result.** After quota recovery, the exact focused test ran and failed semantically because one selection published twice. That is the required red evidence.
- **Do not wrap the old loop.** `apply_one_profile_package` is the publication bypass named by REQ-04. The aggregate path must replace it for direct apply and the post-scaffold init path, not merely call it under a new helper.
- **Do not aggregate only target files.** Records, authenticated v1 rewrites, derived state, and current audit entries must share the same delta and transaction identity.
- **Do not collapse audit semantics yet.** `finalize_audit_append` already accepts multiple events. Wave 8 `9f493686` owns replacing per-profile events with one lifecycle event; Wave 7 only makes existing audit state atomic.
- **Do not lose selector/result semantics while deduplicating writes.** `resolve_profile_selectors` preserves occurrences, while `ResolvedProfileGraph::selected_packages` is dependency-first and unique. Duplicate roots remain ordered unchanged observations without causing another publication.
- **Do not weaken final-closure retry or migration authentication.** One held session must capture every aggregate path, authenticate the exact shipped-v1 image once, and retry when the final closure expands or overlaps unstably.
- **Do not let Sol over-engineer the seam.** Terra xhigh is the selected tier. If a later lead genuinely needs Sol, prohibit generic transaction layers, migration registries, alternative materializers, or Wave 8 behavior.
- **Re-read every prior handoff's trap section.** Their stale-binary, generated-source, shared-ownership, closure, variable-provenance, and migration-boundary warnings remain in force.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show c639cfb5`
- Active issue: `jit issue show 9fad8581`
- Planning docs: `dev/active/c639cfb5-jit-profiles-complete/plan.md`, `dev/active/c639cfb5-jit-profiles-complete/breakdown.json`, `dev/active/c639cfb5-jit-profiles-complete/progress.json`
- Research: `dev/active/c639cfb5-jit-profiles-complete/c639cfb5-research.md`
- Prior wave handoff: `dev/active/c639cfb5-jit-profiles-complete/handoff-6.md`
- Worker branch/worktree: `worktree-agent-9fad8581`, `.agents/worktrees/agent-9fad8581`
- Preserved red checkpoint: `fb58495e`
- Focused red test: `crates/jit/src/commands/profile.rs::tests::test_apply_profile_selection_publishes_multiple_packages_through_one_transaction`
- Exact base binary: `/tmp/jit-wave7-base-install/bin/jit`
- One reusable Cargo target: `/tmp/jit-wave7-target`
