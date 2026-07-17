# Epic Complete: Core maintenance (6eb585bc)

**Started:** 2026-06-16 (umbrella creation); final batch 2026-07-14
**Completed:** 2026-07-17
**Assignee:** agent:jit-execution-lead

## Summary

Umbrella epic for standalone core maintenance and bug-fix work across jit's
CLI, storage, gate, and validation subsystems. Originally a living container,
it was converted to a finite v1.0 prerequisite by @/charter/D-14: with the
profiles MVP complete, closing this epic unblocks the production-readiness
source freeze. All 59 children reached `done` (none rejected) across three
batches; the final core-maintenance batch (2026-07-14..17) drove the last 23
plus the interleaved rust-build-efficiency story.

## Metrics (final batch, 2026-07-14 → 2026-07-17)

| Metric | Value |
|---|---|
| Children completed (epic lifetime) | 59 / 59 (none rejected) |
| Final-batch issues driven | 23 (16 batch + 4 filed mid-batch + 3 adopted interleaves), plus story 73482aa1 with 9 children |
| Waves executed | 6 planned; re-planned 2026-07-16 into two parallel worktree waves per invoker parallelism directive |
| Rework cycles | 47 recorded across 17 issues (peak: d74a9ed1 at 9, f40f1b0a and 45a140ae at 6) |
| Escalations | 6 epic-level + 2 within story 73482aa1 |
| Sessions | 6 (handoffs archived as 6eb585bc-handoff*.md) |

## Success Criteria

- [x] "Every child maintenance item is resolved (done or rejected) with its own
  verifiable criteria." — all 59 children reached `done`, each through its own
  configured gate set (cargo-ci / code-review / doc-review / docs-mechanical /
  more as declared); the final four (45a140ae, 0283ce74, d74a9ed1, 450db193)
  closed 2026-07-17 with all evidence fresh at merged HEADs from
  provenance-verified binaries.

## Wave Execution Log (final batch)

**Wave 1 — trustworthy verification loop:** af4c901a (validator registry
authority), 45e1b7e8 (merged-commit gate evidence), 950256ae (mcp npm-ci
coverage), 7446af34 (stale-binary guard).
**Wave 2 — story 73482aa1 (Rust build efficiency):** 9 children; clean compile
−87.3%, rebuild −94.6%, 144→11 test targets; bounded-rust-build-footprint
invariant enforced by cargo-ci. Report: 73482aa1-completion-report.md.
**Wave 2.5 — charter interleaves:** ef0065ad (scoped bracket validation,
@/charter/D-15), 3c4d6fe8 (secret-like label example removal).
**Waves 3+ (re-planned as parallel worktree waves A/B):** CLI output contracts
(f40f1b0a, 6f881a85, 0daba57d, 8917c558), gate-surface semantics (1d59070d,
52665a07, 8d7fc762, c505031a alias removal), dead paths and validation
correctness (0283ce74, 894337e2, 16402e14), enhancements (45a140ae Archived
lifecycle semantics, 3e12ffbd batch-shape export, 450db193 generic
config-declared projections, 554ad07f working-tree cleanliness evidence,
d74a9ed1 rules.toml membership write-through), and mid-batch fixes (46657f6f
recovery-lock reentrancy, 94096aac foreground-serve deadlock).

## Key Decisions

- Re-planned one-wave-at-a-time into two parallel worktree waves on the
  invoker's 2026-07-16 parallelism directive; worktree dispatch plus leak
  checks kept main clean throughout.
- Broke three one-finding-per-round review loops structurally with
  whole-family closure: the 45a140ae literal-terminality prose family (~20
  sites, one commit), the d74a9ed1 TOML-scanner family (invoker-approved
  toml_edit rebuild, net −137 lines), and the d74a9ed1 byte-exact-promise
  family after the REQ-01 amendment.
- Gate-evidence integrity policy: recorded passes are audited against later
  issue-tied commits; every stale pass re-run at merged HEADs from a
  provenance-verified binary (commit==HEAD, dirty=false). Evaluations run
  strictly sequentially — concurrent evaluations contend on per-issue locks
  and lose results.
- Cross-epic integration of 450db193's `[projection.*]` migration with the
  profile subsystem's manifest (invoker-approved): named projection
  contributions replaced singleton-table targets; the AGENTS.md guidance
  region asset was later restructured so Charter Decisions and Project
  invariants sit under Key Design Principles (REQ-06) without breaking
  fresh-apply marker bootstrap.

## Escalations

- 46657f6f — recovery lock non-reentrant across processes broke gate checkers
  invoking mutating jit; invoker: "file + fix now"; fixed same day.
- 45a140ae — Archived semantic-model interview; invoker pinned a
  terminality-preserving overlay, coupled retirement, and exact-state revive.
- 3c4d6fe8 — secret-detection gate unpassable in this checkout; invoker
  approved scanning staged tracked content (scripts/secret-scan.sh).
- 450db193 × profile subsystem collision — invoker integrated ("I integrate
  now"); manifest adapted to named projection contributions.
- d74a9ed1 — same-root-cause TOML-scanner findings ×5; invoker approved
  rebuilding the membership sync on toml_edit.
- d74a9ed1 — REQ-01 byte-exact wording vs the approved toml_edit drop
  behavior; invoker approved amending REQ-01 (add-only syncs byte-exact,
  drop-path canonicalization semantically lossless).

## Issues Discovered During Execution

- 46657f6f — gate checkers cannot invoke mutating jit (recovery lock); fixed.
- 554ad07f — record working-tree cleanliness in gate-run evidence (split from
  45e1b7e8 REQ-04); done.
- d74a9ed1 — write default-rule membership through to rules.toml (split from
  af4c901a); done.
- 94096aac — foreground serve deadlock (parent holds bootstrap recovery
  lock); fixed.

## Holistic Quality Notes

- The adversarial reviewer surfaces a different finding subset each round;
  convergence required lead-supplied memory (cumulative resolution tables)
  and whole-family closure per round — codified across the handoff chain.
  Late rounds still yielded genuine defects (round 10: a membership sync
  could write a duplicate-name rules.toml that RuleSet::load rejects), so
  the long tail was signal, not noise.
- Generated documentation (exit-codes, storage-records, gate-presets,
  rules-and-gates, AGENTS.md regions) must be edited at the source and
  re-rendered; one hand-edit slipped through and was ported back (62c8025a).
- @/charter/D-14: with this epic closed and the profiles MVP complete, the
  production-readiness source freeze is unblocked.
