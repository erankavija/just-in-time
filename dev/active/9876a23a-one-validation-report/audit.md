# Plan audit against 120402067 (execution lead, 2026-11-03)

The plan's graph and most tasks hold. One core contract has a gap against f8a6b298 DEC-19, other work has done or overtaken some tasks, and three tasks add a new type where an existing primitive meets the criterion. Factual Background corrections are listed per task in the dispatch notes. This file lists only the changes that need owner approval.

## Re-measured costs

Release build of 120402067 on a worktree copy of this repository (1,863 issue files), nine interleaved runs, load average 5–7. The timings are medians.

| View | Background (older snapshot) | Now |
|---|---|---|
| `jit validate` | 0.80 s | 4.53 s |
| `jit validate <id>` | 2.36 s | 5.81 s |
| `jit validate --scope <epic>` | — | 1.86 s |
| `jit validate <id> --explain` | — | 0.155 s |
| `issue create` human / `--quiet` (one run) | 373 / 55 ms | 241 / 107 ms |

Whole validation now runs record fidelity over the whole event log and every gate run (5b0e3d63, DEC-19). REQ-08 still fails: per-issue is 1.28 times whole.

## Finding 1 — record fidelity has no home in the report contract (blocks the core)

- **Today.** Record fidelity reads the whole event log through `RepositoryMutationSession::read_history_log`, outside `CaptureSpec`. Whole `jit validate` and the `repository_validation` checker run it (`validate.rs:968`, `gate_check.rs:1083`). It reaches the report as a `Result<()>` input (`repository.rs:204-209`), and document-reference resolution reaches it the same way.
- **Contract gap.** `validation-report` (`ReportInputs` = instant + claims) and 6ba4f623 REQ-01 delete that path and name neither input. `selection-membership` never mentions fidelity.
- **Criteria conflicts:**
  - Epic REQ-02 says no view reads state outside the one captured image.
  - Epic REQ-10 says each issue file is parsed once, but `record_fidelity::issue_drift` re-parses every issue.
  - Story 420bc076 REQ-04 says the same as epic REQ-10.
  - a106d1be REQ-01 says each checker computes one report over its image.
  - 2c8c13b2 makes `run_rules(None)` the Whole selection, so it would become a history reader or a second, different "Whole".
- **Proposed amendment.**
  - The report takes the history verdicts (record fidelity, document references) as optional captured inputs. Only DEC-19's two readers supply them, and Whole is admitted only from a capture that carries them.
  - Issue, scope, rule and explain views, and the issue, rule and label-target checkers, capture no history, and each view's criteria say so.
  - Epic REQ-02 becomes "one capture per view; only whole `jit validate` and `repository_validation` read history (`@/charter/D-19` reader list)".
  - REQ-10: fidelity consumes the loader's parsed records plus the raw bytes it compares, so the per-issue parse happens once.

## Finding 2 — tasks done or overtaken

- **977d01d8 and 145b4d57.** Both are mostly done: the `JitConfig::content_format`/`strictness` accessors exist (`config.rs:258,264`) and `repository.rs` uses them. What remains are two small wrappers (`projection_render.rs:244`, `commands/mod.rs:2723`) and the accessor tests. Proposal: merge them into 977d01d8 and reject 145b4d57.
- **e16493fc.** Its REQ-01 premise is gone. a04207965 deliberately gives each document reference its own spec (test `validate.rs:3629`). Its REQ-02 (apply `base(spec)` once per round) still applies. Proposal: fold REQ-02 into 2f12d920 and reject e16493fc. Epic REQ-09's batch clause then becomes "capture applies its base requests once per round".
- **1f24e3e9.** 8ffd504de already makes a zero-fix `--fix` fail on an invalid repository, so the REQ-03 test "that fails today" passes today. Proposal: REQ-03 asserts the remaining change instead: the post-fix report is rendered in human and JSON output.
- **b3a768b3.** The index holds only tombstones now (c47d78c8), and membership comes from the listing. Proposal: drop "parses the index" and "index and record disagreements" from REQ-01/REQ-03.
- **03c59566.** REQ-04 cites `tests/fast_rules/cli_warnings_integration_tests.rs`, which c904cb0b9 deleted. Proposal: point it at the current owner.

## Finding 3 — net-line alternatives (the user's goal is net reduction)

| Task | As written | Alternative | Δ lines |
|---|---|---|---|
| ccd6e1fb | New compiled-rule-set type | One shared `SchemaEngine` (it already caches by key, `engine.rs:119`) passed across issues | +40 → +10 |
| ea843d26 | Container-id index type in `GraphEvaluationContext` | Allocation-free `IssueIdPrefix::matches`, which also fixes `label_credits_id` | +40 → +5 |
| 5d5a1794 | A sixth copy of the plan-content plus graph-evaluation block | One captured-graph-evaluation helper replacing the five copies (`mod.rs:1226,2769,3173`, `dependency.rs:382`, `bulk_update.rs:377`) | +45 → negative |
| f968dfe1 | One shared plan reference | Also merge the two near-identical plan-content readers (`validate.rs:1032,1055`) | −25 → −50 |

The task bodies mandate the new types in ccd6e1fb and ea843d26, so taking the alternatives changes their criteria. Epic REQ-07 and REQ-11 still hold under them.

## Finding 4 — missing removals

- **6ac41880.** Also delete `graph::evaluate_graph_scoped` (`graph.rs:354`): no caller remains after 208edf4c and a106d1be, and no task removes it.
- **3acaa5a3.** Also delete the gates `SourceState::Unloadable` arm (`drift.rs`), which becomes dead. Its edge on 612ee9d4 can go, since 6ba4f623 suffices.
- **2c8c13b2.** Delete the `run_rules(None)` arm, which has no production caller (`main.rs:6004`), and point its test callers at the report.
- **a106d1be / `checker-selections`.** At HEAD a missing or repeated target label is an `error` run, not a finding (`gate_check.rs:659-667`). The contract and the Notes should say so.

## Finding 5 — cross-epic order

| Pair | Conflict | Proposal |
|---|---|---|
| 03c59566 / 3f816619 (df129d9e) | Both rewrite the `issue create` arm. 3f816619 REQ-06 keeps payloads unchanged, but ours adds `findings`. | No edge; the second lander adapts (agreed with df129d9e's lead). |
| 1f24e3e9 / 06ee9b67 (df129d9e) | Both rewrite `--fix` rendering (`main.rs:30,6129`). | Same. |
| f22147ec / 578ed10d (b697a3f7) | 578ed10d REQ-01 moves `capture_validation_image_with`, which f22147ec deletes. | Edge 578ed10d → f22147ec; b697a3f7 is still being planned. |
| cf49da17 / 725af076 (f8a6b298) | 725af076 rewrites the test-image helpers that cf49da17 deletes. | Optional reverse edge (cf49da17 first) to avoid churn. |
| DEC-09 claims bridge | Its remover is now f8a6b298's 53d19851 (lease-index-machinery-removal) REQ-04, which lands late. | Keep the bridge; 53d19851's brief names the field (agreed with f8a6b298's lead). This is a factual correction. |

## Estimated net production lines (as written → with Finding 3)

The production baseline at 120402067 is 73,858 (`scripts/production-lines.py`).

| Story | As written | With Finding 3 |
|---|---|---|
| report-core 420bc076 | −60 | −140 |
| report-views b3ef68b5 | −600 | −600 |
| side-paths 91edfa5c | −245 | −300 |
| **Epic total** | **≈ −900** | **≈ −1,050** |

## Unaffected and ready now

d40d6f74, 7eb443b2, bd2534bd, 6100b960 and c92f4011 need only Background corrections and touch nothing in Finding 1.
