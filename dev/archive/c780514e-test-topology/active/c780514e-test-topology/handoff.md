# Handoff — Give each test property one owning test on shared fixtures and contracts (c780514e) — session 1

**Date:** 2026-10-23 (Europe/Helsinki)
**Session number:** 1
**Prior handoffs:** None.

## Current state

- **Paused by explicit user request**, relayed by coordinator thread `01a164c8-d7ba-7812-a630-db8dddcee05b`. Do not resume until ordered. Epic remains `in_progress`; no claim of completion.
- Wave 1 of 10: 20 of 27 tasks done, two in progress, five pending. Across the 148 implementation entries: 22 done, two in progress, 124 pending (including two earlier recovery completions from wave 2). Tracker states are authoritative; progress's `pending` groups ready/backlog work.
- Last epic main commit: `66941bb60`, no conflicted merge. Coordinator subsequently committed its handoff as `9e76157d8` and may update that separate document; this epic changes only its own handoff/progress and document link at checkpoint.
- `df505379`: branch `worktree-agent-df505379`, HEAD `ce23d3b19546d7b2bbac431d7eff91fd9bb54007`, worktree `.agents/worktrees/agent-22355a56`. Clean; no implementation edits or tests yet. Luna worker `/root/finalization_owners` paused.
- `fa9bf296`: branch `worktree-agent-fa9bf296`, WIP commit `163e3a371`, worktree `.agents/worktrees/agent-de880d9c`. Clean; only `.jit/gates.toml` changed: two static input roots added and obsolete planning sentence removed. Projection rendering, self-test, missing-tool probe, authoritative gates and merge are all pending. Root owns this WIP. Luna worker `/root/lock_preconditions` finished a read-only audit and is paused.
- Both task leases released at pause; assignments remain `agent:jit-execution-lead`, states remain `in_progress`. Reacquire on resumption. A pre-existing stale lease on unrelated `3d9a0ce3` was observed and left untouched. Epic lease acquired solely to link this handoff, then released.
- No epic build/test/review process or lock holder remains at checkpoint. Recheck host state on resume. Profile lead `01a164fd-a47b-7d23-b92d-7fb1f95e0fbb` remains paused under its separate user instruction; do not resume or merge its branch.
- Open escalations: none. Two narrow approvals are recorded below and in `progress.json`.
- Progress file: `dev/active/c780514e-test-topology/progress.json`, linked from the epic; includes completed reviews, resource actions, pitfalls, approvals, and pause checkpoint.

## What just happened

- Completed fixture foundations/adoptions, pure query ownership, doctest reduction and several shell-gate tasks; per-task criteria, gates and historical failures are recorded in progress and `.jit/gate-runs/`. All completed task leases were released.
- `5d7020b9` finalization property owners: source `1299a02fb`, main merge `ccec7c26e`, gate evidence `ff6444cd2`; four new pure owners plus extensions to existing owners, full CI 4,584 tests and 25 doctests, review passed without findings. Closed before this checkpoint.
- `9bb30f15` lock preconditions: source `4239fc42f`, main merge `6e1a19d1d`, evidence `d703f7c80`. Removed four permission-based early passes, retaining original lock assertions. Main cargo gate `f5c2b6ec-0deb-4850-8ae1-229271758c3e` and review `2caebc75-6ffd-4511-875b-b77e1905778a` passed. Root/privileged execution was not performed. Closed; all 4,584 tests and 25 doctests passed.
- Review's pre-existing advisory F1 identified unsupported `satisfies:c780514e/REQ-10` on the lock task. Coordinator explicitly approved removing only that label and corresponding manifest/plan credit, retaining invariant link, criteria, gates and code. Applied in `66941bb60`; `jit validate` passed with zero errors, warnings, divergences or rule findings. Fourteen actual REQ-10 contributors remain, recorded in progress. No unresolved finding.
- Coordinator approved `df505379` footprint expansion from three to five files: only `JsonSchema` derives and necessary imports on canonical `PresetInfo` in `gate_presets.rs` and `Lease` in `storage/claim_coordinator.rs`. Manifest/plan/progress updated in `26a67efba`, claim in `ce23d3b19`. No adapters, runtime/serde changes, dependency/features or other schema semantic changes authorized by that expansion.
- `df505379` read-only prep identifies six inline list commands, four distinct response shapes, and wrong schema key `graph_downstream` (must be `graph_rdeps`). Worker was released but paused before editing or testing.
- `fa9bf296` claimed in `703c26a26`; audited gate omits archived JSON schema oracle and storage source read for lock-mechanism prose. WIP `163e3a371` adds both. TOML before/after inspection passed; no behavioral checks yet.
- Read-only remaining-gate audit found `89406e14` missing `profiles/jit-default` input (repository `.jit/config.toml` is not read), and `c11bcdaa` self-test still prints SKIP and passes on actionlint environment failure. No definite missing static roots or skip/pass defect found for manifest or standards scan/fix gates; they still need execution and authoritative reviews.
- Earlier approved cache cleanup deleted ONLY main and `agent-22355a56` `target/debug/incremental` directories under exclusive lock after confirming no build processes. Actual free-space gain 4,741,713,920 bytes; free ~66 GiB. Durable resource action in progress, commit `250c2c816`. No authorization for further broad cleanup.

## What to do next

- [ ] Wait for explicit resumption. Read this handoff, progress, plan, manifest, repository AGENTS, execution-lead/manage/parallel skills, and current issue states; reconcile external changes before work.
- [ ] Reuse exactly the two existing physical worktrees above; do not create another cache pool. Default bounded implementation/test/docs/mechanical workers to `gpt-6-luna`, reasoning `xhigh`, `fork_turns="none"` when spawning. Existing Luna workers can be followed up. Explain a stronger model only for genuinely difficult work.
- [ ] Reacquire leases for the two in-progress tasks. Merge current main metadata into their branches before resuming. Root owns all tracker/gate/projection operations and main commits; workers own scoped source changes only.
- [ ] Resume `df505379`: implement ListEnvelope outputs in `main.rs`, aliases/extras in `output.rs`, registrations in `schema.rs`, plus the two approved derive/import sites. Distinct shapes: preset list, events (tail/query), label values with namespace, leases (status/list) with always-present warnings. Preserve explicit null Lease option fields. Register all six commands and `graph_rdeps`.
- [ ] Test first at existing cheapest owners: schema registration matrix for seven paths; one value-preservation case per distinct shape, not duplicate cases per command. Extend existing schema/wire owner; inspect actual PresetInfo/Lease item schema including nullability. Do not duplicate the later generic count-coverage task. MCP reads live `jit --schema`; no checked-in schema generation. Run `npm test` in `mcp-server` with rebuilt branch binary first on PATH, not stale installed jit. Required gates: cargo-ci, code-review, mcp-ci.
- [ ] Resume `fa9bf296`: inspect WIP, run self-test and isolated missing-tool exit-2 probe, record evidence in commit body, then render `jit project render --name rules-and-gates` and commit projection separately. Keep direct-exec/nonzero propagation. Static roots added: `dev/archive/1cc809de-repository-state-quality/studies/perf/session-cost-27ffbd2d.json` and `crates/jit/src/storage/json.rs`. Git/machine/current lock counts are dynamic observations; synthetic issue fixtures/fake jit are inline in self-test. Run required cargo-ci, session-cost-benchmark-selftest and code-review gates on integrated main before closure.
- [ ] Finish the remaining wave-1 tasks in dependency/conflict-aware order: `2c453337` manifest gate, `89406e14` shipped-policy regions gate, `7495521e` standards-fix gate, `0951556f` standards-scan gate, `c11bcdaa` workflow-contract gate. Configuration edits serialize; source tasks may overlap only in the two available worktrees. Do not start new wave-2 work until wave 1 closes.
- [ ] Audit remaining gate changes from source, not merely the scratch report. Manifest static roots currently complete; standards fixtures are inline. Workflow-contract must fail directly on missing actionlint rather than rely on gate grep of SKIP. Shipped-policy self-test clones checkout, so commit source before testing what clone observes.
- [ ] For every closure: authoritative gates, cumulative prior-findings check, each criterion, whole-workspace stale-narrative sweep, linked-doc deferred audit, holistic review, then done/release/progress. Preserve every failed run; normal in-progress feedback is not a rework cycle.

## Traps — do not repeat these

- **Every build explicitly exports `CARGO_INCREMENTAL=0`.** Main query gate `fe708443` exceeded time budget after launcher omitted this setting; worker had set it. Raw normal-priority nextest timing was diagnostic only. Corrected normal-nice gate `911fbb9e` passed 14.542 s without code/budget changes; causal attribution to incremental mode alone was not proven. Preserve both failed and successful evidence.
- **Environment changes do not invalidate input-digest failure reuse.** Query run `63c1d94d` reused the earlier failed verdict. Investigate first; only justified explicit checker rerun, never blind force or relaxed budgets.
- **Build coordination:** focused builds acquire `flock -o /tmp/jit-ci-turnstile.lock flock -o -s /tmp/jit-full-ci.lock`; full CI/main merge+gates use `-x`. Inside acquired locks require free >=60 GiB and load1 < `nproc*0.17` (4.08 on this 24-CPU host). Exit75 before work if guard fails, wait/retry without relaxing. Hold exclusive window through merge, gates and evidence checkpoint, including failure. Never pre-acquire `/run/user/1000/cargo-ci.lock`; cargo-ci owns it.
- **Sandbox failures are not source failures.** `.agents` and `.git` writes/builds require `require_escalated`; prior scoped operations were approved. Do not ask user routinely or run parallel unguarded builds. Sandbox process inspection can hide host processes; use escalated read-only inspection and select `comm` to avoid printing giant shell/environment wrappers.
- **Empty cargo-ci logs do not prove a lock wait.** The script buffers output. Inspect actual processes; do not delete lock files or terminate unrelated jobs.
- **Do not preserve three-file scope with duplicate schema adapters.** Coordinator approved canonical derives only. Unsupported fields, feature/dependency changes or broader schema semantic changes need assessment/report before further expansion.
- **Coverage labels are contribution claims, not decorative labels.** Read namespace declaration and live requirement. The lock task's unsupported REQ-10 label is now fixed; do not restore it when merging older branches. First correction attempt failed because a completed issue still requires an active lease; reacquire before mutation, then release. Later commands in an unguarded shell can still execute after an earlier command fails: verify results and use fail-fast orchestration.
- **Render projections from authority.** fa9 WIP intentionally has registry-only edits; do not merge as complete until projection is rendered and checks pass. Do not manually edit generated regions.
- **Measurements must be durable.** `871df4b0` initially failed review because timing evidence lived only in `/tmp`; linked benchmark report resolved it. Scratch logs below are convenience only; gate records/commit bodies/report carry accepted evidence.
- **No new worktrees or blanket cache cleanup.** Only the two exact incremental directories were approved and deleted. Shared hard links mean apparent sizes do not equal freed disk. More cleanup needs coordination and explicit scoped approval.
- **Do not grep raw benchmark JSON verbatim.** Use filename-only or bounded source/Markdown searches; raw files contain huge lines. Historical investigation/task backgrounds may name removed functions without being stale current guidance.

## Open questions needing invoker input

None. Resume requires orders because the user explicitly paused work. Both scope/metadata decisions are already approved; do not ask again.

## Reference artefacts

- Epic: `jit issue show c780514e`; authoritative plan `dev/active/c780514e-test-topology/plan.md`, manifest `breakdown.json`, progress `progress.json`, historical audit `investigation.md` in the same directory.
- Linked timing/footprint evidence: `dev/benchmarks/test-topology-c780514e/README.md`. Gate evidence lives in `.jit/gate-runs/*/result.json`; progress maps completed tasks to run IDs and criteria.
- Skills: `/home/vkaskivuo/.codex/skills/jit-execution-lead/SKILL.md`, `jit-manage/SKILL.md`, `jit-parallel/SKILL.md`; execution-lead's full lead-review protocol applies on every review.
- Scratch prep: `/tmp/jit-df505379-preparation.json`, `/tmp/jit-df505379-dispatch.md`, `/tmp/jit-fa9bf296-preparation.json`, `/tmp/jit-fa9bf296-lead-prep.json`, `/tmp/jit-c780514e-remaining-gate-audit.json`. Their key conclusions are preserved above; regenerate if absent.
- Completed orchestration scripts `/tmp/jit-c780514e-land-9bb30f15.sh`, `close-9bb30f15.py`, `jit-df505379-record.py` must NOT be rerun. Older completed land/close scripts likewise are history, not pending actions.
- Profile lead remains paused at clean `2cc3395bc`, with `handoff-4.md` on its owning branch only and no active processes/locks. Standalone prebuild passed after earlier 900 s timeout; final full gate/merge never ran. Do not interpret its branch as integrated main.
