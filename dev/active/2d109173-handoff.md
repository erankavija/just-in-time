# Handoff — Exhaustive documentation audit and drift removal (2d109173) — session 1

**Date:** 2026-07-11
**Session number:** 1
**Prior handoffs:** None (first handoff for this epic)

## Current state

- Epic: `2d109173` — state: `backlog` (correctly cannot go `in_progress`: it depends on its terminal child `6d82de03`, which is last)
- Wave in progress: **Wave 1 COMPLETE**; ready to start Wave 2 (of 5)
- Children summary: **2 done** (`99f4a2b4`, `d24008f0`), 10 backlog. Bracket nodes `3c4cab67` (plan) + `c939d0b6` (breakdown) already done.
- Active claims: none held by workers; epic assignee = `agent:jit-execution-lead`. `2b9a80fb`, `d24008f0`, `99f4a2b4` are claimed (from dispatch) — re-claim Wave-2 issues fresh as needed.
- Open escalations: none open. Two owner decisions were made and applied this session (see Traps).
- Progress file: `dev/active/2d109173-progress.json` (wave plan + `docs_mechanical_scoping` note). rework_counts: `{99f4a2b4:2, d24008f0:2}`.

## Wave plan (impl interior; from progress file)

- **Wave 1 (DONE):** `99f4a2b4` docs-mechanical gate+checkers, `d24008f0` doc-review scope amendment.
- **Wave 2 (NEXT — 6 disjoint content audits, parallel via worktrees):** `2b9a80fb` CLI catalog, `736a069e` concepts, `7c283e95` reference(-cli), `b8924a6d` how-to, `4c33d0e5` tutorials, `a70bac75` examples.
- **Wave 3:** `adc4c6ef` root relocations (serialized: touches README.md/deployment.md/quickstart.md if INSTALL.md moves) + `8682e95a` gap-fill (dep 2b9a80fb).
- **Wave 4:** `36d5451e` root+component audit (dep adc4c6ef).
- **Wave 5:** `6d82de03` file follow-ups (dep all audits + gap-fill). Then epic completion (Section 10).

## What just happened

- Discovered structure, claimed epic, wrote progress file. Bracket already done; planned over impl interior.
- Wave 1 dispatched both enablers in isolated worktrees (via `dispatch-worker-worktree.sh`, absolute path — see Traps).
- `d24008f0`: prompt change correct first try. code-review FAILed twice on REQ-01 wanting real scoped+unscoped runs. **Owner decided** (interactive) the unscoped behavioral demo is premature → amended REQ-01 to defer it to the audit tasks. Evidence = mechanism trace + one real scoped `codex exec` run (`dev/active/d24008f0-req01-evidence.md`). **DONE.**
- `99f4a2b4`: took **8 code-review rounds**. Findings churned (different subset each round). Resolved by: (a) **owner-approved** amendment of REQ-01/REQ-02 to codify the plan's M3 design (repo-rooted, auditor-adjudicated citation check; clean-footprint zero-exit); (b) lead-direct fixes for the legitimate tail (footprint-required checkers, config-derived default, reference-style link resolution, safe self-test in a throwaway clone, exit-2 env-error semantics, fail-closed on nonexistent/unreadable/unreadable-nested-dir footprints, 12-assertion self-test). **DONE.**
- Attached `docs-mechanical` to the 6 Wave-2 audit tasks + container `2d109173`.
- Re-rendered `docs/reference/rules-and-gates.md` (was stale: missing `doc-review`+`docs-mechanical` gate rows) to unblock M5. `jit invariant render` leaves AGENTS.md untouched.
- **Verified end-to-end:** `DOCS_FOOTPRINT` env forwards through `jit gate evaluate`; scoped docs-mechanical on `docs/tutorials/` PASSES; whole-surface FAILS on the real `.jit/claims.jsonl` defect.

## What to do next

- [ ] **Start Wave 2.** Re-claim + dispatch the 6 content audits in parallel via `dispatch-worker-worktree.sh` (footprints are disjoint dirs; safe to parallelize). Use `references/doc-agent-prompt.md`. Each worker: audit its footprint, run the mechanical bar (M1/M4 embedded + `scripts/docs-check-{links,citations}.sh <footprint>` and `scripts/docs-check-projections.sh`), fix all drift class-wide, pass footprint-scoped doc-review.
- [ ] **Evaluate each audit task's `docs-mechanical` gate with `DOCS_FOOTPRINT=<that task's footprint>`** (e.g. `DOCS_FOOTPRINT="docs/concepts/" jit gate evaluate 736a069e docs-mechanical`). Without the env it runs whole-surface and fails on other areas' defects. VERIFIED this env forwards.
- [ ] `2b9a80fb` (CLI catalog) MUST fix the seeded `.jit/claims.jsonl` miscite (`cli-commands.md`; leases live under `.git/jit/`) — this is the one real citation defect on the surface today.
- [ ] Attach `docs-mechanical` to `adc4c6ef`, `8682e95a`, `36d5451e` before their waves (per plan they carry it).
- [ ] After Wave 2 integrates, run `scripts/check-leak-into-main.sh` and `jit validate`.

## Traps — do not repeat these

- **The `code-review` gate (codex exec) is adversarial and surfaces a DIFFERENT finding subset each round.** `99f4a2b4` took 8 rounds. Expect this on every audit task. Mitigation: fix the whole *class* each round, harden proactively, and read the actual `result.json` stdout (`.jit/gate-runs/<run>/result.json` → `.stdout`), not just the summary. Budget minutes per review (codex is slow).
- **Do NOT re-broaden the citation checker (`scripts/docs-check-citations.sh`) to flag every dangling path.** It is deliberately **repo-rooted + auditor-adjudicated** (plan §2 M3): flags a backtick token only if its first segment is a live-derived tracked repo root and the target is missing; excludes external/command tokens by pattern class; DEFERS bare filenames (`MISSING.md`) and non-repo-root / misspelled-root paths (`crate/jit/...`) to the semantic doc-review reviewer. A prior rework dropped the repo-root requirement and produced **13 false positives** (`~/.config/...`, `/etc/...`, `schemas/*.json`, `lib/*.js`, a `curl` line) that make the whole-surface gate red on non-defects. The reviewer demanded flagging them 3× (rounds 1-3); this is an FP-prone over-reach — owner amended REQ-01 to codify the repo-rooted design. If the reviewer raises it again, it's resolved by the amended criterion; do not re-broaden.
- **`REQ-02 "zero on the current tree"` is amended to mean "zero on a *clean footprint*."** The current whole tree correctly exits nonzero on the real `.jit/claims.jsonl` defect (owned by `2b9a80fb`). That is correct detection, not a checker bug. Do not try to make the whole-surface checker exit 0 before `2b9a80fb` lands.
- **`docs-mechanical` scoping = `DOCS_FOOTPRINT` env, NOT positional in the gate command.** The gate command is bare `./scripts/docs-mechanical.sh`; precedence is positional > `DOCS_FOOTPRINT` env > derived default (`permanent_paths` = `docs/`). The lead supplies the per-task footprint via the env at eval time. Do NOT bake a path list into the gate command or the checker scripts (REQ-01: no hardcoded path lists/counts — a hardcoded depth cutoff `NF>2` was also rejected as a "count").
- **M5 (projection freshness) is GLOBAL, not footprint-scoped** — it always diffs `docs/reference/rules-and-gates.md` + AGENTS.md invariant region regardless of `DOCS_FOOTPRINT`. It was stale (missing gate rows) and blocked docs-mechanical on ALL tasks until re-rendered this session. **`rules-and-gates.md`'s projected region is now fresh** — the `7c283e95` (reference audit) worker must NOT hand-edit that region; re-render with `jit reference render` if needed (idempotent). If a future gate/rule/invariant registry change lands, re-render again or M5 goes red everywhere.
- **`4c33d0e5` has a premature `docs-mechanical=passed` record** (from this session's scoping test over clean `docs/tutorials/`). Re-evaluate it (scoped) after its worker actually audits tutorials; do not trust the stale pass.
- **`dispatch-worker-worktree.sh` lives at the SKILL dir, not project `scripts/`:** `/home/vkaskivuo/.agents/skills/jit-execution-lead/scripts/dispatch-worker-worktree.sh` (same for `check-leak-into-main.sh`). Running `scripts/dispatch-...` from the repo fails.
- **Worker "idle" pings during long `codex exec` runs are NOT stalls.** Check the worktree branch for commits (`git log main..worktree-agent-<id>`) and uncommitted state before re-dispatching. One rework agent went "idle" for ~2 min while a full-surface codex review ran.
- **Many stale worktrees from OTHER sessions/epics exist** under `.agents/worktrees/` (agent-287f7bc9, dw-a…dw-d, etc.). They are NOT this epic's — do not remove them.
- **adc4c6ef (Wave 3) overlaps Wave-2 footprints** at `docs/how-to/deployment.md` and `docs/tutorials/quickstart.md` (and `README.md`) IF the INSTALL.md move executes. It is serialized to Wave 3 for this reason (plan §4 risk row). Keep it out of any Wave-2 parallel batch.

## Open questions needing invoker input

None. (Two owner decisions this session are settled and applied: d24008f0 REQ-01 defer-behavioral-demo; 99f4a2b4 REQ-01/REQ-02 codify-plan-M3-design.)

## Reference artefacts

- Epic: `jit issue show 2d109173`
- Plan (authoritative, §2 = the mechanical bar M1–M5, §3 = decomposition/gates): `dev/active/2d109173-plan.md`
- Investigation (grounded findings, cited): `dev/active/2d109173-investigation.md`
- Planning brief (locked interview decisions): `dev/active/2d109173-planning-brief.md`
- Enabler evidence: `dev/active/99f4a2b4-req02-evidence.md`, `dev/active/d24008f0-req01-evidence.md`
- Checkers: `scripts/docs-check-links.sh`, `scripts/docs-check-citations.sh`, `scripts/docs-check-projections.sh`, `scripts/docs-mechanical.sh`, `scripts/docs-check-selftest.sh`
- Progress: `dev/active/2d109173-progress.json`
