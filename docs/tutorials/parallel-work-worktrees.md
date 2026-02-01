# Tutorial: Parallel Work with Git Worktrees

**Status:** 🚧 Coming Soon

This tutorial will guide you through setting up parallel workflows using git worktrees for multi-agent coordination.

## What You'll Learn

- How to set up multiple worktrees for parallel work
- Reading issues across worktrees (local, git, main worktree)
- Understanding the 3-tier fallback strategy
- Creating and claiming issues in secondary worktrees
- Managing dependencies across worktrees

## Prerequisites

- Basic familiarity with `jit` commands (see [Quickstart](./quickstart.md))
- Git worktrees knowledge (see [git-worktree documentation](https://git-scm.com/docs/git-worktree))
- Understanding of issue dependencies (see [Dependency Management](../how-to/dependency-management.md))

## Topics to be Covered

1. **Setting Up Worktrees**
   - Creating a secondary worktree
   - Initializing jit in secondary worktree
   - Understanding worktree isolation

2. **Cross-Worktree Issue Visibility**
   - How issues are resolved (local → git → main)
   - Reading committed issues from git
   - Reading uncommitted issues from main worktree
   - Local modifications and isolation

3. **Parallel Workflows**
   - Working on different issues simultaneously
   - Dependency checking across worktrees
   - Committing and merging work

4. **Advanced Topics**
   - Issue claiming and coordination (future)
   - Lease management (future)
   - Conflict resolution (future)

## Implementation Status

✅ Cross-worktree issue visibility (Phase 1-3 complete)  
⏳ Issue claiming and leases (planned)  
⏳ Pre-commit hooks (planned)  
⏳ Web UI integration (planned)

## See Also

- [How-to: Multi-Agent Coordination](../how-to/multi-agent-coordination.md)
- Design document: `dev/design/worktree-parallel-work.md`
- [CLI Commands Reference](../reference/cli-commands.md)

---

**Documentation TODO:** Full tutorial content pending completion of story for parallel work documentation.
