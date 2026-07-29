# Handoff — Repository-state quality hardening (1cc809de) — session 1

**Date:** 2026-07-24
**Session number:** 1
**Prior handoffs:** None.

## Current state

- Epic: `1cc809de` — state: **in_progress** (claimed `agent:jit-execution-lead`).
- Wave in progress: **wave 1 of 8** (dispatched; workers running in background, NOT yet reviewed or merged — this session did no reviews by instruction).
- Children summary: 35 impl-interior issues + 5 story checkpoints. Bracket nodes done (planning `02dc4bac`, breakdown `24bab642`). All impl issues backlog/ready except the 6 wave-1 tasks now claimed+dispatched.
- Active claims (`agent:worker`, all worktrees recreated at base commit `a95dd58a` after the prune incident below; agents RESUMED and running):
  - `4b2005fe` retry-combinator-foundation → worktree `.agents/worktrees/agent-4b2005fe`, branch `worktree-agent-4b2005fe`, agent `a47904c2abe5eb2c7`
  - `9fda1f86` total-journal-action-extraction → `agent-9fda1f86`, agent `a3997b30440e4c766`
  - `eefbfe84` producer-error-family → `agent-eefbfe84`, agent `a153a21640ca7b6c1`
  - `fee3c528` plan-identity-tail (REQ-04) → `agent-fee3c528`, agent `adacec6613c17bc6f`
  - `73981310` benchmark-harness → `agent-73981310`, agent `a906fc9be44c1a1cf`
  - `fc744df6` lock-hygiene-sidecar → `agent-fc744df6`, agent `ace5b3e2b588edb01`
- Open escalations: None.
- Progress file: `dev/active/1cc809de-progress.json` (full 8-wave plan; wave-1 issues marked `dispatched` with worker agent IDs).
- Git: current `main` HEAD `a95dd58a`. Worker worktrees anchored there. Dogfood binary at HEAD-ish (`77329379`); REINSTALL from clean main before any gate evaluation once wave-1 merges land.

## What just happened

- Discovery: epic already broken down; plan `dev/active/1cc809de-plan.md` complete, all bracket gates passed. Confirmed no unresolved design questions (PD-1 lock-hygiene=eliminate sidecar; PD-2 vendor deck assets; REQ-04 has recorded-exemption exit path owned by `fee3c528`).
- Extracted full dependency DAG; built 8-wave plan over the impl interior (excludes bracket nodes P/B). Fan-out order per plan: **S1+S3 parallel → S2+S4 after S1 story → S5 last**.
- Reinstalled jit from clean main (`./scripts/install-jit.sh`) → provenance now `commit 77329379 dirty=false`, then advanced by two lead commits.
- Claimed epic, set `in_progress`, wrote progress file (commit `1c2281ce`).
- Created 6 wave-1 worktrees via `dispatch-worker-worktree.sh` (all verified anchored at `1c2281ce`). Claimed 6 issues, committed claims (`9ed43066`).
- Dispatched 6 background `general-purpose` workers with full verbatim specs + shared-contract context.
- **INCIDENT (recovered):** ran a stale-worktree prune to clean ~50 leftover `agent-*` worktrees from prior planning/breakdown sessions. The exclusion filter for the 6 active worktrees SILENTLY FAILED (zsh does not word-split unquoted `$PROTECT` — `for p in $PROTECT` iterated once over the whole string), so all 6 active worktrees were force-removed mid-task. Diagnosed via leak-check + a reproduction. No committed work lost (workers had 0 commits); uncommitted edits lost. Verified NO worker leaked into `main` (leak-check showed only lead files). Stopped the 6 broken agents, deleted the 6 anchor-only branches, recreated 6 fresh worktrees at `a95dd58a`, and RESUMED all 6 agents from transcript (SendMessage) so their built-up context was reused rather than re-dispatched cold. The 50 genuinely-stale worktrees WERE successfully pruned.

## What to do next

- [ ] **Wait for the 6 wave-1 workers to finish**, then for EACH: run Section 7 six-tier lead review (`references/lead-review-protocol.md`). This is the first review of each issue → skip Tier 1.5 only, run all other tiers.
- [ ] Review sequence to reduce merge pain: merge S1 producer/plan-identity pair carefully (see Trap on `mod.rs` overlap). Suggested merge order: `9fda1f86`, `73981310`, `fc744df6` (disjoint files) first; then `4b2005fe`; then `eefbfe84` and `fee3c528` (resolve `repository_state/mod.rs` overlap on the second).
- [ ] Per merge: `git merge --no-ff worktree-agent-<id>` then `scripts/verify-commit-builds.sh` BEFORE the next merge (Step 5 of worktree-dispatch-protocol). Then run gates: `jit gate evaluate <id> cargo-ci` and `jit gate evaluate <id> code-review` (sequential, explicit cwd = repo root — see Traps).
- [ ] After all wave-1 merges land on main, **reinstall the binary** (`./scripts/install-jit.sh` from clean main) before evaluating any gate — merged Rust changes make the installed binary stale.
- [ ] Run `.agents/skills/jit-execution-lead/scripts/check-leak-into-main.sh` after the wave completes and before committing on main (leak snapshot: `/tmp/lead-pre-dispatch-latest.txt`, taken at dispatch).
- [ ] Complete passing issues (`jit issue update <id> --state done`), advance progress file to wave 2, dispatch wave 2 (`a4e5ca3d`, `a6ee4e23`).

## Traps — do not repeat these

- **The Bash tool runs under zsh, which does NOT word-split unquoted `$var`.** `for p in $LIST; do ...; done` iterates ONCE over the whole string, so any per-token match silently fails. This destroyed 6 active worktrees this session (a worktree-prune "exclude the active ones" filter matched nothing). When you must iterate a token list in a shell command, use an explicit array (`arr=(a b c); for p in "${arr[@]}"`), zsh split (`${=LIST}`), or `printf '%s\n' $LIST | while read`. NEVER trust unquoted `$var` splitting, and NEVER force-remove worktrees based on an unverified exclusion — echo the to-remove list AND the to-keep list and eyeball both before removing.
- **Do NOT prune worktrees while workers are live in them.** `git worktree remove --force` runs `rm -rf` on the dir out from under a running agent; its uncommitted work is unrecoverable and it may fall back to the main checkout (leak risk). If a prune is needed mid-wave, stop the affected agents first or exclude their worktrees with a verified filter.
- **`eefbfe84` and `fee3c528` both edit `crates/jit/src/repository_state/mod.rs`.** Both workers were told to keep edits localized (error-type defs + `Producer` variant + sink-helper deletions for eefbfe84; `MaterializationRequest`/plan-identity routing region for fee3c528), but a merge conflict on `mod.rs` is still likely. Merge one, build-verify, then merge the second and resolve — do NOT dispatch a third worker to "fix" it.
- **Do NOT evaluate gates with a stale installed binary.** The stale-binary guard requires `commit == HEAD, dirty=false`. Every wave that lands Rust changes invalidates the installed binary — reinstall from clean main before `jit gate evaluate`. (This session already hit this: binary was at `276b0674` vs HEAD `77329379`; reinstalled.)
- **Do NOT run `jit gate evaluate` in parallel or with a drifting cwd.** Parallel evaluations lose results to per-issue locks; a non-repo-root cwd once wrote gate evidence into an install worktree's `.jit`. Evaluate sequentially, always from the repo root.
- **Worker worktrees are anchored at `a95dd58a` (= current `main` HEAD).** After recovery the worktrees were recreated at HEAD, so there is no stale-base offset. Merge normally with `--no-ff` + `verify-commit-builds.sh`.
- **Stale `agent-*` worktrees have been pruned** (50 leftovers from planning/breakdown removed this session). Only the six wave-1 worktrees remain. If new stragglers appear, apply the zsh word-split trap above before any bulk removal.
- **REQ-04 (`fee3c528`) exemption is plan-amendment-only.** If the worker reports Initialize/ApplyProfile genuinely cannot fold into the plan-identity tail, that is an escalation for a formal plan amendment — NOT an in-task exemption and NOT a lead decision to wave through.
- **`28254964` label/parent mismatch (structural, not blocking).** It carries `satisfies:REQ-12` (an S4/hygiene criterion) but is DAG-parented under story `2958105e` (S2). Ordering is correct (shared foundation for S2 cutover guard + S4 demotions). Recorded in progress file `surfaced_pitfalls`; surface in the epic completion report, do not "fix" mid-flight.

## Open questions needing invoker input

None. (The invoker directed: do the handoff now, let workers complete, no reviews this session.)

## Reference artefacts

- Epic: `jit issue show 1cc809de`
- Plan/design: `dev/active/1cc809de-plan.md`
- Breakdown manifest: `dev/active/1cc809de-breakdown.json` (+ `-keymap.json`)
- Progress file: `dev/active/1cc809de-progress.json`
- Audit (finding-level source of truth): `dev/studies/cdc840ad-audit-2026-07-23.md`
- Session-cost template artifact: `dev/studies/perf/session-cost-27ffbd2d.json`
- Dispatch snapshot for leak check: `/tmp/lead-pre-dispatch-latest.txt`
- Protocols: `references/worktree-dispatch-protocol.md`, `references/lead-review-protocol.md`, `references/escalation-policy.md` (MAX_REWORK_ATTEMPTS=2)
