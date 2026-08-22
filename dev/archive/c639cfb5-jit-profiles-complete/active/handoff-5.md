# Handoff — Complete profile lifecycle, composition, and upgrades (c639cfb5) — session 5

**Date:** 2026-08-09
**Session number:** 5
**Prior handoffs:** `dev/active/c639cfb5-jit-profiles-complete/handoff.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-2.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-3.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-4.md`

## Current state

- Epic: `c639cfb5` — state: backlog
- Wave in progress: wave 6 of 18; Wave 5 is complete
- Children summary: 6 done, 0 in_progress, 18 backlog/ready, 0 rejected
- Active claims: None
- Open escalations: None
- Progress file: `progress.json` in the epic's artifact directory, `jit doc dir c639cfb5 dev/active` (reflects the above)

## What just happened

- Received invoker option 1 for the exhausted `cbcd9318` rework escalation and authorized one targeted Terra xhigh retry.
- Dispatched the repair on the preserved Wave 5 branch with the cumulative review verdict and explicit command, initialization, dependency-closure, provenance, and scope constraints.
- Terra changed the first-run regression test before production code, then propagated one selected/closure contribution context through `ProfileApplicationInput` while filtering each package's renderer back to its own identities.
- Added focused coverage proving complete sorted owners on the first ordinary apply, direct dependency-closure apply, and profiled initialization; direct closure and initialization conflicts now fail before any package or scaffold publication.
- Worker commit `a7746277` passed formatting, workspace Clippy, 24 focused profile harness tests, 8 profiled-init tests, 8 CLI apply tests, and full Rust tests.
- Lead Tier 1.5 confirmed all prior findings closed; stale-narrative and deferred-item sweeps found no actionable gap; no new production panic sites or unrelated scope appeared.
- Exact worker cargo-ci run `3ba6a48e-d7fd-420a-8017-49a40f8ce20b` and code-review run `7a28001f-7f86-44ac-9daa-97e643a8d762` passed. Gate evidence commit: `5d1ee848`.
- Merged Wave 5 as `c8d8d99b`; exact merged-main cargo-ci run `2fd99ed6-b428-4428-8e7c-57e43c2367f9` passed.
- MCP verification against the exact merged binary passed 64 unit and 13 integration tests. The initial sandboxed run failed only with `spawnSync jit EPERM`; the required escalated rerun passed.
- Completed `cbcd9318`, released its claim, removed its merged worker worktree, and reclaimed all disposable Cargo targets.

## What to do next

- [ ] Start Wave 6 with `9a9a4cfc`, now Ready: “Carry ownership claims in the applied profile record.”
- [ ] Re-read `AGENTS.md`, content standards, the issue, and the Wave 6 plan row before dispatch.
- [ ] Treat Wave 6 as high-difficulty cross-cutting implementation: current record format, per-target/per-identity base fingerprints, retained repository ownership, one named shipped-format conversion boundary, and provenance-only read semantics are coupled.
- [ ] Prefer Terra xhigh for the initial Wave 6 implementation. Use Sol only if the migration/materialization architecture proves genuinely beyond Terra, and explicitly constrain Sol against frameworks or compatibility machinery beyond REQ-03's one named conversion boundary.
- [ ] Use a fresh manual worktree from current main and snapshot main before dispatch; do not reuse the removed Wave 5 worktree.
- [ ] Review and gate Wave 6 completely before dispatching Wave 7 (`9fad8581`), which owns replacing per-package calls with one recoverable selection transaction.

## Traps — do not repeat these

- **Do not confuse conflict preflight with complete provenance publication.** The failed `abea9028` implementation preflighted the whole selection but still derived the earlier record from only its package; code-review run `ff8253d2` caught the incomplete first-run owner set.
- **Do not accept second-operation convergence.** The superseded test explicitly required a second apply to repair the first record. REQ-01 requires both records to be complete after the first successful operation.
- **Do not fix only the repeatable-selector CLI.** Direct dependency-closure application and profiled initialization are production entry points with the same semantic context requirement; `a7746277` now covers all three.
- **Do not pull Wave 7 into Wave 6.** Per-package sequential publication remains intentionally present after Wave 5. `9fad8581` exclusively owns one-selection/one-transaction cutover.
- **Do not infer that greenfield forbids Wave 6's specified conversion.** AGENTS permits compatibility only in a named, versioned migration boundary with a removal condition, and `9a9a4cfc` REQ-03 explicitly requires one shipped-format read-and-rewrite conversion. Implement exactly that boundary, not general compatibility support.
- **Do not let Sol expand the record task into a migration framework.** If Sol is needed, bind it to the current record, one shipped-format converter, exact ownership/base evidence, and existing materialization transaction.
- **Do not treat sandbox `spawnSync jit EPERM` as an MCP assertion failure.** Rerun with escalation and the exact installed binary first on `PATH`; the Wave 5 rerun passed 64 unit and 13 integration tests.
- **Re-read prior handoffs' trap sections.** Their stale-binary, variable-provenance, generated-doc, stash, and disposable-target warnings remain in force; they are linked above rather than duplicated here.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show c639cfb5`
- Next issue: `jit issue show 9a9a4cfc`
- Planning docs: `dev/active/c639cfb5-jit-profiles-complete/plan.md`, `dev/active/c639cfb5-jit-profiles-complete/breakdown.json`, `dev/active/c639cfb5-jit-profiles-complete/progress.json`
- Research: `dev/active/c639cfb5-jit-profiles-complete/c639cfb5-research.md`
- Completed Wave 5 issue: `jit issue show cbcd9318`
- Wave 5 implementation: `a7746277`; evidence: `5d1ee848`; merge: `c8d8d99b1c69fd4ef448af6bc1979621fe8c7689`
- Exact merged binary: `/tmp/jit-wave5-complete-install/bin/jit`
- Gate runs: `.jit/gate-runs/3ba6a48e-d7fd-420a-8017-49a40f8ce20b/`, `.jit/gate-runs/7a28001f-7f86-44ac-9daa-97e643a8d762/`, `.jit/gate-runs/2fd99ed6-b428-4428-8e7c-57e43c2367f9/`
