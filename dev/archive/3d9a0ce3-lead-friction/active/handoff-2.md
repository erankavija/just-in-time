# Handoff — Remove execution-lead friction (3d9a0ce3) — session 1 of execution

**Date:** 2026-10-22
**Session number:** 2 (first execution session; [handoff.md](handoff.md) is the planning handoff)
**Prior handoffs:** [handoff.md](handoff.md)

## Current state

- Epic `3d9a0ce3`: backlog (blocked by its children), assigned to `agent:jit-execution-lead`. The lease has TTL 0.
- Done (17 including bracket nodes): ae2e3739, fd9bfab8, 66986f88, d6bbe05f, d1ff2273, 72d89161, f9ab44aa, 5a3a886f, 24268b05, 8673dc5e, 2dd6b833, f37d228e, 3754095f, 1a989754, 494f1d4b, plus b5ba1b61 and a3437bab.
- In progress: f866d9d8 (three-way removal), with an opus worker in `.agents/worktrees/agent-f866d9d8`.
- Ready, not dispatched: 39710b3b, d5966ba2, 7f9268ed.
- Waiting on other epics: dbbfcd6c depends on 2195c98f (re-home pending, steward). 5f82e09a was re-homed onto 3a3b86cf (f4d9502a). da12ad66 depends on 5f71a48d.
- Created during execution: 494f1d4b (steward-approved, done).
- Progress file: [progress.json](progress.json).

## What just happened

- Wave 1 merged. Every item failed a review once on documentation or rustdoc, and each got one rework.
- Steward escalations, all resolved:
  - Load-only cargo-ci failures: 494f1d4b was created and is done.
  - Dogfood binary reinstall: repeated after CLI-surface changes.
  - 8673dc5e passed its rework limit on REQ-03. Resolved by a steward-ruled one-off conversion (e2f2693d8): 2405 + 5 gates_status entries were pointed at existing runs, and 210 attested runs were published from entry fields. The tool is out of tree: `scratchpad/convert/target/release/gate-run-id-backfill <repo>`, which is idempotent.
- The lead made two direct changes on main (now forbidden, see Traps): the d6bbe05f test fix and the 5a3a886f registry. Both were later done properly through branches.

## What to do next

- [ ] When the f866d9d8 worker returns: merge main into its branch in its worktree, run the workspace tests there, merge into main under `flock /tmp/jit-ci-turnstile.lock flock -x /tmp/jit-full-ci.lock git merge ...`, then evaluate cargo-ci, mcp-ci and code-review.
- [ ] Then the serial profile chain from the progress file: 5179374d, then 3f217dd2 and 5c0bd3ad, abe963dd, 0dcabcca, 56b89d9a, and so on. Each task's lead notes are in `scratchpad/prompts/<id>.extra`; regenerate those from progress.json if the scratchpad is gone.
- [ ] Fill gaps with 39710b3b, d5966ba2 and 7f9268ed, one live worktree at a time.

## Traps — do not repeat these

- **Do NOT edit, resolve conflicts or merge on main outside the CI locks.** A conflicted merge left on main blocked every lead. Merge main into the task branch inside its worktree, resolve and test there, then merge into main with `flock /tmp/jit-ci-turnstile.lock flock -x /tmp/jit-full-ci.lock git merge --no-ff ...`.
- **Do NOT merge a deletion or a config change without running the affected test modules.** The d6bbe05f deletion and the 5a3a886f gates.toml change each turned main red, through tests in `profile.rs`/`profile_cli_tests.rs` and `fast_docs_templates/template_apply_tests.rs`. Run the whole workspace suite on the merged branch first.
- **Do NOT treat mcp-ci failures right after a CLI-surface merge as code findings.** mcp-ci reads `jit --schema` from the installed dogfood binary. Ask the steward to reinstall, then `jit gate evaluate <id> mcp-ci --force`. Option B, build-then-test in the gate, is pending with the user.
- **Do NOT leave docs for a removed command to the later docs tasks.** code-review fails the deletion; this happened on 24268b05 and d6bbe05f. The prompt generator now says so.
- **Do NOT --force a cargo-ci failure on a quiet host.** Force only load-only failures (TIMEOUTs, suite budget). `cli_warnings_integration_tests` take about 6 s each and time out under concurrency.
- **Respect the steward resource rules:** at most 1 live worktree (2 only with approval), at least 60 GB free to dispatch, `CARGO_INCREMENTAL=0`, full gates under the turnstile plus exclusive flock, other cargo under the turnstile plus shared flock.
- **`jit claim acquire` sets the assignee.** Lease issues with `--agent-id agent:worker` so the assignee stays put.

## Open questions needing invoker input

- Option B for mcp-ci (build the binary before the MCP tests): recorded by the steward for the user.
- Adopter repositories hold legacy gates_status entries without run_id. The steward recorded this for the user.

## Reference artefacts

- Epic: `jit issue show 3d9a0ce3`
- Plan: [plan.md](plan.md). Manifest: [breakdown.json](breakdown.json). Progress: [progress.json](progress.json).
