# Epic Complete: Unguarded duplicate facts, unreliable verification, and tracker ergonomics (f3f7de97)

**Started:** 2026-08-03
**Completed:** 2026-08-05
**Assignee:** agent:jit-execution-lead
**Sessions:** 3

## Summary

Every fact this repository published through a package while also keeping it in
its own registry or rendered files is now bound by a check that fails when the
two disagree. Every check the development workflow trusts either answers for
itself or has been replaced: the merge-integrity guard that reported a cold
build as passing is gone, the authoritative gate's verdict no longer turns on
what else the host was doing, and a guard that refuses to run names the
condition it detected. The hosted continuous-integration workflow, which had not
passed since 2026-07-12 and failed on six consecutive pushes before this
container reached it, now passes on every job.

## Metrics

Every figure below is derived rather than remembered, and the command that
derives it is given so a reader can disagree with the number rather than with
the author. `progress.json` is this container's execution record and is linked
to it alongside this report.

| Metric | Value | Derived from |
|---|---|---|
| Children terminal | 20 (18 done, 2 rejected) | `jit issue search "" --label epic:guards-and-ergonomics --json` |
| Waves executed | 5 | `.waves \| length` in `progress.json` |
| Rework cycles | 9 across 5 issues | `.rework_counts` in `progress.json` |
| Escalations | 12 | `.escalations \| length` in `progress.json` |
| Issues created during execution | 11 | `.created_during_execution` in `progress.json`, enumerated below |
| Findings deferred and later reconciled | 19 | `.surfaced_pitfalls` in `progress.json` |

The fifth wave is not a planned one: it is the remediation `91cc038c`, raised by
the container's own gate and by the owner's question about what the stall bound
costs.

Two children were rejected rather than delivered: `25d25f2f` and `03566554`,
both Windows defects, after the owner ruled Windows out of the v1.0 matrix.
`ee02e514` then removed the Windows leg so the workflow stopped running a
platform nobody acts on.

## Success Criteria

- [x] **REQ-01** — no hand-held duplicate of a published fact — `a2d7d212`
  (packaged contribution against a registry entry), `3923fd06` (every published
  package against the registry it restates), `176d14d3` (packaged region source
  against the region rendered from it). One mechanism with a shared
  `DriftReport` shape serves all three, which was the design question the epic's
  Notes left open for breakdown.
- [x] **REQ-02** — the merge-integrity guard fails a merge that does not build
  and test, or is removed — `3019eacd`. Removed in favour of the check that does,
  on the owner's ruling; the per-merge cost of making it build and test was
  judged not worth paying.
- [x] **REQ-03** — a gate verdict does not depend on unrelated host load —
  `06f1fa95`, `57675b68`, `e3c6c767`, `4c700c80`, `15afa1f8`, `400a8539`,
  `91cc038c`. Five search shapes over the suite across three surveys, each
  finding fewer sites than the last: four, then one, then one. The criterion
  carries the owner's carve-out for a bound that fires only when nothing the
  wait watches has progressed; every bound still reachable is enumerated in
  `4c700c80`'s survey with the argument for its retention, and the comparison
  evidencing the whole-gate claim is re-run in
  `06f1fa95`'s report with the host load sampled and kept.
- [x] **REQ-04** — every guard that refuses to run names the condition it
  detected — `89a7e34e`.
- [x] **REQ-05** — a description and a multi-issue gate evaluation without shell
  quoting or one invocation per issue — `896b5b5d`, `6d496b8c`.
- [x] **REQ-06** — a script taking a destination rejects an unrecognised option —
  `5367fcba`.
- [x] **REQ-07** — no file carries another container's requirement identifier —
  `e516b4f8`.
- [x] **REQ-08** — the hosted workflow completes with every job passing and none
  at its execution ceiling — `76a4bd21`, `ee02e514`. The run, its jobs, their
  conclusions and their durations against the ceiling are recorded in
  [`hosted-ci-evidence.md`](hosted-ci-evidence.md), which is where those numbers
  live; this report cites it rather than restating it. The last run before this
  container's work reached the workflow had four jobs cancelled at 5.36 hours
  each and a failing Windows job.

## Wave Execution Log

**Wave 1** (6 issues) — the workflow's shape and the guards it depends on: the
Windows leg removed, the merge guard replaced, the staleness refusal made to
name its condition, multi-issue gate evaluation, argument parsing, and the serve
test that had been holding four CI jobs to their ceiling.

**Wave 2** (5 issues) — the carrier bindings and the first load-independence
work, including the survey that turned one criterion into four more issues.

**Wave 3** (3 issues) — the region-source binding, and the two issues held back
until the tree they sweep was finished.

**Wave 4** (4 issues) — the remaining timed tests, restated onto one shared
contention mechanism after both workers independently extracted their own.

**Wave 5** (1 issue) — `91cc038c`, raised by the container's own holistic-review
and by the owner's question about what the stall bound actually costs.

## Key Decisions

- **Gate after every merge, not after every wave.** Four wave-1 branches were
  merged before any was gated, and one issue's conformance failures then made
  `cargo-ci` red on main for all of them at once, blocking five otherwise
  finished issues. The protocol `3019eacd` rewrote names exactly this case.
- **One shared contention mechanism, not two.** Two wave-4 workers extracted the
  same test mechanism into different modules. The lead had assigned file
  ownership but not ownership of the shared extraction; `contention_probe.rs`
  was made canonical and the other deleted, without charging the rework budget.
- **Do not widen a gate's timeout when a starved run overruns it.** A `cargo-ci`
  evaluation hit its 900-second runner budget while two workers were building.
  The gate refused to answer rather than answering differently, which is the
  correct behaviour; the remedy was scheduling, not a longer budget.
- **Rebuild a worker's evidence rather than accept its conclusion.** `e516b4f8`
  reported auditing 1545 occurrences with every one sound. Plain `git log`
  ownership made a test-consolidation issue a phantom owner of every file it had
  moved. The conclusion held; the evidence was rebuilt with `git log --follow`
  and fixtures separated from attributions.

## Escalations

Twelve, of which the owner decided ten and the lead decided two on the owner's
already-stated grounds.

1. **Merge-integrity guard's ending** — removed rather than made to build and
   test.
2. **Carrier-binding mechanism shape** — one home, one shared drift-report shape.
3. **Region issue's stale Background** — amended; criteria untouched.
4. **Epic criteria for CI** — REQ-08 added to the container.
5. **Windows leg of the workflow** — dropped from the matrix, both Windows
   defects rejected.
6. **Unbounded retry in 57675b68** — decided by the lead rather than
   re-escalated: an unbounded retry is not acceptable, because REQ-08 exists
   precisely because one unbounded wait held four CI jobs to a six-hour ceiling.
7. **57675b68's REQ-01 versus REQ-08** — owner amended REQ-01 to permit a bound
   that fires only when nothing at all has been answered and fails loudly.
8. **The four load-sensitive sites 57675b68's survey did not file** — owner
   directed one remediation task inside the container rather than follow-up work.
9. **The dead `storage::lease` module** — owner ruled it deleted rather than
   clock-injected; injecting a clock would have produced clock-injected dead code
   and left a duplicate of a canonical type standing.
10. **4c700c80's REQ-01 and REQ-05 versus the same bound** — owner amended both.
11. **e3c6c767's Background accuracy** — owner approved the amendment.
12. **The container's own REQ-03 versus the same bound** — owner amended REQ-03
    with the carve-out its children already carried.

Escalations 7, 10 and 12 are one finding, raised three times by three reviewers
against three different criteria. That is recorded under Holistic Quality Notes
below, because the repetition is the lesson rather than the resolutions.

## Issues Discovered During Execution

| Issue | Why it was created |
|---|---|
| `76a4bd21` | CI scope expansion directed by the owner: the test hanging four jobs to the execution ceiling |
| `25d25f2f` | Windows initialization stack overflow — rejected, Windows out of scope |
| `03566554` | Windows package publication and manifest path escaping — rejected, same |
| `ee02e514` | Follow-on from the Windows rejection: stop running a leg nobody acts on |
| `57675b68` | A load-dependent proptest inside the authoritative gate, which REQ-03 names |
| `3923fd06` | A second published package unbound to the registry it restates, which REQ-01 names |
| `e3c6c767` | The tightest remaining scheduling margins, filed as its own child |
| `4c700c80` | The four load-sensitive sites 57675b68's survey reported but did not file |
| `15afa1f8` | A 50 ms margin in production code that no test-source search could see |
| `400a8539` | A 300 ms startup pause in production code, found by reading the test against what it calls |
| `91cc038c` | A wait that spent the full stall bound on an outcome the lock had already decided |

Six of these came from surveys rather than from review, and each survey needed a
search shape the previous one did not have. The last two needed reading
production code, because the sleep deciding the verdict was not in the test.

## The hosted workflow's history, since the summary leans on it

Read from `gh run list --workflow=ci.yml`, newest last:

| Date | Commit | Conclusion |
| --- | --- | --- |
| 2026-07-12 | `b7176a1e` | success — the last one before the drought |
| 2026-07-18 | `1faedc13` | failure |
| 2026-07-25 | `7b6f24fa` | failure |
| 2026-07-27 | `dc41ae39` | failure |
| 2026-07-31 | `0e1c230f` | failure |
| 2026-07-31 | `5dad802e` | failure |
| 2026-08-03 | `59bfddfe` | failure — this container's first push |
| 2026-08-04 | `a2de402c` | cancelled — four jobs held to 5.36 hours |
| 2026-08-04 | `b4859c87` | success — the first, once `76a4bd21` landed |
| 2026-08-05 | `33c89c44`, `b22ad9cb`, `b94bf7e2` | success |

Two things this table corrects in earlier drafts of this report. The workflow's
first pass came mid-container rather than at completion: `76a4bd21` bounded the
foreground-serve case and `ee02e514` removed the Windows leg, and `b4859c87` is
where that took effect. And the drought began on 2026-07-18, three weeks before
this container was filed, not at its filing.

## Holistic Quality Notes

- **One finding, three criteria, three reviews.** The shared thirty-second stall
  bound was failed by `code-review` against `57675b68`'s REQ-01, again against
  `4c700c80`'s REQ-01 and REQ-05, and by `holistic-review` against the
  container's REQ-03. Each reviewer was right on the text in front of it, and
  each time the resolution was the same amendment. When a criterion is amended
  to permit a mechanism, every sibling criterion naming that mechanism's subject
  needs the same amendment in the same change; amending one at a time buys three
  review cycles for one decision.
- **A document that tabulates its own exceptions must be read against them.**
  `4c700c80`'s survey claimed no assertion in the suite could be flipped by load,
  three sections below a table recording one that could. The reviewer found the
  contradiction rather than the code.
- **The owner's question found a defect three reviewers had not.** Asked whether
  any test could wait thirty seconds for nothing, the answer was yes, in a case
  none of the reviews had named: when the lock admitted a contender it had to
  refuse, the waiter spent the whole bound before reporting a stall that was
  neither the cause nor true. Two seeded tests took 30.00 s to fail; they now
  take 0.00 s and name the lock.
- **Evidence a container states about itself is not evidence.** The container's
  own gate asked for two things it had asserted rather than recorded: the load
  condition under which a comparison ran, and the hosted CI run that closed
  REQ-08. Both existed; neither was persisted. Both now are.
- **Surveys converge, but only if each one changes its method.** Four search
  shapes over three surveys found four sites, then one, then one. The count fell
  because each survey added a shape, not because the tree was cleaner. A sixth
  shape is not ruled out and the method is written down so the next reader
  extends it.
