# Completion Report

## Epic Complete: Transactional repository materialization and derived-state coherence (cdc840ad)

**Started:** 2026-07-18
**Completed:** 2026-07-23
**Assignee:** agent:jit-execution-lead

### Summary

Delivered one domain-agnostic repository-state pipeline that captures a closed
final view, plans coupled materializations without I/O, and publishes declared
and derived state through the recoverable transaction boundary. Profile,
initialization, configuration, project-render, validation, repair, audit, and
managed-region consumers now use the shared architecture; their predecessor
engines were removed rather than retained behind compatibility paths.

Completion was paused when terminal lifecycle work took roughly 90–100 seconds
in this repository. The corrective wave reduced a representative no-op terminal
retry to 2.337 seconds and a real completion plus dependent auto-promotion to
7.499 seconds, while preserving lifecycle, event, retry, and atomic-publication
semantics.

### Metrics

| Metric | Value |
|---|---|
| Transitive dependencies completed | 15 / 15 (including 2 planning-bracket nodes; 0 rejected) |
| Execution-wave issues completed | 13 / 13 |
| Waves executed | 10 |
| Tracked rework cycles | 13 across 9 issues |
| Escalations | 31, all resolved |
| Tracked sub-agent dispatches | At least 26 primary/rework dispatches; independent audit and gate-review processes were not centrally counted |
| Issues created during execution | 2 |
| Final epic gate | `repo-validate` passed in 1.538 seconds at `8b02e864` |
| Performance correction | 130 predicted mutation-session opens reduced to 2 in the deterministic regression; current-repository retry 2.337 seconds |

### Success Criteria

- [x] **REQ-01:** A domain-agnostic, side-effect-free planner derives coupled
  persisted targets from declared inputs and a bounded, closed
  `RepositoryImage`, returning one deterministic delta without publication —
  delivered by `cbc3a7e5` and integrated by `661d6be2`.
- [x] **REQ-02:** Profile initialization and application include rules,
  schemas, assets, managed regions, projections, provenance, and audit state
  from the same final view — delivered by `49adf23b` and integrated by
  `661d6be2`.
- [x] **REQ-03:** Ordinary initialization and configuration mutations use the
  shared planner and recoverable all-old/all-new transaction — delivered by
  `49adf23b` and integrated by `661d6be2`.
- [x] **REQ-04:** `jit project render` plans all configured targets from one
  snapshot and publishes deterministic shared-target composition in one
  transaction — delivered by `49adf23b` and integrated by `661d6be2`.
- [x] **REQ-05:** Static profile regions and registry projections share one
  strict marker parser and splice primitive, reject ambiguous markers, support
  valid nesting, and preserve unmanaged prose — delivered by `cbc3a7e5` and
  integrated by `661d6be2`.
- [x] **REQ-06:** Validation diagnoses stale persisted derived state and
  `--fix` transactionally restores the declared result without overwriting
  authored rule content — delivered by `44d318ab`, `8f1c2595`, and
  `b1621508`.
- [x] **REQ-07:** The registry-first `derived-state-coherence` invariant is
  projected into project guidance without broadening `single-source-prose` —
  delivered by `ed3e773c`.
- [x] **REQ-08:** In-process and CLI coverage exercises integration,
  composition, marker failures, drift and repair, idempotence, concurrency,
  recovery, Git-free operation, and stable human/JSON contracts — delivered
  across the implementation leaves and closed by `c73c7618` and `7f0ac1b0`.
- [x] **REQ-09:** Every affected mutation path cuts over directly and removes
  superseded derivation/publication paths without adapters, dual writes,
  fallbacks, or duplicate inventories — delivered by `49adf23b`, audited by
  `a3a788f3`, and integrated by `661d6be2`.

### Wave Execution Log

**Wave 1:** 1 issue — declarations, closed capture, repository image, and
shared managed-document mechanics.

**Wave 2:** 1 issue — layout-aware store, transaction kernel, rollback, and
recovery.

**Wave 3:** 1 issue — typed mutation planning, audit publication, and claim
synchronization.

**Wave 4:** 1 issue — materializers, drift diagnosis, and transactional repair
mechanics.

**Wave 5:** 1 issue — consumer migration and predecessor deletion, including
profile, init, config, render, validation, and event paths.

**Wave 6:** 1 issue — integrated repository-state cutover and full cross-surface
gate checkpoint.

**Wave 7:** 4 issues — direct-cutover audit, authored-rule preservation,
drift/repair acceptance, and the project invariant.

**Wave 8:** 1 issue — stable public contracts and comprehensive release
evidence.

**Wave 9:** 1 issue — whole-epic coherence and release-assurance rollup.

**Wave 10:** 1 issue — lifecycle performance correction discovered during
completion validation.

### Key Decisions

- Enforced the planned clean cutover: predecessor publishers, marker engines,
  planners, and inventories were deleted instead of hidden behind adapters or
  compatibility re-exports.
- Kept declared registries and configuration as semantic authority. Schemas,
  default memberships, projections, and managed regions remain reproducible
  materializations rather than alternate sources of truth.
- Preserved authored rule text during validation repair. Repair recomputes
  derived assertions but does not treat projection machinery as authority over
  user-authored semantic content.
- Added repository validation to the release-assurance rollup after review
  found that the original gate set lacked durable whole-repository evidence;
  this issue-scope change was explicitly approved by the invoker.
- Stopped epic completion when lifecycle transitions were measured near 100
  seconds. The correction filters readiness candidates from one loaded graph
  before opening mutation sessions; it adds no cache, background worker,
  compatibility path, or second transition engine.

### Escalations

All 31 recorded escalations were resolved: 22 bounded-rework renewals, 2
gate-versus-plan scope amendments, 2 non-convergent review batch audits, and one
each for an architectural decision, a plan-invariant/live-defect conflict,
shared infrastructure, an issue-scope change, and external review egress.
Invoker decisions consistently favored narrow guided correction or systematic
conformance audits over gate bypasses. No required gate was removed or waived.

### Issues Discovered During Execution

- **8f1c2595 — Preserve authored rules while repairing derived assertions.**
  Created during wave 7 after integration review found that derived-state repair
  could overwrite semantic rule authorship.
- **e10ba548 — Bound terminal-transition readiness work.** Created during final
  completion validation after a terminal state retry took roughly 90–100
  seconds. The predecessor opened two full repository sessions for every
  Backlog issue; the corrected path opens them only for snapshot-eligible
  transitions.

### Final Holistic Review

**Verdict:** PASS

- All 15 transitive dependencies are Done and all required issue gates passed.
- The epic's refreshed `repo-validate` gate passed against the final source
  change; repository validation reported no coherence, link, or DAG errors.
- Every hard epic criterion maps to concrete implementation and acceptance
  evidence, and the direct-cutover audit found no surviving alternate engine.
- Lifecycle acceptance covers both deterministic work scaling (2 session opens
  instead of 130) and end-to-end repository timing (2.337-second no-op retry,
  unchanged tracked `.jit` state). Completing the blocker and automatically
  promoting the epic took 7.499 seconds; the final epic transition took 3.747
  seconds.
- The final stale-narrative and public-contract reviews found no current claim
  that the superseded publication or derivation paths remain authoritative.
  Historical epic research still described the work as open and referenced a
  retired integration branch, so the complete epic work packet is archived
  rather than left under `dev/active`.

### Holistic Quality Notes

- The final architecture has one captured repository view, one pure planning
  boundary, and one recoverable publication mechanism. This is materially
  simpler than the command-specific engines it replaces.
- A first read of a fresh repository created 665 ignored zero-byte
  `.jit/**/*.lock` files; this working repository currently contains 693. They
  do not alter tracked semantic state, but per-record advisory-lock artifacts
  are poor storage hygiene and contribute avoidable system-call cost. A focused
  follow-up should consolidate or lazily create them, with concurrency tests.
- `JsonError::new` still accepts an unused `_command` parameter explicitly
  retained for API compatibility. This is a greenfield repository where
  compatibility is not required, so the parameter and its call-site ceremony
  are stale abstraction debt.
- MCP schema tests pass but emit numerous unresolved-definition and circular
  reference warnings. The warnings should be fixed at the generator boundary;
  their volume can hide new regressions and should not be suppressed.
- `docs/reference/cli-commands.md` says every JSON `project render` failure uses
  `PROJECT_COMMAND_FAILED`, while typed validation/projection failures use
  `VALIDATION_FAILED`. The canonical reference should describe the typed split.
- `scripts/test-ci-manual.sh` skips a missing Cargo audit tool and converts npm
  audit failures into warnings while ending with “All CI checks complete.” It
  is fail-open and must not be cited as blocking security or release evidence;
  the configured issue gates remain the authority.
