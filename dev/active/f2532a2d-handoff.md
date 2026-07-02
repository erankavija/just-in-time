# Handoff — jit-project-lead skill: milestone-level vision steward (f2532a2d) — session 3

**Date:** 2026-07-03
**Session number:** 3
**Prior handoffs:** none for this epic (this is the first). Prior sessions logged only in `dev/active/f2532a2d-progress.json` `notes[]`.

## Current state

- Epic: `f2532a2d` — state: `backlog` (claimed by `agent:jit-execution-lead`)
- Wave in progress: wave 3 (the dependency-ready batch after wave 2 closed). Waves 4–6 (per the progress file's original grouping) not started; true readiness is dependency-driven, see below.
- Children summary (whole tree): **10 done**, **2 in_progress (open)**, rest backlog. Done: c6325c5b, b46e13d9, faed8ffa (wave 1); 87c8ba2d, a5b04c9f, 6662f738 (wave 2); 66aeee5f, 82e04ab0, 28731a53, e7d41080, 02a2bbb9 (wave 3). Open: 0b7e864d, 41aa1b75. Backlog: 7a9eb806, c23dfe71, 206bd960, e8b1cee3, eff48a6e, 634b2382, 304f6d94, 3c192f5e, 6c5f70ad.
- Active claims: 0b7e864d, e7d41080, 02a2bbb9, 41aa1b75 were claimed `agent:claude` this session (e7d41080 + 02a2bbb9 now done; 0b7e864d + 41aa1b75 still claimed/open).
- Open escalations: none awaiting input. The flaky-test question was resolved by the invoker (see Traps #1).
- Progress file: `dev/active/f2532a2d-progress.json` (reflects the above; `reviewed{}` holds per-issue verdicts).

## What just happened (session 3)

- Resumed mid-flight: wave 2 had 3 cut workers (a5b04c9f, 6662f738, 66aeee5f) and 66aeee5f rework merged with a failing gate.
- `66aeee5f` — lead take-over (invoker-approved): added `STD-STRUCT-SUMMARY` + `STD-STRUCT-BACKGROUND` scanner checks (required-structure rule was genuinely in-scope, listed in the issue's own description). Scanner tests 25/25. Gate passed → **done**.
- `a5b04c9f` — merged; failed its OWN code-review on a valid self-inconsistency (negative-item verdict lacked run-record evidence its own method demanded). Rework-1 (Route A): reconciled the equivalent-runner method + added an on-disk-grounded command/gate-invocation log. Gate passed → **done**.
- `6662f738` — landed the richer jit-planning-lead description to MAIN (symlink trap, Trap #2), re-ran trigger evals at runs_per_query=5 → both skills 17/17 failed:0. Rework-2 fixed non-atomic writes in `run_trigger_eval.py` (lines 103 + 382). Gate passed → **done**.
- Wave 2 closed. Dispatched wave-3 ready batch (4 parallel Opus worktrees @ main `2ebf6402`): 41aa1b75, 0b7e864d, e7d41080, 02a2bbb9.
- `e7d41080` — per-container dispatch prose (`references/container-dispatch.md` + SKILL.md body section). Reuses dispatch scripts verbatim, no re-implementation, frontmatter untouched. Gate passed → **done**.
- `02a2bbb9` — jit-project-lead skeleton trigger (17/17) + 4 scenario evals (all PASS). First to hit the flaky-test gate failure (Trap #1); resolved via cargo-ci-before-review. → **done**.
- `0b7e864d` — mechanical auto-fixer + 36-test fixture harness. Rework-1 fixed 2/3 findings (reclassified LABEL-SLUG→judgment in scanner; widened title strip; recorded exhausted-id skip). Merged, cargo-ci green, but code-review STILL FAILS on a residual title mismatch (Trap #4). **Left in_progress; rework-2 deferred per invoker ("no more rework this session").**
- `41aa1b75` — jit-planning-lead scenario evals. **Interrupted mid-run.** WIP preserved on branch `worktree-agent-41aa1b75` @ `a4cf8dc7` (evals.json + setup-test-repo.sh drafted; transcripts/ empty; no results.md). **Not merged, not reviewed.**
- Adopted invoker-directed process: for every issue, ADD the `cargo-ci` gate and RUN it BEFORE code-review (Trap #1).
- Leak check clean after the wave. Main HEAD after session: see `git log`.

## What to do next

- [ ] **Finish 41aa1b75** (resume, don't restart). Worktree `worktree-agent-41aa1b75` @ `a4cf8dc7` already has `evals.json` + `setup-test-repo.sh`. Run its 3 scenarios (research-and-plan, plan-from-existing, plan-from-import) via the equivalent sub-agent runner, grade under `docs/reference/skill-eval-adjudication.md`, write `evals/results.md` with checklists + transcripts. Then: add cargo-ci gate, run cargo-ci, run code-review, merge, done. (Its scenario agents read jit-planning-lead from `~/.claude/skills` → main, which is stable — that's correct.)
- [ ] **Rework-2 on 0b7e864d** (one attempt left; count=1, MAX=2). The ONLY remaining finding is the title scanner/fixer mismatch (Trap #4). Fix: anchor the scanner's position-code title pattern to `^` (matches the standards-doc "leading ordinal" definition and the fixer), so mid-title codes like `Build S0/W1: worker` are not flagged as mechanical. Add fixture tests for that case. Re-run both harnesses (test-standards-scan.sh, test-standards-fix.sh). Then cargo-ci + code-review, merge, done.
- [ ] **7a9eb806 is READY now** (dep e7d41080 done): "Run cross-container coherence review before accepting a container as complete." Can dispatch immediately.
- [ ] After 41aa1b75 done → **c23dfe71** ready (dep 41aa1b75 + 6662f738✓). After 0b7e864d done → **206bd960** ready (dep 0b7e864d).
- [ ] **e8b1cee3** (dep c23dfe71 + 02a2bbb9✓) is the LINCHPIN — the whole final wave (eff48a6e, 634b2382, 304f6d94, 3c192f5e, 6c5f70ad) depends on it. Prioritize the c23dfe71 chain to unblock it.
- [ ] **6c5f70ad** has an EXTERNAL dep `eed6750c` (jit-planning-lead work, outside this epic, per plan D6). Check its state before dispatching 6c5f70ad; if not done when 6c5f70ad's turn comes, escalate per escalation-policy entry 7 (see `eed6750c-handoff.md`).
- [ ] **Every issue: add `cargo-ci` gate + run it before code-review** (Trap #1).

## Traps — do not repeat these

- **Trap #1 — Flaky Rust tests block code-review non-deterministically; run cargo-ci FIRST for evidence.** 5 tests are flaky under the full parallel `cargo-ci` run but PASS in isolation: `commands::serve::tests` (bind ports in a fixed 3000–3099 range and race — `serve.rs:712,720,796`) and `commands::issue::tests::test_claim_issue_*` (`issue.rs:1041,1108`; share process-global state). The full suite is GREEN for the lead (`./scripts/cargo-ci.sh` → 2836 passed, 0 failed). The code-review reviewer runs cargo-ci inconsistently; when it does and hits the flakiness, it blocks the (all-docs) issue. **Invoker-directed fix (in force):** for every issue, `jit gate add <id> cargo-ci` then `jit gate pass <id> cargo-ci` BEFORE `jit gate pass <id> code-review`, so a passed cargo-ci sits in run_history and the reviewer trusts it instead of running its own flaky copy. Verified working on 02a2bbb9 + 0b7e864d. Do NOT "fix" the flaky tests (invoker did not choose that) and do NOT bypass any gate.
- **Trap #2 — The skill symlink defeats worktree isolation for eval runs.** `~/.claude/skills/jit-execution-lead`, `~/.claude/skills/jit-planning-lead`, `~/.claude/skills/jit-project-lead` are symlinks to the MAIN repo's `.claude/skills/*` (NOT any worktree). Any issue that RUNS trigger/scenario evals reads the skill's LIVE description/body from main. A description edited only in a worktree is INVISIBLE to the eval. 6662f738's first run recorded stale failing numbers for exactly this reason. **Before measuring a changed description, land it on main.** Issues that only ADD eval files (41aa1b75, 02a2bbb9) are fine in a worktree because they don't change the skill under test — the skill on main stays stable during their run.
- **Trap #3 — code-review reviews WHOLE-TREE state, not the per-issue diff.** a5b04c9f's gate FAILED on a bug in `run_trigger_eval.py` — a file owned by 6662f738, not a5b04c9f — because both were in the tree. A dependent issue's gate cannot pass until shared-file findings owned by a sibling are fixed and merged. Merge/fix shared-file bugs before gating anything that shares the file.
- **Trap #4 — Scanner and fixer title patterns drift; keep STD-TITLE-EMBEDDED-ID detection and correction in lockstep.** The scanner's position-code check at `standards-scan.sh:329` (`[[ "$title" =~ [A-Za-z][0-9]+/[A-Za-z]?[0-9]*: ]]`) is UNANCHORED — it flags `S0/W1:` anywhere in a title. The fixer only strips leading forms (`^...`), so `Build S0/W1: worker` is flagged mechanical but never fixed → survives re-scan → REQ-01/02 fail. This class of finding (scanner detects more than the fixer corrects) has now recurred twice on 0b7e864d. **Correct fix: anchor the scanner's position-code pattern to `^`** (the standards doc defines STD-TITLE-EMBEDDED-ID as a *leading* short-id/ordinal/prefix — a mid-title `S0/W1` is not that rule). Then scanner, fixer, and rule all agree and no flagged title survives. Do NOT just keep widening the fixer to strip mid-title codes — a mid-title strip has no deterministic clean result (same non-determinism that made LABEL-SLUG a judgment rule).
- **Trap #5 — LABEL-SLUG-style rules: "mechanically detectable" ≠ "mechanically correctable."** A rule can be deterministically DETECTED but have no single safe CORRECTION (STD-LABEL-SLUG: no meaningful kebab slug derivable from a hex id). REQ-01 requires every *mechanical* finding be corrected, so such rules must be classified `judgment`, not `mechanical`, or the fixer can never satisfy REQ-01. This was the fix for 0b7e864d finding 1 (reclassified LABEL-SLUG in the scanner; 66aeee5f's contract never required it mechanical, so this is safe). Apply the same test to any rule you add.
- **Trap #6 — cargo-ci and code-review gates take ~2 min; use a ≥300s shell timeout.** `jit gate pass <id> code-review` / `cargo-ci` runs an LLM/full-suite subprocess ~2 min. The default 120s Bash timeout kills it mid-run (leaving the gate `pending`, no result recorded). Run gate commands with `timeout: 360000`.

## Open questions needing invoker input

None. The one escalation this session (flaky-test gate) was resolved by the invoker's cargo-ci-before-review directive (Trap #1), now standing process.

## Reference artefacts

- Epic: `jit issue show f2532a2d`
- Plan (bracket): `dev/active/f2532a2d-138d-46bf-9547-e1ea517f0170-plan.md` (decisions D1–D9)
- Progress + verdicts: `dev/active/f2532a2d-progress.json` (`waves`, `reviewed{}`, `rework_counts`, `notes[]`)
- Adjudication method (used by all eval issues): `docs/reference/skill-eval-adjudication.md`
- Content standards (scanner/fixer target): `docs/reference/jit-content-standards.md`
- Scanner + fixer: `.claude/skills/jit-project-lead/scripts/standards-scan.sh`, `standards-fix.sh`, `references/standards-scan.md`, `references/standards-fix.md`
- Dispatch prose (e7d41080): `.claude/skills/jit-project-lead/references/container-dispatch.md`
- 41aa1b75 WIP: branch `worktree-agent-41aa1b75` @ `a4cf8dc7`
- External dep for 6c5f70ad: `dev/active/eed6750c-handoff.md`
- Dispatch scripts: `.claude/skills/jit-execution-lead/scripts/dispatch-worker-worktree.sh`, `check-leak-into-main.sh`
