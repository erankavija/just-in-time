# Epic Complete: Give each test property one owning test on shared fixtures and contracts (c780514e)

**Started:** 2026-10-22
**Completed:** 2026-11-01
**Assignee:** agent:jit-execution-lead

## Summary

Each test property now has one owning test on shared fixtures and contracts, and the default suite neither reads this repository's tracker nor runs a nested cargo build. Against `0896fc29b`, the warm cargo-ci suite clock fell from 21.4-21.5 s to 14.6-14.7 s and tests from 4,674 to 4,601, with no test over 5 s.

## Metrics

| Metric | Value |
|---|---|
| Children completed | 170 / 170, none rejected |
| Rework cycles | Session 5: 955aaa9a 3 (the last on a new design), 8d64ffae 2, 8ac93475 2, d6635a93 2, 45e3e442 2, 7721a2a9 1, c5aafdc0 1, epic holistic review 2. Earlier sessions are in `progress.json`. |
| Escalations | 11 in earlier sessions; 15 decisions from the coordinator in session 5, listed below |
| Sub-agent dispatches | 34 logged in earlier sessions; session 5 used 10 named workers, several resumed many times |
| Issues created during execution | 11 in session 5 under this epic, 3 under 3d9a0ce3 |

## Success Criteria

- [x] REQ-01 exit-code bindings have one owner: 134ee921, 1afbea78, 69341a9d, 8d64ffae and others labelled `satisfies:c780514e/REQ-01` (18)
- [x] REQ-02 query, auto-transition and cycle properties at the domain and graph layers: 28e6703c and others labelled (14), plus the final sweep at `966a14d45`, which deleted 35 tests
- [x] REQ-03 mutation finalization has one owner: 5d7020b9, e2eb8862, eae17510
- [x] REQ-04 list-envelope `count` has one owner: 510daa14, 791344c8, 7a69fc74, 8d64ffae and others labelled (7)
- [x] REQ-05 one subprocess fixture: 62 labelled, including c5aafdc0
- [x] REQ-06 one definition of each test constructor: 38 labelled
- [x] REQ-07 shipped kinds derived: 0bf97911, 20ab27e8, c5aafdc0 and others labelled (6)
- [x] REQ-08 one `IssueStore` conformance suite: 2c9badcd, 6f9f5d1e, 94bbdbd6, eae17510
- [x] REQ-09 no process-wide state; passes alone, in any order, in parallel: 2a6bc44c, 2b49f95b, 55043636, c5aafdc0, 87d5fd34, 7984d807
- [x] REQ-10 missing tool fails, no nested cargo, self-tests as gates: 0951556f, 2f63adec, 340a846d, c882f1f8, 952ed1e7, 14c08f51 and others labelled (14)
- [x] REQ-11 fixture key from inputs only, no hidden subcommand, no per-test runner lists: 07f4b1d4, 2a6bc44c, 2f63adec, a3dd995a, d423752b, 955aaa9a
- [x] REQ-12 footprint budget passes, measured before and after: e2764021, 45e3e442
- [x] REQ-13 suite clock measured before and after, no test over 5 s: 7721a2a9, 45e3e442 and others labelled (8)

Measurements, with commit, load and command for each figure, and the raw output: `dev/benchmarks/test-topology-c780514e/README.md`.

## Key Decisions

- Measure the after figures on the final tree from a clean checkout, because the closing sweep moved the test count.
- Re-measure the before figures at `0896fc29b` with only c882f1f8's fixture fix applied, so `cargo-ci` runs whole on today's host.
- Serialize worker builds in the two slots to stay above the 60 GiB disk guard, and reseed slots with isolated build state before measuring.
- Delete re-proofs in a repository-wide sweep once review had found them one site at a time.

## Decisions from the coordinator (session 5)

- 952ed1e7 REQ-01 narrowed to `.jit/` records, with package drift moved to a gate (14c08f51).
- 14c08f51 uses a dev-only example, since no existing command can compare the copies; `jit profile diff` reports what a publication does, and is not a drift check.
- e7d98542: the package is the authority, one copy per kind. `contribution_drift.rs`, `drift_report.rs` and `DECLARED_OVERRIDES` are deleted, net -1,884 lines. `gate-semantics`, `bounded-rust-build-footprint` and `single-source-prose` left the package; the first and last had false `enforced-by` bindings. Accepted loss: gate, rule and namespace fields go unchecked.
- 955aaa9a past its rework limit after four races in one mechanism: the failure handoff is deleted, net -206 lines. Recorded limit: a fixture build longer than the 20 s test limit shows timeouts and no cause.
- 86f78db2: worktree seeds keep hardlinks but unshare dot-named entries, anything under `build`, and `*.d`.
- 7984d807 takes the product fix: git guards act on the repository they serve. Five of 13 spawns in `commands/` read the cwd.
- c5aafdc0 REQ-04 replaced by epic REQ-09, since the scoped guards it required were deliberately removed.
- 87d5fd34 forbids a spawn retry. d2b7d19b closes as gone, on its evidence.
- 4269f866: CI runs the suite through the gate's definition.
- Host: the shared sccache daemon had a deleted `TMPDIR` from a gf2 run; no session replaced it, and it later exited. `CARGO_CI_NO_SCCACHE=1` was used for landings. Disk cleanup was limited to the ruled paths.

Earlier escalations are in `progress.json`.

## Issues Discovered During Execution (session 5)

- 14c08f51: package drift runs as a gate; needed by 952ed1e7.
- 28a7bde1: digest self-check went stale after merges; its cause became 86f78db2.
- c882f1f8: the selftest fixture looped through a cargo shim.
- d6635a93: unbounded silent build-lock waits in scripts.
- 8ac93475: an unrunnable compiler wrapper was reported as repository content.
- 8c6adea6: a benchmark timeout lost saved samples.
- e7d98542: two copies of the package's declarations.
- 86f78db2: seeded worktrees shared Cargo build state.
- 7984d807: git guards read the cwd, which refused clean repositories and allowed a bypass.
- 87d5fd34: `Text file busy` and a shutdown race when tests share a process.
- 4269f866: CI used a different runner from the gate.
- Under 3d9a0ce3: 1b0c2b75 and ab797eed (profile preview gaps), ddca5431 (`jit gate evaluate` does not advance a gated issue).

## Holistic Quality Notes

- **Cold-fixture evidence:** early cargo-ci first runs failed on the cold fixture and were force-rerun warm, with the failed runs preserved. 955aaa9a removed the cause, and the README holds its before and after evidence.
- **Stalls, both mine to prevent:** a silent disk-guard retry held landings for about 2.4 h, and a worker that ended its turn waiting on a background job stalled the epic for about 2.5 h while I waited with no bound. Land scripts now refuse at once, and every wait prints what it waits for and has a bound.
- **Actions on the host:** I killed a hung selftest from this session (twice, across two runs) and ran `cargo clean` in my own idle slot. w5-fixture killed its own load generators. Under the coordinator's ruling I deleted `target/debug/incremental` and `target/jit-stale-child-test-cache` in the main target.
- **Reports I got wrong and corrected:** I reported d2b7d19b closed while it was `gated`, and gave the coordinator an uncounted "26 children". A worker's commit deleted a test function, and I had taken its summary without reading the diff; review caught it, and a test that cannot run now fails the selftest.
- **Known limits:** shuffling covers order within a target, using libtest's unstable flag, not across targets. CI inherits nextest's 20 s per-test limit, which has not been measured on a GitHub runner, and the GitHub workflow itself has not run here. No mechanical check enforces that tests write executables through the shared helper. The MSRV job still runs `cargo test --workspace`.
- **Surfaced pitfalls:** each entry in `progress.json` is resolved, owned by a named child, or an operational note; none is a deferred violation of a criterion.
