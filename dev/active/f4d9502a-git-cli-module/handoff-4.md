# Handoff — Git CLI module (f4d9502a) — session 4

**Date:** 2026-11-05  
**Session number:** 4  
**Prior handoffs:** [handoff.md](handoff.md), [handoff-2.md](handoff-2.md), [handoff-3.md](handoff-3.md). Live checkpoint; work continues.

## Current state

- Epic f4d9502a remains backlog, wave6 of10; no issue closed since handoff3. Execution wave entries remain37 Done,3 in_progress,11 backlog; bracket/container records are separate. Lead scope is this epic plus reconciled external504 only.
- Main clean/free at5fa92e83d before this checkpoint; no source slot reserved.504 isolated HEAD376978ed05c011717ab7c2d88fab54954f045f15 plus exactly ten dirty source/test files,673insertions/171deletions. Binary diff SHA256 f0c5fd8a3e18932804cd4f02b412b9d1540321d408522733e66601a7ce22b9a3, independently verified by lead. Preserve this partial implementation.
- All commands collected; no active build. `/root/split_root_impl` Astra/xhigh returned partial and now does read-only/proposal follow-up: concretize selectedA and reproduce actor failures on unchanged main; no source edits. `/root/split_root_diagnostics_review` Astra/high is idle after bounded design R2 PASS. No implementation review has started.
- 504 cycle3 was explicitly authorized as ONE additional bounded cycle, without resetting the prior two failures. Steward chose boundary optionA and the in-footprint Io fix, with13files subject to fresh independent bounded design review before edits. Source remains frozen; actor work is separate. No new configured full Cargo has run.
- Active leases/assignments a273667c,c140921c,5046408a remain agent:f4d9502a-lead. Actual renew at06:31UTC extends all three to about10:31UTC. Read live expiry before long work; acquire did not renew existing leases.
- Other holds unchanged: explicit user Git>=2.42/runtimeTrixie decision for a273/c140, plus separate06177584/38df8a6c. Never land excluded ancestors d0c98247f or7701663df. Main source queue is ready-only.
- Authority: [progress.json](progress.json), live tracker and steward thread01a1a886-d199-7093-b096-7acbab6adb48.

## What just happened

- Primary proposed one canonical parsed config document with consumed typed bootstrap projections and always-data-only config validation. Independent diagnostics R1 FAIL found omitted existing storage test owner; proposal corrected from9to10files, still only3new maintained owners. R2 PASS closed the finding.
- Steward accepted exact semantics/matrix/footprint and only cycle3. Durable linked proposal, both reviews and amended504 design committed356f7596a, reference refresh5fa92e83d; native validation0errors/warnings. Original proposal/reviews retain their submission status, explicitly superseded by maintained acceptance.
- Cycle3 implements ten authorized files; original project-config report/repair and exit assertions unchanged. Data-only capture admits index and all declaration sources together; one report computation/registered findings outcome; storage observation assertions mechanically migrated without weakening absence/publication/admission.
- Actual RED preserved, including new unnecessary Git-discovery failure. A new test initially expected a nonexistent JSON data wrapper; only that new assertion corrected to existing top-level contract before genuine RED. Compile failures unused import/dead loader, then7existing loader-test calls, preserved; production loader retained over shared admission/validator, original tests unchanged.
- Collected passes: declarations108, storage512, config-related183 (overlaps), bootstrap13, unchanged exit projection1, project-config20; fmt and workspace/all-target Clippy zero warnings. No fresh broad library/doctest rerun solely for freshness.
- Full non-fail-fast CLI/contracts FAIL: CLI1222pass/3fail; contracts548pass/2fail. No skipped target. CLI permission errno13 classified1instead5; corpus item list PARSE_ERROR insteadINVALID_ARGUMENT; worktree exec pending redo12instead1. Contracts two release tests have no acting identity.
- Exhaustive registry sweep of all9malformed-config/pending-redo arms finds6matching and3mismatching: item list/search expectedINVALID_ARGUMENT and recover expectedrecovery_failed, allpreempted by bootstrapPARSE_ERROR. Separate11command retryable-read projection passes both output forms. No corpus sibling hidden by its first assertion is omitted.
- Primary returned result with40cumulative rows and concrete boundary proposal. Lead read both fully and sent steward recommendationA plus explicit actor scope. No assertion or source changed after final batch; no ambient identity supplied.

## What to do next

- [ ] Collect revised concrete optionA proposal from primary and dispatch fresh Astra/high bounded independent design review before edits, especially initialized-store/pending ordering, lock cleanup, child cwd/stdio/status and D25. Steward selectedA and13files, conditional on this review; no B/registry cutover or retryreset.
- [ ] SelectedA preserves item/recovery typed adapters at early authored failure and factors worktree exec to a data-only command entry preserving under-lock pending refusal, initialized clean-store checks, child streams/status/cwd, no replay or fake root. Additional owners commands/mod.rs, commands/worktree.rs, dispatch/worktree.rs. AlternateB intentionally changes3registry codes plus exec assertions/docs and requires explicit contract-cutover approval.
- [ ] Actor concern is explicitly separate from504. Primary must reproduce both failures on unchanged main under same sanitized inputs and audit existing injection. If reproduced, prepare minimal task/criteria/four-file explicitAssignee-input proposal (commands/mod.rs, commands/issue.rs and twofast_issue tests), sameGit epic outside original53/noREQcredit, mainfix then504refresh; obtain approval before creation/dispatch. No ambient workaround or test-only actor provider. No baseline result yet.
- [ ] Permission fix already fits ten-file scope: preserve concrete std::io::Error source in RepositoryStateStoreError::Io and add owning source-chain assertion; keep unchanged permission projection. It remains unedited while combined source is frozen.
- [ ] Apply any required independent bounded review and maintained acceptance before resumed implementation. After all focused failures close, freeze/commit, refresh main preserving source, force actual configured Cargo. Only after PASS run fresh independent Astra/high IMPLEMENTATION review and unchanged configured code-review, both mandatory.
- [ ] Ready-only landing, forced actual merged Cargo, then immediate closure/commit/release/notify before ancillary work. Any new full-gate failure returns to steward. Never infer user floor decision; c140 waits floorapproval+a273Done+504Done.

## Traps — do not repeat these

- **`claim acquire` did not extend existing expiry.** Live list showeda273expiring06:33despite earlier acquire calls. Actual `claim renew --extension14400` verified10:31expiry. Use scoped help/current response; handoff3 summary renewal assumption was wrong.
- **Full non-fail-fast targets still contain aggregate tests that stop at first assertion.** Failure-envelope corpus first exposed item list only; registry sweep proved item search and recover too. Enumerate every relevant registry arm before proposing one fix.
- **New shared bootstrap changes failure phase.** Required-root parse and pending observation can preempt established command adapters; do not force numeric exits, catch strings, or weaken assertions silently. Concrete options now await steward decision.
- **Do not mask release-test identity with ambient JIT_AGENT_ID.** Tests need explicit attributable actor while preserving no-coordinator property. Original full gates export a lead actor; focused contract run exposed this latent independence concern. Baseline execution has not been performed, so do not claim a proven pre-existing runtime failure.
- **Do not delete a private loader solely because production use moved.** First cleanup broke7existing test calls. Final production load_index reuses shared data-only admission/validator and retains assertions; no test-only compatibility copy.
- **Progress JSON uses ASCII escaping, unlike the manifest.** ensure_ascii=False caused unrelated escaped-Unicode churn; corrected before356f7596a. Preserve existing format. Recover bounded relevant output whenever a read is truncated.
- **All earlier traps remain binding.** Read prior handoffs for no nested tracker in worktree exec, locks/shortTMPDIR/cache rules, actual --force gates, no idle source reservations, data/code authority and unchanged held proposals.

## Open questions needing invoker input

- 504 selectedA exact13-file boundary correction requires fresh independent bounded review before source edits. Primary concretizes ordering details now; steward choiceA is authoritative, not the older proposal's open alternatives. Actor concern requires main reproduction and separate task proposal/approval; it is not authorized inside504.
- User Git-floor/runtime approval remains outstanding through steward. Other holds unchanged; do not ask again or infer answer.

## Reference artefacts

- [504 maintained design](../5046408a/design.md), [diagnostic proposal](../5046408a/diagnostics-proposal.md), [R1](../5046408a/diagnostics-design-review-r1.md), [R2](../5046408a/diagnostics-design-review-r2.md).
- `/tmp/f4d9502a-lead/5046408a-rework-3-result.md` (complete40-row result), `5046408a-rework-3-boundary-proposal.md`, `5046408a-rework-3-full-boundary-registry.json`, subset repro JSON and all command logs. Current dispatch5046408a-rework-3.md retains original brief.
- Lead review scratch file records both full-gate failures and current partial status. Independent implementation brief updated for accepted diagnostics but must receive final new boundary ruling/evidence before dispatch.
- Full gate helpercargo-5046408a.sh, configuredreview helperreview-5046408a.sh and mergedhelpermerged-5046408a.sh remain available; do not run before proper phase. Previous fullgate35fd9abc andf604a1fc failures preserved.
- Contact tools, scripts, a273/c140 frozen hashes and excluded sources are in handoff2/3. No goal was created.
