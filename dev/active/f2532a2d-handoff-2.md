# Handoff — jit-project-lead skill: milestone-level vision steward (f2532a2d) — session 4

**Date:** 2026-07-03
**Session number:** 4
**Prior handoffs:** `dev/active/f2532a2d-handoff.md` (session 3). Sessions 1–2 logged in `dev/active/f2532a2d-progress.json` `notes[]`.

## Current state

- Epic: `f2532a2d` — state: `backlog` (claimed by `agent:jit-execution-lead`)
- Wave in progress: past the original wave grouping; true readiness is dependency-driven (below). The whole final tranche funnels through the linchpin `e8b1cee3`.
- Children summary: **15 done**, **1 in_progress (`e8b1cee3`, verify-and-close near done)**, **1 ready (`206bd960`)**, **5 backlog (`304f6d94`, `3c192f5e`, `6c5f70ad`, `eff48a6e`, `634b2382`)**. 0 rejected.
- Active claims: `e8b1cee3` (claimed `agent:claude` this session; reword committed, gates not yet run). `c23dfe71` was claimed+done.
- Open escalations: none awaiting input.
- Progress file: `dev/active/f2532a2d-progress.json` (`waves`, `reviewed{}`, `rework_counts`, `notes[]` all current).

## What just happened (session 4)

- Resumed with 2/3 wave-3 unblockers open. Dispatched 3 disjoint-file workers in parallel (worktrees @ main): `41aa1b75` 3rd eval, `0b7e864d` rework-2, `7a9eb806` coherence doc.
- `0b7e864d` — rework-2: anchored scanner `standards-scan.sh:329` position-code pattern to `^` (Trap #4, resolved). Worker audited all 4 other position/prefix branches → each has a matching fixer strip, no residual gap. Harnesses 28+41 green. cargo-ci + code-review PASS → **done**.
- `7a9eb806` — authored `references/coherence-review.md` (tiers C1/C2/C3, PASS/FAIL verdict, stale-forward sweep, no-argue stop). code-review FAIL round 1: protocol existed but was not WIRED (REQ-01/03 unenforced). Lead reworked (rework-1): wired it into the dispatch flow at the container-dispatch.md step-8→9 seam + SKILL.md dispatch tail + softened the red flag; reconciled all 5 "separate story/concern" mentions. Re-run PASS → **done**.
- `41aa1b75` — ran+graded 3rd scenario `plan-from-import` (PASS 8/8). All 3 planning-lead entry-path scenarios now recorded. **Discovered a real bug** (coverage-preview vacuity, see below) → **done**.
- Filed bug `16402e14` (coverage-preview passes vacuously on checkbox-prefixed `[hard]` criteria; type:bug, priority high, component:validation, milestone:v1.0). Root cause: start-anchored `^\[hard\]` match at `crates/jit/src/validation/engine.rs:1317`. Wired to milestone `9db27a3a` (see Trap #7).
- `c23dfe71` — verify-and-close rollup. Authored `docs/reference/lead-skills-eval-baseline.md`. code-review FAIL round 1: exec-lead `evals.json` had a **4th** scenario `parent-invoked-escalation` unrun. Ran+graded it PASS 6/6 (validates b46e13d9 escalation-to-parent), recorded, baseline → 4/4. Re-run PASS → **done**.
- `e8b1cee3` — claimed; verified skeleton meets REQ-01..05 (all backed by prior work + 02a2bbb9 evals); reworded the one `epic-level target` domain-leak in `container-dispatch.md` for REQ-02. **Gates not yet run** (stopped here for handoff).
- Every completed issue got `cargo-ci` added + passed BEFORE code-review (Trap #1, standing process). Leak checks clean after every worktree wave.

## What to do next

- [ ] **Finish `e8b1cee3`** (near done): `jit gate add e8b1cee3 cargo-ci` → `jit gate pass e8b1cee3 cargo-ci` → `jit gate pass e8b1cee3 code-review` (≥360s timeout) → `jit issue update e8b1cee3 --state done`. It is a verify-and-close; the skeleton + references already exist and are 02a2bbb9-verified. This unblocks the whole final tranche.
- [ ] **Then the final tranche** (all edit `jit-project-lead/SKILL.md` — SERIALIZE them; do not parallel-dispatch onto that one file):
  - `206bd960` (ready now, dep `0b7e864d`✓): standards-sweep mode workflow + report (SKILL.md mode-4 body).
  - `304f6d94` (dep `7a9eb806`✓+`e8b1cee3`): wave-dispatch mode; wire in existing `container-dispatch.md` + `coherence-review.md`, no re-implementation.
  - `3c192f5e` (dep `206bd960`+`e8b1cee3`): REQ-06 standards-sweep-mode satisfier (builds on 206bd960's mode).
  - `6c5f70ad` (dep `eed6750c` EXTERNAL + `e8b1cee3`): four-mode front-door routing (replaces the "Mode dispatch (stub)" block). **Check `eed6750c` state first** — still `in_progress` as of this session; if not done when 6c5f70ad's turn comes, escalate per escalation-policy entry 7 (see `dev/active/eed6750c-handoff.md`).
  - `eff48a6e` (dep `e8b1cee3`): vision/charter + progress artifacts (mostly `references/` + `docs/` charter template; light SKILL.md touch).
  - `634b2382` (dep `e8b1cee3`+`b46e13d9`✓): parent-escalation policy (a `references/` doc — can run in parallel with ONE SKILL.md-body issue since it barely touches SKILL.md).
- [ ] **Every issue: add+run `cargo-ci` BEFORE `code-review`** (Trap #1).
- [ ] After all children done → Section 10 epic completion (coverage map, epic gate `code-review`, completion report, `--state done`). Note c6325c5b's REQ-03 amendment (progress `escalations[]`) must be flagged in the completion report for user review.

## Traps — do not repeat these

Traps #1–#6 from `dev/active/f2532a2d-handoff.md` **remain in force** — re-read them (flaky-test cargo-ci-before-review; skill symlink defeats worktree eval isolation; code-review reviews whole-tree; scanner/fixer title lockstep; mechanically-detectable≠correctable; ≥300s gate timeout). New this session:

- **Trap #7 — Filing a NEW standalone issue with zero dependency edges breaks `jit-validate.sh` (exit 4, "isolated issue") — an integrity ERROR, not the orphan-leaf WARNING.** code-review reviews whole-tree and runs jit-validate, so an isolated issue fails EVERY concurrently-gating issue's code-review. Hit right after filing bug `16402e14`: it failed `41aa1b75`'s code-review with a finding about the unrelated isolated issue. **Fix:** immediately connect any new standalone issue to the graph with a dep EDGE (`jit dep add 9db27a3a <bug>` makes the v1.0 milestone depend on it) AND add a `milestone:v1.0` parent-association label. A membership label ALONE does not fix isolation — an EDGE is required (the 3 pre-existing orphan bugs only WARN because something depends on them).
- **Trap #8 — Eval-set issues can carry MORE scenarios than their success criteria's prose count; code-review enforces the whole `evals.json`, not the prose number.** `c23dfe71`/REQ-02 said "three existing scenario evals," but `jit-execution-lead/evals/evals.json` had a 4th (`parent-invoked-escalation`, added later for b46e13d9) that was never run. code-review FAILED the "3/3" baseline as incomplete. **When closing any eval-verification issue, count the scenarios in `evals.json` directly and run/record ALL of them — do not trust the criteria's prose count.**
- **Trap #9 — `jit-planning-lead` runs can emit criteria as GitHub checkboxes `- [ ] [hard] REQ-NN`, which the `^\[hard\]` parser hides, so `coverage-preview` passes vacuously (bug `16402e14`).** If a future scenario/breakdown appears to pass coverage with zero items, suspect this. Not yet fixed; tracked in `16402e14`. Does not block f2532a2d.

## Open questions needing invoker input

None. (Bug `16402e14` was filed at the user's explicit request this session and needs no decision to proceed on f2532a2d.)

## Reference artefacts

- Epic: `jit issue show f2532a2d`
- Plan (bracket): `dev/active/f2532a2d-138d-46bf-9547-e1ea517f0170-plan.md` (D1–D9)
- Progress + verdicts: `dev/active/f2532a2d-progress.json`
- Prior handoff: `dev/active/f2532a2d-handoff.md` (Traps #1–#6)
- Adjudication method: `docs/reference/skill-eval-adjudication.md`
- Eval baseline rollup (c23dfe71): `docs/reference/lead-skills-eval-baseline.md`
- Content standards: `docs/reference/jit-content-standards.md`
- jit-project-lead skeleton + references: `.claude/skills/jit-project-lead/SKILL.md`, `references/{tier-derivation,wave-layering,container-dispatch,coherence-review,standards-scan,standards-fix}.md`
- Filed bug: `jit issue show 16402e14`; parser site `crates/jit/src/validation/engine.rs:1317`
- External dep for 6c5f70ad: `dev/active/eed6750c-handoff.md`
- Dispatch scripts: `.claude/skills/jit-execution-lead/scripts/{dispatch-worker-worktree,check-leak-into-main}.sh`
