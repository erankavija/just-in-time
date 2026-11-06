# Handoff — Git CLI module (f4d9502a) — session 3

**Date:** 2026-11-05  
**Session number:** 3  
**Prior handoffs:** [handoff.md](handoff.md), [handoff-2.md](handoff-2.md). Live continuation checkpoint; work remains active.

## Current state

- Epic f4d9502a remains backlog, wave6 of10. The51 entries in execution waves currently comprise37 Done,3 in_progress and11 backlog; bracket/container records lie outside that count. No issue closed in this checkpoint phase.
- Active assignments remain a273667c, c140921c and5046408a to agent:f4d9502a-lead. Claimed-at timestamps:02:33,03:58 and04:27 UTC respectively; advisory leases were renewed separately as prior handoff records. Do not reclaim held worktrees.
- Main clean/free at dfac58b19 before this checkpoint; no source slot reserved, no active root build/session.504 isolated source HEAD376978ed0, clean. Source implementation0ecd649ce plus reworkccc0d3cea remains unlanded.
- Primary `/root/split_root_impl` Astra/xhigh is preparing a bounded diagnostic/repair bootstrap design proposal only. All source edits are held pending independent bounded design review and steward acceptance. Consumer/docs workers are complete; primary owns all35 accepted implementation files.
- 504 rework count2 has reached the skill limit. Steward explicitly requires design acceptance before further edits. Implementation review has NOT started because full Cargo has not passed.
- Open escalations:504 diagnostic/repair design correction; explicit user Git>=2.42/Docker Trixie decision still pending for a273/c140.06177584 and38df8a6c remain separate user holds. Excluded commits d0c98247f and7701663df must never become landing ancestors.
- Authority: [progress.json](progress.json), current tracker and steward messages. Single epic scope plus reconciled external504 only.

## What just happened

- 504 primary completed35 approved files, source0ecd649ce. Refreshed36f2ea3c1 with exact byte preservation. Focused storage509, bootstrap11, consumer/server tests, fmt and all-target Clippy passed. Reports and RED/failure history remain in `/tmp/f4d9502a-lead/5046408a-implementation-result.md` and consumer/docs reports.
- First actual full Cargo35fd9abc on36f2ea3c1 FAILED:191 tests passed,4 failed before fail-fast;22 doctests passed,3 failed. Fmt/clippy/build/budget passed. Shared ConfigurationObservation required an existing immediate parent, breaking existing in-memory/synthetic-layout config defaults. Evidence committed2fb068c61.
- Rework1 fixed the shared DataReadSession: retain requested data path, observe first missing component with existing-parent identity, reject ancestor contents during publication race, re-observe requested path for admission. Removed exact-store private absence fallback. Existing failing tests/doctests unchanged. Three files only:storage/json.rs,repository_state_store.rs,and its existing tests.
- Rework1 actual RED and final8 focused cases passed; broad non-fail-fast libraries3192JIT+127server and25doctests passed, fmt/all-target Clippy passed. Sourceccc0d3cea, refreshed70e934693 preserving35 source files. Report `5046408a-rework-1-result.md` has21-row cumulative resolution table.
- Second actual full Cargof604a1fc on70e934693 FAILED:3386 tests passed,1 failed before fail-fast; all25doctests/fmt/clippy/build/budget passed. Existing malformed-config `config validate` projection expects findings exit4; eager bootstrap full assembly emits ordinary parse exit1 first. Evidence committed376978ed0.
- Rework2 diagnosis also reproduced existing config repair failure: `config set project.name` cannot repair an invalid project name; project_config tests10pass/4fail including diagnostic report shapes. No source edits since second gate. An exit-only patch is insufficient.
- Steward requires concrete alternatives/footprint covering malformed TOML, unrelated semantic invalidity, invalid root/policy, env override, absent config, pending redo and data-only diagnostics; one canonical syntax parser, bootstrap's required projection only, no fabricated diagnostic code root, no mutation-policy bypass. Independent bounded design review precedes acceptance and source changes. declarations/mod.rs was already named in accepted step3; verify exact additional owners rather than overstating scope.
- Steward-approved future-consumer reconciliation527a2dd65: c140 actual dependency on504 added with native cycle/redundancy checks. Native reduction removed direct c140→head/layout and348→504;348 still reaches504 through downstream chain.504 remains outside53-node manifest; internal contract prerequisites remain represented there with explicit external reconciliation inDEC23. c140/d9/933 explicitly use code_checkout for code documents/source metadata. Native plan and tracker validation passed with0warnings. c140 remains held until floor approval+a273Done+504Done.
- Steward-approved obsolete committed-HEAD fallback note removed from504 issue7282e41c2; no criterion weakened and no HEAD tier restored. Progressccae06bea/fa64bf162/dfac58b19 records both gate failures and design hold.

## What to do next

- [ ] Collect primary's concrete bounded design proposal and exact footprint; no dependent source edits. Route to steward thread01a1a886-d199-7093-b096-7acbab6adb48 and dispatch fresh independent Astra/high bounded design review as required. Resolve every finding; get steward design acceptance and rework-limit ruling before implementation resumes.
- [ ] Update maintained504 design/footprint transparently after accepted correction; link durable artifacts. Preserve D25/D26/D27, existing diagnostic JSON/plain report and repair assertions, and original source history.
- [ ] After correction, run relevant config CLI plus wholeCLI/contracts non-fail-fast to expose failures hidden by gate stops, owning tests and fmt/clippy. Do not repeat unchanged broad suites just for freshness. Then lead commits/freezes and reruns full configured Cargo with --force.
- [ ] After actual fullCargoPASS, run BOTH fresh independent Astra/high implementation review and unchanged configured code-review gate. No CI lock during AI reviews. Prepared brief `/tmp/f4d9502a-lead/5046408a-independent-review-brief.md` needs latest correction evidence before dispatch. Do not reuse design review as implementation acceptance.
- [ ] Only fully ready source enters main queue. Canonical land, mandatory forced actual mergedCargo, immediate close/commit/lease release/notify before ancillary work. Source and failure evidence stay isolated until review/gates pass.
- [ ] User Git-floor/runtime decision remains pending through steward; do not infer approval or repeat the user question. Prior handoff contains exact a273 proposal and c140 conditional producer scope. No snapshot edit belongs to504.

## Traps — do not repeat these

- **Full config assembly is not a universal bootstrap prerequisite.** It blocks malformed-config diagnostic output and valid repair of unrelated invalid fields. f604a1fc and primary project_config RED are concrete evidence. Do not fix only exit numbers, swallow strings, invent default code authority, add a private parser or bypass linked policy. Await reviewed bounded design.
- **Missing config can have missing ancestors.** New DataReadSession initially reused mutation-root evidence requiring the immediate parent.35fd9abc failed existing dependency/document workflows and3doctests. Shared read-only absence observation now fixes this; preserve its publication/replacement admission and ancestor-not-data guard.
- **Do not retain parallel absence fallbacks.** The first exact-store special case hid the broader configuration bug. Rework1 removes it and converges on DataReadSession. Actual race RED proves an existing ancestor must never become selected data authority.
- **Focused green is not full gate green.** First full gate stopped at195/5097; second stopped at3387/5099. Never report remaining tests as passed. Preserve failures, use non-fail-fast relevant suites for diagnosis, then run mandatory configured gate unchanged.
- **Canonical fixtures remain mandatory.** Lead found manual Git init/add/commit in new JSON subdirectory wiring test; worker replaced it with shared GitRepository before source commit. Existing projection and repair assertions must remain unchanged in rework.
- **Manifest is a list and uses one-space JSON indent.** A root script assumed dict and failed before edits; a later two-space serialization caused whole-file churn. Corrected to original format; final527a2dd65 has3 description-line changes. Native external504 edges are reconciled inDEC23, not a54th manifest node.
- **Prior handoff traps remain binding.** Read handoff2 for locks, short TMPDIR, cache cleanup, no nested tracker inside worktree exec, --force gate reuse, data guard versus code evidence, source/metadata ownership and ready-only main queue.

## Open questions needing invoker input

- 504 bounded bootstrap diagnostic/repair design and exact additional footprint, followed by rework-limit reset/authorization. Primary is preparing alternatives; steward requires independent review before edits. Recommendation: one canonical parsed declaration representation with required bootstrap projection and honest data-only diagnostic path, subject to concrete design review.
- Explicit user Git>=2.42/runtime Trixie proposal remains pending; frozen a273 decision package unchanged. No answer inferred.

## Reference artefacts

- [504 maintained design](../5046408a/design.md), its R1/R2/R3 review reports; [c140 conditional plan](../c140921c/revision-producer-plan.md).
- `/tmp/f4d9502a-lead/5046408a-rework-2.md` is current bounded diagnosis dispatch. Original implementation/rework1 briefs and reports, RED/failure logs, source-preservation proofs and lead review remain beside it.
- Failed full gate JSON copies: `5046408a-cargo-ci-r1-failed.json`, `5046408a-cargo-ci-r2-failed.json`; both durable gate records preserved on source branch.
- Prepared helpers: `cargo-5046408a.sh` (only full branchCargo), `review-5046408a.sh` (configured code-review, no CI lock), `merged-5046408a.sh` (forced actual mainCargo). Execute only at the proper accepted phase.
- Main coordination, all contact threads, held proposal SHAs and canonical scripts are in handoff2. `jit doc list f4d9502a` locates these checkpoints.
