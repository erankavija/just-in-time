# Handoff — Dispatch layering, executor surface, and build hygiene in crates/jit (df129d9e) — session 6

**Date:** 2026-10-22
**Session number:** 6
**Prior handoffs:** [handoff.md](handoff.md) through [handoff-5.md](handoff-5.md)

## Current state

- d8176fd3 is done: cargo-ci, mcp-ci and code-review passed, recorded in 2195289d2. Its worktree and branch are removed.
- Wave 3 is ready. 27f0a575 (issues facade) is in progress and leased by agent:df129d9e-lead (lease d92de6ca, 4 h). A sonnet worker, `w-27f0a575`, is running in `.agents/worktrees/agent-27f0a575` (base b81e5227f). Its prompt is `scratchpad/df129-lead/prompt-27f0a575.md`.

## What to do next

- [ ] Collect the w-27f0a575 return. Review it against REQ-01..03 (REQ-03: no assertion changes). Check `git log main..<branch>` for late commits, merge under the lock, then run cargo-ci and code-review in that order.
- [ ] Then the rest of wave 3, one worktree at a time: 2f15d48d, 9fa12e75, 9cc3056a, d4373d07, 2e7e1348, a807e699, 5d0a9a90.

## Traps — do not repeat these

- `git add .jit/issues/*.json` stages other leads' issue files. Stage named paths only.
- `jit claim release` takes the issue id, not the lease id.
- Traps in handoff-2 through handoff-5 remain in force.

## Open questions needing invoker input

None.
