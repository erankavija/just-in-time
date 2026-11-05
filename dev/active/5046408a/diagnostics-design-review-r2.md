# 5046408a bootstrap diagnostics — independent bounded design review R2

Verdict: **PASS — design review only.** R1 finding 1 is closed by the revised proposal. No new blocking design finding was identified. The proposal is sufficiently specified for steward acceptance; this verdict does not authorize implementation or accept source, tests, gates, or issue completion.

Reviewed the revised `/tmp/f4d9502a-lead/5046408a-bootstrap-diagnostics-proposal.md` and `/tmp/f4d9502a-lead/5046408a-rework-2-result.md`, retaining the original brief's scope and the contracts assessed in R1. Rechecked the affected storage tests, dispatch findings admission and current validation handler against the clean frozen source at `376978ed05c011717ab7c2d88fab54954f045f15`. R1 remains unchanged at `/tmp/f4d9502a-lead/5046408a-diagnostics-design-review-r1.md`; its original FAIL verdict is historical and has not been rewritten.

## Finding closure

1. **R1 F1 — omitted storage test owner: CLOSED at design level.** Proposal lines 69–82 now name ten files, including `crates/jit/src/storage/repository_state_store_tests.rs` at line 76. Line 103 explicitly requires migration of the observation-binding fixture at source lines 3193–3202 and the eager `config.worktree` accesses at lines 3316, 3333 and 3354 to canonical full-view or required-root projection APIs. It preserves the absence, no-write, publication, replacement and admission assertions in their current owner. Eager compatibility fields, partial/default `JitConfig` substitutes, test-only representations and moving/copying tests are explicitly prohibited. This directly resolves the representation/footprint conflict without weakening the rework1 safeguards.

## Criterion and contract coverage

| Criterion | R2 assessment |
| --- | --- |
| Canonical parsing and required projections | Preserved. One syntax document supplies full assembly and consumed-field projections. Malformed syntax is not absence; unrelated project/lease errors do not become root-selection requirements. Governed mutations still require valid original policy even with an override. |
| Initialized store and diagnostic coherence | Explicitly strengthened. Index and all declaration bytes belong to the same admitted diagnostic capture; separate successful reads are insufficient. Existing index parsing and data-only admission remain shared owners, with no history scan or fabricated code root. |
| D-25 and missing-ancestor safeguards | Preserved. Pending redo/crossed commits yield bounded conflicts without reader locks/replay. Selected data remains distinct from the ancestor proving absence, and the existing tests retain their assertions. |
| D-26/D-27 and config repair | Preserved. Recovery completes the original record before new root/policy selection. Original recovered declarations authorize repair; the proposed config cannot authorize itself. Edited-document validation and coupled redo publication remain unchanged. |
| Explicit inputs and errors | Preserved and clarified. Invocation policy-token validation remains the outer boundary; report construction consumes captured inputs without rereading environment. After data admission, effective explicit-root failures remain typed and precede authored findings. Storage/capability/index/admission failures are not converted to configuration findings. |
| Full diagnostics and presentation | Explicitly strengthened. The shared computation always assembles sibling template/invariant declarations, including absent config. The registered `config_validate` findings admission remains before the common emitter, matching `dispatch/mod.rs:488–492`; no copied envelope or new leaf is proposed. Existing plain/JSON/exit and repair assertions remain unchanged. |
| Footprint and architecture | Sufficient. Ten files include the affected existing storage test owner. Only `commands/config.rs`, `dispatch/config.rs` and `project_config_tests.rs` are new to the maintained footprint. Declaration purity, storage admission, command orchestration and dispatch presentation retain their boundaries. |
| REQ-01–08 and diagnostic-route choice | R1 coverage remains intact. Ordinary root precedence/default behavior, code/data authority separation and supplied-root errors are preserved. Always-data-only diagnostics remain preferable to conditional fallback because they share one capture/report route regardless of configuration validity. |

## Footprint and test assessment

The revised footprint resolves the only mandatory expansion identified in R1. No new source owner, Cargo target, generated schema, Git policy or Docker change is required by this design. Pure declaration tests own projection/full-assembly semantics; bootstrap tests own root/policy input selection; the existing storage suite owns admission and any necessary index/declaration composition property; CLI tests own routing, precedence and presentation. These assignments preserve `@/inv/orthogonal-tests` and do not duplicate the storage race matrix.

The proposed post-implementation focused tests, full CLI/contracts runs without fail-fast, formatting/Clippy, and subsequent lead-run configured gate remain appropriate. Known report/repair regressions remain unresolved in the frozen source; design PASS is not evidence that they now pass.

## Advisory pre-existing issue

The R1 advisory about `ConfigOps::validate`'s raw config-existence guard and broad load-error conversion is now explicitly addressed in the proposal. Its matrix distinguishes absent config with valid siblings from invalid sibling declarations; typed storage failures remain errors. No separate unresolved advisory finding remains.

## Next decision

Steward acceptance of the revised semantics and ten-file footprint, plus the requested implementation/rework-limit ruling, remain necessary before a new implementation dispatch. No additional design decision is requested by this review.

Only this R2 scratch artifact was written. No source, R1 history or tracker changes; no build, test, gate, recovery, claim, commit or merge was performed.
