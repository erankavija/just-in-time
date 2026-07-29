# Handoff — Core maintenance (6eb585bc) — session 4

**Date:** 2026-07-16
**Session number:** 4 (wave-A collection + wave-B dispatch)
**Prior handoffs:** dev/active/6eb585bc-handoff.md, dev/active/6eb585bc-handoff-2.md, dev/active/6eb585bc-handoff-3.md (their Traps remain in force except where superseded below)

## Current state

- Epic: `6eb585bc` — in_progress; finite v1.0 prerequisite (@/charter/D-14).
- Wave in progress: wave A fully merged; wave B all dispatched. A detached gate sweep (take 5) is evaluating the merged issues' gates.
- Children summary: 24 done (incl. 8d7fc762 + 16402e14 this session), 10 in_progress, 1 ready (94096aac, filed this session), 0 rejected.
- Active claims: epic (agent:jit-execution-lead); agent:worker on 8917c558 52665a07 3e12ffbd 45a140ae 0daba57d d74a9ed1 1d59070d 894337e2 6f881a85 450db193 0283ce74 c505031a 554ad07f.
- Open escalations: none.
- Progress file: `dev/active/6eb585bc-progress.json` (current).

### Merged to main this session (all verify-commit-builds green, leak checks clean)

| Issue | Merge | Gates at handoff |
|---|---|---|
| 8d7fc762 | prior session (0a468803) | all green → **DONE** |
| 16402e14 | 4e7dc03e | all green → **DONE** |
| 8917c558 | 1e72fbf1 + lead fixes f23479ed, 2a5aff7e | cargo-ci/code-review/docs-mech green; doc-review failed pre-2a5aff7e, re-run pending |
| 52665a07 | 74071a9c + lead doc fix d3d64146 | sweep evaluating |
| 3e12ffbd | 1c26833b; design doc linked 7ca34a54 | sweep pending |
| 45a140ae | ffde113d; leaked .jit-bootstrap.lock untracked c03d1ba9 | sweep pending |
| 0daba57d | 78a6181a | sweep pending |
| d74a9ed1 | 271c650b | sweep pending |
| 1d59070d | 6b24ca42 | sweep pending |
| 894337e2 | 9934f89d + rework merge dbe1a50b (fd inheritance) | cargo-ci green (pre-rework); code-review failed pre-rework, re-run pending |

### Worker states (in-flight, session-bound — they die with this session)

| Issue | Worktree state | Next lead action |
|---|---|---|
| 450db193 | committed 8aa61b84 (generic [projection.*] + jit project render), clean; cargo-ci on flock at handoff | collect report / desk-review + merge |
| 0283ce74 | 4 decision commits (3 removals + cycle-guard exit-4 fix), clean; cargo-ci running | collect report / desk-review + merge |
| c505031a | ~29 dirty files, functionally complete per its status msg (aliases removed, MCP verified, bonus schema.rs gate_check-all key fix); waiting to commit after cargo-ci | expect report; if session died first, review dirty tree + commit lead-preserve |
| 554ad07f | committed 99072933 (tree_dirty in gate-run evidence), cargo-ci green, desk-review **PASS** by this lead | merge + gates directly |
| 6f881a85-r1 | replacement worker dispatched into worktree (7 uncommitted files from wedged r0); no commits yet at handoff | check progress; if dead, re-dispatch from the r1 prompt in this handoff's session |

## What just happened

- 46657f6f landed prior session-3 continuation; docs-mechanical unblocked → completed 8d7fc762 (all 5 gates green).
- Desk-reviewed + merged 7 worktrees (16402e14, 8917c558, 52665a07, 3e12ffbd, 45a140ae, 0daba57d, then d74a9ed1, 1d59070d) and the 894337e2 rework; verify-commit-builds after every merge.
- 894337e2 rework (attempt 1): post-merge code-review F1 (listener dropped before child re-bind) fixed via fd-inheritance handoff (listenfd protocol, child adopts the parent's bound socket); merged dbe1a50b; gates re-run pending in sweep.
- 8917c558 rework (attempt 1, lead-direct): code-review F1 — re-add preserved a stale commit pin; fixed f23479ed (pin always reflects the invocation). Follow-up doc-review F1 — stale "defaults to HEAD" claim; fixed 2a5aff7e. doc-review re-run pending.
- 52665a07 integration fix (lead-direct): docs-mechanical M3 failed repo-wide on the how-to's hypothetical `.jit/config/gate-presets/rust-ci.json` citation; replaced with placeholder notation (d3d64146).
- Filed 94096aac (foreground-serve bootstrap-lock deadlock, pre-existing, flagged by 894337e2 rework worker); wired under epic with cargo-ci + code-review.
- Dispatched wave B: 450db193 (anchor 7ca34a54), 0283ce74 (04e45b89), c505031a + 554ad07f (c86fdd54). Replaced the wedged 6f881a85 worker with a fresh agent in the same worktree.
- Nudged stalled workers 3× (6f881a85 r0 twice — wedged, replaced; 1d59070d/d74a9ed1 once — turned out already complete, merged); 1 false-positive nudge (c505031a was flock-queued, not stalled).
- Gate sweep went through 5 takes; takes 1–4 were killed by environment races (see Traps); take 5 (clean-worktree installs) is running and green so far (16402e14 all passed → DONE; 8917c558 3/4).
- Isolated full-workspace test run on merged main: 0 failures (the gate's earlier test failure was the shared-cache flake).

## What to do next

- [ ] Let gate sweep take 5 finish (`scratchpad/gate-sweep.sh` in this session's scratchpad; log beside it). Then relaunch the same script once — its SKIP-ALREADY-PASSED logic re-runs only failed/pending gates (8917c558 doc-review and 894337e2 code-review re-runs are queued this way; both have fixes already merged).
- [ ] After sweep: `jit issue update <id> --state done` + commit for every issue with all gates green (16402e14 pattern).
- [ ] **3e12ffbd round-1 findings are ALREADY FIXED on main** — code-review F1 (scoped json/dot/mermaid leaked boundary edges) fixed d291bf11 with regression test `test_scoped_graph_exports_carry_no_out_of_scope_edges`; doc-review F1 (pipe-into-batch-create claim) fixed 76bed774. Also the resolve_listener env race c89133aa. Only gate RE-RUNS are needed for 3e12ffbd; do not re-fix.
- [ ] **45a140ae round-1 findings are ALREADY FIXED on main** — code-review F1 (DependencyTreeNode lacked archived_from / effective-terminal symbol) + F2 and doc-review F1–F4 (met-predicate prose) fixed in 2856ad0f + cea1dbfa, sweeping ~15 doc copies total. core-model.md:793 deliberately kept literal (describes the re-scan trigger, not the met predicate) — if a reviewer flags it, cite that distinction or reword the trigger sentence, do not change the semantics. Only gate RE-RUNS needed; rework count 45a140ae → 1.
- [ ] **0daba57d round-1 doc-review findings ALREADY FIXED on main** — F1/F2 (missing issue-delete reference section + DELETION_NOT_CONFIRMED envelope docs) fixed 7d19cf69, refusal exit codes live-verified in both modes. cargo-ci/docs-mechanical/code-review already green; only the doc-review RE-RUN is needed; rework count 0daba57d → 1.
- [ ] **d74a9ed1 round-1 findings ALREADY FIXED on main** — code-review F1 (sync could strand rules.toml when a custom rule fails full load after config.toml saved; now identity-only via ruleset_store::read_rule_identities + regression test test_sync_survives_custom_rule_that_fails_full_load) and F2 (sorted diff) in d9d67143; doc-review F1 (write-through documented across init/config-set refs, configuration.md, labels.md, example-config.toml; init --json modified_paths claim corrected to shipped reporting) in d9d67143 + a1adb140. Only gate RE-RUNS needed; rework count d74a9ed1 → 1.
- [ ] **1d59070d docs-mechanical failure at 19:09 was transient** — it scanned MY in-flight working-tree doc edit (`@/rule/namespace-unique-<ns>`, fixed to the `<name>` placeholder form in a1adb140; standalone docs-mechanical passes). Re-run only. TRAP reminder: a templated `@/…` citation whose id STARTS with literal text dangles under M3; ids beginning with `<placeholder>` are exempt.
- [ ] **1d59070d code-review round 1 — analyzed, fix designed, NOT yet implemented** (session stopped by invoker at this point). F1: in `pass_gate` (crates/jit/src/commands/gate.rs:~400) the already-passed-at-HEAD short-circuit runs BEFORE the manual-gate --by check, so a stale AUTO-era pass bypasses attestation after the gate is redefined to manual. Designed fix: load the gate registry BEFORE the short-circuit (reuse it in the auto arm); allow the short-circuit for a Manual-mode gate only when the recorded pass is attested (`gates_status.updated_by` is Some and != "auto:executor"); mint a shared `AUTO_EXECUTOR` const at the stamping layer (gate_execution.rs:183 has the literal; also gate_check.rs:535 and 3 gate_runs.rs sites) and use it in the check. Genuinely-attested manual passes keep skipping (evaluate-all bare must not error on an already-attested manual gate); stale auto passes fall through to the ManualGateAttestationRequiredError. Add a regression test: auto gate passes at HEAD → redefine to manual → bare evaluate errors exit 2, --by records a fresh attested run. Its doc-review had not run yet at stop time.
- [ ] **52665a07 rework round 1** (code-review FAILED post-handoff, run in `.jit/gate-runs`, 2 findings): F1 — REQ-03's "retention stated where the domain-agnostic boundary is documented" needs the boundary statement in the @/inv/domain-agnostic invariant text itself (.jit/invariants.toml + re-render projection), not only in builtin.rs module docs; the trio's retention IS sanctioned by the issue's pinned intake decision, so amend the invariant text per REQ-03 rather than removing the trio. F2 — REQ-02 requires TestHarness (in-process) end-to-end coverage of a project-defined preset; the worker's CLI-integration test doesn't satisfy the literal criterion — add a fast-suite harness test (save/list/show/apply). doc-review ALSO failed (3 findings, same rework round): D1 — the preset-list output example at docs/reference/cli-commands.md:1806 omits the breakdown-review builtin; D2 — CLI help at crates/jit/src/cli.rs:1786 pipes human-readable `jit query all` output to xargs instead of extracting ids from JSON (use the `--json | jq -r '.issues[].id'` form the docs use elsewhere); D3 — docs/how-to/custom-gates.md:617 implies the plan template ships, without stating templates.toml is optional project-declared configuration. Fix all five (F1, F2, D1–D3) in ONE submission, then re-evaluate code-review + doc-review.
- [ ] Collect the 5 in-flight workers (session-bound — inspect worktrees per the table above): desk-review per lead-review-protocol, merge sequentially with verify-commit-builds after EACH merge, evaluate gates post-merge. 554ad07f is already desk-reviewed PASS — merge directly.
- [ ] Merge-order caution: c505031a (alias removal) and 1d59070d landed overlapping gate-surface files — c505031a's anchor (c86fdd54) already contains 1d59070d, so it merges clean; merge 450db193 before c505031a only if conflicts demand an order (both touch cli.rs/docs).
- [ ] Dispatch nothing new: every open child is merged, in-flight, or 94096aac (ready — dispatch when a worker slot frees).
- [ ] Epic completion per Section 10 once all children are done: completion report, epic gates, close 6eb585bc (@/charter/D-14), archive progress/handoffs, jit doc add.

## Traps — do not repeat these

- **The stale-binary guard refuses `dirty_build` provenance, and the parallel codex session keeps main's working tree dirty** (its progress files, .jit writes) **and moves HEAD mid-sweep.** Any `install → evaluate` chain from the shared checkout is a race. Working recipe (gate-sweep take 5): install from a dedicated clean worktree `.agents/worktrees/lead-install-clean` synced to main HEAD (`git -C <wt> checkout --detach $(git rev-parse HEAD)` then run `scripts/install-jit.sh` from inside it), verify with `jit --version` (must show `commit == HEAD, dirty=false`) before every evaluation, and NEVER merge/commit on main while a sweep is mid-flight — batch completions after SWEEP COMPLETE. Takes 1–4 died to: my own claim/progress commits after install (take 1), install racing my merges (takes 2–3 family), codex's dirty tree at install time (take 4, `reason: dirty_build`).
- **Do NOT `git add -A` auto-commit from a background script to clean the tree for installs** — it would sweep up the codex session's in-flight files; the permission classifier denies it and the denial is correct. The clean-worktree install above is the sanctioned route.
- **`target/debug/incremental` is repopulated mid-session by the codex session's gate runs from the repo root** (verified live: its `jit gate evaluate <its-issue> --force` at 17:16). cargo-ci's incremental-state step then fails. Pre-clean `find target -type d -name incremental -exec rm -rf {} +` immediately before EVERY cargo-ci evaluation, not once per sweep.
- **cargo-ci "test: FAILED (exit 101)" with no named tests under parallel worker load is the shared-cache flake family** (handoff-3 trap extended). Confirm with an isolated run (`CARGO_TARGET_DIR=~/.cache/... cargo test --workspace`) before treating it as a defect — this session's only such failure was clean in isolation. RESOLVED in part: several of these were actually the `resolve_listener_tests` LISTEN_* env race (listenfd's `from_env` consumes `LISTEN_FDS`/`LISTEN_PID`, so the adopt/no-fd tests raced process-global env; the 554ad07f worker misattributed the same failures to incremental-state). Fixed c89133aa (shared lock + env scrub); if these two tests flake again, the fix regressed.
- **Idle ping + no cargo process is NOT sufficient evidence of a stall: check for `flock` waiters too.** c505031a was flock-queued on `/tmp/jit-cargo-ci.lock` (the shell holds `flock`, no cargo yet) and got a needless nudge. Correct probe: `ps aux | grep -E "cargo|flock"` mapped to worktrees via `/proc/<pid>/cwd`, plus `git log`/`status` delta since dispatch.
- **A worker that ignores two nudges with an unchanged tree is wedged — replace it, don't keep nudging.** 6f881a85 r0 idled through 2 nudges with 0 commits; the replacement agent picked the worktree up cleanly.
- **`jit doc add` re-add semantics: the commit pin always reflects the invocation** (omitted --commit = unpinned; f23479ed). Two review rounds were spent because the worker's partial-update convention and the pre-existing "defaults to HEAD" help text both contradicted the steward re-point workflow. If any doc/help text about doc add resurfaces "defaults to HEAD", it is wrong.
- **Merged-worktree branches can carry tracked junk — check `git show --stat` for lock files before merging.** 45a140ae's merge introduced a tracked `.jit-bootstrap.lock` (untracked again in c03d1ba9); .gitignore already listed it, the worker had force-added it.
- **docs-mechanical M3 treats any backticked repo-rooted path as a citation that must exist on disk** (scripts/docs-check-citations.sh rule 1; placeholder notation `<name>` is exempt class (a)). Hypothetical example paths in docs must use placeholders — hit live on 52665a07's `.jit/config/gate-presets/rust-ci.json`.
- **zsh `===` separator trap re-hit AGAIN this session** (`echo ===H2===` → `no matches found`). Use plain words or `-----`. Also `pgrep -f <pattern>` matches the calling shell's own command line — use `ps aux | grep | grep -v grep`.
- Unresolved from prior handoffs: reviewer batch-enumeration on output contracts (handoff-2); /tmp quota + background-Bash 10-min cap (handoff-2; this session used `setsid nohup` + `~/.cache` target dirs throughout); `jit issue status` may omit gates (handoff-2 — Tier-1 must read `gate status-all`); rust-analyzer diagnostics are stale buffer snapshots (handoff-1; re-verified — `verify-commit-builds` is the truth).

## Open questions needing invoker input

None. (User directive mid-session: finish the current wave, then hand off — this handoff is that close-out; the sweep and 5 workers were still in flight at handoff time and the next session collects them.)

## Reference artefacts

- Epic: `jit issue show 6eb585bc`; progress: dev/active/6eb585bc-progress.json (session_4 key; rework counts: 894337e2→1, 8917c558→1 this session).
- Gate sweep: script + log in this session's scratchpad (`/tmp/claude-1000/-home-vkaskivuo-Projects-just-in-time/729b1d1b-9963-4e92-a27a-4a5ec67e0563/scratchpad/gate-sweep.{sh,log}`) — scratchpads are session-scoped; the take-5 recipe is reproduced in the Traps entry above if the files are gone.
- Clean install worktree: `.agents/worktrees/lead-install-clean` (detached; sync before use).
- New issue: `jit issue show 94096aac` (foreground-serve deadlock; full analysis in its description).
- Worker design notes on main: dev/active/45a140ae-archived-semantics.md, dev/active/3e12ffbd-batch-export-design.md, dev/active/d74a9ed1-write-through-namespace-unique-membership.md (all doc-linked to their issues).
- Dispatch/leak/build-verify protocol: ~/.claude/skills/jit-execution-lead/references/worktree-dispatch-protocol.md.
