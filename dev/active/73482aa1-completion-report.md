# Completion report — Bound Rust build artifacts and compilation cost (73482aa1)

**Closed:** 2026-07-15. Story-level gates: cargo-ci, code-review, repo-validate,
doc-review, docs-mechanical — all passed on main (final green at 9c9faf05).

## Delivered (9 children, all done)

| Child | Delivery |
|---|---|
| 4e22a20d | Reproducible measurement harness (`scripts/benchmark-rust-build.sh`) + baseline |
| 5d862134 | Build provenance stabilized to injected inputs (stale-binary guard stays effective) |
| 8d4f7084 | 141 one-file integration-test targets consolidated into 11 cohesive suites |
| 57d0eb79 | `line-tables-only` dev/test profiles; explicit interactive incremental; gate `CARGO_INCREMENTAL=0` + `incremental-state` step |
| 3398bc19 | jsonschema remote-resolution stack and duplicate TLS backend removed (rustls-only ureq) |
| 83efbcb4 | Provenance test fixture bounded to a `git ls-files`-derived input inventory (was an unbounded working-tree copy) |
| 3f73423b | `scripts/rust-build-budget.sh` budget checker wired as cargo-ci's `budget` step |
| 26f97dc2 | Optimized benchmark published; baseline re-collected for protocol symmetry |
| 362e3fec | `@/invariant/bounded-rust-build-footprint` registered (enforced by `@/gate/cargo-ci`) + contributor policy in dev/TESTING.md |

## Measured outcome (full evidence: dev/benchmarks/rust-build-efficiency/report.md)

Clean workspace test-compilation median −87.3% (threshold ≥40%); incremental
rebuild median −94.6% (threshold ≥60%); clean validation target directory
19.6 → 3.7 GiB (budget ≤10 GiB); unique active test executables
13.8 → 0.94 GiB (budget ≤2 GiB); integration-test targets 144 → 11 (cap 12,
gate-enforced).

## Process notes

- Escalations: 2, both invoker-resolved — 57d0eb79 REQ-04 amended to codify the
  routine-gate vs benchmark-protocol split; 26f97dc2's round-3 asymmetry
  resolved by re-collecting the baseline under the corrected zero-warning
  protocol at the identical revision.
- Rework rounds: 4e22a20d ×2, 5d862134 ×1, 57d0eb79 ×1, 3398bc19 ×3
  (lead-direct), 83efbcb4 ×1 (lead-direct), 26f97dc2 ×3 (worker + lead-direct
  baseline re-collection).
- Cross-stream integration during execution: 83efbcb4 filed and nested here by
  the project-lead session mid-story; the story's landed gate machinery
  (stale-binary guard, incremental-state step, budget step) actively policed
  its own delivery process throughout.
