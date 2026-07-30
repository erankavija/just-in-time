# Disposition record — study records

One blanket disposition over every record the study area held, the measurement subtree carved out of
it, and the path list both cover. Issue `66c467c1`, criteria REQ-01 through REQ-07.

The disposition lives in this record rather than inside each disposed file, because writing a file's
retirement into its own bytes is the content rewriting archival is forbidden to do
(`@/issue/8e071e18/decision/D-8`).

**Snapshot.** The commit that adds this record, on a tree where the 24 container archival executions
are complete and committed.

## The blanket call

**A study record follows its owning issue's archival outcome, and a record no terminal owner claims
is project-scoped.** The area was close to homogeneous, so the call is one decision rather than
nineteen judgements (`@/issue/8e071e18/decision/D-15`). The enumeration below states its scope.

## The carve-out

`dev/studies/perf/` is carved out of the blanket call and retained where it is, because a committed
self-test script opens one of its measurement records by path at runtime:
`scripts/benchmark-session-cost-selftest.sh:89` reads `dev/studies/perf/session-cost-27ffbd2d.json`
as the template its output is validated against. Relocating that record would break the script.

Two of the three files in the subtree are additionally commit-pinned, so a reference to them is
retained by design wherever its owner sits: `session-cost-27ffbd2d.json` by `73981310` at
`b38c5b94`, and `session-cost-c488ef85.json` by `a4b0fadf` at `4ead5f12`. The third,
`session-cost-27ffbd2d.md`, is the prose companion of the record the script reads and is retained
with it; no run selected it.

## Disposition kinds

| kind | what it asserts | checked by |
|---|---|---|
| archived location | the file is at that location and absent from its source path | reading both paths on disk |
| mirrored location | the file is at each location and retained at its source path | reading source and every destination |
| retained | the file stays at its current path | reading that path |

Every archived and mirrored location below preserves the part of the source path relative to the
configured development root beneath the container's directory in the archive.

## Arithmetic

| kind | paths |
|---|---|
| archived location | 10 |
| mirrored location | 1 |
| retained | 8 |
| **set** | **19** |

## The enumeration

| path | disposition | location |
|---|---|---|
| `dev/studies/addressing-v2-item-use-cases.md` | archived location | `dev/archive/2821e177-addressing-v2/studies/addressing-v2-item-use-cases.md` |
| `dev/studies/addressing-v2-rule-gate-items.md` | archived location | `dev/archive/2821e177-addressing-v2/studies/addressing-v2-rule-gate-items.md` |
| `dev/studies/agent-ux-observations.md` | archived location | `dev/archive/9d427a6b-production-polish/studies/agent-ux-observations.md` |
| `dev/studies/ai-tool-worktree-compatibility.md` | archived location | `dev/archive/4a00b2b0-agent-validation/studies/ai-tool-worktree-compatibility.md` |
| `dev/studies/architecture-pitfalls.md` | retained | `dev/studies/architecture-pitfalls.md` |
| `dev/studies/cdc840ad-audit-2026-07-23.md` | archived location | `dev/archive/1cc809de-repository-state-quality/studies/cdc840ad-audit-2026-07-23.md` |
| `dev/studies/clippy-suppressions.md` | retained | `dev/studies/clippy-suppressions.md` |
| `dev/studies/documentation-lifecycle-strategy.md` | archived location | `dev/archive/71373e37-docs-lifecycle/studies/documentation-lifecycle-strategy.md` |
| `dev/studies/documentation-organization-strategy.md` | mirrored location | `dev/archive/71373e37-docs-lifecycle/studies/documentation-organization-strategy.md`, `dev/archive/cfb3ba94-docs/studies/documentation-organization-strategy.md`, source retained at `dev/studies/documentation-organization-strategy.md` |
| `dev/studies/documentation-tooling-evaluation.md` | archived location | `dev/archive/a4e3cfb0/studies/documentation-tooling-evaluation.md` |
| `dev/studies/jit-vs-agentic-trends-2026.md` | retained | `dev/studies/jit-vs-agentic-trends-2026.md` |
| `dev/studies/multi-agent-parallelism-analysis.md` | retained | `dev/studies/multi-agent-parallelism-analysis.md` |
| `dev/studies/perf/session-cost-27ffbd2d.json` | retained | `dev/studies/perf/session-cost-27ffbd2d.json` |
| `dev/studies/perf/session-cost-27ffbd2d.md` | retained | `dev/studies/perf/session-cost-27ffbd2d.md` |
| `dev/studies/perf/session-cost-c488ef85.json` | retained | `dev/studies/perf/session-cost-c488ef85.json` |
| `dev/studies/research-workflow-examples.md` | retained | `dev/studies/research-workflow-examples.md` |
| `dev/studies/session-mining-jit-improvements.md` | archived location | `dev/archive/53e3fa36-agent-ergonomics/studies/session-mining-jit-improvements.md` |
| `dev/studies/short-hash-implementation-plan.md` | archived location | `dev/archive/14303b30-phase5-2/studies/short-hash-implementation-plan.md` |
| `dev/studies/worktree-merge-analysis.md` | archived location | `dev/archive/4a00b2b0-agent-validation/studies/worktree-merge-analysis.md` |

Three of the retained rows are the carve-out; the other five carry the project-scoped
reclassification the blanket call assigns to a record no terminal owner claims. The owning issue
behind each archived and mirrored row, and the run that produced each location, are the
corresponding `move` and `copy` rows of `archive-run-evidence.md`.

## Corrections against the issue's mapping

The issue's mapping was written before the runs executed. Four of its nineteen rows do not describe
the tree the runs left, and the enumeration above records what happened instead. Each correction was
established from the tree and cross-read against the `move`/`copy` rows of
`archive-run-evidence.md`.

| path | the mapping states | the runs did | recorded as |
|---|---|---|---|
| `dev/studies/architecture-pitfalls.md` | mirrored to 7 container directories, source retained | nothing — no run planned it, and no copy of it exists under the archive root | retained |
| `dev/studies/clippy-suppressions.md` | mirrored to 7 container directories, source retained | nothing — no run planned it, and no copy of it exists under the archive root | retained |
| `dev/studies/documentation-organization-strategy.md` | mirrored to 8 container directories | mirrored to 2 — `71373e37` and `cfb3ba94`; the other 6 named directories hold no copy | mirrored location, at the 2 that exist |
| `dev/studies/cdc840ad-audit-2026-07-23.md` | mirrored to `1cc809de`, source retained | relocated to `1cc809de`: the destination exists and the source is gone | archived location |

The bearing on the criteria is direct. Recording the first three as mirrored would make REQ-03 false,
because 20 of the 22 destinations those rows name hold no copy, and two of the three files have no
destination at all. Recording the fourth as mirrored would make REQ-03 false the other way, because
its source is not retained; recorded as an archived location it satisfies REQ-02 instead, which is
the criterion its actual outcome answers.

The two never-selected records are the mechanism working as specified, not an omission: it owns an
artifact by document reference, and neither file is named by one. `archive-completeness-record.md`
carries both under `no document reference` in its `dev/studies` table.

`documentation-organization-strategy.md` appears in two containers' plans, and both actions are
mirrors, so one disposition covers both: the source is retained, which is what a mirror asserts, and
the two destinations are two locations of the same disposition rather than two dispositions.

## Verification

Every assertion was read off the working tree rather than carried over from the issue's mapping.

| check | result |
|---|---|
| each archived location present on disk | 10/10 |
| each archived source path absent on disk | 10/10 |
| each mirrored location present on disk | 2/2 |
| the mirrored source retained on disk | 1/1 |
| each retained path present on disk | 8/8 |
| copies of an archived source found anywhere under the archive root | exactly 1 for each of the 10 |
| copies of a retained path found anywhere under the archive root | none |
| the path `scripts/benchmark-session-cost-selftest.sh` reads at runtime | present, and tracked in git |
| paths carrying more than one disposition | none |
| paths in the set carrying none | none |
| bytes of any file in the set edited by this record | none |
| `jit validate` | valid, 0 errors, 0 warnings, 0 divergences |

The nine paths this record retains are the whole of what `dev/studies/` holds, which is the same
count the `dev/studies` row of the `archive-completeness-record.md` REQ-03 table reports.

## Findings

- **The mapping's mirror rows were the unreliable ones; its archived rows all held.** All ten
  archived locations and every retained path check out as stated. The four corrections are confined
  to rows the mapping called mirrored, and in three of them the mapping named destinations no run
  ever wrote.

- **A mirror claim carries two assertions, and each failed independently here.** A mirror asserts
  destinations and a retained source. Three rows failed on destinations while their source claim held;
  one row named a single destination that does exist and failed on the source, because the run that
  wrote that destination was a relocation. Checking only one half of a mirror row would have passed
  each of the four.
