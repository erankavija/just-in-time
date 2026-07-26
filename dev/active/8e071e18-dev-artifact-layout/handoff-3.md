# Handoff — Issue-based development artifact layout and serviceable archival (8e071e18) — session 4

**Date:** 2026-07-26
**Session number:** 4 (session 3 produced no handoff — it died on a server error mid wave-6 close)
**Prior handoffs:** `dev/active/8e071e18-handoff.md` (session 1), `dev/active/8e071e18-handoff-2.md` (session 2). Their Traps sections remain in force except where this handoff records a resolution.

> **Location note.** This is the first artifact written to the epic's canonical directory, `dev/active/8e071e18-dev-artifact-layout`, resolved with `jit doc dir 8e071e18 dev/active` — dogfooding the convention this epic ships. The plan, investigation, breakdown manifest, progress file, and the two earlier handoffs stay at their flat `dev/active/8e071e18-*` paths, deliberately: `@/issue/8e071e18/decision/D-4` governs newly created artifacts and tolerates legacy flat files, and REQ-13 forbids restructuring them in place.

## Current state

- Epic: `8e071e18` — state: in_progress, claimed by `agent:jit-execution-lead`
- Waves 1–7 **complete**. Wave 8 partially complete: both implementation issues merged, the 24 archive executions **blocked on an owner decision** (E9, below).
- Children summary: 83 issues carrying `epic:dev-artifact-layout` — **33 done**, 3 in_progress, 25 ready, 22 backlog. (79 manifest entries + `fd88adda` + bracket nodes `55fe8ec2`/`644bfae1` + the epic itself.)
- Active claims: none outstanding beyond the epic itself (`agent:jit-execution-lead`). `3be32cad` and `429995e1` are Done.
- Open escalations: **E9 open** — the out-of-root copy policy. Nothing else.
- Progress file: `dev/active/8e071e18-progress.json` (wave plan, per-issue status, escalations E1–E9, pitfalls P1–P17, lead notes N1–N8)

**The wave-8 gate batch completed after this handoff was first written.** All five passed at HEAD `9f106406`, both code-reviews returning zero findings: `3be32cad` cargo-ci + code-review, `429995e1` cargo-ci + repo-validate + code-review. Both issues are Done and `jit validate` is green. **Wave 8's only remaining work is the 24 archive executions, blocked on E9.**

## What just happened

- **Resumed from a crashed session with no handoff.** Reconstructed wave-6 state from `git log`, `jit issue show`, and `.jit/gate-runs/*/result.json` rather than the progress file, which was last written at wave-6 dispatch and was missing three review rounds, an owner-approved criteria amendment, and eleven closures.
- `e2902949`: its sole outstanding gate failure was cargo-ci's `incremental-state` step — 3941 tests passed, 0 failed, only a leftover `target/debug/incremental` directory. Cleared it, re-ran, passed. Lead review PASS: all three prior code-review findings verified closed at HEAD (lock-sidecar sweep removed via new `IssueStore::read_issues`; `.index.lock` creation resolved by the E8 amendment; empty misplaced directories now reported via `ArtifactListingScope::RecursiveEntries`). **Wave 6 closed, 13/13.**
- Recorded **E8** into the progress file: the owner-approved REQ-03 amendment on `e2902949`, whose only prior record was commit `1bdae2f5`'s message.
- Pruned 14 merged worktrees.
- **Wave 7 complete (3/3), every gate green first try, all three code-reviews zero findings.**
  - `daf4fa46` (Sonnet, worktree): four tests proving deck archival eligibility through both target forms, asserted against the target-level blocker array, with a negative control proving the absence assertions are reachable. One rework round: its fixture doc comment misattributed the vacuous-terminal-owner finding to `f2cac2bd`; verified against gate evidence that it was `1f80212b`, and had it corrected along with an overstated "mirrors this repository's shipped classification" claim that crossed the dogfooding boundary.
  - `d2870182` (Opus, worktree): made `InterpolationContext::for_node` the single per-node constructor (`for_container`/`with_doc` demoted to private), added `node_document_path` and `located_document_area_error`, and deleted the hand-rolled `render_template_document_path` with its duplicated `extract_hard_criteria` — the actual drift source. Widened into `template_expand.rs`, reported and justified. One rework round for a stale doc comment (see P14).
  - `2bc344fc` (checkpoint story): **lead-verified, not dispatched** — a pure aggregation node with no footprint. Confirmed all 61 declared profile source/target asset pairs byte-identical, that the only remaining flat paths under `.agents/skills` are eval fixtures and transcripts (which its REQ-04 requires unchanged), and that no skill document derives a name from an issue title. Its code-review ran under the no-attributable-commit fallback and passed.
- **Wave 8 partial.** `3be32cad` (Opus) and `429995e1` (Sonnet) dispatched to worktrees, both merged with per-merge build verification and the N3 test-compile guard, leak check clean.
  - `3be32cad`: whole-tree content-based classification (`unchanged`/`rewritten`/`relocated`/`synthesized`/`lost`) with a totality assertion, so REQ-04's "only" is a claim about the entire tree. Broke the guarantee four ways and observed the right failures; removing the relink failed **only** test 3, proving the four tests are not redundant.
  - `429995e1`: `.jit/templates.toml` and the profile twin now declare `doc_area = "dev/active"` / `doc = "{container.dir}/plan.md"`.
- **Previewed all 24 archive containers: every one eligible, zero blockers.** This is the epic's central repair demonstrably working. Then found what execution would actually do — see E9.
- Recorded pitfalls **P13–P17** and lead note **N8**.

## What to do next

- [ ] **Resolve E9 first — it gates 12 of the 24 archive executions.** See "Open questions" below. The other 12 containers copy nothing and can run under either answer.
- [ ] Confirm the in-flight gate batch's recorded results; on pass, close `3be32cad` and `429995e1`. On any failure, the log path is above.
- [ ] Run the 24 archive executions. **Take each run's REQ-05 citation-warning evidence from `jit archive container <id> --json` (the preview), not from `--execute --json`** — see P16; execution reports no citation warnings at all.
- [ ] `35499d1e` (checkpoint story, 5 gates) is available now and independent of the sweeps — it can be verified and gated at any point.
- [ ] Before the epic's own gates, reconcile `surfaced_pitfalls` P1–P17 against the epic's 15 `[hard]` criteria per `lead-review-protocol.md`. P2/P4/P5/P6 need verification at their owning waves; P7, P13, P15 need a disposition call.
- [ ] The user has asked that the **next session begin by addressing the surfaced findings** (P13–P17 in particular) rather than resuming fan-out immediately.

## Traps — do not repeat these

All session-1 and session-2 traps remain in force. Read `dev/active/8e071e18-handoff.md` and `dev/active/8e071e18-handoff-2.md`. New or newly sharpened:

- **Do NOT trust the progress file as the record of what happened.** Session 3 died between doing the work and writing it down. Git history, `jit issue show`, and `.jit/gate-runs/*/result.json` carried three review rounds, an owner-approved criteria amendment, and eleven issue closures that `progress.json` did not. On resume, reconcile all four sources before acting; the progress file is the *plan*, the repository is the *record*.

- **Do NOT assume a `cargo-ci` failure is a real failure.** `e2902949` showed `exit_code: 1` with `3941 passed, 0 failed` — the only failing step was `incremental-state`, tripped by a `target/debug/incremental` directory that rust-analyzer or an ad-hoc `cargo test` recreated. Always read the recorded `stdout` in `.jit/gate-runs/<run>/result.json` and find the `✗` line before concluding anything about the code.

- **Do NOT word-split an unquoted variable in this shell — it is zsh, not bash.** A gate batch written as `for spec in "id gate"; do set -- $spec; jit gate evaluate "$1" "$2"; done` passed the whole string as one argument, and all six gates exited 3 with `Issue not found: daf4fa46 cargo-ci`. Harmless here because it failed before mutating anything, but the same construct in an *execution* loop would be much worse. Write the arguments explicitly, or use `${=var}`.

- **The stale-binary guard is much narrower than sessions 1–2 assumed.** It refuses only when the build was dirty **or** a declared build-input path changed. `BINARY_BUILD_INPUTS` (`crates/jit/src/domain/build_provenance.rs:117`) is `Cargo.toml`, `Cargo.lock`, `crates/jit/Cargo.toml`, `crates/jit/Cargo.lock`, `crates/jit/build.rs`, `crates/jit/src/`, `profiles/jit-dogfood/`, `scripts/hooks/pre-commit`, `scripts/hooks/pre-push`. **A bare HEAD move past the build commit is not staleness.** `crates/jit/tests/`, `dev/`, `.jit/`, and `docs/` are all outside it. This matters for the 24 archive executions: they write only `dev/` and `.jit/`, so they should not each need a reinstall. Note `429995e1` touched `profiles/jit-dogfood/` and `d2870182` touched `crates/jit/src/`, so both *did* require one.

  **Confirmed empirically this session.** The wave-8 gate batch ran gates 2–5 while the working tree carried an uncommitted new `dev/active/**` file and a modified `dev/active/8e071e18-progress.json`. All four passed and recorded `tree_dirty: true`. So a dirty tree of non-build-input paths neither refuses a verdict nor invalidates one — only the nine `BINARY_BUILD_INPUTS` paths and a dirty *build* do.

- **Do NOT take a footprint's "N artifacts for copying" at face value without previewing.** Seven wave-8 issue descriptions say "schedules N artifacts for relocation and 90 for copying". Ninety sounded like an ordinary number until previewed: it is the whole `docs/` tree plus `README.md`, `AGENTS.md`, `INSTALL.md`, and six `crates/jit/src/**.rs` files, mirrored per container. Preview before executing anything whose blast radius is stated only as a count. The counts have also drifted upward since planning (132/92/91 against a predicted 90) because `docs/` grew.

- **Do NOT read a worker's "I could not find the test that supposedly enforces X" as the worker failing to look.** `429995e1` reported it could not find the assertion binding `.jit/templates.toml` to its profile twin, which its own issue text asserts exists. Verified independently: the only test naming both files (`crates/jit/src/profile/dogfood.rs:683-700`) greps each for stale *phrases* and never compares declarations. The issue's premise is simply false. Verify such a report rather than pushing back on it — and see P15.

- **Do NOT let a worker's flagged pre-existing staleness sit unfixed when it is inside that worker's own footprint.** `d2870182` correctly reported that `PLAN_DOC_LABEL`'s doc comment claimed apply writes a plan-labelled `DocumentReference`, which `crates/jit/tests/fast_docs_templates/template_apply_tests.rs:243-247` explicitly asserts it must not do. The worker was right that its change did not *invalidate* the text, so the repository's sweep rule did not compel a fix — but leaving a live falsehood in a heavily rewritten file just defers a reviewer round trip. Direct the fix. Recorded as P14.

- **`jit doc dir` creates nothing.** It resolves a path. `mkdir -p "$(jit doc dir <id> <area>)"` before writing.

## Open questions needing invoker input

- **Question (E9, OPEN — blocks 12 of wave 8's 24 archive executions): should a linked artifact outside the development root be mirrored into the archive, or retained in place?**
  - Context: all 24 containers preview eligible with zero blockers, but the 24 plans together schedule **684 copy operations over 139 distinct files, ~16.6 MB**, duplicated into `dev/archive/`. Seven containers copy 90–132 files each. The set is 56 adopter docs under `docs/`, 44 `.jit`/`.github`/`scripts`/`mcp-server` files, 29 `dev/` files, 6 production sources under `crates/`, and 4 repository-root files including `README.md` and `AGENTS.md`. Destination shape is a full path mirror, e.g. `docs/reference/gates.md` → `dev/archive/2e926e39-agent-seamlessness/docs/reference/gates.md`.
  - Mechanism (verified in code, not inferred): the classifier picks an action from two independent booleans at `crates/jit/src/domain/artifact_classifier.rs:765` — `(wants_destination, needs_source)` maps to Copy / Move / Retain. "Permanent" was designed for `dev/architecture`, living documentation where the archive genuinely wants a frozen snapshot *and* readers keep the live file, so it sets `needs_source = true` and Copy falls out. `@/issue/8e071e18/decision/D-14` needed out-of-root artifacts to stop *blocking*, and expressed that by extending `is_permanent()` to return true for anything outside the development root (`:129-137`). That flag only says *keep the source*; it leaves *wants destination* true. The copying is an inherited side effect, not a chosen behaviour.
  - Options: (A) flip the other bit — classify out-of-root sources to `Retain` (already an existing action; 93 artifacts already take it in the first container's plan). Epic REQ-14 still holds literally: no `unmanaged-selected-root` blocker, and no file relocated outside the development root. Configured permanent roots keep Copy. 684 copies → 8. (B) execute as planned, as the reviewed plan and breakdown review approved. (C) unlink the out-of-root documents first — explicitly rejected by D-14 because it destroys traceability from ten terminal issues.
  - Recommendation: **(A)**. It matches what D-14 was actually trying to buy, and (B) risks a finding at the epic's own `holistic-review` gate against `@/inv/single-source-prose` ("a hand-maintained copy is a staleness defect") and `@/charter/D-13`, both of which the epic's Notes cite as binding cross-cutting constraints — seven frozen copies of the adopter docs tree are precisely that. Cost of (A): amending D-14's recorded mechanism is an epic decision-log change requiring approval, plus a remediation task against `4f9af089`'s shipped classification and a re-preview of the 12 affected containers.
  - **Not yet decided.** The user asked the clarifying question "Why copy sources? Sounds weird.", received the mechanism explanation above, and then asked for this handoff. No option has been chosen.

## Reference artefacts

- Epic: `jit issue show 8e071e18`
- Prior handoffs: `dev/active/8e071e18-handoff.md`, `dev/active/8e071e18-handoff-2.md` (traps still in force)
- Plan: `dev/active/8e071e18-plan.md`
- Authoritative manifest: `dev/active/8e071e18-breakdown.json` (79 entries; `fd88adda` is not in it)
- Investigation: `dev/active/8e071e18-investigation.md` (its addendum supersedes earlier sections on disagreement)
- Progress file: `dev/active/8e071e18-progress.json`
- Wave-8 sweep targets (issue → container → destination): `/tmp/claude-1000/-home-vkaskivuo-Projects-just-in-time/4025753c-0590-45c7-9fc7-ee662d8cfa08/scratchpad/wave8-sweeps.json`; per-container preview summary alongside it as `wave8-preview.json`. **Both are scratchpad files and will not survive indefinitely — regenerate by previewing rather than trusting them.**
- New public surface this session: `IssueStore::read_issues` and `ArtifactListingScope::RecursiveEntries` (from `e2902949`); `InterpolationContext::for_node`, `node_document_path`, `located_document_area_error`, `PlanDocContainer` (from `d2870182`).
