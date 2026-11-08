# Handoff — Dispatch layering, executor surface, and build hygiene in crates/jit (df129d9e) — session 3

**Date:** 2026-10-21
**Session number:** 3 (second execution session)
**Prior handoffs:** [handoff.md](handoff.md) (planning), [handoff-2.md](handoff-2.md)

## Current state

- Epic `df129d9e`: backlog, assigned agent:df129d9e-lead.
- Waves 1 and 2 are done except `d8176fd3` (global-json-flag). It waits on external `3754095f` (backlog, behind `f37d228e`) and `8673dc5e` (in progress, another lead).
- Every wave-3+ task depends on `d8176fd3`, so `jit query available --label epic:rust-code-quality` is empty. The epic is blocked until those two external bugs close.
- No live worktree. No df129d9e task lease is held.
- Progress file: [progress.json](progress.json).

## What just happened

- Done this session:
  - 1874e265: classifier into `output::failure`, plus a lead rustdoc fix in errors.rs.
  - ef9897ab: harness at `crates/jit/src/test_harness.rs`.
  - 719a5501: verb stubs removed.
  - f7f01654: `ListEnvelope` in `domain/list_envelope.rs`.
  - 233f4a09: schema omits clap-hidden items.
  - 650e6a56: 24 output structs as `ListEnvelope` aliases. Code-review R1 failed on marker rustdoc that restated each key. The lead fix 79713c95a passed.
- Steward rulings applied:
  - D-24: 9fa12e75 REQ-01/REQ-03 narrowed after 72d89161. The `real_repo_root` reference became `real_repo_layout` in investigation.md and a24b11aa.
  - D-25: f7f01654 REQ-04 voided, because 24268b05 deleted `ProfileAgreement`.
- Other leads were told about the new harness path. Their 2195c98f and 842b0ac6 manifests still name `tests/common/harness.rs`.

## What to do next

- [ ] When `3754095f` and `8673dc5e` are done, dispatch `d8176fd3` (opus, `cargo-ci`, `code-review`, `mcp-ci`).
- [ ] Then wave 3: 27f0a575, 2f15d48d, 9fa12e75 (amended criteria), 9cc3056a, d4373d07, 2e7e1348, a807e699, 5d0a9a90. Run one worktree at a time.

## Traps — do not repeat these

- **The scratchpad root is shared with other sessions.** Another session overwrote `scratchpad/review.sh` mid-session with a different script, which leases as `agent:worker`. Keep lead scripts in `scratchpad/df129-lead/`: `review.sh <sid>`, `mkprompt.sh`, `dispatch.sh` (clean check ignores `.jit/`) and `jr.sh`.
- **mcp-ci tests whatever `jit` is first on PATH, and its digest excludes that binary.** Evaluating against the stale dogfood binary pins a failed run, and later evaluations reuse it by digest. The auto-mode classifier denied `--force` for mcp-ci. Always `cargo build -p jit --bin jit` at HEAD first, then run `PATH=$PWD/target/debug:$PATH jit gate evaluate <id> mcp-ci`.
- **Merges into main run under the lock**: `flock /tmp/jit-ci-turnstile.lock flock -x /tmp/jit-full-ci.lock git merge --no-ff ...`. First check `git merge-tree --write-tree main <branch>`. On conflict, merge main into the branch in its worktree. Never resolve conflicts in main's checkout.
- **cargo-ci flake**: six profiled-fixture tests (`test_utils::tests::test_profiled_*`, `fast_rules cli_warnings_integration_tests::test_create_*`) sometimes time out at 20 s together at low load. They pass in isolation in about 2 s. A timeouts-only failure was rerun with `--force` and passed (1874e265, f7f01654). A budget failure on a quiet host is a real finding and gets no `--force`.
- **Workers can die on the usage limit with uncommitted work.** Tell workers to commit per step. A SendMessage to a named worker resumes it. Detect idleness with `find -mmin`, not `-newermt '-30 minutes'` (that form is invalid and matches nothing). Check `flock`/`cargo`/`node` cwd under `/proc`, because the worktree path does not appear in their args.
- **jit lease mechanics**: `jit claim renew` and `release` need `JIT_AGENT_ID=agent:df129d9e-lead`. `jit issue unassign` and `update` need a held lease, so unassign before releasing. Other leads sometimes commit this epic's gate-run dirs in their own commits; that is harmless.
- **Workers do not see main move.** Tell them to `git merge main` in their worktree before the final commit. f7f01654 needed a conflict merge after 24268b05 deleted its subject.
- Handoff-2 traps still in force: never implement on main; steward resource policy; reviewer infra back-off; bootstrap-lock retries; 2195c98f edge gating for handler tasks.

## Open questions needing invoker input

None. The epic is blocked on external `3754095f` and `8673dc5e`.

## Reference artefacts

- Epic: `jit issue show df129d9e`; plan: [plan.md](plan.md) (D-24, D-25 added); breakdown: [breakdown.json](breakdown.json)
