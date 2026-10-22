# Handoff — Dispatch layering, executor surface, and build hygiene in crates/jit (df129d9e) — session 7

**Date:** 2026-10-22
**Session number:** 7
**Prior handoffs:** [handoff.md](handoff.md) through [handoff-6.md](handoff-6.md)

## Current state

- 27f0a575 is done. Its first merge, d144d3d84, broke main: five doctests still called the moved methods, and every lead was held. The fix was be0d9ab53. cargo-ci and code-review then passed (f9242bd3c).
- Wave-4 facade tasks became ready when 27f0a575 closed. Finish wave 3 first.
- 2f15d48d (issue list renderer) is in progress and leased (lease 92840de7, 4 h). A sonnet worker, `w-2f15d48d`, is running in `.agents/worktrees/agent-2f15d48d` (base 24385f797).

## What to do next

- [ ] Collect the w-2f15d48d return. Review it against REQ-01..04. Run `cargo test --doc -p jit` on the branch, merge under the lock, then run cargo-ci and code-review in that order.
- [ ] The rest of wave 3, one worktree at a time: 9fa12e75, 9cc3056a, d4373d07, 2e7e1348, a807e699, 5d0a9a90. Then wave 4.

## Traps — do not repeat these

- **Doctests are outside clippy and focused tests.** The 27f0a575 merge turned main red for every lead. Run `cargo test --doc -p jit` on the branch before every merge. common.md now tells workers to do the same.
- **Leases expire across usage-limit stalls.** `jit gate evaluate` then fails with "No active lease". Re-acquire before gating or updating state.
- **zsh does not word-split `$files`.** Use `${(f)"$(...)"}` arrays for pathspec lists.
- Traps in handoff-2 through handoff-6 remain in force.

## Open questions needing invoker input

None.
