# How-to: Multi-Agent Coordination

**Status:** 🚧 Coming Soon

This guide covers setting up and managing multi-agent parallel work using git worktrees.

## Quick Reference

### Basic Worktree Setup

```bash
# Create a secondary worktree
git worktree add ../secondary-worktree feature-branch

# Initialize jit in the secondary worktree
cd ../secondary-worktree
jit init
```

### Reading Issues Across Worktrees

Issues are automatically readable from any worktree using a 3-tier fallback:

1. **Local `.jit/`** - Issues modified/created in this worktree
2. **Git HEAD** - Committed issues (canonical state)
3. **Main worktree `.jit/`** - Uncommitted issues in main worktree

```bash
# From secondary worktree - reads work automatically
jit issue show <issue-id>     # Works for ANY issue
jit query all                  # Shows all issues from all sources
jit graph show <issue-id>      # Dependency graph across worktrees
```

## Common Scenarios

### Scenario 1: Agent Working on Different Issues in Parallel

**TODO:** Step-by-step guide for parallel work

### Scenario 2: Checking Dependencies Across Worktrees

**TODO:** How to verify dependencies are resolved

### Scenario 3: Claiming Issues (Future)

**TODO:** Using `jit claim` commands for coordination

### Scenario 4: Handling Conflicts

**TODO:** Resolving merge conflicts in issue data

## Troubleshooting

### Issue not found in secondary worktree

**Solution:** Check if the issue exists in:
- Git: `git show HEAD:.jit/issues/<id>.json`
- Main worktree: Check `../main-worktree/.jit/issues/`

### Changes in secondary worktree not visible in main

**Expected behavior:** Worktrees are isolated. Commit and merge to share changes.

## Best Practices

- **Commit frequently** - Makes issues visible across worktrees via git
- **Use dependencies** - Explicitly model work relationships
- **Clean up worktrees** - Remove when work is complete: `git worktree remove`

## Implementation Status

✅ **Available Now:**
- Cross-worktree issue reading (local → git → main)
- Query commands work across worktrees
- Dependency graphs span worktrees

⏳ **Coming Soon:**
- Issue claiming (`jit claim acquire/release/status`)
- Lease coordination and TTL
- Pre-commit hooks for enforcement
- Web UI for claim visualization

## See Also

- [Tutorial: Parallel Work with Git Worktrees](../tutorials/parallel-work-worktrees.md)
- Design document: `dev/design/worktree-parallel-work.md`
- [Dependency Management](./dependency-management.md)

---

**Documentation TODO:** Complete guide pending story completion for parallel work documentation.
