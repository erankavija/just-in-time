# Disposition record — unprefixed active-area reference material

The disposition of every guide, requirement list, example set and scope brief the active area held
that carries no issue-shaped filename prefix. Issue `2d7ae27e`, criteria REQ-01 through REQ-06.

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
| assigned owning issue | the file is linked to that issue and stays where it is | reading the issue's document references, and the path on disk |

Every archived location below is the source path reproduced under the container's directory beneath
the archive root; the mechanism preserves the path.

## Arithmetic

| kind | paths |
|---|---|
| archived location | 3 |
| mirrored location | 0 |
| assigned owning issue | 1 |
| **set** | **4** |

## The enumeration

| path | disposition | location or owner |
|---|---|---|
| `dev/active/config-consolidation-documentation-requirements.md` | archived location | `dev/archive/5fe00921-production-stability/active/config-consolidation-documentation-requirements.md` |
| `dev/active/doc-archive-implementation-guide.md` | archived location | `dev/archive/71373e37-docs-lifecycle/active/doc-archive-implementation-guide.md` |
| `dev/active/gate-examples.md` | archived location | `dev/archive/14303b30-phase5-2/active/gate-examples.md` |
| `dev/active/v1-production-readiness-scope-brief.md` | assigned owning issue | `8b05a612`, in `backlog`; the brief stays at `dev/active/v1-production-readiness-scope-brief.md` until that container's own artifacts are archived |

The owning issue behind each archived row, and the run that produced each location, are the
corresponding `move` rows of `archive-run-evidence.md`; each of the three has exactly one.

## The assigned owning issue

`8b05a612` carries the link already; nothing was created to satisfy REQ-03. Its document reference
reads:

| path | doc type | label | commit pin |
|---|---|---|---|
| `dev/active/v1-production-readiness-scope-brief.md` | `design` | Production Readiness Scope Brief | none |

The issue is in `backlog`, which is not a terminal state, so no run selected the brief: the mechanism
plans an artifact only when every issue referencing it is terminal. That is the same reason
`archive-completeness-record.md` gives in its `dev/active` table, where the brief carries `live
owner` and cites this issue.

## Verification

Every assertion was read off the working tree rather than carried over from the issue's mapping.

| check | result |
|---|---|
| each archived location present on disk | 3/3 |
| each archived source path absent from the active area | 3/3 |
| copies of an archived source found anywhere under the archive root | exactly 1 for each of the 3 |
| the owned brief present at its path on disk | yes |
| copies of the owned brief found anywhere under the archive root | none |
| the owning issue's document reference names that path | yes, read from the issue record |
| the owning issue is live | yes, `backlog` |
| paths carrying more than one disposition | none |
| paths in the set carrying none | none |
| bytes of any file in the set edited by this record | none |
| `jit validate` | valid, 0 errors, 0 warnings, 0 divergences |

## Findings

The issue's mapping matches the tree row for row. No path in this set needed a corrected disposition,
and no criterion required a link that did not already exist.
