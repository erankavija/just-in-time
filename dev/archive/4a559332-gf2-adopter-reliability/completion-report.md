# Epic Complete: gf2 adopter reliability — safe worktrees, durable gates, actionable profiles (4a559332)

**Started:** 2026-08-16 (session 1: waves 1–2; session 2: waves 3–10)
**Completed:** 2026-08-17
**Assignee:** agent:jit-execution-lead

## Summary

Linked non-primary checkouts now follow a declared, auditable write policy enforced before
any repository write; an already-divergent checkout store is detectable with
`jit worktree store-divergence` and recoverable through a documented, executably witnessed
lossless procedure; gate-evaluation durability is pinned by failure-injection regression
coverage on both evaluation paths; and profile refusals and divergences carry their typed
remedies in machine-readable output with one canonical documentation home.

## Metrics

| Metric | Value |
|---|---|
| Children completed | 20 / 20 (19 planned + 1 created during execution) |
| Waves executed | 10 |
| Rework cycles | 10, per progress.json `rework_counts` (9757ab31 ×3; 9ea7f672, dd26c204, 1c8f00bd, 0c8d38be, 176cc9ed, 86ec2c50, 62518095 ×1 each) |
| Escalations | 3, per progress.json `escalations` (all resolved by the invoker) |
| Sub-agent dispatches | 20 initial (one per completed child); rework cycles were handled by re-engaging each issue's live worker rather than fresh dispatches, except 9757ab31's and 1c8f00bd's session-1 rework dispatches recorded in the session-1 handoff |
| Issues created during execution | 3 (1 in-epic, 2 standalone; see progress.json `created_during_execution`) |

## Success Criteria

- [x] REQ-01 (no silent divergent history; refuse-or-audit) — delivered by 9757ab31
  (selected-root authority), 9ea7f672 (context factory), 1c8f00bd (write-policy key),
  99ac9679 (audit event), 0c8d38be (atomic audit publication + env override),
  f52567ed (dispatch-time refusal), 1b6925a9 (deletion convergence).
- [x] REQ-02 (detect divergence; documented lossless recovery) — delivered by fc773c22
  (exact store read), f980db67 (`worktree store-divergence`), 176cc9ed + 62518095
  (recovery procedure, corrected against its executable witness), 86ec2c50 (journey).
- [x] REQ-03 (gate success implies durable record; regression coverage) — delivered by
  6a489764 (postcheck error surfacing) and 7ca9a0dd (failure-injection coverage on the
  automated and manual-attestation paths, including a cross-process assertion).
- [x] REQ-04 (actionable machine-readable profile remedies) — delivered by dd26c204
  (typed remedy data) and 7b820ccc (structured details + suggestions on refusal and
  divergence output).
- [x] REQ-05 (profile show positional selector) — delivered by 45de0b2b.
- [x] REQ-06 (end-to-end adopter-boundary proof) — delivered by 995a1903 (write-policy
  journey on one real-git checkout across three stances) and 86ec2c50 (divergence and
  recovery journey executing the documented procedure), plus 7ca9a0dd's subprocess
  assertion; guides updated by dd3d4a07 and 03678c53 so the taught workflows succeed as
  instructed.

## Wave Execution Log

**Wave 1** (3): selected-root worktree authority; one mutation-context factory; typed profile remedies.
**Wave 2** (3): exact single-store read; postcheck error propagation; positional profile show.
**Wave 3** (3): write-policy config key; override audit event; gate-durability regression coverage.
**Wave 4** (1): override audit record published atomically with its mutation; `JIT_WORKTREE_WRITE_POLICY` dispatch site.
**Wave 5** (1): `jit worktree store-divergence` (read-only, both directions, conflict class, MCP-classified).
**Wave 6** (1): structured details/suggestions on profile refusal and divergence output.
**Wave 7** (3): dispatch-time linked-checkout write refusal; divergence recovery guidance; profile remedy reference.
**Wave 8** (3): deletion refusal converged into the policy; guides state the stance; write-policy journey.
**Wave 9** (1): divergence-and-recovery journey — found three doc-procedure defects and two product gaps.
**Wave 10** (1): recovery procedure corrected to match its executable witness.

## Key Decisions

- Accepted worker-negotiated convergence of `LinkedCheckoutWriteStance` into domain with a
  byte-identical dual-branch declaration, deduplicated at merge.
- Corrected my own dispatch tooling mid-wave: the outer-flock cargo-ci invocation
  self-deadlocks (script re-execs under its own lock); switched all workers to direct
  invocation.
- Accepted `exclude` MCP classification for the store-divergence bridge tool (curation
  bound at 55/55; sibling worktree tools excluded) over my initial include instruction.
- Named the command `store-divergence` to avoid collision with branch-ancestry and
  membership-label divergence.
- Resequenced wave 10 ahead of the journey's final review when the review correctly held
  that REQ-02 requires the *documented* procedure to work: the doc fix landed first, then
  the journey re-reviewed green against the corrected page.
- Reduced the write-policy journey to one continuous-arc test after the worker's
  duplication audit showed the guard's landed tests covered the per-fact criteria.

## Escalations

- 9757ab31 (session 1): rework limit exceeded; invoker authorized one exceptional third
  rework, which passed with zero findings.
- 0c8d38be: code-review demanded a production dispatch site the breakdown had assigned to
  f52567ed; invoker chose to wire it in 0c8d38be with an env-var-only override surface.
- Suite-clock budget under sustained host load (wave 9–10 gate runs): resolved per the
  invoker's direction; the measured suite cost itself is unchanged (~24s idle against the
  30s threshold).

## Issues Discovered During Execution

- 62518095 — Correct the divergent-store recovery procedure (wave 9; the journey proved
  three defects in the documented procedure; in-epic because epic REQ-02 names the
  documented recovery path). Completed in wave 10.
- 525bd61a — Re-validate after zero-fix `jit validate --fix` (standalone): the zero-fix
  path never re-validates, so a stale `.jit/index.json` gets a false clean bill while
  preserved records stay invisible to listing and lookup.
- ea1745ee — Fresh init excludes machine-local .jit state from version control
  (standalone): `jit init` writes no `.gitignore`, so `git add .jit/` in a fresh adopter
  repository commits lock files and `worktree.json`; guide `git add -A` snippets share the
  hazard.

## Holistic Quality Notes

- The epic's own journeys audited the docs they exercise: wave 9's test found the recovery
  page's index-conflict gap, the broken `--fix` pointer, and the unreachable clean re-run
  before any adopter could. Executable witnesses for adopter procedures caught defects
  three review tiers had passed.
- The suite-clock budget sits on a knife edge under host load: genuine suite cost is
  ~23.7–24.2s against a 30s threshold, but concurrent builds or an interactive workload
  push measured runs to 30–35s. Multiple pre-epic gate runs breached it the same way.
  Worth revisiting under `@/inv/bounded-rust-build-footprint` (tracked observation, no
  test or threshold was changed during this epic).
- Convention convergence held under pressure: the stance type, precedence function,
  detection authority, mutation-context factory, and injection mechanism each have exactly
  one home, and three attempted parallel variants (a rival config enum, a private
  worktree-detection method, a second refusal rule) were dissolved rather than shipped.
