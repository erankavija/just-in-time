# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 3

Status: archived after epic completion on 2026-07-23.

**Date:** 2026-07-19T00:30:00+03:00
**Session number:** 3
**Prior handoffs:** `dev/active/cdc840ad-handoff.md`, `dev/active/cdc840ad-handoff-2.md`

## Current state

- Epic: `cdc840ad` — state: backlog, assigned to `agent:jit-execution-lead`
- Wave in progress: **wave 2 of 9** (wave 1 complete)
- Children summary: 1 implementation child **done** (`cbc3a7e5`), wave-2 `bacf2cd4` now **ready**, rest backlog; bracket nodes done
- Open escalations: **none** — both `cbc3a7e5` escalations resolved
- Progress file: `dev/active/cdc840ad-progress.json` (wave 2, cbc3a7e5 done)
- Branch state: **wave 1 merged to `main`** at merge commit `0d224f35` (invoker-directed consolidation — deviates from the plan's "integration never merged individually" staging, done at the invoker's explicit request). The `integration/cdc840ad` branch still exists at `a7a95d8d`.

## What just happened

- Resolved the session-2 open escalation: invoker chose **lead take-over** of the boundary finding. Lead moved the ruleset loader to `storage::ruleset_store::load_ruleset`, deleted `validation::rule_loader` and its mixed caller-bytes/live-schema helper, routed all callers, fixed stale declarations prose, added boundary tests. Committed `6f4f11ac`; cargo-ci passed.
- code-review then surfaced two **new** foundational deviations (5th round, non-convergence). Invoker chose a **batch plan-conformance audit**: lead fanned out 8 disjoint-footprint auditors over `declarations` + `repository_state` vs plan §2 with an adversarial verify stage. 5 deviations confirmed (all medium, no correctness bugs; 3 candidates rejected by verify):
  1. `RootRelativePath` empty-string sentinel → `Root`/`Descendant` enum (subsumed the code-review F1).
  2. `CaptureSpec::phase_one` accepted Worktree → now Data-only (`PhaseOneRequiresDataPath`).
  3. `ExpectedPreimage` collapsed kinds → now mirrors `RepositoryEntry` (file+mode/dir/symlink/unsupported); SetMode/DeleteFile require a File preimage.
  4. managed-document engine silently re-appended an outer-removed `AppendIfAbsent` child → now rejects (`ClaimedChildRemoved`).
  5. `ConfigurationDeclarations` transitional whole-`JitConfig` field removed; consumers take narrowed components.
- Fixed all with targeted tests; committed `ff542ec1`. **Both gates passed** at `ff542ec1`; the whole-diff code-review found nothing new — the audit broke the non-convergence.
- Completed `cbc3a7e5` (done), then **merged wave 1 to main** (`0d224f35`). `jit validate` on main passes; events.jsonl union-merged (main-only parallel-session events preserved).

## What to do next

- [ ] **Reinstall jit from main first**: the installed binary is `ff542ec1`, but main HEAD is now `0d224f35`. Run `scripts/install-jit.sh --force` from a clean main worktree before any `jit gate evaluate`, or the stale-binary guard rejects it.
- [ ] Begin **wave 2**: `bacf2cd4` "Implement layout-aware store, kernel, and recovery" (ready, gates cargo-ci + code-review). Dispatch a worker (Terra) per Section 6.
- [ ] **Branch model is decided: waves land directly on `main`.** The `integration/cdc840ad` staging branch is retired — the split-brain JIT-state divergence it caused is not worth it. Wave-2+ worker commits target `main` (branch off main, land back on main). If cross-workstream JIT-state propagation is needed later, use a mechanism other than a long-lived staging branch.
- [ ] Run cargo-ci + code-review on each wave-2 leaf; complete through gates.

## Traps — do not repeat these

- **Reinstall jit after the merge.** Installed `ff542ec1` ≠ main HEAD `0d224f35`; `jit gate evaluate` will reject the stale binary. Reinstall from a clean main checkout.
- **Set `CARGO_INCREMENTAL=0` for debug test runs.** The `cargo-ci` gate's `bounded-rust-build-footprint` check FAILS if a `debug/incremental` (or `release/incremental`) directory is left under the target dir. Debug builds enable incremental by default; disable it, or `rm -rf <target>/*/incremental` before the gate.
- **Use a disk-backed target, not `/tmp`.** `/tmp` is tmpfs here; large builds exhaust it. Point `CARGO_TARGET_DIR` at a `$HOME`-backed path (this session used `/home/vkaskivuo/.cache/jit-cdc840ad-verify`).
- **Batch-audit non-convergent reviews.** When the whole-diff code-review keeps surfacing a *new* finding each round, do not fix one-at-a-time. Fan out disjoint-footprint auditors over the whole surface vs the plan, fix all confirmed deviations in one pass, then let code-review confirm. This is what finally cleared `cbc3a7e5`.
- Prior traps remain in force (handoff.md, handoff-2.md): closed-image read-error propagation; explicit doctests after ownership moves; storage owns the ruleset filesystem loader, validation evaluates only; no mixed caller-bytes/live-schema helper.

## Open questions needing invoker input

- None. Branch model resolved (waves land on `main`); invoker paused execution at the wave-1/wave-2 boundary. Resume by dispatching `bacf2cd4` on `main`.

## Reference artefacts

- Epic: `jit issue show cdc840ad`
- Design docs: `dev/archive/cdc840ad-plan.md` (§2 is the binding target architecture)
- Planning docs: `dev/active/cdc840ad-research.md`, `dev/active/cdc840ad-investigation.md`
- Result artefacts: wave-1 merge `0d224f35`; product commits `6f4f11ac`, `ff542ec1`; passing gate runs cargo-ci `a9c08886`, code-review `f96dadb3`
- Audit transcript: workflow `wf_bc2c48dc-2c7` (8 auditors + verify; 5 confirmed, 3 rejected)
- External references: None.
