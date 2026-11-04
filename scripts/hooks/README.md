# Git Hooks for JIT

This directory holds the git hook `jit hooks install` copies to `.git/hooks/`. An
existing hook is left in place.

## pre-commit

A commit changing `.jit/config.toml` or `.jit/gates.toml` requires common history
with main. The check runs unless `jit config get worktree.enforce_leases` answers
`off`, so the hook and `jit config` cannot disagree. Without a readable mode, for
example with no `jit` on `PATH`, the hook warns and allows the commit.

Skip it in an emergency with `git commit --no-verify`.

## Leases

No hook checks leases. Lease enforcement happens when a mutation is written; see
[`enforce_leases`](../../docs/reference/configuration.md#enforce_leases). The limit
is that a hand edit of an issue record is not caught at commit.

## Worktrees

Hooks live in the common git directory, so every linked worktree runs them.
