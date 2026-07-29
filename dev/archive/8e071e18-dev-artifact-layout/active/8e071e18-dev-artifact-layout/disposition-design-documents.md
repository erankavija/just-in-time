# Disposition record — unprefixed active-area design documents

The disposition of every design document the active area held that carries no issue-shaped filename
prefix. Issue `93aaa2b2`, criteria REQ-01 through REQ-06.

The disposition lives in this record rather than inside each disposed file, because writing a file's
retirement into its own bytes is the content rewriting archival is forbidden to do
(`@/issue/8e071e18/decision/D-8`). The set is per-file rather than a blanket call, because these are
genuinely issue-owned documents and a blanket call would sweep them into a project-scoped bucket
without inspection (`@/issue/8e071e18/decision/D-15`).

**Snapshot.** The commit that adds this record, on a tree where the 24 container archival executions
are complete and committed.

## Disposition kinds

| kind | what it asserts | checked by |
|---|---|---|
| archived location | the file is at that location and absent from the active area | reading both paths on disk |
| mirrored location | the file is at each location and retained at its source path | reading source and every destination |
| retained | the file stays at its current path | reading that path |

Every archived and mirrored location below is the source path reproduced under the container's
directory beneath the archive root; the mechanism preserves the path.

## Arithmetic

| kind | paths |
|---|---|
| archived location | 8 |
| mirrored location | 1 |
| retained | 0 |
| **set** | **9** |

## The enumeration

| path | disposition | location |
|---|---|---|
| `dev/active/agent-validation-design.md` | archived location | `dev/archive/4a00b2b0-agent-validation/active/agent-validation-design.md` |
| `dev/active/ci-gate-integration-design.md` | archived location | `dev/archive/14303b30-phase5-2/active/ci-gate-integration-design.md` |
| `dev/active/documentation-lifecycle-design.md` | mirrored location | `dev/archive/71373e37-docs-lifecycle/active/documentation-lifecycle-design.md`, source retained at `dev/active/documentation-lifecycle-design.md` |
| `dev/active/documentation-lifecycle-phase2-design.md` | archived location | `dev/archive/94f873c8-docs-lifecycle-p2/active/documentation-lifecycle-phase2-design.md` |
| `dev/active/observability-design.md` | archived location | `dev/archive/d7bfd4a4-observability/active/observability-design.md` |
| `dev/active/planning-bracket-design.md` | archived location | `dev/archive/2fbd2a82-planning-bracket/active/planning-bracket-design.md` |
| `dev/active/production-polish-design.md` | archived location | `dev/archive/9d427a6b-production-polish/active/production-polish-design.md` |
| `dev/active/production-stability-design.md` | archived location | `dev/archive/5fe00921-production-stability/active/production-stability-design.md` |
| `dev/active/rejection-state-design.md` | archived location | `dev/archive/93f3e4df-rejection-state/active/rejection-state-design.md` |

The owning issue behind each row, and the run that produced each location, are the corresponding
`move` and `copy` rows of `archive-run-evidence.md`; every row above has exactly one.

## Corrections against the issue's mapping

The issue's mapping was written before the runs executed. One of its nine rows overstates the
destinations that exist, and the enumeration above records what the run actually wrote. The
correction was established from the tree and cross-read against the `copy` rows of
`archive-run-evidence.md`.

| path | the mapping states | the runs did | recorded as |
|---|---|---|---|
| `dev/active/documentation-lifecycle-design.md` | mirrored to 8 container directories | mirrored to 1 — `71373e37`; the other 7 named directories hold no copy | mirrored location, at the one that exists |

Both halves of a mirror hold at that one location: the destination is present and the source is
retained, so REQ-03 is satisfied as recorded. Recording all eight would have made it false. The kind
is unchanged — the run was a `copy` — so only the destination list needed correcting.

The retained source is why the file is still in the active area at all:
`archive-completeness-record.md` carries it in its `dev/active` table as a mirror source whose owning
reference the run repointed to the archived copy.

## Verification

Every assertion was read off the working tree rather than carried over from the issue's mapping.

| check | result |
|---|---|
| each archived location present on disk | 8/8 |
| each archived source path absent from the active area | 8/8 |
| the mirrored location present on disk | 1/1 |
| the mirrored source retained on disk | 1/1 |
| copies of an archived source found anywhere under the archive root | exactly 1 for each of the 8 |
| copies of the mirrored source found anywhere under the archive root | exactly 1 |
| paths carrying more than one disposition | none |
| paths in the set carrying none | none |
| bytes of any file in the set edited by this record | none |
| `jit validate` | valid, 0 errors, 0 warnings, 0 divergences |

## Findings

**The mapping's error was in a destination count, not in a disposition.** Every design document in
this set reached the outcome the plan decided for it; only the breadth of one mirror was overstated.
That is a narrower failure than the study area's, where two files the mapping called mirrored were
never selected by any run at all.
