# Disposition record — session records

One blanket disposition over every record the session area held, and the path list that call covers.
Issue `5a19fffb`, criteria REQ-01 through REQ-05.

The disposition lives in this record rather than inside each disposed file, because writing a file's
retirement into its own bytes is the content rewriting archival is forbidden to do
(`@/issue/8e071e18/decision/D-8`).

**Snapshot.** The commit that adds this record, on a tree where the 24 container archival executions
are complete and committed.

## The blanket call

**A session record follows its owning issue's archival outcome.** The area was a uniform set of
dated records, every one of them named by a terminal issue's document reference, so the call is one
decision rather than thirty judgements (`@/issue/8e071e18/decision/D-15`). The enumeration below
states that call's scope.

Applied to this area the call resolves to one outcome for every path: the run of the container
dominating the record's owner relocated it. `dev/sessions/` is absent from the working tree.

## Disposition kinds

| kind | what it asserts | checked by |
|---|---|---|
| archived location | the file is at that location and absent from its source path | reading both paths on disk |
| mirrored location | the file is at each location and retained at its source path | reading source and every destination |
| retained | the file stays at its current path | reading that path |

Every archived location below preserves the part of the source path relative to the configured
development root beneath the container's directory in the archive, so the two columns share a
basename by construction.

## Arithmetic

| kind | paths |
|---|---|
| archived location | 30 |
| mirrored location | 0 |
| retained | 0 |
| **set** | **30** |

## The enumeration

| path | disposition | archived location |
|---|---|---|
| `dev/sessions/session-2024-12-24-check-links-incomplete.md` | archived location | `dev/archive/71373e37-docs-lifecycle/sessions/session-2024-12-24-check-links-incomplete.md` |
| `dev/sessions/session-2025-12-21-short-hash-progress.md` | archived location | `dev/archive/14303b30-phase5-2/sessions/session-2025-12-21-short-hash-progress.md` |
| `dev/sessions/session-2025-12-22-doc-consolidation.md` | archived location | `dev/archive/71373e37-docs-lifecycle/sessions/session-2025-12-22-doc-consolidation.md` |
| `dev/sessions/session-2025-12-26-config-consolidation.md` | archived location | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-26-config-consolidation.md` |
| `dev/sessions/session-2025-12-27-doc-archive.md` | archived location | `dev/archive/71373e37-docs-lifecycle/sessions/session-2025-12-27-doc-archive.md` |
| `dev/sessions/session-2025-12-29-quiet-flag-implementation.md` | archived location | `dev/archive/14303b30-phase5-2/sessions/session-2025-12-29-quiet-flag-implementation.md` |
| `dev/sessions/session-2025-12-30-bulk-cli-integration.md` | archived location | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-cli-integration.md` |
| `dev/sessions/session-2025-12-30-bulk-operations-progress.md` | archived location | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-operations-progress.md` |
| `dev/sessions/session-2025-12-30-bulk-phase5-docs.md` | archived location | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-phase5-docs.md` |
| `dev/sessions/session-2025-12-30-bulk-state-decision.md` | archived location | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-state-decision.md` |
| `dev/sessions/session-2025-12-30-bulk-validation.md` | archived location | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-validation.md` |
| `dev/sessions/session-2026-01-01-example-md-migration.md` | archived location | `dev/archive/cfb3ba94-docs/sessions/session-2026-01-01-example-md-migration.md` |
| `dev/sessions/session-20260103-parallel-work-design-review.md` | archived location | `dev/archive/ad601a15-parallel-work/sessions/session-20260103-parallel-work-design-review.md` |
| `dev/sessions/session-20260109-query-consolidation.md` | archived location | `dev/archive/9d427a6b-production-polish/sessions/session-20260109-query-consolidation.md` |
| `dev/sessions/session-20260111-validate-implementation.md` | archived location | `dev/archive/ad601a15-parallel-work/sessions/session-20260111-validate-implementation.md` |
| `dev/sessions/session-20260115-cli-quality-phase3.md` | archived location | `dev/archive/ad601a15-parallel-work/sessions/session-20260115-cli-quality-phase3.md` |
| `dev/sessions/session-20260115-phase3-issue7-complete.md` | archived location | `dev/archive/ad601a15-parallel-work/sessions/session-20260115-phase3-issue7-complete.md` |
| `dev/sessions/session-20260115-story-review-f023.md` | archived location | `dev/archive/ad601a15-parallel-work/sessions/session-20260115-story-review-f023.md` |
| `dev/sessions/session-20260118-f849-manual-testing.md` | archived location | `dev/archive/ad601a15-parallel-work/sessions/session-20260118-f849-manual-testing.md` |
| `dev/sessions/session-20260201-cli-enforcement-82b17394.md` | archived location | `dev/archive/ad601a15-parallel-work/sessions/session-20260201-cli-enforcement-82b17394.md` |
| `dev/sessions/session-20260201-enforcement-modes-5e1d5f02.md` | archived location | `dev/archive/ad601a15-parallel-work/sessions/session-20260201-enforcement-modes-5e1d5f02.md` |
| `dev/sessions/session-20260201-refactor-1bdc5395-analysis.md` | archived location | `dev/archive/ad601a15-parallel-work/sessions/session-20260201-refactor-1bdc5395-analysis.md` |
| `dev/sessions/session-20260201-refactor-items-5-7.md` | archived location | `dev/archive/ad601a15-parallel-work/sessions/session-20260201-refactor-items-5-7.md` |
| `dev/sessions/session-20260620-planning-skill-design.md` | archived location | `dev/archive/f2532a2d-jit-project-lead/sessions/session-20260620-planning-skill-design.md` |
| `dev/sessions/session-20260622-planning-failures-and-churn.md` | archived location | `dev/archive/f2532a2d-jit-project-lead/sessions/session-20260622-planning-failures-and-churn.md` |
| `dev/sessions/session-20260623-planning-skill-observations.md` | archived location | `dev/archive/f2532a2d-jit-project-lead/sessions/session-20260623-planning-skill-observations.md` |
| `dev/sessions/session-20260624-project-lead-role-design.md` | archived location | `dev/archive/f2532a2d-jit-project-lead/sessions/session-20260624-project-lead-role-design.md` |
| `dev/sessions/session-20260625-planning-skill-authoring.md` | archived location | `dev/archive/f2532a2d-jit-project-lead/sessions/session-20260625-planning-skill-authoring.md` |
| `dev/sessions/session-claim-coordination-parallel.md` | archived location | `dev/archive/ad601a15-parallel-work/sessions/session-claim-coordination-parallel.md` |
| `dev/sessions/session-claim-coordination-review.md` | archived location | `dev/archive/ad601a15-parallel-work/sessions/session-claim-coordination-review.md` |

The owning issue behind each row, and the run that produced each location, are the corresponding
`move` rows of `archive-run-evidence.md`; every row above has one, in exactly one run.

## Verification

Every assertion was read off the working tree rather than carried over from the issue's mapping.

| check | result |
|---|---|
| each archived location present on disk | 30/30 |
| each source path absent on disk | 30/30 |
| copies of a source path found anywhere under the archive root | exactly 1 for each of the 30 |
| paths carrying more than one disposition | none |
| paths in the set carrying none | none |
| bytes of any file in the set edited by this record | none |
| `jit validate` | valid, 0 errors, 0 warnings, 0 divergences |

The area itself is gone: `dev/sessions/` is absent from the working tree, which is the same fact the
`dev/sessions` row of the `archive-completeness-record.md` REQ-03 table reports as zero remaining
files.

## Findings

The issue's mapping matches the tree row for row. No path in this set needed a corrected
disposition.
