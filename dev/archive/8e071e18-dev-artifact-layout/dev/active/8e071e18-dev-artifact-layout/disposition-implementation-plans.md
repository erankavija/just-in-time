# Disposition record — unprefixed active-area implementation plans

The disposition of every implementation plan the active area held that carries no issue-shaped
filename prefix. Issue `079ea42e`, criteria REQ-01 through REQ-06.

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

Every archived location below is the source path reproduced under the container's directory beneath
the archive root; the mechanism preserves the path.

## Arithmetic

| kind | paths |
|---|---|
| archived location | 10 |
| mirrored location | 0 |
| retained | 0 |
| **set** | **10** |

## The enumeration

| path | disposition | location |
|---|---|---|
| `dev/active/bulk-operations-plan.md` | archived location | `dev/archive/14303b30-phase5-2/dev/active/bulk-operations-plan.md` |
| `dev/active/config-consolidation-plan.md` | archived location | `dev/archive/5fe00921-production-stability/dev/active/config-consolidation-plan.md` |
| `dev/active/dependency-display-improvements-plan.md` | archived location | `dev/archive/9d427a6b-production-polish/dev/active/dependency-display-improvements-plan.md` |
| `dev/active/gate-modification-flags-plan.md` | archived location | `dev/archive/14303b30-phase5-2/dev/active/gate-modification-flags-plan.md` |
| `dev/active/gate-presets-implementation-plan.md` | archived location | `dev/archive/9d427a6b-production-polish/dev/active/gate-presets-implementation-plan.md` |
| `dev/active/json-output-standardization-plan.md` | archived location | `dev/archive/9d427a6b-production-polish/dev/active/json-output-standardization-plan.md` binds; a second archived copy stands at `dev/archive/14303b30-phase5-2/dev/active/json-output-standardization-plan.md` |
| `dev/active/multi-issue-bulk-operations-plan.md` | archived location | `dev/archive/5fe00921-production-stability/dev/active/multi-issue-bulk-operations-plan.md` |
| `dev/active/quiet-mode-plan.md` | archived location | `dev/archive/14303b30-phase5-2/dev/active/quiet-mode-plan.md` |
| `dev/active/snapshot-export-implementation-plan.md` | archived location | `dev/archive/71373e37-docs-lifecycle/dev/active/snapshot-export-implementation-plan.md` |
| `dev/active/transitive-reduction-validation-plan.md` | archived location | `dev/archive/5fe00921-production-stability/dev/active/transitive-reduction-validation-plan.md` |

The owning issue behind each row, and the run that produced each location, are the corresponding
`move` and `copy` rows of `archive-run-evidence.md`.

## Corrections against the issue's mapping

The issue's mapping was written before the runs executed. One of its ten rows describes a disposition
the runs did not leave in place, and the enumeration above records what happened instead. The
correction was established from the tree and cross-read against the `copy` and `move` rows of
`archive-run-evidence.md`.

| path | the mapping states | the runs did | recorded as |
|---|---|---|---|
| `dev/active/json-output-standardization-plan.md` | mirrored to `14303b30` and `9d427a6b`, source retained | `14303b30` mirrored it and retained the source; `9d427a6b`, later in the run order, relocated it | archived location |

The plan is owned by two issues, `0db719b1` and `32f804f1`, which sit in different containers.
`14303b30`'s run mirrored rather than relocated because `32f804f1` lay outside its subtree and still
named the live source; the run wrote the mirror, retained the source, and repointed `0db719b1` to the
mirrored copy. When `9d427a6b` ran, the only owner still naming the live source was `32f804f1`, and
that owner lies inside `9d427a6b`'s subtree — so no reference from outside stood in the way and the
run relocated. Both archived copies exist, the live source is gone, and each owner now names the copy
under its own container: `0db719b1` the `14303b30` one, `32f804f1` the `9d427a6b` one.

Which disposition binds, and why the file carries only one: two runs acted on it, so it could be read
as matching both kinds, but a mirror's defining assertion is a retained source and this source is not
retained. The archived-location kind is the only one true of the tree. The location that binds is
`9d427a6b`'s, because that is where the source path's own bytes went. `14303b30`'s copy is a second
archived copy the earlier mirror wrote and the later relocation left standing; it is recorded because
REQ-02 asserts presence at a recorded location and both are present.

The bearing on the criteria is direct. Recorded as mirrored, REQ-03 would be false for this file — it
asserts a retained source, and there is none. Recorded as an archived location, REQ-02 holds at both
locations and REQ-03 has nothing to range over in this set.

`archive-completeness-record.md` states the same case in its REQ-02 section, where the file is the
one mirror row of nineteen whose source is not in place, and notes that a check demanding every
mirrored source survive would read the second run as breaking the first.

## Verification

Every assertion was read off the working tree rather than carried over from the issue's mapping.

| check | result |
|---|---|
| each archived location present on disk | 11/11 across the 10 paths |
| each source path absent from the active area | 10/10 |
| copies of a source path found anywhere under the archive root | exactly 1 for nine paths, 2 for `json-output-standardization-plan.md` |
| paths carrying more than one disposition | none |
| paths in the set carrying none | none |
| bytes of any file in the set edited by this record | none |
| `jit validate` | valid, 0 errors, 0 warnings, 0 divergences |

## Findings

- **The whole set landed on one disposition.** No implementation plan in the active area is mirrored
  or retained; every one of the ten is at an archived location with its source gone. The mapping's
  single mirror row was overtaken by a later run.

- **Run order decides the disposition when two containers plan the same file.** The mirror was
  correct when `14303b30` executed and stopped being correct when `9d427a6b` did. A disposition
  record written against a plan rather than against the tree cannot see that, which is why the tree
  is what this record was checked against.
