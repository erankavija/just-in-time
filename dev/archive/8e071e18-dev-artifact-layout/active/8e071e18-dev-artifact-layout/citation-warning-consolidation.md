# Consolidated in-content citation warnings

Every in-content citation warning the 24 archival runs reported, in one list. Issue `aa38b236`,
REQ-06; the completeness record for REQ-01 through REQ-05 is at `archive-completeness-record.md`.

A warning names a **citing site** — file, line, column — and the **relocated artifact** whose path
it spells out. Archival relocates without rewriting content, so every occurrence below still reads
the pre-run path.

## Shape of the set

| measure | value |
|---|---|
| occurrences | 641 |
| distinct citing files | 105 |
| distinct (citing file, artifact) pairs | 370 |
| distinct relocated artifacts cited | 176 |
| runs contributing at least one | 24 |

The cited artifacts are exactly the relocated set: every relocated artifact is cited at least once,
and no citation names a mirrored artifact, because a mirror leaves its source in place and the
citation keeps resolving. No occurrence is reported twice across runs, and a whole-tree grep for the
relocated paths finds no citing file the runs missed.

A citing file is in one of three positions, which decides whether repointing it edits live prose or
rewrites a historical record:

| position | citing files | occurrences |
|---|---|---|
| active | 22 | 428 |
| archived (relocated by a run) | 42 | 104 |
| archived (in place) | 41 | 109 |

`archived (relocated by a run)` files were live when the warning was reported; a run has since moved
them under the archive root, and the tables below name their current path.

Two `active` citing files are records of the archival work itself — `archive-run-evidence.md` with
12 occurrences and `dev/active/ca832358/req05-archival-execution-evidence.md` with 1 — where the
pre-run path is the fact being recorded. Repointing those would falsify what the runs did, as would
repointing the paths quoted in this list.

## Citing files that must change

One row per citing file, ordered by position, then by occurrence count.

The largest single citing file is this epic's breakdown manifest,
`dev/active/8e071e18-breakdown.json`, whose 337 occurrences name every relocated artifact: it
records the manifest as approved, so whether it is repointed is a call the consuming issues make
rather than a mechanical edit.

| citing file (current path) | position | occurrences | artifacts cited | path when reported |
|---|---|---|---|---|
| `dev/active/8e071e18-breakdown.json` | active | 337 | 176 | — |
| `dev/active/8e071e18-investigation.md` | active | 41 | 18 | — |
| `dev/active/c639cfb5-investigation.md` | active | 16 | 5 | — |
| `dev/active/8e071e18-dev-artifact-layout/archive-run-evidence.md` | active | 12 | 5 | — |
| `dev/active/f2532a2d-handoff-2.md` | active | 3 | 2 | — |
| `dev/active/8e071e18-progress.json` | active | 2 | 2 | — |
| `dev/active/f2532a2d-handoff.md` | active | 2 | 2 | — |
| `crates/jit/src/storage/claim_coordinator.rs` | active | 1 | 1 | — |
| `crates/jit/tests/cli_repo_workflow/config_get_tests.rs` | active | 1 | 1 | — |
| `crates/jit/tests/fast_docs_templates/bracket_breakdown_tests.rs` | active | 1 | 1 | — |
| `crates/jit/tests/fast_docs_templates/research_bracket_tests.rs` | active | 1 | 1 | — |
| `crates/jit/tests/fast_docs_templates/sdd_bracket_tests.rs` | active | 1 | 1 | — |
| `dev/active/2fbd2a82-14ba-4e6e-90f6-e0c34f0f912c-plan.md` | active | 1 | 1 | — |
| `dev/active/73482aa1-progress.json` | active | 1 | 1 | — |
| `dev/active/9b7b5f9c-handoff-3.md` | active | 1 | 1 | — |
| `dev/active/9b7b5f9c-handoff-4.md` | active | 1 | 1 | — |
| `dev/active/9b7b5f9c-handoff-5.md` | active | 1 | 1 | — |
| `dev/active/9b7b5f9c-handoff.md` | active | 1 | 1 | — |
| `dev/active/abfd6016-progress.json` | active | 1 | 1 | — |
| `dev/active/ca832358/req05-archival-execution-evidence.md` | active | 1 | 1 | — |
| `docs/how-to/multi-agent-coordination.md` | active | 1 | 1 | — |
| `docs/tutorials/parallel-work-worktrees.md` | active | 1 | 1 | — |
| `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-plan.md` | archived (relocated by a run) | 13 | 2 | `dev/active/8b05a612-plan.md` |
| `dev/archive/2821e177-addressing-v2/active/637764ef-acceptance-evidence.md` | archived (relocated by a run) | 10 | 9 | `dev/active/637764ef-acceptance-evidence.md` |
| `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-investigation.md` | archived (relocated by a run) | 8 | 5 | `dev/active/9b7b5f9c-investigation.md` |
| `dev/archive/ad601a15-parallel-work/sessions/session-claim-coordination-parallel.md` | archived (relocated by a run) | 5 | 2 | `dev/sessions/session-claim-coordination-parallel.md` |
| `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | archived (relocated by a run) | 4 | 3 | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` |
| `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-investigation.md` | archived (relocated by a run) | 4 | 4 | `dev/active/8b05a612-investigation.md` |
| `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-cli-integration.md` | archived (relocated by a run) | 4 | 2 | `dev/sessions/session-2025-12-30-bulk-cli-integration.md` |
| `dev/archive/2d109173-docs-exhaustive-audit/active/6d82de03-followup-manifest.md` | archived (relocated by a run) | 3 | 2 | `dev/active/6d82de03-followup-manifest.md` |
| `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-research.md` | archived (relocated by a run) | 3 | 1 | `dev/active/9b7b5f9c-research.md` |
| `dev/archive/f2532a2d-jit-project-lead/active/c23dfe71-breakdown-spec.md` | archived (relocated by a run) | 3 | 1 | `dev/active/c23dfe71-breakdown-spec.md` |
| `dev/archive/f2532a2d-jit-project-lead/active/f2532a2d-138d-46bf-9547-e1ea517f0170-plan.md` | archived (relocated by a run) | 3 | 1 | `dev/active/f2532a2d-138d-46bf-9547-e1ea517f0170-plan.md` |
| `dev/archive/2821e177-addressing-v2/active/0efbc594-breakdown-spec.md` | archived (relocated by a run) | 2 | 2 | `dev/active/0efbc594-breakdown-spec.md` |
| `dev/archive/2821e177-addressing-v2/active/37506c12-breakdown-spec.md` | archived (relocated by a run) | 2 | 2 | `dev/active/37506c12-breakdown-spec.md` |
| `dev/archive/2821e177-addressing-v2/active/71ebd1e8-breakdown-spec.md` | archived (relocated by a run) | 2 | 2 | `dev/active/71ebd1e8-breakdown-spec.md` |
| `dev/archive/6eb585bc-core-maintenance/active/76cb968b-ssot-adoption-sweep.md` | archived (relocated by a run) | 2 | 2 | `dev/active/76cb968b-ssot-adoption-sweep.md` |
| `dev/archive/2821e177-addressing-v2/active/7f22d6cf-breakdown-spec.md` | archived (relocated by a run) | 2 | 2 | `dev/active/7f22d6cf-breakdown-spec.md` |
| `dev/archive/2821e177-addressing-v2/active/9a7106ae-breakdown-spec.md` | archived (relocated by a run) | 2 | 2 | `dev/active/9a7106ae-breakdown-spec.md` |
| `dev/archive/2821e177-addressing-v2/active/bb7d57a2-breakdown-spec.md` | archived (relocated by a run) | 2 | 2 | `dev/active/bb7d57a2-breakdown-spec.md` |
| `dev/archive/f2532a2d-jit-project-lead/active/eed6750c-handoff.md` | archived (relocated by a run) | 2 | 1 | `dev/active/eed6750c-handoff.md` |
| `dev/archive/9d427a6b-production-polish/design/phase3-advanced-features.md` | archived (relocated by a run) | 2 | 2 | `dev/design/phase3-advanced-features.md` |
| `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/talk.html` | archived (relocated by a run) | 2 | 1 | `dev/presentations/1cc809de/talk.html` |
| `dev/archive/1cc809de-repository-state-quality/presentations/cdc840ad/README.md` | archived (relocated by a run) | 2 | 2 | `dev/presentations/cdc840ad/README.md` |
| `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-operations-progress.md` | archived (relocated by a run) | 2 | 2 | `dev/sessions/session-2025-12-30-bulk-operations-progress.md` |
| `dev/archive/ad601a15-parallel-work/sessions/session-20260103-parallel-work-design-review.md` | archived (relocated by a run) | 2 | 1 | `dev/sessions/session-20260103-parallel-work-design-review.md` |
| `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md` | archived (relocated by a run) | 1 | 1 | `dev/active/2821e177-investigation.md` |
| `dev/archive/2e926e39-agent-seamlessness/active/2e926e39-906e-4952-8ae0-38215a7e5aac-plan.md` | archived (relocated by a run) | 1 | 1 | `dev/active/2e926e39-906e-4952-8ae0-38215a7e5aac-plan.md` |
| `dev/archive/f2532a2d-jit-project-lead/active/304f6d94-breakdown-spec.md` | archived (relocated by a run) | 1 | 1 | `dev/active/304f6d94-breakdown-spec.md` |
| `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-review-round-4.md` | archived (relocated by a run) | 1 | 1 | `dev/active/36d5451e-review-round-4.md` |
| `dev/archive/f2532a2d-jit-project-lead/active/3c192f5e-breakdown-spec.md` | archived (relocated by a run) | 1 | 1 | `dev/active/3c192f5e-breakdown-spec.md` |
| `dev/archive/53e3fa36-agent-ergonomics/active/53e3fa36-c0cb-4206-8e8e-0a21aafb213e-plan.md` | archived (relocated by a run) | 1 | 1 | `dev/active/53e3fa36-c0cb-4206-8e8e-0a21aafb213e-plan.md` |
| `dev/archive/6eb585bc-core-maintenance/active/71be6ae9-code-review-reliability.md` | archived (relocated by a run) | 1 | 1 | `dev/active/71be6ae9-code-review-reliability.md` |
| `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-research.md` | archived (relocated by a run) | 1 | 1 | `dev/active/8b05a612-research.md` |
| `dev/archive/2d109173-docs-exhaustive-audit/active/99f4a2b4-req02-evidence.md` | archived (relocated by a run) | 1 | 1 | `dev/active/99f4a2b4-req02-evidence.md` |
| `dev/archive/f2532a2d-jit-project-lead/active/e8b1cee3-breakdown-spec.md` | archived (relocated by a run) | 1 | 1 | `dev/active/e8b1cee3-breakdown-spec.md` |
| `dev/archive/9d427a6b-production-polish/design/phase2-collapse-expand.md` | archived (relocated by a run) | 1 | 1 | `dev/design/phase2-collapse-expand.md` |
| `dev/archive/ad601a15-parallel-work/experiments/worktree-manual-coordination-experiment.md` | archived (relocated by a run) | 1 | 1 | `dev/experiments/worktree-manual-coordination-experiment.md` |
| `dev/archive/5fe00921-production-stability/plans/5dbc3548-deletion-tracking.md` | archived (relocated by a run) | 1 | 1 | `dev/plans/5dbc3548-deletion-tracking.md` |
| `dev/archive/5fe00921-production-stability/sessions/session-2025-12-26-config-consolidation.md` | archived (relocated by a run) | 1 | 1 | `dev/sessions/session-2025-12-26-config-consolidation.md` |
| `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-phase5-docs.md` | archived (relocated by a run) | 1 | 1 | `dev/sessions/session-2025-12-30-bulk-phase5-docs.md` |
| `dev/archive/cfb3ba94-docs/sessions/session-2026-01-01-example-md-migration.md` | archived (relocated by a run) | 1 | 1 | `dev/sessions/session-2026-01-01-example-md-migration.md` |
| `dev/archive/ad601a15-parallel-work/sessions/session-20260118-f849-manual-testing.md` | archived (relocated by a run) | 1 | 1 | `dev/sessions/session-20260118-f849-manual-testing.md` |
| `dev/archive/2821e177-addressing-v2/studies/addressing-v2-rule-gate-items.md` | archived (relocated by a run) | 1 | 1 | `dev/studies/addressing-v2-rule-gate-items.md` |
| `dev/archive/1cc809de-breakdown.json` | archived (in place) | 16 | 1 | — |
| `dev/archive/features/2821e177/2821e177-handoff.md` | archived (in place) | 9 | 9 | — |
| `dev/archive/1cc809de-plan.md` | archived (in place) | 8 | 1 | — |
| `dev/archive/cdc840ad-investigation.md` | archived (in place) | 6 | 5 | — |
| `dev/archive/7d3a3a47/active/7d3a3a47-investigation.md` | archived (in place) | 5 | 2 | — |
| `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md` | archived (in place) | 4 | 2 | — |
| `dev/archive/4a00b2b0-agent-validation/studies/ai-tool-worktree-compatibility.md` | archived (in place) | 4 | 2 | — |
| `dev/archive/features/2821e177/completion-report.md` | archived (in place) | 4 | 3 | — |
| `dev/archive/2d109173-handoff.md` | archived (in place) | 3 | 2 | — |
| `dev/archive/2d109173-investigation.md` | archived (in place) | 3 | 2 | — |
| `dev/archive/6eb585bc-handoff-4.md` | archived (in place) | 3 | 3 | — |
| `dev/archive/cdc840ad-research.md` | archived (in place) | 3 | 1 | — |
| `dev/archive/features/2821e177/2821e177-handoff-2.md` | archived (in place) | 3 | 3 | — |
| `dev/archive/1cc809de-completion-report.md` | archived (in place) | 2 | 1 | — |
| `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | archived (in place) | 2 | 2 | — |
| `dev/archive/2821e177-addressing-v2/active/7f22d6cf-breakdown-spec.md` | archived (in place) | 2 | 2 | — |
| `dev/archive/2d109173-handoff-3.md` | archived (in place) | 2 | 2 | — |
| `dev/archive/6eb585bc-handoff-2.md` | archived (in place) | 2 | 1 | — |
| `dev/archive/6eb585bc-progress.json` | archived (in place) | 2 | 2 | — |
| `dev/archive/76cb968b-completion-report.md` | archived (in place) | 2 | 2 | — |
| `dev/archive/9ac9fdac-handoff.md` | archived (in place) | 2 | 2 | — |
| `dev/archive/cdc840ad-handoff-12.md` | archived (in place) | 2 | 2 | — |
| `dev/archive/features/dbe1e821/dbe1e821-progress.json` | archived (in place) | 2 | 2 | — |
| `dev/archive/1cc809de-handoff-2.md` | archived (in place) | 1 | 1 | — |
| `dev/archive/1cc809de-handoff-3.md` | archived (in place) | 1 | 1 | — |
| `dev/archive/1cc809de-handoff-4.md` | archived (in place) | 1 | 1 | — |
| `dev/archive/1cc809de-handoff.md` | archived (in place) | 1 | 1 | — |
| `dev/archive/2821e177-addressing-v2/studies/addressing-v2-rule-gate-items.md` | archived (in place) | 1 | 1 | — |
| `dev/archive/2d109173-docs-exhaustive-audit/active/4c33d0e5-audit-notes.md` | archived (in place) | 1 | 1 | — |
| `dev/archive/2d109173-docs-exhaustive-audit/active/a70bac75-audit-notes.md` | archived (in place) | 1 | 1 | — |
| `dev/archive/2d109173-handoff-2.md` | archived (in place) | 1 | 1 | — |
| `dev/archive/2d109173-handoff-4.md` | archived (in place) | 1 | 1 | — |
| `dev/archive/6eb585bc-handoff-3.md` | archived (in place) | 1 | 1 | — |
| `dev/archive/76cb968b-progress.json` | archived (in place) | 1 | 1 | — |
| `dev/archive/b4e55aa2-completion-report.md` | archived (in place) | 1 | 1 | — |
| `dev/archive/b4e55aa2-handoff.md` | archived (in place) | 1 | 1 | — |
| `dev/archive/f2532a2d-progress.json` | archived (in place) | 1 | 1 | — |
| `dev/archive/features/2821e177/2821e177-progress.json` | archived (in place) | 1 | 1 | — |
| `dev/archive/features/2821e177/showcase/talk.html` | archived (in place) | 1 | 1 | — |
| `dev/archive/features/2fbd2a82-progress.json` | archived (in place) | 1 | 1 | — |
| `dev/archive/features/53e3fa36-progress.json` | archived (in place) | 1 | 1 | — |

### Categories with no occurrence

Nothing under `scripts/`, no occurrence in `CHANGELOG.md`, none in `dev/TESTING.md`, and none in any
benchmark generator (`scripts/benchmark-rust-build.sh`,
`dev/benchmarks/rust-build-efficiency/verify-consolidation-inventory.py`) or anywhere else under
`dev/benchmarks/`. All four roots are inside `[documentation].citation_scan_roots`, so the runs did
read them; grepping them for every relocated path confirms the absence rather than trusting the
scan's coverage. The only citing files outside `dev/` are five Rust sources and tests under
`crates/` and two adopter documents under `docs/`, each citing
`dev/design/worktree-parallel-work.md` or `dev/active/planning-bracket-design.md`.

## Occurrences

One row per citing file and cited artifact, carrying every coordinate the runs reported. Coordinates
are `line:column`, one-based, and each still lands on the cited path exactly.

### `dev/active/8e071e18-breakdown.json`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 1607:1021, 1607:1116, 2495:14 | `dev/design/subgraph-clustering-layout.md` | `dev/archive/9d427a6b-production-polish/design/subgraph-clustering-layout.md` | `9d427a6b` |
| 1607:1067, 1644:14, 2493:14 | `dev/design/phase3-advanced-features.md` | `dev/archive/9d427a6b-production-polish/design/phase3-advanced-features.md` | `9d427a6b` |
| 1607:974, 1607:1160, 1643:14, 2492:14 | `dev/design/phase2-collapse-expand.md` | `dev/archive/9d427a6b-production-polish/design/phase2-collapse-expand.md` | `9d427a6b` |
| 1653:1080, 1689:14, 2726:14, 3777:2101, 3777:2207 | `dev/studies/ai-tool-worktree-compatibility.md` | `dev/archive/4a00b2b0-agent-validation/studies/ai-tool-worktree-compatibility.md` | `4a00b2b0` |
| 1653:1149, 1653:1329, 3002:14, 3274:1259, 3274:1366 | `dev/design/worktree-parallel-work.md` | `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` | `ad601a15` |
| 1653:1196, 1653:1260, 1690:14, 3003:14 | `dev/experiments/worktree-manual-coordination-experiment.md` | `dev/archive/ad601a15-parallel-work/experiments/worktree-manual-coordination-experiment.md` | `ad601a15` |
| 1744:14 | `dev/active/2b9a80fb-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/2b9a80fb-audit-notes.md` | `2d109173` |
| 1745:14 | `dev/active/36d5451e-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-audit-notes.md` | `2d109173` |
| 1746:14 | `dev/active/36d5451e-review-round-4.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-review-round-4.md` | `2d109173` |
| 1747:14 | `dev/active/36d5451e-review-round-5.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-review-round-5.md` | `2d109173` |
| 1748:14 | `dev/active/36d5451e-review-round-6.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-review-round-6.md` | `2d109173` |
| 1749:14 | `dev/active/4c33d0e5-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/4c33d0e5-audit-notes.md` | `2d109173` |
| 1750:14 | `dev/active/6d82de03-followup-manifest.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/6d82de03-followup-manifest.md` | `2d109173` |
| 1751:14 | `dev/active/6d82de03-lead-review.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/6d82de03-lead-review.md` | `2d109173` |
| 1752:14 | `dev/active/736a069e-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/736a069e-audit-notes.md` | `2d109173` |
| 1753:14 | `dev/active/7c283e95-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/7c283e95-audit-notes.md` | `2d109173` |
| 1754:14 | `dev/active/8682e95a-gapfill-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/8682e95a-gapfill-notes.md` | `2d109173` |
| 1755:14 | `dev/active/99f4a2b4-req02-evidence.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/99f4a2b4-req02-evidence.md` | `2d109173` |
| 1756:14 | `dev/active/a70bac75-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/a70bac75-audit-notes.md` | `2d109173` |
| 1757:14 | `dev/active/adc4c6ef-relocation-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/adc4c6ef-relocation-notes.md` | `2d109173` |
| 1758:14 | `dev/active/b8924a6d-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/b8924a6d-audit-notes.md` | `2d109173` |
| 1759:14 | `dev/active/d24008f0-req01-evidence.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/d24008f0-req01-evidence.md` | `2d109173` |
| 1813:14 | `dev/active/3e12ffbd-batch-export-design.md` | `dev/archive/6eb585bc-core-maintenance/active/3e12ffbd-batch-export-design.md` | `6eb585bc` |
| 1814:14 | `dev/active/3e12ffbd-batch-export-use-cases.md` | `dev/archive/6eb585bc-core-maintenance/active/3e12ffbd-batch-export-use-cases.md` | `6eb585bc` |
| 1815:14 | `dev/active/450db193-generic-projection-design.md` | `dev/archive/6eb585bc-core-maintenance/active/450db193-generic-projection-design.md` | `6eb585bc` |
| 1816:14 | `dev/active/45a140ae-archived-semantics.md` | `dev/archive/6eb585bc-core-maintenance/active/45a140ae-archived-semantics.md` | `6eb585bc` |
| 1817:14 | `dev/active/71be6ae9-code-review-reliability.md` | `dev/archive/6eb585bc-core-maintenance/active/71be6ae9-code-review-reliability.md` | `6eb585bc` |
| 1818:14 | `dev/active/71be6ae9-live-review-report.md` | `dev/archive/6eb585bc-core-maintenance/active/71be6ae9-live-review-report.md` | `6eb585bc` |
| 1819:14, 3459:1338 | `dev/active/73482aa1-completion-report.md` | `dev/archive/6eb585bc-core-maintenance/active/73482aa1-completion-report.md` | `6eb585bc` |
| 1820:14 | `dev/active/76cb968b-citation-check.md` | `dev/archive/6eb585bc-core-maintenance/active/76cb968b-citation-check.md` | `6eb585bc` |
| 1821:14 | `dev/active/76cb968b-ssot-adoption-sweep.md` | `dev/archive/6eb585bc-core-maintenance/active/76cb968b-ssot-adoption-sweep.md` | `6eb585bc` |
| 1822:14 | `dev/active/76cb968b-sweep-table.md` | `dev/archive/6eb585bc-core-maintenance/active/76cb968b-sweep-table.md` | `6eb585bc` |
| 1823:14 | `dev/active/949cd9d0-gate-verb-semantics.md` | `dev/archive/6eb585bc-core-maintenance/active/949cd9d0-gate-verb-semantics.md` | `6eb585bc` |
| 1824:14 | `dev/active/af4c901a-derive-default-rules-at-load.md` | `dev/archive/6eb585bc-core-maintenance/active/af4c901a-derive-default-rules-at-load.md` | `6eb585bc` |
| 1825:14 | `dev/active/b4e55aa2-code-review-live-verification.md` | `dev/archive/6eb585bc-core-maintenance/active/b4e55aa2-code-review-live-verification.md` | `6eb585bc` |
| 1826:14 | `dev/active/b4e55aa2-ground-code-review-policy.md` | `dev/archive/6eb585bc-core-maintenance/active/b4e55aa2-ground-code-review-policy.md` | `6eb585bc` |
| 1827:14 | `dev/active/d74a9ed1-write-through-namespace-unique-membership.md` | `dev/archive/6eb585bc-core-maintenance/active/d74a9ed1-write-through-namespace-unique-membership.md` | `6eb585bc` |
| 1828:14 | `dev/benchmarks/rust-build-efficiency/consolidation-inventory-diff.json` | `dev/archive/6eb585bc-core-maintenance/benchmarks/rust-build-efficiency/consolidation-inventory-diff.json` | `6eb585bc` |
| 1829:14 | `dev/benchmarks/rust-build-efficiency/verify-consolidation-inventory.py` | `dev/archive/6eb585bc-core-maintenance/benchmarks/rust-build-efficiency/verify-consolidation-inventory.py` | `6eb585bc` |
| 1883:14, 3734:3173, 3734:3277 | `dev/sessions/session-2026-01-01-example-md-migration.md` | `dev/archive/cfb3ba94-docs/sessions/session-2026-01-01-example-md-migration.md` | `cfb3ba94` |
| 1937:14 | `dev/active/304f6d94-breakdown-spec.md` | `dev/archive/f2532a2d-jit-project-lead/active/304f6d94-breakdown-spec.md` | `f2532a2d` |
| 1938:14 | `dev/active/3c192f5e-breakdown-spec.md` | `dev/archive/f2532a2d-jit-project-lead/active/3c192f5e-breakdown-spec.md` | `f2532a2d` |
| 1939:14 | `dev/active/c23dfe71-breakdown-spec.md` | `dev/archive/f2532a2d-jit-project-lead/active/c23dfe71-breakdown-spec.md` | `f2532a2d` |
| 1940:14 | `dev/active/e8b1cee3-breakdown-spec.md` | `dev/archive/f2532a2d-jit-project-lead/active/e8b1cee3-breakdown-spec.md` | `f2532a2d` |
| 1941:14, 3459:2070, 3459:2208 | `dev/active/eed6750c-handoff.md` | `dev/archive/f2532a2d-jit-project-lead/active/eed6750c-handoff.md` | `f2532a2d` |
| 1942:14, 3459:2104, 3459:2242 | `dev/active/f2532a2d-138d-46bf-9547-e1ea517f0170-plan.md` | `dev/archive/f2532a2d-jit-project-lead/active/f2532a2d-138d-46bf-9547-e1ea517f0170-plan.md` | `f2532a2d` |
| 1943:14, 3734:5250, 3734:5365 | `dev/sessions/session-20260620-planning-skill-design.md` | `dev/archive/f2532a2d-jit-project-lead/sessions/session-20260620-planning-skill-design.md` | `f2532a2d` |
| 1944:14, 3734:5425, 3734:5546 | `dev/sessions/session-20260622-planning-failures-and-churn.md` | `dev/archive/f2532a2d-jit-project-lead/sessions/session-20260622-planning-failures-and-churn.md` | `f2532a2d` |
| 1945:14, 3734:5612, 3734:5733 | `dev/sessions/session-20260623-planning-skill-observations.md` | `dev/archive/f2532a2d-jit-project-lead/sessions/session-20260623-planning-skill-observations.md` | `f2532a2d` |
| 1946:14, 3734:5799, 3734:5917 | `dev/sessions/session-20260624-project-lead-role-design.md` | `dev/archive/f2532a2d-jit-project-lead/sessions/session-20260624-project-lead-role-design.md` | `f2532a2d` |
| 1947:14, 3734:5980, 3734:6098 | `dev/sessions/session-20260625-planning-skill-authoring.md` | `dev/archive/f2532a2d-jit-project-lead/sessions/session-20260625-planning-skill-authoring.md` | `f2532a2d` |
| 2001:14 | `dev/active/0efbc594-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/0efbc594-breakdown-spec.md` | `2821e177` |
| 2002:14 | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `2821e177` |
| 2003:14 | `dev/active/2821e177-investigation.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md` | `2821e177` |
| 2004:14 | `dev/active/37506c12-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/37506c12-breakdown-spec.md` | `2821e177` |
| 2005:14 | `dev/active/637764ef-acceptance-evidence.md` | `dev/archive/2821e177-addressing-v2/active/637764ef-acceptance-evidence.md` | `2821e177` |
| 2006:14 | `dev/active/71ebd1e8-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/71ebd1e8-breakdown-spec.md` | `2821e177` |
| 2007:14 | `dev/active/7f22d6cf-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/7f22d6cf-breakdown-spec.md` | `2821e177` |
| 2008:14 | `dev/active/9a7106ae-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/9a7106ae-breakdown-spec.md` | `2821e177` |
| 2009:14 | `dev/active/bb7d57a2-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/bb7d57a2-breakdown-spec.md` | `2821e177` |
| 2010:14, 3777:1659, 3777:1760 | `dev/studies/addressing-v2-item-use-cases.md` | `dev/archive/2821e177-addressing-v2/studies/addressing-v2-item-use-cases.md` | `2821e177` |
| 2011:14, 3777:1809, 3777:1911 | `dev/studies/addressing-v2-rule-gate-items.md` | `dev/archive/2821e177-addressing-v2/studies/addressing-v2-rule-gate-items.md` | `2821e177` |
| 2065:14 | `dev/active/8b05a612-investigation.md` | `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-investigation.md` | `9b7b5f9c` |
| 2066:14 | `dev/active/8b05a612-plan.md` | `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-plan.md` | `9b7b5f9c` |
| 2067:14 | `dev/active/8b05a612-research.md` | `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-research.md` | `9b7b5f9c` |
| 2068:14, 3459:1839 | `dev/active/9b7b5f9c-investigation.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-investigation.md` | `9b7b5f9c` |
| 2069:14, 3459:1879 | `dev/active/9b7b5f9c-mvp-scope-brief.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-mvp-scope-brief.md` | `9b7b5f9c` |
| 2070:14, 3459:1425, 3459:1501, 3459:1577, 3459:1653, 3459:1921 | `dev/active/9b7b5f9c-plan.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-plan.md` | `9b7b5f9c` |
| 2071:14, 3459:1952 | `dev/active/9b7b5f9c-research.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-research.md` | `9b7b5f9c` |
| 2072:14 | `dev/presentations/9b7b5f9c/base.css` | `dev/archive/9b7b5f9c-jit-profiles/presentations/9b7b5f9c/base.css` | `9b7b5f9c` |
| 2073:14, 3459:1987 | `dev/presentations/9b7b5f9c/talk.html` | `dev/archive/9b7b5f9c-jit-profiles/presentations/9b7b5f9c/talk.html` | `9b7b5f9c` |
| 2074:14 | `dev/presentations/9b7b5f9c/themes/rust.css` | `dev/archive/9b7b5f9c-jit-profiles/presentations/9b7b5f9c/themes/rust.css` | `9b7b5f9c` |
| 2128:14 | `dev/active/57269494-apply-plan-doc.md` | `dev/archive/9ac9fdac-graph-templates/active/57269494-apply-plan-doc.md` | `9ac9fdac` |
| 2129:14 | `dev/active/9ac9fdac-828a-40f2-9d73-71af14f44ff8-plan.md` | `dev/archive/9ac9fdac-graph-templates/active/9ac9fdac-828a-40f2-9d73-71af14f44ff8-plan.md` | `9ac9fdac` |
| 2130:14 | `dev/active/9ac9fdac-completion-report.md` | `dev/archive/9ac9fdac-graph-templates/active/9ac9fdac-completion-report.md` | `9ac9fdac` |
| 2131:14 | `dev/active/9ac9fdac-graph-templates-showcase/base.css` | `dev/archive/9ac9fdac-graph-templates/active/9ac9fdac-graph-templates-showcase/base.css` | `9ac9fdac` |
| 2132:14 | `dev/active/9ac9fdac-graph-templates-showcase/talk.html` | `dev/archive/9ac9fdac-graph-templates/active/9ac9fdac-graph-templates-showcase/talk.html` | `9ac9fdac` |
| 2133:14 | `dev/active/9ac9fdac-graph-templates-showcase/themes/rust.css` | `dev/archive/9ac9fdac-graph-templates/active/9ac9fdac-graph-templates-showcase/themes/rust.css` | `9ac9fdac` |
| 2142:1512, 2441:1642, 3648:1929, 3648:2022, 3648:2111, 3648:2189 | `dev/active/json-output-standardization-plan.md` | `dev/archive/9d427a6b-production-polish/active/json-output-standardization-plan.md` | `9d427a6b` |
| 2187:14, 3648:1182, 3648:1269 | `dev/active/bulk-operations-plan.md` | `dev/archive/14303b30-phase5-2/active/bulk-operations-plan.md` | `14303b30` |
| 2188:14, 3605:1315, 3605:1408 | `dev/active/ci-gate-integration-design.md` | `dev/archive/14303b30-phase5-2/active/ci-gate-integration-design.md` | `14303b30` |
| 2189:14, 3691:1461, 3691:1541 | `dev/active/gate-examples.md` | `dev/archive/14303b30-phase5-2/active/gate-examples.md` | `14303b30` |
| 2190:14, 3648:1626, 3648:1721 | `dev/active/gate-modification-flags-plan.md` | `dev/archive/14303b30-phase5-2/active/gate-modification-flags-plan.md` | `14303b30` |
| 2191:14, 3648:2465, 3648:2547 | `dev/active/quiet-mode-plan.md` | `dev/archive/14303b30-phase5-2/active/quiet-mode-plan.md` | `14303b30` |
| 2192:14, 3734:1427, 3734:1534 | `dev/sessions/session-2025-12-21-short-hash-progress.md` | `dev/archive/14303b30-phase5-2/sessions/session-2025-12-21-short-hash-progress.md` | `14303b30` |
| 2193:14, 3734:2101, 3734:2214 | `dev/sessions/session-2025-12-29-quiet-flag-implementation.md` | `dev/archive/14303b30-phase5-2/sessions/session-2025-12-29-quiet-flag-implementation.md` | `14303b30` |
| 2194:14, 3777:6446, 3777:6544 | `dev/studies/short-hash-implementation-plan.md` | `dev/archive/14303b30-phase5-2/studies/short-hash-implementation-plan.md` | `14303b30` |
| 2248:14 | `dev/active/0d593d90-invariant-projection-design.md` | `dev/archive/25064508-structured-knowledge/active/0d593d90-invariant-projection-design.md` | `25064508` |
| 2249:14 | `dev/active/1e1ea81d-item-kind-six-tuple.md` | `dev/archive/25064508-structured-knowledge/active/1e1ea81d-item-kind-six-tuple.md` | `25064508` |
| 2250:14 | `dev/active/21558ace-invariants-registry.md` | `dev/archive/25064508-structured-knowledge/active/21558ace-invariants-registry.md` | `25064508` |
| 2251:14 | `dev/active/56ab0224-item-model-design.md` | `dev/archive/25064508-structured-knowledge/active/56ab0224-item-model-design.md` | `25064508` |
| 2252:14 | `dev/active/93480b00-invariant-registry-first-kind.md` | `dev/archive/25064508-structured-knowledge/active/93480b00-invariant-registry-first-kind.md` | `25064508` |
| 2306:14, 3691:1111, 3691:1237 | `dev/active/config-consolidation-documentation-requirements.md` | `dev/archive/5fe00921-production-stability/active/config-consolidation-documentation-requirements.md` | `5fe00921` |
| 2307:14, 3648:1309, 3648:1413 | `dev/active/config-consolidation-plan.md` | `dev/archive/5fe00921-production-stability/active/config-consolidation-plan.md` | `5fe00921` |
| 2308:14, 3648:2302, 3648:2413 | `dev/active/multi-issue-bulk-operations-plan.md` | `dev/archive/5fe00921-production-stability/active/multi-issue-bulk-operations-plan.md` | `5fe00921` |
| 2309:14, 3605:2921, 3605:3027 | `dev/active/production-stability-design.md` | `dev/archive/5fe00921-production-stability/active/production-stability-design.md` | `5fe00921` |
| 2310:14, 3648:2745, 3648:2860 | `dev/active/transitive-reduction-validation-plan.md` | `dev/archive/5fe00921-production-stability/active/transitive-reduction-validation-plan.md` | `5fe00921` |
| 2311:14 | `dev/plans/5dbc3548-deletion-tracking.md` | `dev/archive/5fe00921-production-stability/plans/5dbc3548-deletion-tracking.md` | `5fe00921` |
| 2312:14 | `dev/plans/benchmarks-8d80b5dd.md` | `dev/archive/5fe00921-production-stability/plans/benchmarks-8d80b5dd.md` | `5fe00921` |
| 2313:14 | `dev/plans/error-recovery-0587a73a.md` | `dev/archive/5fe00921-production-stability/plans/error-recovery-0587a73a.md` | `5fe00921` |
| 2314:14, 3734:1763, 3734:1883 | `dev/sessions/session-2025-12-26-config-consolidation.md` | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-26-config-consolidation.md` | `5fe00921` |
| 2315:14, 3734:2280, 3734:2400 | `dev/sessions/session-2025-12-30-bulk-cli-integration.md` | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-cli-integration.md` | `5fe00921` |
| 2316:14, 3734:2461, 3734:2585 | `dev/sessions/session-2025-12-30-bulk-operations-progress.md` | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-operations-progress.md` | `5fe00921` |
| 2317:14, 3734:2650, 3734:2766 | `dev/sessions/session-2025-12-30-bulk-phase5-docs.md` | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-phase5-docs.md` | `5fe00921` |
| 2318:14, 3734:2823, 3734:2942 | `dev/sessions/session-2025-12-30-bulk-state-decision.md` | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-state-decision.md` | `5fe00921` |
| 2319:14, 3734:3002, 3734:3117 | `dev/sessions/session-2025-12-30-bulk-validation.md` | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-validation.md` | `5fe00921` |
| 2373:14, 3274:1482, 3274:1598, 3274:1709, 3459:1251, 3605:2636, 3605:2734 | `dev/active/planning-bracket-design.md` | `dev/archive/2fbd2a82-planning-bracket/active/planning-bracket-design.md` | `2fbd2a82` |
| 2374:14 | `dev/active/planning-bracket-showcase/base.css` | `dev/archive/2fbd2a82-planning-bracket/active/planning-bracket-showcase/base.css` | `2fbd2a82` |
| 2375:14 | `dev/active/planning-bracket-showcase/talk.html` | `dev/archive/2fbd2a82-planning-bracket/active/planning-bracket-showcase/talk.html` | `2fbd2a82` |
| 2376:14 | `dev/active/planning-bracket-showcase/themes/rust.css` | `dev/archive/2fbd2a82-planning-bracket/active/planning-bracket-showcase/themes/rust.css` | `2fbd2a82` |
| 2430:14 | `dev/active/5c060496-raw-assets-design.md` | `dev/archive/94f873c8-docs-lifecycle-p2/active/5c060496-raw-assets-design.md` | `94f873c8` |
| 2431:14, 3459:1730 | `dev/active/abfd6016-multi-format-doc-rendering-design.md` | `dev/archive/94f873c8-docs-lifecycle-p2/active/abfd6016-multi-format-doc-rendering-design.md` | `94f873c8` |
| 2432:14, 3605:2334, 3605:2447 | `dev/active/documentation-lifecycle-phase2-design.md` | `dev/archive/94f873c8-docs-lifecycle-p2/active/documentation-lifecycle-phase2-design.md` | `94f873c8` |
| 2487:14, 3648:1458, 3648:1570 | `dev/active/dependency-display-improvements-plan.md` | `dev/archive/9d427a6b-production-polish/active/dependency-display-improvements-plan.md` | `9d427a6b` |
| 2488:14, 3648:1769, 3648:1877 | `dev/active/gate-presets-implementation-plan.md` | `dev/archive/9d427a6b-production-polish/active/gate-presets-implementation-plan.md` | `9d427a6b` |
| 2489:14, 3605:2777, 3605:2877 | `dev/active/production-polish-design.md` | `dev/archive/9d427a6b-production-polish/active/production-polish-design.md` | `9d427a6b` |
| 2490:14 | `dev/design/exploration-dag.md` | `dev/archive/9d427a6b-production-polish/design/exploration-dag.md` | `9d427a6b` |
| 2491:14 | `dev/design/hierarchy-icons-config.md` | `dev/archive/9d427a6b-production-polish/design/hierarchy-icons-config.md` | `9d427a6b` |
| 2494:14 | `dev/design/search-focus-navigation.md` | `dev/archive/9d427a6b-production-polish/design/search-focus-navigation.md` | `9d427a6b` |
| 2496:14 | `dev/plans/7004d5b6-reorganize-commands.md` | `dev/archive/9d427a6b-production-polish/plans/7004d5b6-reorganize-commands.md` | `9d427a6b` |
| 2497:14, 3734:3522, 3734:3636 | `dev/sessions/session-20260109-query-consolidation.md` | `dev/archive/9d427a6b-production-polish/sessions/session-20260109-query-consolidation.md` | `9d427a6b` |
| 2498:14, 3777:1961, 3777:2059 | `dev/studies/agent-ux-observations.md` | `dev/archive/9d427a6b-production-polish/studies/agent-ux-observations.md` | `9d427a6b` |
| 2552:14 | `dev/active/2e926e39-906e-4952-8ae0-38215a7e5aac-plan.md` | `dev/archive/2e926e39-agent-seamlessness/active/2e926e39-906e-4952-8ae0-38215a7e5aac-plan.md` | `2e926e39` |
| 2553:14 | `dev/active/2e926e39-completion-report.md` | `dev/archive/2e926e39-agent-seamlessness/active/2e926e39-completion-report.md` | `2e926e39` |
| 2554:14 | `dev/presentations/2e926e39/base.css` | `dev/archive/2e926e39-agent-seamlessness/presentations/2e926e39/base.css` | `2e926e39` |
| 2555:14 | `dev/presentations/2e926e39/talk.html` | `dev/archive/2e926e39-agent-seamlessness/presentations/2e926e39/talk.html` | `2e926e39` |
| 2556:14 | `dev/presentations/2e926e39/themes/rust.css` | `dev/archive/2e926e39-agent-seamlessness/presentations/2e926e39/themes/rust.css` | `2e926e39` |
| 2610:14, 3691:1304, 3691:1409 | `dev/active/doc-archive-implementation-guide.md` | `dev/archive/71373e37-docs-lifecycle/active/doc-archive-implementation-guide.md` | `71373e37` |
| 2611:14, 3648:2582, 3648:2690 | `dev/active/snapshot-export-implementation-plan.md` | `dev/archive/71373e37-docs-lifecycle/active/snapshot-export-implementation-plan.md` | `71373e37` |
| 2612:14, 3734:1248, 3734:1364 | `dev/sessions/session-2024-12-24-check-links-incomplete.md` | `dev/archive/71373e37-docs-lifecycle/sessions/session-2024-12-24-check-links-incomplete.md` | `71373e37` |
| 2613:14, 3734:1594, 3734:1705 | `dev/sessions/session-2025-12-22-doc-consolidation.md` | `dev/archive/71373e37-docs-lifecycle/sessions/session-2025-12-22-doc-consolidation.md` | `71373e37` |
| 2614:14, 3734:1944, 3734:2049 | `dev/sessions/session-2025-12-27-doc-archive.md` | `dev/archive/71373e37-docs-lifecycle/sessions/session-2025-12-27-doc-archive.md` | `71373e37` |
| 2615:14, 3777:3970, 3777:4076 | `dev/studies/documentation-lifecycle-strategy.md` | `dev/archive/71373e37-docs-lifecycle/studies/documentation-lifecycle-strategy.md` | `71373e37` |
| 2669:14 | `dev/active/90a2dbfd-be54-46f2-b84a-b19382c6b0f2-plan.md` | `dev/archive/90a2dbfd-item-sources/active/90a2dbfd-be54-46f2-b84a-b19382c6b0f2-plan.md` | `90a2dbfd` |
| 2670:14 | `dev/active/90a2dbfd-kinds-over-sources.md` | `dev/archive/90a2dbfd-item-sources/active/90a2dbfd-kinds-over-sources.md` | `90a2dbfd` |
| 2725:14, 3605:1174, 3605:1272 | `dev/active/agent-validation-design.md` | `dev/archive/4a00b2b0-agent-validation/active/agent-validation-design.md` | `4a00b2b0` |
| 2727:14, 3777:6595, 3777:6694 | `dev/studies/worktree-merge-analysis.md` | `dev/archive/4a00b2b0-agent-validation/studies/worktree-merge-analysis.md` | `4a00b2b0` |
| 2781:14 | `dev/active/53e3fa36-c0cb-4206-8e8e-0a21aafb213e-plan.md` | `dev/archive/53e3fa36-agent-ergonomics/active/53e3fa36-c0cb-4206-8e8e-0a21aafb213e-plan.md` | `53e3fa36` |
| 2782:14, 3777:6287, 3777:6394 | `dev/studies/session-mining-jit-improvements.md` | `dev/archive/53e3fa36-agent-ergonomics/studies/session-mining-jit-improvements.md` | `53e3fa36` |
| 2836:14 | `dev/active/7095769d-smell-hunt-report.md` | `dev/archive/7095769d-code-smell-cleanup/active/7095769d-smell-hunt-report.md` | `7095769d` |
| 2890:14, 3605:3074, 3605:3170 | `dev/active/rejection-state-design.md` | `dev/archive/93f3e4df-rejection-state/active/rejection-state-design.md` | `93f3e4df` |
| 2944:14, 3605:2504, 3605:2596 | `dev/active/observability-design.md` | `dev/archive/d7bfd4a4-observability/active/observability-design.md` | `d7bfd4a4` |
| 2945:14 | `dev/plans/metrics-713ff59d.md` | `dev/archive/d7bfd4a4-observability/plans/metrics-713ff59d.md` | `d7bfd4a4` |
| 2946:14 | `dev/plans/stalled-detection-c802a9b0.md` | `dev/archive/d7bfd4a4-observability/plans/stalled-detection-c802a9b0.md` | `d7bfd4a4` |
| 3001:14 | `dev/design/cli-quality-improvements.md` | `dev/archive/ad601a15-parallel-work/design/cli-quality-improvements.md` | `ad601a15` |
| 3004:14, 3734:3338, 3734:3456 | `dev/sessions/session-20260103-parallel-work-design-review.md` | `dev/archive/ad601a15-parallel-work/sessions/session-20260103-parallel-work-design-review.md` | `ad601a15` |
| 3005:14, 3734:3694, 3734:3808 | `dev/sessions/session-20260111-validate-implementation.md` | `dev/archive/ad601a15-parallel-work/sessions/session-20260111-validate-implementation.md` | `ad601a15` |
| 3006:14, 3734:3870, 3734:3979 | `dev/sessions/session-20260115-cli-quality-phase3.md` | `dev/archive/ad601a15-parallel-work/sessions/session-20260115-cli-quality-phase3.md` | `ad601a15` |
| 3007:14, 3734:4036, 3734:4149 | `dev/sessions/session-20260115-phase3-issue7-complete.md` | `dev/archive/ad601a15-parallel-work/sessions/session-20260115-phase3-issue7-complete.md` | `ad601a15` |
| 3008:14, 3734:4210, 3734:4318 | `dev/sessions/session-20260115-story-review-f023.md` | `dev/archive/ad601a15-parallel-work/sessions/session-20260115-story-review-f023.md` | `ad601a15` |
| 3009:14, 3734:4374, 3734:4484 | `dev/sessions/session-20260118-f849-manual-testing.md` | `dev/archive/ad601a15-parallel-work/sessions/session-20260118-f849-manual-testing.md` | `ad601a15` |
| 3010:14, 3734:4542, 3734:4657 | `dev/sessions/session-20260201-cli-enforcement-82b17394.md` | `dev/archive/ad601a15-parallel-work/sessions/session-20260201-cli-enforcement-82b17394.md` | `ad601a15` |
| 3011:14, 3734:4720, 3734:4837 | `dev/sessions/session-20260201-enforcement-modes-5e1d5f02.md` | `dev/archive/ad601a15-parallel-work/sessions/session-20260201-enforcement-modes-5e1d5f02.md` | `ad601a15` |
| 3012:14, 3734:4902, 3734:5019 | `dev/sessions/session-20260201-refactor-1bdc5395-analysis.md` | `dev/archive/ad601a15-parallel-work/sessions/session-20260201-refactor-1bdc5395-analysis.md` | `ad601a15` |
| 3013:14, 3734:5084, 3734:5193 | `dev/sessions/session-20260201-refactor-items-5-7.md` | `dev/archive/ad601a15-parallel-work/sessions/session-20260201-refactor-items-5-7.md` | `ad601a15` |
| 3014:14, 3734:6161, 3734:6270 | `dev/sessions/session-claim-coordination-parallel.md` | `dev/archive/ad601a15-parallel-work/sessions/session-claim-coordination-parallel.md` | `ad601a15` |
| 3015:14, 3734:6327, 3734:6434 | `dev/sessions/session-claim-coordination-review.md` | `dev/archive/ad601a15-parallel-work/sessions/session-claim-coordination-review.md` | `ad601a15` |
| 3024:1656, 3777:2982, 3777:3085, 3777:3157 | `dev/studies/cdc840ad-audit-2026-07-23.md` | `dev/archive/1cc809de-repository-state-quality/studies/cdc840ad-audit-2026-07-23.md` | `1cc809de` |
| 3070:14 | `dev/presentations/1cc809de/base.css` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/base.css` | `1cc809de` |
| 3071:14 | `dev/presentations/1cc809de/talk.html` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/talk.html` | `1cc809de` |
| 3072:14 | `dev/presentations/1cc809de/themes/rust.css` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/themes/rust.css` | `1cc809de` |
| 3073:14 | `dev/presentations/1cc809de/vendor/fonts/fonts.css` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/fonts/fonts.css` | `1cc809de` |
| 3074:14 | `dev/presentations/1cc809de/vendor/fonts/jetbrains-mono-latin-400-normal.woff2` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/fonts/jetbrains-mono-latin-400-normal.woff2` | `1cc809de` |
| 3075:14 | `dev/presentations/1cc809de/vendor/fonts/jetbrains-mono-latin-500-normal.woff2` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/fonts/jetbrains-mono-latin-500-normal.woff2` | `1cc809de` |
| 3076:14 | `dev/presentations/1cc809de/vendor/fonts/open-sans-latin-400-italic.woff2` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/fonts/open-sans-latin-400-italic.woff2` | `1cc809de` |
| 3077:14 | `dev/presentations/1cc809de/vendor/fonts/open-sans-latin-400-normal.woff2` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/fonts/open-sans-latin-400-normal.woff2` | `1cc809de` |
| 3078:14 | `dev/presentations/1cc809de/vendor/fonts/open-sans-latin-600-normal.woff2` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/fonts/open-sans-latin-600-normal.woff2` | `1cc809de` |
| 3079:14 | `dev/presentations/1cc809de/vendor/fonts/open-sans-latin-700-normal.woff2` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/fonts/open-sans-latin-700-normal.woff2` | `1cc809de` |
| 3080:14 | `dev/presentations/1cc809de/vendor/fonts/source-code-pro-latin-400-normal.woff2` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/fonts/source-code-pro-latin-400-normal.woff2` | `1cc809de` |
| 3081:14 | `dev/presentations/1cc809de/vendor/fonts/source-code-pro-latin-500-normal.woff2` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/fonts/source-code-pro-latin-500-normal.woff2` | `1cc809de` |
| 3082:14 | `dev/presentations/1cc809de/vendor/reveal.js/plugin/highlight/highlight.js` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/reveal.js/plugin/highlight/highlight.js` | `1cc809de` |
| 3083:14 | `dev/presentations/1cc809de/vendor/reveal.js/reset.css` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/reveal.js/reset.css` | `1cc809de` |
| 3084:14 | `dev/presentations/1cc809de/vendor/reveal.js/reveal.css` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/reveal.js/reveal.css` | `1cc809de` |
| 3085:14 | `dev/presentations/1cc809de/vendor/reveal.js/reveal.js` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/reveal.js/reveal.js` | `1cc809de` |
| 3086:14 | `dev/presentations/cdc840ad/README.md` | `dev/archive/1cc809de-repository-state-quality/presentations/cdc840ad/README.md` | `1cc809de` |
| 3142:14, 3777:5069, 3777:5160 | `dev/studies/documentation-tooling-evaluation.md` | `dev/archive/a4e3cfb0/studies/documentation-tooling-evaluation.md` | `a4e3cfb0` |

### `dev/active/8e071e18-investigation.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 120:45 | `dev/presentations/1cc809de/talk.html` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/talk.html` | `1cc809de` |
| 121:45 | `dev/presentations/cdc840ad/README.md` | `dev/archive/1cc809de-repository-state-quality/presentations/cdc840ad/README.md` | `1cc809de` |
| 129:2 | `dev/presentations/1cc809de/vendor/reveal.js/reveal.css` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/vendor/reveal.js/reveal.css` | `1cc809de` |
| 132:23, 140:54, 142:12 | `dev/presentations/9b7b5f9c/talk.html` | `dev/archive/9b7b5f9c-jit-profiles/presentations/9b7b5f9c/talk.html` | `9b7b5f9c` |
| 141:12 | `dev/presentations/9b7b5f9c/base.css` | `dev/archive/9b7b5f9c-jit-profiles/presentations/9b7b5f9c/base.css` | `9b7b5f9c` |
| 143:12 | `dev/presentations/9b7b5f9c/themes/rust.css` | `dev/archive/9b7b5f9c-jit-profiles/presentations/9b7b5f9c/themes/rust.css` | `9b7b5f9c` |
| 647:52 | `dev/active/doc-archive-implementation-guide.md` | `dev/archive/71373e37-docs-lifecycle/active/doc-archive-implementation-guide.md` | `71373e37` |
| 648:4 | `dev/active/documentation-lifecycle-phase2-design.md` | `dev/archive/94f873c8-docs-lifecycle-p2/active/documentation-lifecycle-phase2-design.md` | `94f873c8` |
| 648:59 | `dev/active/config-consolidation-plan.md` | `dev/archive/5fe00921-production-stability/active/config-consolidation-plan.md` | `5fe00921` |
| 650:28 | `dev/studies/documentation-lifecycle-strategy.md` | `dev/archive/71373e37-docs-lifecycle/studies/documentation-lifecycle-strategy.md` | `71373e37` |
| 783:5, 1304:77, 1305:104, 1306:75, 1307:73, 1308:88, 1309:62, 1310:62, 1311:83, 1316:2, 1362:43, 1383:58 | `dev/design/worktree-parallel-work.md` | `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` | `ad601a15` |
| 1137:5 | `dev/benchmarks/rust-build-efficiency/consolidation-inventory-diff.json` | `dev/archive/6eb585bc-core-maintenance/benchmarks/rust-build-efficiency/consolidation-inventory-diff.json` | `6eb585bc` |
| 1140:5 | `dev/benchmarks/rust-build-efficiency/verify-consolidation-inventory.py` | `dev/archive/6eb585bc-core-maintenance/benchmarks/rust-build-efficiency/verify-consolidation-inventory.py` | `6eb585bc` |
| 1308:4, 1309:4, 1310:4, 1360:4, 1363:22 | `dev/studies/ai-tool-worktree-compatibility.md` | `dev/archive/4a00b2b0-agent-validation/studies/ai-tool-worktree-compatibility.md` | `4a00b2b0` |
| 1311:4, 1355:35, 1360:62, 1370:55 | `dev/experiments/worktree-manual-coordination-experiment.md` | `dev/archive/ad601a15-parallel-work/experiments/worktree-manual-coordination-experiment.md` | `ad601a15` |
| 1312:4, 1314:55 | `dev/design/phase2-collapse-expand.md` | `dev/archive/9d427a6b-production-polish/design/phase2-collapse-expand.md` | `9d427a6b` |
| 1312:53, 1313:55 | `dev/design/subgraph-clustering-layout.md` | `dev/archive/9d427a6b-production-polish/design/subgraph-clustering-layout.md` | `9d427a6b` |
| 1313:4, 1314:4 | `dev/design/phase3-advanced-features.md` | `dev/archive/9d427a6b-production-polish/design/phase3-advanced-features.md` | `9d427a6b` |

### `dev/active/c639cfb5-investigation.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 153:199, 153:336, 682:4, 725:13, 726:41 | `dev/active/9b7b5f9c-investigation.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-investigation.md` | `9b7b5f9c` |
| 680:24, 718:3 | `dev/active/9b7b5f9c-mvp-scope-brief.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-mvp-scope-brief.md` | `9b7b5f9c` |
| 683:4, 684:4, 721:3 | `dev/active/9b7b5f9c-plan.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-plan.md` | `9b7b5f9c` |
| 686:4, 687:4, 727:59 | `dev/active/9b7b5f9c-research.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-research.md` | `9b7b5f9c` |
| 695:5, 696:4, 697:4 | `dev/presentations/9b7b5f9c/talk.html` | `dev/archive/9b7b5f9c-jit-profiles/presentations/9b7b5f9c/talk.html` | `9b7b5f9c` |

### `dev/active/8e071e18-dev-artifact-layout/archive-run-evidence.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 14:11, 14:92 | `dev/active/json-output-standardization-plan.md` | `dev/archive/9d427a6b-production-polish/active/json-output-standardization-plan.md` | `9d427a6b` |
| 32:4, 373:4, 513:4, 804:4 | `dev/active/8b05a612-investigation.md` | `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-investigation.md` | `9b7b5f9c` |
| 57:4 | `dev/sessions/session-2026-01-01-example-md-migration.md` | `dev/archive/cfb3ba94-docs/sessions/session-2026-01-01-example-md-migration.md` | `cfb3ba94` |
| 127:4, 327:4, 532:4, 572:4 | `dev/active/9b7b5f9c-investigation.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-investigation.md` | `9b7b5f9c` |
| 169:4 | `dev/active/2e926e39-906e-4952-8ae0-38215a7e5aac-plan.md` | `dev/archive/2e926e39-agent-seamlessness/active/2e926e39-906e-4952-8ae0-38215a7e5aac-plan.md` | `2e926e39` |

### `dev/active/f2532a2d-handoff-2.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 34:290, 63:31 | `dev/active/eed6750c-handoff.md` | `dev/archive/f2532a2d-jit-project-lead/active/eed6750c-handoff.md` | `f2532a2d` |
| 55:20 | `dev/active/f2532a2d-138d-46bf-9547-e1ea517f0170-plan.md` | `dev/archive/f2532a2d-jit-project-lead/active/f2532a2d-138d-46bf-9547-e1ea517f0170-plan.md` | `f2532a2d` |

### `dev/active/8e071e18-progress.json`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 1501:154 | `dev/active/2b9a80fb-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/2b9a80fb-audit-notes.md` | `2d109173` |
| 1720:221 | `dev/active/json-output-standardization-plan.md` | `dev/archive/9d427a6b-production-polish/active/json-output-standardization-plan.md` | `9d427a6b` |

### `dev/active/f2532a2d-handoff.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 56:20 | `dev/active/f2532a2d-138d-46bf-9547-e1ea517f0170-plan.md` | `dev/archive/f2532a2d-jit-project-lead/active/f2532a2d-138d-46bf-9547-e1ea517f0170-plan.md` | `f2532a2d` |
| 63:31 | `dev/active/eed6750c-handoff.md` | `dev/archive/f2532a2d-jit-project-lead/active/eed6750c-handoff.md` | `f2532a2d` |

### `crates/jit/src/storage/claim_coordinator.rs`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 13:22 | `dev/design/worktree-parallel-work.md` | `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` | `ad601a15` |

### `crates/jit/tests/cli_repo_workflow/config_get_tests.rs`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 369:42 | `dev/design/worktree-parallel-work.md` | `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` | `ad601a15` |

### `crates/jit/tests/fast_docs_templates/bracket_breakdown_tests.rs`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 2:6 | `dev/active/planning-bracket-design.md` | `dev/archive/2fbd2a82-planning-bracket/active/planning-bracket-design.md` | `2fbd2a82` |

### `crates/jit/tests/fast_docs_templates/research_bracket_tests.rs`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 27:7 | `dev/active/planning-bracket-design.md` | `dev/archive/2fbd2a82-planning-bracket/active/planning-bracket-design.md` | `2fbd2a82` |

### `crates/jit/tests/fast_docs_templates/sdd_bracket_tests.rs`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 19:7 | `dev/active/planning-bracket-design.md` | `dev/archive/2fbd2a82-planning-bracket/active/planning-bracket-design.md` | `2fbd2a82` |

### `dev/active/2fbd2a82-14ba-4e6e-90f6-e0c34f0f912c-plan.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 18:9 | `dev/active/planning-bracket-design.md` | `dev/archive/2fbd2a82-planning-bracket/active/planning-bracket-design.md` | `2fbd2a82` |

### `dev/active/73482aa1-progress.json`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 7:90 | `dev/active/73482aa1-completion-report.md` | `dev/archive/6eb585bc-core-maintenance/active/73482aa1-completion-report.md` | `6eb585bc` |

### `dev/active/9b7b5f9c-handoff-3.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 65:10 | `dev/active/9b7b5f9c-plan.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-plan.md` | `9b7b5f9c` |

### `dev/active/9b7b5f9c-handoff-4.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 54:10 | `dev/active/9b7b5f9c-plan.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-plan.md` | `9b7b5f9c` |

### `dev/active/9b7b5f9c-handoff-5.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 55:10 | `dev/active/9b7b5f9c-plan.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-plan.md` | `9b7b5f9c` |

### `dev/active/9b7b5f9c-handoff.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 54:10 | `dev/active/9b7b5f9c-plan.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-plan.md` | `9b7b5f9c` |

### `dev/active/abfd6016-progress.json`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 64:18 | `dev/active/abfd6016-multi-format-doc-rendering-design.md` | `dev/archive/94f873c8-docs-lifecycle-p2/active/abfd6016-multi-format-doc-rendering-design.md` | `94f873c8` |

### `dev/active/ca832358/req05-archival-execution-evidence.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 142:23 | `dev/active/4c33d0e5-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/4c33d0e5-audit-notes.md` | `2d109173` |

### `docs/how-to/multi-agent-coordination.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 472:21 | `dev/design/worktree-parallel-work.md` | `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` | `ad601a15` |

### `docs/tutorials/parallel-work-worktrees.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 257:21 | `dev/design/worktree-parallel-work.md` | `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` | `ad601a15` |

### `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-plan.md`

Reported at `dev/active/8b05a612-plan.md`; relocated by run `9b7b5f9c`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 23:549, 26:307, 33:612, 38:288 | `dev/active/8b05a612-investigation.md` | `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-investigation.md` | `9b7b5f9c` |
| 24:340, 27:429, 28:636, 29:604, 30:669, 31:498, 34:787, 35:975, 36:684 | `dev/active/8b05a612-research.md` | `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-research.md` | `9b7b5f9c` |

### `dev/archive/2821e177-addressing-v2/active/637764ef-acceptance-evidence.md`

Reported at `dev/active/637764ef-acceptance-evidence.md`; relocated by run `2821e177`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 149:4 | `dev/active/bb7d57a2-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/bb7d57a2-breakdown-spec.md` | `2821e177` |
| 150:4 | `dev/active/71ebd1e8-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/71ebd1e8-breakdown-spec.md` | `2821e177` |
| 151:4 | `dev/active/7f22d6cf-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/7f22d6cf-breakdown-spec.md` | `2821e177` |
| 152:4 | `dev/active/0efbc594-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/0efbc594-breakdown-spec.md` | `2821e177` |
| 153:4 | `dev/active/9a7106ae-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/9a7106ae-breakdown-spec.md` | `2821e177` |
| 154:4 | `dev/active/37506c12-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/37506c12-breakdown-spec.md` | `2821e177` |
| 155:4 | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `2821e177` |
| 156:4 | `dev/active/2821e177-investigation.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md` | `2821e177` |
| 163:34, 169:15 | `dev/studies/addressing-v2-rule-gate-items.md` | `dev/archive/2821e177-addressing-v2/studies/addressing-v2-rule-gate-items.md` | `2821e177` |

### `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-investigation.md`

Reported at `dev/active/9b7b5f9c-investigation.md`; relocated by run `9b7b5f9c`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 6:2, 11:3, 116:27, 310:5 | `dev/active/9b7b5f9c-mvp-scope-brief.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-mvp-scope-brief.md` | `9b7b5f9c` |
| 320:4 | `dev/active/57269494-apply-plan-doc.md` | `dev/archive/9ac9fdac-graph-templates/active/57269494-apply-plan-doc.md` | `9ac9fdac` |
| 324:4 | `dev/active/2821e177-investigation.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md` | `2821e177` |
| 325:4 | `dev/active/76cb968b-ssot-adoption-sweep.md` | `dev/archive/6eb585bc-core-maintenance/active/76cb968b-ssot-adoption-sweep.md` | `6eb585bc` |
| 328:4 | `dev/active/d24008f0-req01-evidence.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/d24008f0-req01-evidence.md` | `2d109173` |

### `dev/archive/ad601a15-parallel-work/sessions/session-claim-coordination-parallel.md`

Reported at `dev/sessions/session-claim-coordination-parallel.md`; relocated by run `ad601a15`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 82:24, 143:24, 188:24, 308:16 | `dev/design/worktree-parallel-work.md` | `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` | `ad601a15` |
| 309:24 | `dev/experiments/worktree-manual-coordination-experiment.md` | `dev/archive/ad601a15-parallel-work/experiments/worktree-manual-coordination-experiment.md` | `ad601a15` |

### `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md`

Reported at `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md`; relocated by run `2821e177`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 5:14, 490:28 | `dev/studies/addressing-v2-rule-gate-items.md` | `dev/archive/2821e177-addressing-v2/studies/addressing-v2-rule-gate-items.md` | `2821e177` |
| 6:9 | `dev/active/2821e177-investigation.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md` | `2821e177` |
| 492:4 | `dev/active/637764ef-acceptance-evidence.md` | `dev/archive/2821e177-addressing-v2/active/637764ef-acceptance-evidence.md` | `2821e177` |

### `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-investigation.md`

Reported at `dev/active/8b05a612-investigation.md`; relocated by run `9b7b5f9c`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 43:463 | `dev/active/gate-examples.md` | `dev/archive/14303b30-phase5-2/active/gate-examples.md` | `14303b30` |
| 60:4 | `dev/sessions/session-2025-12-22-doc-consolidation.md` | `dev/archive/71373e37-docs-lifecycle/sessions/session-2025-12-22-doc-consolidation.md` | `71373e37` |
| 61:4 | `dev/active/36d5451e-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-audit-notes.md` | `2d109173` |
| 61:45 | `dev/active/production-polish-design.md` | `dev/archive/9d427a6b-production-polish/active/production-polish-design.md` | `9d427a6b` |

### `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-cli-integration.md`

Reported at `dev/sessions/session-2025-12-30-bulk-cli-integration.md`; relocated by run `5fe00921`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 51:29, 238:4 | `dev/sessions/session-2025-12-30-bulk-validation.md` | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-validation.md` | `5fe00921` |
| 88:29, 239:4 | `dev/sessions/session-2025-12-30-bulk-state-decision.md` | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-state-decision.md` | `5fe00921` |

### `dev/archive/2d109173-docs-exhaustive-audit/active/6d82de03-followup-manifest.md`

Reported at `dev/active/6d82de03-followup-manifest.md`; relocated by run `2d109173`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 256:6 | `dev/active/2b9a80fb-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/2b9a80fb-audit-notes.md` | `2d109173` |
| 297:6, 337:6 | `dev/active/36d5451e-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-audit-notes.md` | `2d109173` |

### `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-research.md`

Reported at `dev/active/9b7b5f9c-research.md`; relocated by run `9b7b5f9c`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 9:16, 33:6, 162:5 | `dev/active/9b7b5f9c-mvp-scope-brief.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-mvp-scope-brief.md` | `9b7b5f9c` |

### `dev/archive/f2532a2d-jit-project-lead/active/c23dfe71-breakdown-spec.md`

Reported at `dev/active/c23dfe71-breakdown-spec.md`; relocated by run `f2532a2d`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 28:12, 32:13, 97:6 | `dev/active/eed6750c-handoff.md` | `dev/archive/f2532a2d-jit-project-lead/active/eed6750c-handoff.md` | `f2532a2d` |

### `dev/archive/f2532a2d-jit-project-lead/active/f2532a2d-138d-46bf-9547-e1ea517f0170-plan.md`

Reported at `dev/active/f2532a2d-138d-46bf-9547-e1ea517f0170-plan.md`; relocated by run `f2532a2d`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 66:33, 108:12, 112:13 | `dev/active/eed6750c-handoff.md` | `dev/archive/f2532a2d-jit-project-lead/active/eed6750c-handoff.md` | `f2532a2d` |

### `dev/archive/2821e177-addressing-v2/active/0efbc594-breakdown-spec.md`

Reported at `dev/active/0efbc594-breakdown-spec.md`; relocated by run `2821e177`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 3:162 | `dev/active/2821e177-investigation.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md` | `2821e177` |
| 3:55 | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `2821e177` |

### `dev/archive/2821e177-addressing-v2/active/37506c12-breakdown-spec.md`

Reported at `dev/active/37506c12-breakdown-spec.md`; relocated by run `2821e177`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 3:162 | `dev/active/2821e177-investigation.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md` | `2821e177` |
| 3:55 | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `2821e177` |

### `dev/archive/2821e177-addressing-v2/active/71ebd1e8-breakdown-spec.md`

Reported at `dev/active/71ebd1e8-breakdown-spec.md`; relocated by run `2821e177`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 3:162 | `dev/active/2821e177-investigation.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md` | `2821e177` |
| 3:55 | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `2821e177` |

### `dev/archive/6eb585bc-core-maintenance/active/76cb968b-ssot-adoption-sweep.md`

Reported at `dev/active/76cb968b-ssot-adoption-sweep.md`; relocated by run `6eb585bc`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 77:970 | `dev/active/76cb968b-sweep-table.md` | `dev/archive/6eb585bc-core-maintenance/active/76cb968b-sweep-table.md` | `6eb585bc` |
| 87:233 | `dev/active/76cb968b-citation-check.md` | `dev/archive/6eb585bc-core-maintenance/active/76cb968b-citation-check.md` | `6eb585bc` |

### `dev/archive/2821e177-addressing-v2/active/7f22d6cf-breakdown-spec.md`

Reported at `dev/active/7f22d6cf-breakdown-spec.md`; relocated by run `2821e177`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 3:162 | `dev/active/2821e177-investigation.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md` | `2821e177` |
| 3:55 | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `2821e177` |

### `dev/archive/2821e177-addressing-v2/active/9a7106ae-breakdown-spec.md`

Reported at `dev/active/9a7106ae-breakdown-spec.md`; relocated by run `2821e177`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 3:162 | `dev/active/2821e177-investigation.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md` | `2821e177` |
| 3:55 | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `2821e177` |

### `dev/archive/2821e177-addressing-v2/active/bb7d57a2-breakdown-spec.md`

Reported at `dev/active/bb7d57a2-breakdown-spec.md`; relocated by run `2821e177`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 3:162 | `dev/active/2821e177-investigation.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md` | `2821e177` |
| 3:55 | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `2821e177` |

### `dev/archive/f2532a2d-jit-project-lead/active/eed6750c-handoff.md`

Reported at `dev/active/eed6750c-handoff.md`; relocated by run `f2532a2d`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 13:4, 49:32 | `dev/sessions/session-20260625-planning-skill-authoring.md` | `dev/archive/f2532a2d-jit-project-lead/sessions/session-20260625-planning-skill-authoring.md` | `f2532a2d` |

### `dev/archive/9d427a6b-production-polish/design/phase3-advanced-features.md`

Reported at `dev/design/phase3-advanced-features.md`; relocated by run `9d427a6b`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 297:43 | `dev/design/subgraph-clustering-layout.md` | `dev/archive/9d427a6b-production-polish/design/subgraph-clustering-layout.md` | `9d427a6b` |
| 298:43 | `dev/design/phase2-collapse-expand.md` | `dev/archive/9d427a6b-production-polish/design/phase2-collapse-expand.md` | `9d427a6b` |

### `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/talk.html`

Reported at `dev/presentations/1cc809de/talk.html`; relocated by run `1cc809de`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 30:12, 635:55 | `dev/studies/cdc840ad-audit-2026-07-23.md` | `dev/archive/1cc809de-repository-state-quality/studies/cdc840ad-audit-2026-07-23.md` | `1cc809de` |

### `dev/archive/1cc809de-repository-state-quality/presentations/cdc840ad/README.md`

Reported at `dev/presentations/cdc840ad/README.md`; relocated by run `1cc809de`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 5:3 | `dev/studies/cdc840ad-audit-2026-07-23.md` | `dev/archive/1cc809de-repository-state-quality/studies/cdc840ad-audit-2026-07-23.md` | `1cc809de` |
| 9:3 | `dev/presentations/1cc809de/talk.html` | `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/talk.html` | `1cc809de` |

### `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-operations-progress.md`

Reported at `dev/sessions/session-2025-12-30-bulk-operations-progress.md`; relocated by run
`5fe00921`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 330:4 | `dev/active/multi-issue-bulk-operations-plan.md` | `dev/archive/5fe00921-production-stability/active/multi-issue-bulk-operations-plan.md` | `5fe00921` |
| 331:4 | `dev/active/production-stability-design.md` | `dev/archive/5fe00921-production-stability/active/production-stability-design.md` | `5fe00921` |

### `dev/archive/ad601a15-parallel-work/sessions/session-20260103-parallel-work-design-review.md`

Reported at `dev/sessions/session-20260103-parallel-work-design-review.md`; relocated by run
`ad601a15`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 34:26, 358:21 | `dev/design/worktree-parallel-work.md` | `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` | `ad601a15` |

### `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md`

Reported at `dev/active/2821e177-investigation.md`; relocated by run `2821e177`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 3:49 | `dev/studies/addressing-v2-rule-gate-items.md` | `dev/archive/2821e177-addressing-v2/studies/addressing-v2-rule-gate-items.md` | `2821e177` |

### `dev/archive/2e926e39-agent-seamlessness/active/2e926e39-906e-4952-8ae0-38215a7e5aac-plan.md`

Reported at `dev/active/2e926e39-906e-4952-8ae0-38215a7e5aac-plan.md`; relocated by run `2e926e39`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 38:361 | `dev/studies/session-mining-jit-improvements.md` | `dev/archive/53e3fa36-agent-ergonomics/studies/session-mining-jit-improvements.md` | `53e3fa36` |

### `dev/archive/f2532a2d-jit-project-lead/active/304f6d94-breakdown-spec.md`

Reported at `dev/active/304f6d94-breakdown-spec.md`; relocated by run `f2532a2d`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 96:6 | `dev/active/eed6750c-handoff.md` | `dev/archive/f2532a2d-jit-project-lead/active/eed6750c-handoff.md` | `f2532a2d` |

### `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-review-round-4.md`

Reported at `dev/active/36d5451e-review-round-4.md`; relocated by run `2d109173`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 38:4 | `dev/active/36d5451e-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-audit-notes.md` | `2d109173` |

### `dev/archive/f2532a2d-jit-project-lead/active/3c192f5e-breakdown-spec.md`

Reported at `dev/active/3c192f5e-breakdown-spec.md`; relocated by run `f2532a2d`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 107:6 | `dev/active/eed6750c-handoff.md` | `dev/archive/f2532a2d-jit-project-lead/active/eed6750c-handoff.md` | `f2532a2d` |

### `dev/archive/53e3fa36-agent-ergonomics/active/53e3fa36-c0cb-4206-8e8e-0a21aafb213e-plan.md`

Reported at `dev/active/53e3fa36-c0cb-4206-8e8e-0a21aafb213e-plan.md`; relocated by run `53e3fa36`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 4:19 | `dev/studies/session-mining-jit-improvements.md` | `dev/archive/53e3fa36-agent-ergonomics/studies/session-mining-jit-improvements.md` | `53e3fa36` |

### `dev/archive/6eb585bc-core-maintenance/active/71be6ae9-code-review-reliability.md`

Reported at `dev/active/71be6ae9-code-review-reliability.md`; relocated by run `6eb585bc`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 203:11 | `dev/active/71be6ae9-live-review-report.md` | `dev/archive/6eb585bc-core-maintenance/active/71be6ae9-live-review-report.md` | `6eb585bc` |

### `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-research.md`

Reported at `dev/active/8b05a612-research.md`; relocated by run `9b7b5f9c`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 4:19 | `dev/active/8b05a612-investigation.md` | `dev/archive/9b7b5f9c-jit-profiles/active/8b05a612-investigation.md` | `9b7b5f9c` |

### `dev/archive/2d109173-docs-exhaustive-audit/active/99f4a2b4-req02-evidence.md`

Reported at `dev/active/99f4a2b4-req02-evidence.md`; relocated by run `2d109173`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 15:104 | `dev/active/99f4a2b4-req02-evidence.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/99f4a2b4-req02-evidence.md` | `2d109173` |

### `dev/archive/f2532a2d-jit-project-lead/active/e8b1cee3-breakdown-spec.md`

Reported at `dev/active/e8b1cee3-breakdown-spec.md`; relocated by run `f2532a2d`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 107:6 | `dev/active/eed6750c-handoff.md` | `dev/archive/f2532a2d-jit-project-lead/active/eed6750c-handoff.md` | `f2532a2d` |

### `dev/archive/9d427a6b-production-polish/design/phase2-collapse-expand.md`

Reported at `dev/design/phase2-collapse-expand.md`; relocated by run `9d427a6b`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 248:43 | `dev/design/subgraph-clustering-layout.md` | `dev/archive/9d427a6b-production-polish/design/subgraph-clustering-layout.md` | `9d427a6b` |

### `dev/archive/ad601a15-parallel-work/experiments/worktree-manual-coordination-experiment.md`

Reported at `dev/experiments/worktree-manual-coordination-experiment.md`; relocated by run
`ad601a15`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 377:47 | `dev/design/worktree-parallel-work.md` | `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` | `ad601a15` |

### `dev/archive/5fe00921-production-stability/plans/5dbc3548-deletion-tracking.md`

Reported at `dev/plans/5dbc3548-deletion-tracking.md`; relocated by run `5fe00921`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 150:1 | `dev/plans/5dbc3548-deletion-tracking.md` | `dev/archive/5fe00921-production-stability/plans/5dbc3548-deletion-tracking.md` | `5fe00921` |

### `dev/archive/5fe00921-production-stability/sessions/session-2025-12-26-config-consolidation.md`

Reported at `dev/sessions/session-2025-12-26-config-consolidation.md`; relocated by run `5fe00921`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 391:10 | `dev/active/config-consolidation-plan.md` | `dev/archive/5fe00921-production-stability/active/config-consolidation-plan.md` | `5fe00921` |

### `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-phase5-docs.md`

Reported at `dev/sessions/session-2025-12-30-bulk-phase5-docs.md`; relocated by run `5fe00921`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 85:4 | `dev/sessions/session-2025-12-30-bulk-phase5-docs.md` | `dev/archive/5fe00921-production-stability/sessions/session-2025-12-30-bulk-phase5-docs.md` | `5fe00921` |

### `dev/archive/cfb3ba94-docs/sessions/session-2026-01-01-example-md-migration.md`

Reported at `dev/sessions/session-2026-01-01-example-md-migration.md`; relocated by run `cfb3ba94`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 190:3 | `dev/active/gate-examples.md` | `dev/archive/14303b30-phase5-2/active/gate-examples.md` | `14303b30` |

### `dev/archive/ad601a15-parallel-work/sessions/session-20260118-f849-manual-testing.md`

Reported at `dev/sessions/session-20260118-f849-manual-testing.md`; relocated by run `ad601a15`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 60:24 | `dev/design/worktree-parallel-work.md` | `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` | `ad601a15` |

### `dev/archive/2821e177-addressing-v2/studies/addressing-v2-rule-gate-items.md`

Reported at `dev/studies/addressing-v2-rule-gate-items.md`; relocated by run `2821e177`.

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 196:6 | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `2821e177` |

### `dev/archive/1cc809de-breakdown.json`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 5:394, 31:6, 38:309, 73:248, 103:289, 143:256, 448:6, 1013:6, 1173:6, 1213:6, 1251:6, 1288:6, 1305:315, 1305:1098, 1305:6568, 1343:1278 | `dev/studies/cdc840ad-audit-2026-07-23.md` | `dev/archive/1cc809de-repository-state-quality/studies/cdc840ad-audit-2026-07-23.md` | `1cc809de` |

### `dev/archive/features/2821e177/2821e177-handoff.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 51:40 | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `2821e177` |
| 52:18 | `dev/studies/addressing-v2-rule-gate-items.md` | `dev/archive/2821e177-addressing-v2/studies/addressing-v2-rule-gate-items.md` | `2821e177` |
| 52:81 | `dev/active/2821e177-investigation.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md` | `2821e177` |
| 53:103 | `dev/active/71ebd1e8-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/71ebd1e8-breakdown-spec.md` | `2821e177` |
| 53:144 | `dev/active/bb7d57a2-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/bb7d57a2-breakdown-spec.md` | `2821e177` |
| 53:185 | `dev/active/7f22d6cf-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/7f22d6cf-breakdown-spec.md` | `2821e177` |
| 53:21 | `dev/active/37506c12-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/37506c12-breakdown-spec.md` | `2821e177` |
| 53:226 | `dev/active/9a7106ae-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/9a7106ae-breakdown-spec.md` | `2821e177` |
| 53:62 | `dev/active/0efbc594-breakdown-spec.md` | `dev/archive/2821e177-addressing-v2/active/0efbc594-breakdown-spec.md` | `2821e177` |

### `dev/archive/1cc809de-plan.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 10:170, 192:250, 203:244, 217:222, 221:211, 222:221, 223:231, 224:220 | `dev/studies/cdc840ad-audit-2026-07-23.md` | `dev/archive/1cc809de-repository-state-quality/studies/cdc840ad-audit-2026-07-23.md` | `1cc809de` |

### `dev/archive/cdc840ad-investigation.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 1095:4 | `dev/active/af4c901a-derive-default-rules-at-load.md` | `dev/archive/6eb585bc-core-maintenance/active/af4c901a-derive-default-rules-at-load.md` | `6eb585bc` |
| 1098:4 | `dev/active/d74a9ed1-write-through-namespace-unique-membership.md` | `dev/archive/6eb585bc-core-maintenance/active/d74a9ed1-write-through-namespace-unique-membership.md` | `6eb585bc` |
| 1100:4 | `dev/active/450db193-generic-projection-design.md` | `dev/archive/6eb585bc-core-maintenance/active/450db193-generic-projection-design.md` | `6eb585bc` |
| 1102:4, 1103:4 | `dev/active/9b7b5f9c-plan.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-plan.md` | `9b7b5f9c` |
| 1105:4 | `dev/active/9b7b5f9c-mvp-scope-brief.md` | `dev/archive/9b7b5f9c-jit-profiles/active/9b7b5f9c-mvp-scope-brief.md` | `9b7b5f9c` |

### `dev/archive/7d3a3a47/active/7d3a3a47-investigation.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 43:247, 43:376, 138:173 | `dev/active/5c060496-raw-assets-design.md` | `dev/archive/94f873c8-docs-lifecycle-p2/active/5c060496-raw-assets-design.md` | `94f873c8` |
| 46:119, 49:158 | `dev/active/documentation-lifecycle-phase2-design.md` | `dev/archive/94f873c8-docs-lifecycle-p2/active/documentation-lifecycle-phase2-design.md` | `94f873c8` |

### `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 180:53, 196:4 | `dev/active/90a2dbfd-kinds-over-sources.md` | `dev/archive/90a2dbfd-item-sources/active/90a2dbfd-kinds-over-sources.md` | `90a2dbfd` |
| 180:7, 213:4 | `dev/active/21558ace-invariants-registry.md` | `dev/archive/25064508-structured-knowledge/active/21558ace-invariants-registry.md` | `25064508` |

### `dev/archive/4a00b2b0-agent-validation/studies/ai-tool-worktree-compatibility.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 81:66, 210:37, 226:39 | `dev/design/worktree-parallel-work.md` | `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` | `ad601a15` |
| 229:39 | `dev/experiments/worktree-manual-coordination-experiment.md` | `dev/archive/ad601a15-parallel-work/experiments/worktree-manual-coordination-experiment.md` | `ad601a15` |

### `dev/archive/features/2821e177/completion-report.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 31:139, 83:25 | `dev/active/637764ef-acceptance-evidence.md` | `dev/archive/2821e177-addressing-v2/active/637764ef-acceptance-evidence.md` | `2821e177` |
| 81:18 | `dev/studies/addressing-v2-rule-gate-items.md` | `dev/archive/2821e177-addressing-v2/studies/addressing-v2-rule-gate-items.md` | `2821e177` |
| 82:26 | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `2821e177` |

### `dev/archive/2d109173-handoff.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 28:305, 65:63 | `dev/active/d24008f0-req01-evidence.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/d24008f0-req01-evidence.md` | `2d109173` |
| 65:22 | `dev/active/99f4a2b4-req02-evidence.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/99f4a2b4-req02-evidence.md` | `2d109173` |

### `dev/archive/2d109173-investigation.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 192:106, 215:6 | `dev/active/76cb968b-citation-check.md` | `dev/archive/6eb585bc-core-maintenance/active/76cb968b-citation-check.md` | `6eb585bc` |
| 216:6 | `dev/sessions/session-2024-12-24-check-links-incomplete.md` | `dev/archive/71373e37-docs-lifecycle/sessions/session-2024-12-24-check-links-incomplete.md` | `71373e37` |

### `dev/archive/6eb585bc-handoff-4.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 94:119 | `dev/active/d74a9ed1-write-through-namespace-unique-membership.md` | `dev/archive/6eb585bc-core-maintenance/active/d74a9ed1-write-through-namespace-unique-membership.md` | `6eb585bc` |
| 94:32 | `dev/active/45a140ae-archived-semantics.md` | `dev/archive/6eb585bc-core-maintenance/active/45a140ae-archived-semantics.md` | `6eb585bc` |
| 94:75 | `dev/active/3e12ffbd-batch-export-design.md` | `dev/archive/6eb585bc-core-maintenance/active/3e12ffbd-batch-export-design.md` | `6eb585bc` |

### `dev/archive/cdc840ad-research.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 931:4, 936:4, 1265:244 | `dev/active/af4c901a-derive-default-rules-at-load.md` | `dev/archive/6eb585bc-core-maintenance/active/af4c901a-derive-default-rules-at-load.md` | `6eb585bc` |

### `dev/archive/features/2821e177/2821e177-handoff-2.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 58:65 | `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md` | `2821e177` |
| 59:18 | `dev/studies/addressing-v2-rule-gate-items.md` | `dev/archive/2821e177-addressing-v2/studies/addressing-v2-rule-gate-items.md` | `2821e177` |
| 59:81 | `dev/active/2821e177-investigation.md` | `dev/archive/2821e177-addressing-v2/active/2821e177-investigation.md` | `2821e177` |

### `dev/archive/1cc809de-completion-report.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 9:217, 98:91 | `dev/studies/cdc840ad-audit-2026-07-23.md` | `dev/archive/1cc809de-repository-state-quality/studies/cdc840ad-audit-2026-07-23.md` | `1cc809de` |

### `dev/archive/2821e177-addressing-v2/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 279:57 | `dev/active/21558ace-invariants-registry.md` | `dev/archive/25064508-structured-knowledge/active/21558ace-invariants-registry.md` | `25064508` |
| 280:4 | `dev/active/90a2dbfd-kinds-over-sources.md` | `dev/archive/90a2dbfd-item-sources/active/90a2dbfd-kinds-over-sources.md` | `90a2dbfd` |

### `dev/archive/2821e177-addressing-v2/active/7f22d6cf-breakdown-spec.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 46:57 | `dev/active/21558ace-invariants-registry.md` | `dev/archive/25064508-structured-knowledge/active/21558ace-invariants-registry.md` | `25064508` |
| 47:4 | `dev/active/90a2dbfd-kinds-over-sources.md` | `dev/archive/90a2dbfd-item-sources/active/90a2dbfd-kinds-over-sources.md` | `90a2dbfd` |

### `dev/archive/2d109173-handoff-3.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 46:25 | `dev/active/36d5451e-review-round-6.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-review-round-6.md` | `2d109173` |
| 47:24 | `dev/active/36d5451e-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-audit-notes.md` | `2d109173` |

### `dev/archive/6eb585bc-handoff-2.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 10:108, 46:10 | `dev/active/73482aa1-completion-report.md` | `dev/archive/6eb585bc-core-maintenance/active/73482aa1-completion-report.md` | `6eb585bc` |

### `dev/archive/6eb585bc-progress.json`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 55:274 | `dev/active/73482aa1-completion-report.md` | `dev/archive/6eb585bc-core-maintenance/active/73482aa1-completion-report.md` | `6eb585bc` |
| 254:210 | `dev/active/d74a9ed1-write-through-namespace-unique-membership.md` | `dev/archive/6eb585bc-core-maintenance/active/d74a9ed1-write-through-namespace-unique-membership.md` | `6eb585bc` |

### `dev/archive/76cb968b-completion-report.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 25:160 | `dev/active/76cb968b-sweep-table.md` | `dev/archive/6eb585bc-core-maintenance/active/76cb968b-sweep-table.md` | `6eb585bc` |
| 28:78 | `dev/active/76cb968b-citation-check.md` | `dev/archive/6eb585bc-core-maintenance/active/76cb968b-citation-check.md` | `6eb585bc` |

### `dev/archive/9ac9fdac-handoff.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 52:52 | `dev/sessions/session-20260622-planning-failures-and-churn.md` | `dev/archive/f2532a2d-jit-project-lead/sessions/session-20260622-planning-failures-and-churn.md` | `f2532a2d` |
| 62:49 | `dev/active/9ac9fdac-828a-40f2-9d73-71af14f44ff8-plan.md` | `dev/archive/9ac9fdac-graph-templates/active/9ac9fdac-828a-40f2-9d73-71af14f44ff8-plan.md` | `9ac9fdac` |

### `dev/archive/cdc840ad-handoff-12.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 62:111 | `dev/active/d74a9ed1-write-through-namespace-unique-membership.md` | `dev/archive/6eb585bc-core-maintenance/active/d74a9ed1-write-through-namespace-unique-membership.md` | `6eb585bc` |
| 62:56 | `dev/active/af4c901a-derive-default-rules-at-load.md` | `dev/archive/6eb585bc-core-maintenance/active/af4c901a-derive-default-rules-at-load.md` | `6eb585bc` |

### `dev/archive/features/dbe1e821/dbe1e821-progress.json`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 177:626 | `dev/active/planning-bracket-design.md` | `dev/archive/2fbd2a82-planning-bracket/active/planning-bracket-design.md` | `2fbd2a82` |
| 177:669 | `dev/design/worktree-parallel-work.md` | `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` | `ad601a15` |

### `dev/archive/1cc809de-handoff-2.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 60:11 | `dev/studies/cdc840ad-audit-2026-07-23.md` | `dev/archive/1cc809de-repository-state-quality/studies/cdc840ad-audit-2026-07-23.md` | `1cc809de` |

### `dev/archive/1cc809de-handoff-3.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 67:11 | `dev/studies/cdc840ad-audit-2026-07-23.md` | `dev/archive/1cc809de-repository-state-quality/studies/cdc840ad-audit-2026-07-23.md` | `1cc809de` |

### `dev/archive/1cc809de-handoff-4.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 59:29 | `dev/studies/cdc840ad-audit-2026-07-23.md` | `dev/archive/1cc809de-repository-state-quality/studies/cdc840ad-audit-2026-07-23.md` | `1cc809de` |

### `dev/archive/1cc809de-handoff.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 64:43 | `dev/studies/cdc840ad-audit-2026-07-23.md` | `dev/archive/1cc809de-repository-state-quality/studies/cdc840ad-audit-2026-07-23.md` | `1cc809de` |

### `dev/archive/2821e177-addressing-v2/studies/addressing-v2-rule-gate-items.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 208:32 | `dev/active/90a2dbfd-kinds-over-sources.md` | `dev/archive/90a2dbfd-item-sources/active/90a2dbfd-kinds-over-sources.md` | `90a2dbfd` |

### `dev/archive/2d109173-docs-exhaustive-audit/active/4c33d0e5-audit-notes.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 145:4 | `dev/design/worktree-parallel-work.md` | `dev/archive/ad601a15-parallel-work/design/worktree-parallel-work.md` | `ad601a15` |

### `dev/archive/2d109173-docs-exhaustive-audit/active/a70bac75-audit-notes.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 102:38 | `dev/active/planning-bracket-design.md` | `dev/archive/2fbd2a82-planning-bracket/active/planning-bracket-design.md` | `2fbd2a82` |

### `dev/archive/2d109173-handoff-2.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 50:17 | `dev/active/36d5451e-audit-notes.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/36d5451e-audit-notes.md` | `2d109173` |

### `dev/archive/2d109173-handoff-4.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 56:21 | `dev/active/6d82de03-followup-manifest.md` | `dev/archive/2d109173-docs-exhaustive-audit/active/6d82de03-followup-manifest.md` | `2d109173` |

### `dev/archive/6eb585bc-handoff-3.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 72:47 | `dev/active/45a140ae-archived-semantics.md` | `dev/archive/6eb585bc-core-maintenance/active/45a140ae-archived-semantics.md` | `6eb585bc` |

### `dev/archive/76cb968b-progress.json`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 4:139 | `dev/active/76cb968b-ssot-adoption-sweep.md` | `dev/archive/6eb585bc-core-maintenance/active/76cb968b-ssot-adoption-sweep.md` | `6eb585bc` |

### `dev/archive/b4e55aa2-completion-report.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 36:214 | `dev/active/b4e55aa2-code-review-live-verification.md` | `dev/archive/6eb585bc-core-maintenance/active/b4e55aa2-code-review-live-verification.md` | `6eb585bc` |

### `dev/archive/b4e55aa2-handoff.md`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 53:17 | `dev/active/b4e55aa2-ground-code-review-policy.md` | `dev/archive/6eb585bc-core-maintenance/active/b4e55aa2-ground-code-review-policy.md` | `6eb585bc` |

### `dev/archive/f2532a2d-progress.json`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 6:177 | `dev/active/f2532a2d-138d-46bf-9547-e1ea517f0170-plan.md` | `dev/archive/f2532a2d-jit-project-lead/active/f2532a2d-138d-46bf-9547-e1ea517f0170-plan.md` | `f2532a2d` |

### `dev/archive/features/2821e177/2821e177-progress.json`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 192:59 | `dev/active/637764ef-acceptance-evidence.md` | `dev/archive/2821e177-addressing-v2/active/637764ef-acceptance-evidence.md` | `2821e177` |

### `dev/archive/features/2821e177/showcase/talk.html`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 226:52 | `dev/studies/addressing-v2-rule-gate-items.md` | `dev/archive/2821e177-addressing-v2/studies/addressing-v2-rule-gate-items.md` | `2821e177` |

### `dev/archive/features/2fbd2a82-progress.json`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 6:18 | `dev/active/planning-bracket-design.md` | `dev/archive/2fbd2a82-planning-bracket/active/planning-bracket-design.md` | `2fbd2a82` |

### `dev/archive/features/53e3fa36-progress.json`

| coordinates | cited artifact (as written) | artifact now at | run |
|---|---|---|---|
| 9:18 | `dev/active/53e3fa36-c0cb-4206-8e8e-0a21aafb213e-plan.md` | `dev/archive/53e3fa36-agent-ergonomics/active/53e3fa36-c0cb-4206-8e8e-0a21aafb213e-plan.md` | `53e3fa36` |

