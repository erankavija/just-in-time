# Handoff — Complete profile lifecycle, composition, and upgrades (c639cfb5) — session 6

**Date:** 2026-08-10
**Session number:** 6
**Prior handoffs:** `dev/active/c639cfb5-jit-profiles-complete/handoff.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-2.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-3.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-4.md`, `dev/active/c639cfb5-jit-profiles-complete/handoff-5.md`

## Current state

- Epic: `c639cfb5` — state: backlog
- Wave in progress: wave 7 of 18; Wave 6 is complete
- Children summary: 7 done, 0 in_progress, 17 backlog/ready, 0 rejected
- Active claims for this wave: None
- Open escalations: None
- Progress file: `progress.json` in the epic's artifact directory, `jit doc dir c639cfb5 dev/active` (reflects the above)

## What just happened

- Completed `9a9a4cfc`, “Carry ownership claims in the applied profile record,” after two cumulative lead-review rounds and one bounded final Terra xhigh rework.
- Replaced the old applied-profile record with strict wire version 2: canonical `ProfileId`, version and compatibility provenance, origin, package digest, resolved public inputs, and sorted unique semantic/asset/managed-region claims with domain-separated base fingerprints and `retain_if_unowned` intent.
- Kept records provenance-only. Effective behavior still loads from repository registries; ordinary list/show/plan/validation paths deserialize only the current v2 record.
- Added exactly one private shipped-v1 dogfood migration boundary. It pins the exact historical five-field wire and embedded evidence, fails closed on provenance, target, unit, mode, or managed-region drift, and emits only v2 records. No general compatibility or embedded-package discovery framework was introduced.
- Made direct apply, source apply, CLI profiled-init validation, and profiled initialization mutation-aware without weakening ordinary reads. Each mutating operation authenticates legacy evidence once under its held session and carries one immutable conversion map through derivation.
- Co-publishes authenticated record conversions with initialization's existing recoverable delta. The ordinary no-migration path reuses its first captured base, preserving the final-closure race retry contract.
- Added focused regressions for strict and duplicate-member wires, typed/unique claim identities, adopted baselines, migration authentication failures, mutation-only routing, no-partial-publication initialization, and same-delta record publication.
- Exact implementation commit `78752b83` passed cargo-ci run `628e2db5-17da-4c0a-9043-5814df3286ef` (4482 tests, zero failures, plus fmt, Clippy, provenance, build-budget, and incremental checks) and code-review run `db603bcd-6035-401d-a1df-07ed2a0dfa66` with zero findings.
- Merged Wave 6 as `56d35429`. Exact merged-main cargo-ci run `33dfdb02-e50f-4d45-a6f2-aa9eb938f676` passed. The earlier run `543f46e7-5aa6-43d1-b925-69cdca2132c1` stopped in 282 ms on a pre-existing non-empty `target/debug/incremental`; that reproducible cache was quarantined and no code checker had run in the failed attempt.
- MCP verification against the exact merged binary passed 64 unit and 13 integration tests. The initial sandboxed run failed only because Node could not `spawnSync jit`; the required escalated rerun passed.
- Transitioned `9a9a4cfc` to Done, released its claim, and JIT readied Wave 7 issue `9fad8581`.

## What to do next

- [ ] Start Wave 7 with `9fad8581`, now Ready: “Publish one selection through one recoverable transaction.”
- [ ] Re-read `AGENTS.md`, content standards, the issue, the Wave 7 plan row, and this handoff before dispatch.
- [ ] Treat Wave 7 as high-difficulty cross-cutting implementation: selection/closure planning, repository bytes, v2 records, migration overlays, and audit state must enter one recoverable transaction with one no-op decision.
- [ ] Prefer Terra xhigh for the initial implementation. Use Sol only if a concrete architectural obstacle exceeds Terra, and constrain Sol to removing the sequential publication seam rather than introducing a transaction framework alongside the existing materialization session.
- [ ] Start from current main with a fresh manual worktree and an exact installed base binary; do not reuse the completed Wave 6 branch or Cargo targets.
- [ ] Review and gate Wave 7 completely before dispatching Wave 8 (`44ec7192` and `9f493686`).

## Traps — do not repeat these

- **Do not re-generalize the shipped-v1 boundary.** It exists only for the exact pinned dogfood record and historical image. Current readers remain v2-only, and `ProfileOrigin::Embedded` remains provenance-only rather than a restored discovery source.
- **Do not leave a callable per-package publication path.** Wave 7 REQ-04 requires callers to reach publication only through the aggregate selection path; wrapping the old loop in another helper does not satisfy it.
- **Do not aggregate only repository bytes.** The one transaction must cover selected closure bytes, every applied record or authenticated migration rewrite, and audit state. A failure must reveal either the complete prior image or the complete new image.
- **Do not lose no-op semantics.** Repeating an already-present selection must publish nothing and report the whole selection unchanged, not a list of per-package no-ops.
- **Do not make Git part of correctness.** Wave 7 explicitly requires the aggregate operation to work without a `.git` directory; claims and agent worktrees are orchestration concerns, not product dependencies.
- **Preserve the capture/race contracts.** The ordinary path intentionally reuses its first base when there is no migration candidate, while an expanded final closure retries. Do not mask a race test with unconditional recapture.
- **Keep Wave 8 out of Wave 7.** Base/current/candidate lifecycle decisions and the one-event-per-mutation lifecycle contract are next-wave responsibilities. Wave 7 owns transaction aggregation and the minimum event atomicity needed for its own all-or-nothing criteria.
- **Do not let Sol over-engineer the seam.** If Sol becomes necessary, prohibit parallel transaction abstractions, generic migration registries, new storage layers, or compatibility machinery beyond the issue criteria.
- **Use exact build provenance.** Install with `./scripts/install-jit.sh`; stale or dirty binaries can record gates against the wrong commit.
- **Keep the Cargo incremental preflight clean.** A non-empty repository `target/debug/incremental` causes cargo-ci to stop before testing. Quarantine or remove only that reproducible cache before a gate; preserve the failed run as environmental evidence.
- **Treat sandbox `spawnSync jit EPERM` as infrastructure.** Rerun MCP tests with escalation and the exact installed binary first on `PATH`.
- **Re-read prior handoffs' trap sections.** Their selector, closure, variable-provenance, generated-doc, shared-ownership, and stale-binary warnings remain in force.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show c639cfb5`
- Next issue: `jit issue show 9fad8581`
- Planning docs: `dev/active/c639cfb5-jit-profiles-complete/plan.md`, `dev/active/c639cfb5-jit-profiles-complete/breakdown.json`, `dev/active/c639cfb5-jit-profiles-complete/progress.json`
- Research: `dev/active/c639cfb5-jit-profiles-complete/c639cfb5-research.md`
- Completed Wave 6 issue: `jit issue show 9a9a4cfc`
- Wave 6 implementation: `78752b83`; gate evidence: `b9c98750`; merge: `56d354296beece0c6388ab6257e870ee563c728e`
- Exact merged binary: `/tmp/jit-wave6-merged-install/bin/jit`
- Gate runs: `.jit/gate-runs/628e2db5-17da-4c0a-9043-5814df3286ef/`, `.jit/gate-runs/db603bcd-6035-401d-a1df-07ed2a0dfa66/`, `.jit/gate-runs/543f46e7-5aa6-43d1-b925-69cdca2132c1/`, `.jit/gate-runs/33dfdb02-e50f-4d45-a6f2-aa9eb938f676/`
