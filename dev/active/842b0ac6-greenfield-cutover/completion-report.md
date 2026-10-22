## Epic Complete: Remove dead code and compatibility shims across the workspace (842b0ac6)

**Started:** 2026-10-21
**Completed:** 2026-10-22
**Assignee:** agent:jit-execution-lead

### Summary

Removed unused APIs, obsolete configuration and wire forms, and completed cutover shims across the Rust workspace and web UI. Surviving configuration, parsing, rendering, and import paths now have one owner.

### Metrics

| Metric | Value |
|---|---|
| Implementation tasks completed | 40 / 40 |
| Planning and breakdown nodes | 2 / 2 |
| Execution waves | 9 |
| Rework cycles | 18 in recorded counters, including the final criterion reconciliation; prior reset history is preserved in progress.json |
| Escalations | 4 recorded |
| Sub-agent dispatches | Inherited total not recorded; this resumption used four audit/rework turns |
| Tasks added during execution | 2 |

### Success Criteria

The issue owns the criterion text; the table records delivery attribution. The lead independently inspected the resulting implementation and reconciled every recorded pitfall against these criteria.

| Criterion | Delivering tasks |
|---|---|
| @/issue/842b0ac6/requirement/REQ-01 | 866bdde0, 84745746, e1af39db |
| @/issue/842b0ac6/requirement/REQ-02 | f11514dd |
| @/issue/842b0ac6/requirement/REQ-03 | af9a5509, 459bb995 |
| @/issue/842b0ac6/requirement/REQ-04 | 5e846743, cd7b6c4a, fbb661f2 |
| @/issue/842b0ac6/requirement/REQ-05 | 5e846743 |
| @/issue/842b0ac6/requirement/REQ-06 | 756ec5ec |
| @/issue/842b0ac6/requirement/REQ-07 | 4251137e |
| @/issue/842b0ac6/requirement/REQ-08 | 47a082ef, 2492ab29 |
| @/issue/842b0ac6/requirement/REQ-09 | 66bd4b5c |
| @/issue/842b0ac6/requirement/REQ-10 | d125490a, fe761000 |
| @/issue/842b0ac6/requirement/REQ-11 | 9aa54649 |
| @/issue/842b0ac6/requirement/REQ-12 | bc0bd663 |
| @/issue/842b0ac6/requirement/REQ-13 | afc2b698 |
| @/issue/842b0ac6/requirement/REQ-14 | b4929342, 63c11d08 |
| @/issue/842b0ac6/requirement/REQ-15 | 0ec8b001 |
| @/issue/842b0ac6/requirement/REQ-16 | 72d4990c, c998dfd3, 71b0af0e, 81e7cdc3, 58282882 |
| @/issue/842b0ac6/requirement/REQ-17 | 4c216a28 |
| @/issue/842b0ac6/requirement/REQ-18 | 52732879 |
| @/issue/842b0ac6/requirement/REQ-19 | d759d606 |
| @/issue/842b0ac6/requirement/REQ-20 | 86d3625d, 287eef78 |
| @/issue/842b0ac6/requirement/REQ-21 | 45b442f3 |
| @/issue/842b0ac6/requirement/REQ-22 | 0e3b69fc |
| @/issue/842b0ac6/requirement/REQ-23 | 8397eb74 |
| @/issue/842b0ac6/requirement/REQ-24 | 69af08af, ec190bd5 |
| @/issue/842b0ac6/requirement/REQ-25 | dc6fac0c |
| @/issue/842b0ac6/requirement/REQ-26 | 34361448 |
| @/issue/842b0ac6/requirement/REQ-27 | ea5f4536 |

### Wave Execution Log

- Wave 1 (1a): 6 implementation tasks; final status recorded in progress.json.
- Wave 2 (1b): 6 implementation tasks; final status recorded in progress.json.
- Wave 3 (1c): 7 implementation tasks; final status recorded in progress.json.
- Wave 4 (1d): 5 implementation tasks; final status recorded in progress.json.
- Wave 5 (2): 8 implementation tasks; final status recorded in progress.json.
- Wave 6 (3): 5 implementation tasks; final status recorded in progress.json.
- Wave 7 (4): 1 implementation tasks; final status recorded in progress.json.
- Wave 8 (5): 1 implementation tasks; final status recorded in progress.json.
- Wave 9 (6): 1 implementation tasks; final status recorded in progress.json.

### Key Decisions

- Preserved the private domain type facade and routed other domain items through their owning modules. Removed the remaining profile/output aliases found during the final review.
- Regenerated the event reference from its authoritative renderer after the path change; no generated content was edited by hand.
- Preserved unrelated worktrees. The interrupted 9cc3056a passing gate evidence was committed separately at ee3fcbc84 with its state unchanged.

### Escalations

- 66bd4b5c: Steward approved option 1: counter reset once, sonnet rework with exhaustive case-insensitive sweep recorded in sweep-66bd4b5c.md; escalate before reworking if round 4 fails on this class.
- 34361448: Option 1: accept 93a6be35d; add task ea5f4536 (REQ-27).
- 63c11d08: Steward authorized canonical domain::list_envelope ownership, preserving wire/schema behavior; amend plan, manifest, f7f01654 criterion and 650e6a56 description without reopening completed tasks.
- 842b0ac6: Steward explicitly approved option 1 on 2026-10-22: reconcile contradictory naming prose with approved DEC-drift-name and completed e1af39db. Delete the original unused strict implementation, retain the sole production implementation under enforcement_drift, preserve all behavior and other REQ-01 clauses. Re-run holistic-review; no manual gate pass.

### Issues Discovered During Execution

- 34361448: Steward-ruled scope addition REQ-26 (2026-10-21); depends on state-config-defaults-once and external bedf0962.
- ea5f4536: Steward-ruled REQ-27 (2026-10-22): after REQ-26, user/system config.toml layers had no reader.

### Holistic Quality Notes

- The final static audit covered all 27 hard criteria. Its remaining duplicate-path finding and both stale-comment advisories were resolved before final review.
- The approved cross-epic amendment changed only ListEnvelope ownership paths in the df129d9e plan, manifest, and f7f01654/650e6a56 issue descriptions. Their completed states and wire/schema contracts were preserved.
- Earlier host-load and fixture-timeout observations remain in progress.json as execution history. The final Rust run passed 4,588 tests and 58 doctests with no skipped/ignored tests; the measured suite clock was 18.434 seconds.
- Formatting, zero-warning Clippy, and build-budget checks passed in cargo-ci run 788882e7-e067-4b26-96bc-76afca3628d9 at abbb84126.
- Final task code-review a3462790-a2bf-46bd-afc4-360f4d392c5e passed with zero findings. Epic repo-validation d63f7806-b535-4fa6-84a2-e11253f815c2 and holistic-review 050581f4-da58-45f9-b5e1-ff3d107a92bf passed. The cutover merged into main at 109e944f6.

The first holistic review (04a46d76) found a conflict between the literal drift-function deletion criterion and the approved DEC-drift-name convergence. The steward approved the precise wording reconciliation at 90506332b, with rationale and prior wording recorded in progress.json; the required independent review was rerun. The original strict wrapper deletion and surviving production implementation are visible in f16fd85bb.
