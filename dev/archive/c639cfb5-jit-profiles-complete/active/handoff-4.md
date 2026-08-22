# Handoff — Complete profile lifecycle, composition, and upgrades (c639cfb5) — session 4

**Date:** 2026-08-09
**Session number:** 4
**Prior handoffs:** `dev/active/c639cfb5-jit-profiles-complete/handoff.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-2.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-3.md`

## Current state

- Epic: `c639cfb5` — state: backlog
- Wave in progress: wave 5 of 18
- Children summary: 5 done, 1 in_progress, 18 backlog/ready, 0 rejected
- Active claims: `cbcd9318` is assigned to `agent:worker-cbcd9318`; its isolated branch `worktree-agent-cbcd9318` is preserved at evidence commit `4228d655`
- Open escalations: `cbcd9318` exceeded two rework retries after exact cargo-ci passed and code-review found first-apply ownership evidence incomplete
- Progress file: `progress.json` in the epic's artifact directory, `jit doc dir c639cfb5 dev/active` (reflects the above)

## What just happened

- Completed the authorized final Wave 4 repair for `fc47a7bf`; exact worker and merged-main Rust gates plus the MCP suite passed, then advanced to Wave 5.
- Dispatched `cbcd9318` to Terra xhigh; commit `406e22b9` implemented a pure semantic composer but production profile application bypassed it. Exact cargo-ci passed and code-review run `81ea0849-aa3f-4fbd-8839-a436292082a3` failed.
- Dispatched rework attempt 1 to Terra xhigh; commit `97a9c19d` connected production composition but exact cargo-ci run `775c00b2-a0b4-4d16-8f6e-5fb9236f24ad` exposed declaration-order regression. Lead probes also found selector-order publication, installed-record ownership conflict, and new panic-prone sites.
- Dispatched final rework attempt 2 to Luna xhigh through `codex exec`; commit `abea9028` added whole-selection conflict preflight, stable typed identities, ownership-only record convergence, and first-seen declaration grouping.
- Installed exact `abea9028`; exact cargo-ci run `ebeb613c-8442-4a8f-bd4b-f48ae24cd775` passed.
- Code-review run `ff8253d2-8c98-47df-a912-c30f3236415d` failed REQ-01: on the first apply of two equal contributors, the earlier package record contains only itself and the later record contains both owners. The new test requires a second apply to converge the first record.
- Lead review also found initialization and the public package-closure application path remain sequential without whole-selection contribution preflight.
- Preserved the complete isolated implementation and all gate history at branch commit `4228d655`; nothing from Wave 5 has merged to main.

## What to do next

- [ ] Apply the invoker's decision for `cbcd9318`. Recommended: authorize one targeted Terra xhigh retry and reset the counter, with the complete final review verdict as its literal contract.
- [ ] Require the repair to make every affected applied-profile record carry the full stable shared owner set on the first successful multi-package operation; a second convergence apply is not acceptable evidence.
- [ ] Audit ordinary repeatable-selector apply, initialization, and direct package-closure application before publication, while leaving Wave 7's one-transaction work out of scope.
- [ ] Re-read the lead review protocol in full; rerun prior-finding, criteria, stale-narrative, deferred-item, and holistic sweeps.
- [ ] Install the exact repaired commit and evaluate exact cargo-ci plus code-review. Preserve all earlier failed/pass gate runs.
- [ ] If review passes, commit evidence on the worker branch, merge into current main, install exact merged main, rerun cargo-ci plus MCP, complete `cbcd9318`, and advance to Wave 6.

## Traps — do not repeat these

- **Do not treat conflict preflight as complete composition publication.** `commands/profile.rs:606-631` preflights all claims but then publishes each package separately; code-review run `ff8253d2` proved the earlier record lacks later owners.
- **Do not accept eventual convergence as REQ-01.** `profile_cli_tests.rs:376-387` explicitly needs a second apply before the first record is repaired. The required stable shared ownership must exist after the first successful operation.
- **Do not fix only the repeatable-selector CLI path.** `apply_profile_package_with_inputs` and initialization both apply a resolved closure sequentially; lead review found the same pre-publication boundary is absent there.
- **Do not over-engineer Wave 5 into Wave 7.** `9fad8581` owns one recoverable transaction for a full selection. This issue needs correct semantic composition and initial ownership records without prematurely replacing the transaction architecture.
- **Do not derive semantic identity from `Debug`.** The earlier rework did so; `profile_apply.rs:77-114` now has the correct typed stable identity and must remain.
- **Do not restore lexicographic output grouping.** Exact cargo-ci previously failed because profile declarations must retain package declaration order; the first-seen grouping at `profile_apply.rs:326-339` closes that regression.
- **Do not add impossible-state panics.** The prior rework introduced `unreachable!`/`expect` sites for fallible production data; keep the final optional/exhaustive handling.
- **Do not apply `stash@{0}`.** It preserves separately owned pre-Wave-2 documentation and remains recoverable; current docs are a newer superset.
- **Do not run cargo-ci with a reused incremental target.** Install the exact commit and use a fresh disposable target so build provenance and footprint evidence remain authoritative.

## Open questions needing invoker input

- Question: May the lead perform one further targeted repair for `cbcd9318` after two rework retries?
  - Context: Exact cargo-ci passes, but code-review found a hard REQ-01 violation in initial shared-owner records; Wave 6 is blocked on this issue.
  - Options: authorize a targeted retry and reset the counter; take over the repair manually; or reject `cbcd9318` and stop this dependency chain.
  - Recommendation: authorize one targeted Terra xhigh retry because the failure is now concrete but spans command, initialization, and provenance boundaries; Terra is less likely than Luna to miss a coupled path and less likely than Sol to broaden the design.

## Reference artefacts

- Epic: `jit issue show c639cfb5`
- Current issue: `jit issue show cbcd9318`
- Planning docs: `dev/active/c639cfb5-jit-profiles-complete/plan.md`, `dev/active/c639cfb5-jit-profiles-complete/breakdown.json`, `dev/active/c639cfb5-jit-profiles-complete/progress.json`
- Research: `dev/active/c639cfb5-jit-profiles-complete/c639cfb5-research.md`
- Worker worktree: `.agents/worktrees/agent-cbcd9318`
- Worker branch: `worktree-agent-cbcd9318`, implementation commit `abea9028`, evidence commit `4228d655`
- Exact reviewed binary: `/tmp/jit-cbcd9318-r2-install/bin/jit`
- Latest exact gates: `.jit/gate-runs/ebeb613c-8442-4a8f-bd4b-f48ae24cd775/` (cargo pass), `.jit/gate-runs/ff8253d2-8c98-47df-a912-c30f3236415d/` (code-review fail) on the worker branch
