# JIT State Commit Patterns

JIT state changes (`.jit/` directory) are always committed separately from
code changes. Commits are batched per logical workflow step — one commit per
user-visible action, not one per jit command.

## Command

All patterns use:

```bash
git add .jit && git commit -m "<message>"
```

## Patterns by Workflow Step

| Workflow Step | Commit Message |
|---------------|---------------|
| Claim issue | `chore: claim issue <short-id> (<title>)` |
| Create single issue + deps | `chore: create issue <short-id> (<title>)` |
| Create batch of issues | `chore: create <N> issues for <parent-title>` |
| Link document to issue | `chore: link document to <short-id>` |
| Update success criteria | `chore: update criteria for <short-id>` |
| Pass manual gate | `chore: pass gate <gate-key> for <short-id>` |
| Complete issue | `chore: complete issue <short-id> (<title>)` |
| Reject issue | `chore: reject issue <short-id> (<title>)` |
| Generic session batch | `chore: update jit state (<summary>)` |

## Rules

1. **Never mix code and JIT state** in the same commit. Code changes get
   their own commits with descriptive messages. JIT state gets `chore:` commits.

2. **Batch related JIT operations.** Creating an issue, adding its deps,
   and assigning labels is one logical step = one commit. But claiming an
   issue and completing it are separate steps = separate commits.

3. **Include the short-id** in every commit message for traceability.

4. **Title in parentheses** helps humans scanning `git log` understand
   what changed without running `jit show`.
