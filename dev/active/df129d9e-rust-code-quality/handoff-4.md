# Handoff — Dispatch layering, executor surface, and build hygiene in crates/jit (df129d9e) — session 4

**Date:** 2026-10-21
**Session number:** 4
**Prior handoffs:** [handoff.md](handoff.md), [handoff-2.md](handoff-2.md), [handoff-3.md](handoff-3.md)

## Current state

- External blockers 3754095f and 8673dc5e are done. d8176fd3 (global-json-flag) is in progress, leased to agent:df129d9e-lead (TTL 4 h, from 20:57 UTC).
- Worktree `.agents/worktrees/agent-d8176fd3` (branch `worktree-agent-d8176fd3`, base 188be65ee) is the one live worktree.
- An opus worker (`w-d8176fd3`) is running in it. Its prompt is `scratchpad/df129-lead/prompt-d8176fd3.md`.
- Progress file: [progress.json](progress.json).

## What to do next

- [ ] Collect the w-d8176fd3 return. If it went idle, resume it with SendMessage.
- [ ] Lead review, then `git merge-tree` and merge under the lock. Then run cargo-ci and mcp-ci with `target/debug/jit` built at HEAD first on PATH. Run code-review with `review.sh d8176fd3`.
- [ ] Complete d8176fd3, reclaim the worktree, then start wave 3 one worktree at a time: 27f0a575, 2f15d48d, 9fa12e75, 9cc3056a, d4373d07, 2e7e1348, a807e699, 5d0a9a90.

## Traps — do not repeat these

- A pathspec commit of `.jit/events.jsonl` on main also commits other leads' uncommitted events (188be65ee did). That is harmless for the append-only log, but stage only the issue file when the log has foreign hunks.
- All traps in [handoff-3.md](handoff-3.md) and [handoff-2.md](handoff-2.md) remain in force.

## Open questions needing invoker input

None.
