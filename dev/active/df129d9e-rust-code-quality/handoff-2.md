# Handoff — Dispatch layering, executor surface, and build hygiene in crates/jit (df129d9e) — session 2

**Date:** 2026-10-21
**Session number:** 2 (first execution session)
**Prior handoffs:** [handoff.md](handoff.md) (planning)

## Current state

- Epic `df129d9e`: backlog, assigned agent:df129d9e-lead (it can't be claimed while its stories block it).
- Wave 1 of 13. Done: 9ca31c64 (workspace-lints), 83e6cb8d (production-line baseline), bc7eb715 (MCP global json). In progress: 1874e265 (classifier move). An opus worker is on branch `worktree-agent-1874e265` in worktree `.agents/worktrees/agent-1874e265`, based on ace3e76ec. Not started: ef9897ab, 719a5501, f7f01654.
- Leases: agent:df129d9e-lead holds 6h leases on the wave-1 issues.
- Open escalations: none new. The planning handoff's D-21, D-22, D-11, D-12 and D-14 escalations still stand.
- Progress file: [progress.json](progress.json).

## What just happened

- 83e6cb8d: `scripts/production-lines.py` committed; baseline pinned at dc2ea870 (78,801 default / 73,407 flagged) in [line-count-baseline.md](line-count-baseline.md). code-review passed.
- bc7eb715: `buildCliArgs` honours `global_options` json (3f26f295). mcp-ci and code-review passed.
- 9ca31c64: workspace lints merged as e84318f9. It went to main before its gates, which the steward accepted as a one-off. cargo-ci and code-review passed on rerun.
- 1874e265: dispatched to an opus worker. Its prompt is in the lead scratchpad. The common prompt body carries the steward's CI convention.

## What to do next

- [ ] Collect w-1874e265's result: leak check, lead review, merge `worktree-agent-1874e265` --no-ff, cargo-ci (turnstile + exclusive flock), code-review, then done. Reclaim the worktree.
- [ ] Dispatch ef9897ab, 719a5501 and f7f01654, one worktree at a time, only when `df -h /` shows at least 60 GB free.
- [ ] Wave 2: d8176fd3 (global-json-flag, opus) waits on external 2d10f1dd, 16d63038, 3754095f and 8673dc5e; 650e6a56 and 233f4a09.

## Traps — do not repeat these

- **Never implement on main.** The steward forbade it after the lints edits blocked other leads' dispatch and gates. With no worktree allowed, only review, gate and plan.
- **Steward resource policy is binding:** at most 1 live worktree; dispatch only when at least 60 GB is free; `CARGO_INCREMENTAL=0`; full gates run as `flock /tmp/jit-ci-turnstile.lock flock -x /tmp/jit-full-ci.lock <cmd>`; other cargo runs as `flock /tmp/jit-ci-turnstile.lock true && flock -s /tmp/jit-full-ci.lock <cmd>`; `--force` on cargo-ci only for load-only failures.
- **The dispatch script refuses main while other leads' `.jit/issues` files are dirty.** That happens constantly. Use a copy whose clean check ignores `.jit/`, and never commit or stash their files.
- **A code-review failure with "Agent produced no output" is infra**, for example codex "Selected model is at capacity". Retry with 15+ minute back-off, never pass manually, and keep the issue gated.
- **jit commands hit `.jit-bootstrap.lock` timeouts** while another lead's gate runs. Retry them; that is not an error.
- **The cargo-ci test failure in profile.rs:8234** came from d6bbe05f (another epic) and was fixed in af530c67. Check `git log` before blaming your own change.
- **Handler tasks** must not be leased before the steward's 2195c98f `cli-issue-id-boundary` edges exist, unless their prerequisites are done. 0eca332a also waits on `error-code-vocabulary`.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show df129d9e`; plan: [plan.md](plan.md); breakdown: [breakdown.json](breakdown.json)
