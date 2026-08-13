# Epic Complete: The workspace test suite runs in under 30 seconds (4b7c06d0)

**Started:** 2026-08-11
**Completed:** 2026-08-13
**Assignee:** agent:jit-execution-lead

## Summary

The default workspace suite runs in 22.1 s warm, down from a 192,994 ms gate, and
the budget is now enforced on every `cargo-ci` run rather than asserted in prose.
`dev/TESTING.md` attributes what the remaining time is spent on, test by test,
against the profile and transaction artifacts it derives from.

## Metrics

| Metric | Value |
|---|---|
| Children completed | 30 done, 2 rejected / 32 |
| Waves executed | 9 |
| Rework cycles | 14 across 10 issues |
| Escalations | 3 |
| Issues created during execution | 19 |

## Success Criteria

- [x] **REQ-01** — suite under 30 s warm. Three idle runs at `0e71077d9`: suite-clock
  22,175 / 23,628 / 22,095 ms. Delivered by `6d10e5d4` (the reduction), `355565c2`
  (removing the provenance step and the disk `TMPDIR`), and waves 1-5 (fixture
  sharing and the nextest cutover). Evidence:
  `dev/benchmarks/suite-enforcement-4b7c06d0/`.
- [x] **REQ-02** — enforced automatically. `scripts/cargo-ci.sh` passes the measured
  clock to `scripts/rust-build-budget.sh --test-suite-ms`, which fails at
  `MAX_TEST_SUITE_SECONDS`. Delivered by `5b60d40e` (the checker) and `94d85bf1`
  (the wiring). The threshold is cited, never restated.
- [x] **REQ-03** — per-step wall clock in every run's evidence. Delivered by
  `2516ad90`; visible in every gate record in this epic.
- [x] **REQ-04** — no covered property lost. Delivered across every wave: six issues
  returned reviewed `no_change` rather than buy the budget with coverage. Amended
  mid-close (see Escalations) to scope it to surface the product still has.
- [x] **REQ-05** — inherent costs attributed. `dev/TESTING.md` §5 names 64 tests with
  their measured warm cost and mechanism, plus the 7 excluded tests with theirs.
  Delivered by `21ba4bb4` (the profile) and `76ecc11f` (the attribution).

## Wave Execution Log

**Wave 1:** 3 issues — nextest foundation, the injectable duration checker, the
single-barrier journal cutover.
**Wave 2:** 5 issues — shared stale-binary fixture, journal crash coverage,
transaction model-limit measurement, nextest and durability documentation.
**Wave 3:** 2 issues — the cargo-ci runner swap and the benchmark-driven fsync
decision.
**Wave 4:** 2 issues — warm per-test timing evidence and the named suite clock.
**Wave 5:** 13 issues — the optimization campaign: fixture sharing, profile
screens, concurrency tuning. Four of the screens returned `no_change` on their
build-cost ceilings.
**Wave 6:** 2 issues — `6d10e5d4` reduced warm suite work below the bound;
`355565c2` removed build provenance, its stale-binary guard, and the disk `TMPDIR`.
**Wave 7:** 1 issue — `94d85bf1` wired enforcement in, with a reported build step
before the clock so a cold target pays compilation outside the measured span.
**Wave 8:** 1 issue — `25faa21d` tightened the per-test ceiling to 10 s x 2.
**Wave 9:** 1 issue — `76ecc11f` attributed the remaining cost.

## Key Decisions

- **The winning lever was the dependency graph, not this project's code.** Four
  issues (`123efe74`, `efaec52f`, `7a2fc6f6`, `b883f916`) screened optimizations of
  jit's own artifacts and all four failed the 25% rebuild ceiling. `6d10e5d4`'s
  `[profile.dev.package."*"] opt-level = 1` cleared it, because dependencies compile
  once and cache while the suite runs their code constantly.
- **A negative result is a deliverable.** Six issues closed on measured `no_change`
  or rejection with evidence retained. None was reworked into a forced positive.
- **Enforcement measures an already-built target.** `94d85bf1` adds a reported
  `suite-build` step before the clock: 356-377 ms warm, absorbing the whole compile
  cold. Without it the gate would fail on every fresh clone and CI runner.
- **Records of the past are not edited to satisfy a criterion.** A `355565c2` review
  finding was resolved by correcting the criterion's scope, not by rewriting session
  handoffs across four containers.

## Escalations

1. **Wave 6, `355565c2` review loop.** A review finding required live `dev/active/`
   session records to stop describing the removed guard. Rewriting them produced a
   second finding that the annotations themselves were prohibited prose. The invoker
   amended REQ-08's exemption to name the repository's records of its own past.
2. **Wave 9 dispatch, `76ecc11f` REQ-04 and epic REQ-01.** Both named the
   "ignored provenance subset", deleted by `355565c2`. The invoker approved
   rewording both to name the tests actually excluded from the default run.
3. **Epic close, REQ-04 versus REQ-01.** The epic's own review found REQ-04 unmet:
   `355565c2` deleted the stale-binary tests, and REQ-04 forbids meeting the budget
   by deleting tests. Complying literally was impossible — restoring the guard
   restores 116,971 ms of provenance step and the 5.8x `TMPDIR` tax, breaking
   REQ-01. The invoker amended REQ-04 to scope it to surface the product still has,
   conditional on each deleted assertion being named with why it no longer applies.

## Issues Discovered During Execution

19 issues were created mid-epic; `progress.json` carries the full list with the
evidence that motivated each. The ones that changed the epic's shape:

- `b1f2f001` — `main` was already red at epic start: a test pinned the live dogfood
  package's identity hash to a literal, so any change to packaged config broke it.
- `6d10e5d4` — enforcement was unreachable until warm suite work came down; created
  as an in-epic prerequisite rather than weakening the checker.
- `355565c2` — build provenance cost 61% of a warm gate and forced `TMPDIR` off
  tmpfs for the whole run, a measured 5.8x tax on every other test.
- `7094dfa5`, `527bd363` — defects the optimization campaign exposed in fixture
  cache identity and preset isolation.
- `abe2c2bd` — filed at close, wired downstream of `76ecc11f`: seven tests excluded
  from the default run assert nothing. Found by measuring them to satisfy REQ-05.

## Holistic Quality Notes

- **Measuring to satisfy a criterion is how you find out the criterion pointed at
  nothing.** REQ-05 required the excluded tests' measured cost. Measuring them
  showed all seven are placeholders whose assertions pass on their own scaffolding.
  Attributing them from the profile without measuring would have documented them as
  inherently costly — false on both halves, in the file contributors read first.
- **A criterion can become unsatisfiable while its text stays accurate**, because
  another criterion in the same set moved under it. REQ-04 was correct when written;
  work pursued for REQ-01 invalidated it three waves later. The pitfall
  reconciliation this protocol requires checks deferred findings against criteria —
  it would not have caught this, and did not.
- **The budget is enforced but the margin is host-sensitive.** The same tree measures
  22,095 ms idle, 24,528 ms with a finished build's load still decaying, and
  29,477 ms under one live concurrent build. A `test suite duration` failure is
  provisional until re-measured quiet, and `--force` is required because a recorded
  verdict survives anything done to `target/`. Documented in `dev/TESTING.md` §4.
