# Handoff — Core maintenance (6eb585bc) — session 5

**Date:** 2026-07-17
**Session number:** 5 (full collection, cross-epic integration, review convergence)
**Prior handoffs:** dev/active/6eb585bc-handoff.md … handoff-4.md (their Traps remain in force except where superseded below)

## Current state

- Epic: `6eb585bc` — in_progress; finite v1.0 prerequisite (@/charter/D-14). ALL 16 batch children are merged to main.
- Children: everything is done EXCEPT 8 issues in_progress awaiting the FINAL WAVE results: 45a140ae, 1d59070d, 52665a07, c505031a, 554ad07f, 0283ce74 (doc-review re-runs only — fixes already merged), d74a9ed1 (cargo-ci + code-review of the toml_edit rebuild; doc-review already passed), 450db193 (cargo-ci + code-review + doc-review of the REQ-07/D1-D3 submission).
- FINAL WAVE in flight at handoff: 11 evaluations launched detached at HEAD 46e8c5ae (binary installed commit-matched, dirty=false): d74a9ed1/450db193 cargo-ci, d74a9ed1/450db193 code-review, doc-reviews for the seven listed issues. Script: freshness-batch.sh in this session's scratchpad (gone next session — recipe below); results append to review-pass.log there and, durably, to `.jit/gate-runs` + each issue's recorded gate status. **Next session reads `jit gate status-all <id>` per issue — do not trust the scratchpad to exist.**
- DONE this session (all gates green, transitioned bc9ee111): 8917c558, 3e12ffbd, 0daba57d, 6f881a85, 894337e2, 94096aac. (8d7fc762 + 16402e14 were done in session 4.)
- Codex stream: profile epic 9b7b5f9c COMPLETE and landed (d4c57540 + showcase 487051c0); post-1.0 lifecycle epic c639cfb5 unblocked but not started. Codex has been quiet since.
- Progress file: dev/active/6eb585bc-progress.json (session_5 key + updated rework counts).
- Active claims: epic (agent:jit-execution-lead); agent:worker on the 8 open issues.

## What happened this session

- Collected all five session-4 dead workers: merged 554ad07f, 450db193 (see below), 0283ce74 (+2 lead stale-comment fixes), c505031a (committed its 29-file complete tree), 6f881a85 (its abandoned tree was near-complete; committed + merged).
- **Cross-epic integration (invoker-approved):** 450db193's `[projection.*]` migration collided with codex's profile subsystem (manifest modeled the old projection tables as singleton-table targets; embedded jit-dogfood contributed them). An integrator adapted the manifest to named projection contributions (`kind = "projection"`, `name = "<n>"`, value carries kind/mode/target/style), planner writes nested `[projection.<name>]`, generalizes managed-target/render hooks; embedded + fixture manifests converted. Landed through three merge waves as codex's epic landed mid-integration; final state on main since fbe2bb36/2abed5ba.
- Landed lead-direct on main: c5330d4f (codex's own broken redirect test — was failing every full-suite run), the 1d59070d attestation fix (via fixer worker, c5e56064), 52665a07 five-finding rework (48c4fd11), 94096aac serve-deadlock fix (56d6463d), 554ad07f TOCTOU stable-pair probe (44ee4258) + unknown-on-instability refinement, six fix-commit windows closing ~25 review findings total (b301eded, 640854b9, 6d4efad1, 6cad67f0 the biggest).
- **Review convergence:** the code-reviewer surfaces one finding subset per round. Two loops were broken structurally: (1) 45a140ae literal-terminality prose — whole-surface audit swept ~20 sites across cli.rs/docs to "effectively terminal (done, rejected, or archived from one of those)"; code-review passed round 6. (2) d74a9ed1 TOML hand-scanner — five rounds hit the same root cause; escalated per MAX_SAME_FINDING_REPEATS; invoker approved rebuilding the membership sync on toml_edit (ff365b3a, net −137 lines, all prior regression tests kept green).
- Gate evidence integrity: audited and re-ran every stale pass (recorded commit predating later issue-tied commits). All cargo-ci/docs-mechanical evidence is fresh at merged HEADs.
- Six done-transitions committed (bc9ee111).

## What to do next (session 6)

1. **Collect the final wave**: `jit issue status 45a140ae 1d59070d 52665a07 c505031a 554ad07f 0283ce74 d74a9ed1 450db193`. For every issue with all gates passed: `jit issue update <id> --state done`, batch the commits.
2. **If a doc-review fails again**: findings have been one-liners lately; fix lead-direct, but CHECK THE FAMILY FIRST (see Traps — one-finding-per-round). 45a140ae and d74a9ed1 are at rework count 5: another failure in their SAME family goes to the invoker per escalation policy, not another patch.
3. **If d74a9ed1 code-review flags drop-canonicalization**: the toml_edit rebuild canonicalizes exotic header spellings elsewhere in the file when a drop occurs (semantically lossless; add-only syncs are byte-exact). This is documented, inherent to toml_edit, and accepted by the lead — cite this and the invoker-approved rebuild decision; reword docs if asked, do not revert the design.
4. **Epic completion (Section 10)**: all children done → map success criteria (batch report + usability audit in dev/active/6eb585bc-*), run `jit gate evaluate-all 6eb585bc`, completion report per template, `--state done`, archive progress/handoffs per doc lifecycle, `jit doc add` the report. @/charter/D-14: closing this epic + profiles MVP (already done) gates the production-readiness source freeze — tell the invoker when it closes.
5. Re-evaluation recipe (scratchpad scripts are session-scoped): install from `.agents/worktrees/lead-install-clean` detached at main HEAD, verify `jit --version` commit==HEAD dirty=false, `export CARGO_TARGET_DIR=$HOME/.cache/jit-sweep-target`, pre-clean `find $CARGO_TARGET_DIR -type d -name incremental -exec rm -rf {} +`, then bare `jit gate evaluate <id> <gate>` (bare evaluate auto-re-runs anything not passed at current HEAD; no --force needed).

## Traps — do not repeat these

- **pgrep -f matches its own wrapper** — cost 40 min this session; now a durable memory (`feedback_no_pgrep_self_match_polling.md`). Poll conditions (`jit gate status ... | jq .status`), never process liveness; bracket the first char (`[g]ate evaluate`) when pattern-matching is unavoidable.
- **`jit gate evaluate` detaches checker chains into their own process groups.** Killing a sweep runner orphans its in-flight checker (reparented to init, result discarded — one killed evaluator wasted an ai-review). After stopping any runner, sweep for orphans by mapping `/proc/<pid>/cwd` to worktrees before killing; never kill the user's own `codex` process (pts-attached).
- **The codex session commits `.jit` evidence-preserve commits on main whenever the tree is dirty** — each moves HEAD and aborts in-flight evaluation batches (abort guard) or invalidates the installed binary. Batch all fixes/merges into commit windows; launch evaluation waves only from a clean, freshly installed HEAD; expect to cut a wave at an evaluation boundary when a fix must land (kill the runner only, let the in-flight eval record, then window).
- **Shared `target/` is contaminated continuously** (codex builds `target/jit-exact-<id>` binaries with incremental on, mid-evaluation). cargo-ci's incremental-state check honors CARGO_TARGET_DIR — every lead-side evaluation runs with `CARGO_TARGET_DIR=$HOME/.cache/jit-sweep-target`. A signal-killed cargo-ci with empty output and null exit code (status `error`) was environmental, not a defect; re-run isolated first.
- **Workers self-contaminate their private CARGO_TARGET_DIR** with ad hoc `cargo build/test` before running cargo-ci.sh (three workers hit the incremental-state failure this way). Dispatch prompts must say: pre-clean incremental in YOUR target dir immediately before cargo-ci.sh.
- **Skip-already-passed logic hides stale evidence.** A recorded `passed` at an old commit is not current evidence; bare `jit gate evaluate` re-runs automatically unless passed at CURRENT HEAD — prefer direct evaluation loops over skip-guarded scripts, and audit `.commit` on recorded passes against later issue-tied commits.
- **One-finding-per-round reviewer loops end only with whole-family closure**: enumerate the defect FAMILY (grep every phrasing variant / grammar production), fix all sites in one submission, and say so in the gate-run context. Per policy, three same-root-cause rounds = escalate (worked twice this session: terminality prose family; TOML scanner → toml_edit rebuild).
- **Do not amend a commit another branch has already merged** (dogfood.rs redirect fix): the amend orphans the merged parent and forces a manual conflict resolution on the next merge. Land follow-up fixes as new commits.
- **Idle notification ≠ finished** (invoker confirmed workers go idle by design; nudging might be required): an idle worker with a running cargo in its worktree cwd is flock-queued, not stalled; an idle worker with NO build activity and no report needs a nudge (two nudges then takeover — the integrator's final report never arrived; its committed work was verified lead-side instead).
- **Check a doc's generated-from header before hand-editing.** storage-records.md is generated from `crate::storage::reference`; a lead hand-edit to its tree_dirty row passed doc checks but failed the conformance test in the next cargo-ci (caught post-handoff; fixed by porting the wording into the source and regenerating). Generated pages in docs/reference: exit-codes.md (schema.rs, UPDATE_EXIT_CODE_DOC=1), storage-records.md (storage/reference.rs, `--ignored regenerate`), gate-presets.md (gate_presets/reference.rs), rules-and-gates.md + AGENTS.md regions (`jit project render`).
- Unresolved from prior handoffs: reviewer batch-enumeration on output contracts (handoff-2); `jit issue status` may omit gates (Tier-1 must read `gate status-all`); zsh `===` separator (hit again this session — use `-----`).

## SESSION-END ADDENDUM (final state — supersedes "Current state" above where they differ)

Invoker directive at close: stop all; next session picks up.

- **DONE: 12 of 16 batch children.** This addendum's batch: 1d59070d, 52665a07, c505031a, 554ad07f (all gates green, transitioned in the session-close commit). Earlier: 8917c558, 3e12ffbd, 0daba57d, 6f881a85, 894337e2, 94096aac (bc9ee111) + 8d7fc762, 16402e14 (session 4).
- **4 children remain in_progress:**
  1. **d74a9ed1** — round-6 decor-transfer fix READY UNMERGED: `4b0b39e7` on worktree-agent-d74a9ed1-r2 (desk-reviewed PASS, CI green 3589 tests). Merge it, verify-commit-builds, then re-run code-review. Its doc-review + cargo-ci are green (cargo-ci re-runs automatically at the new HEAD via bare evaluate).
  2. **450db193** — round-2 config-keyed dispatch fix committed UNMERGED: `d0bb829a` on worktree-agent-450db193 ("dispatch full-style projection on registry source, not kind name"); its CI was running at stop; NO final report received (twice now — verify lead-side like last time: fast_rules + lib projection tests in the worktree). Merge, verify, then re-run code-review AND doc-review.
  3. **45a140ae** — doc-review failed with FIVE findings (F1 cli-commands.md:2203 dep-add contract; F2 cli-commands.md:2610 — the archived_from sentence over-promises: legacy archived nodes omit the field, an absent origin blocks; F3 README.md:111 diagram label; F4 dependency-management.md:37 table cell "Blocks until terminal"; F5 core-model.md:682 diagram label). A WHOLE-REPO audit was completed read-only — full stale-site list beyond the findings: containment-and-completion.md:78/91/107 (phrasing; :80 is already correct), core-model.md:1015 comment, design-philosophy.md:121 pseudocode `is_terminal` → `is_effectively_terminal`. Deliberately literal, do NOT change: core-model.md:793 (re-scan trigger), core-model.md:638/658 (gate bypass — about gate enforcement, not closure). Apply ALL of these in ONE commit, then re-run 45a140ae doc-review. **If the terminality family fails doc-review AGAIN after this, escalate to the invoker with options incl. criteria amendment — do not patch a fourth family round.**
  4. **0283ce74** — its doc-review was in flight when stopped (evaluation killed; will show error or stale failed). Just re-run doc-review — its fix (exit-codes apply/4 row) is merged and all other gates are green.
- Wave results recorded this session after the handoff body above was written: doc-review PASSED for 1d59070d, 52665a07, c505031a, 554ad07f (hence the done batch); cargo-ci PASSED for d74a9ed1 + 450db193 at 62c8025a.
- Resume recipe unchanged (What-to-do-next §5). Re-runs needed at next HEAD after the two merges: d74a9ed1 code-review; 450db193 code-review + doc-review; 45a140ae doc-review (after the terminality commit); 0283ce74 doc-review. Then the final done-transitions and Section 10 epic completion.

## Open questions needing invoker input

None pending. Two closed this session: profile-manifest integration ownership ("I integrate now"); d74a9ed1 scanner → toml_edit rebuild (approved). Standing: when the epic closes, @/charter/D-14 unblocks the production-readiness source freeze — notify the invoker.

## Reference artefacts

- Progress: dev/active/6eb585bc-progress.json (session_5 key; rework counts current).
- Final wave: results land in `.jit/gate-runs` + per-issue gate status; HEAD at launch 46e8c5ae.
- Merge log this session (main): cb669c79, 3a078eed, ae271000, c5330d4f, 180314b6, 56d6463d, fbe2bb36, 2abed5ba, c7699f38, a32e1707, 44ee4258, 46e8c5ae (+ fix commits b301eded, 640854b9, 6cad67f0, e5839de6 family, bc9ee111 done-batch).
- Cross-epic integration record: profile manifest now carries `kind = "projection"` contributions (profiles/jit-dogfood/manifest.toml); codex's epic completion report at dev/ (archived by their session).
- Worker branches (all merged; worktrees removable after the wave settles): agent-450db193, agent-d74a9ed1-r2, agent-45a140ae-r2, agent-554ad07f-r1, agent-894337e2-r3, agent-1d59070d-r2, agent-52665a07-r1, agent-94096aac, agent-c505031a, agent-6f881a85, agent-0283ce74.
- Dispatch/leak/build-verify protocol: ~/.claude/skills/jit-execution-lead/references/worktree-dispatch-protocol.md.
