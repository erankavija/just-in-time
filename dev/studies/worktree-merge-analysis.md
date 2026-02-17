# Worktree Merge Friction Analysis

**Date:** 2026-02-17
**Context:** First multi-agent parallel work session using git worktrees.
Two agents (main worktree + `agents/claude-1`) worked on separate issues and merged back.

## Observed Friction Points

### 1. Issue state changes (.jit/) are conflict magnets

Both agents wrote to the same issue JSON — one claiming it, the other completing it.
The `.jit/events.jsonl` also had concurrent appends. With more agents and more issues,
this will get worse fast.

### 2. Forgotten .jit/ commits create a second merge round

Issue state updates were made via CLI in the agent worktree but weren't committed
alongside the code commits. This created a second merge round that caused the conflict.

### 3. Worktree isolation doesn't cover .jit/ writes

The claim/update commands wrote to whichever worktree they ran from, but both agents
could modify the same issue files. Worktree isolation only helped with source code,
not coordination data.

## Recommendations

### Short-term: Process discipline

- **Commit `.jit/` changes alongside the code change that completes them** — don't
  leave issue state updates uncommitted as a separate merge step.
- **Rebase agent branch onto main before merging** — surface conflicts early in the
  agent's own branch rather than during the merge into main.

### Medium-term: Custom merge strategies

- **Last-write-wins merge driver for issue JSON**: Issue files are single-document JSON
  with an `updated_at` timestamp. A custom git merge driver could auto-resolve by picking
  the version with the later timestamp — no manual conflict resolution needed.

  ```gitattributes
  .jit/issues/*.json merge=jit-last-write-wins
  ```

  ```gitconfig
  [merge "jit-last-write-wins"]
      name = JIT issue last-write-wins
      driver = jit merge-driver --last-write-wins %O %A %B
  ```

- **Union merge for events.jsonl**: Append-only logs should use `merge=union` in
  `.gitattributes` so git concatenates both sides instead of conflicting.

  ```gitattributes
  .jit/events.jsonl merge=union
  .jit/claims.jsonl merge=union
  ```

### Longer-term: Architecture (per design doc)

- **Move coordination state to `.git/jit/`** (the shared control plane). Claims, leases,
  and control events would never be committed and never conflict.
- **Issue data writes gated by lease ownership**: Only the agent holding the lease commits
  changes to that issue file, eliminating concurrent writes to the same issue JSON.

## Impact Assessment

| Strategy | Effort | Conflict reduction |
|----------|--------|--------------------|
| Process discipline | None (behavioral) | Reduces double-merge, not conflicts |
| `.gitattributes` union merge | 1 line | Eliminates events.jsonl conflicts |
| Custom merge driver | Small tool | Eliminates issue JSON conflicts |
| Control plane in `.git/jit/` | Architecture change | Eliminates all coordination conflicts |

The custom merge driver for `.jit/issues/*.json` (last-write-wins on `updated_at`) would
eliminate nearly all the conflicts observed, and is a small addition to repo setup.
