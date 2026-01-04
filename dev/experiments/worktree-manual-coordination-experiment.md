# Manual Worktree Coordination Experiment

**Epic:** ad601a15-5217-439f-9b3c-94a52c06c18b  
**Date:** 2026-01-04  
**Status:** Planned  
**Purpose:** Validate git worktree workflow for parallel agent work before implementing automated coordination

## Overview

This experiment tests manual parallel work using git worktrees to inform the automated coordination design. Multiple agents will work on different tasks simultaneously using separate worktrees, coordinating manually to avoid conflicts.

## Goals

1. **Validate workflow:** Confirm git worktrees enable parallel development
2. **Identify pain points:** Discover what manual coordination is needed
3. **Test merge strategy:** Verify independent work can be integrated cleanly
4. **Inform automation:** Learn what the claim/lease system must handle

## Prerequisites

```bash
# Ensure you're on feature/worktree-parallel-work branch
git status

# Commit any uncommitted work
git add -A && git commit -m "checkpoint before worktree experiment"
```

## Setup: Create Worktrees for Parallel Tasks

```bash
# Create a directory for worktrees (convention: keep them organized)
mkdir -p ../just-in-time-worktrees

# Create worktree 1 for task A
git worktree add ../just-in-time-worktrees/task-foundation agents/foundation-work

# Create worktree 2 for task B  
git worktree add ../just-in-time-worktrees/task-identity agents/identity-work

# Create worktree 3 for task C
git worktree add ../just-in-time-worktrees/task-config agents/config-work

# List worktrees
git worktree list
```

**Expected output:**
```
/home/vkaskivuo/Projects/just-in-time              <branch>  
/path/to/just-in-time-worktrees/task-foundation    agents/foundation-work
/path/to/just-in-time-worktrees/task-identity      agents/identity-work
/path/to/just-in-time-worktrees/task-config        agents/config-work
```

## Experiment: Manual Coordination Pattern

### Agent 1 Workflow (Main Worktree)

**Task:** b69fa9a9 - Implement WorktreePaths detection

```bash
# Stay in main repo
cd /home/vkaskivuo/Projects/just-in-time

# Claim the task
jit issue show b69fa9a9
jit issue update b69fa9a9 --assignee agent:agent-1 --state in-progress

# Work on it
# ... implement, test, commit ...

git add crates/jit/src/paths.rs
git commit -m "feat: implement WorktreePaths detection

Implements git worktree detection using rev-parse.
Resolves task b69fa9a9."

# Update issue
jit issue update b69fa9a9 --state done
```

### Agent 2 Workflow (Worktree 1)

**Task:** 5186a98d - Implement worktree identity generation

```bash
# Switch to worktree
cd ../just-in-time-worktrees/task-foundation

# This has isolated file system but shared .jit/ (for now)
# MANUAL COORDINATION: Only work on issues NO ONE ELSE is working on

# Claim the task
jit issue show 5186a98d
jit issue update 5186a98d --assignee agent:agent-2 --state in-progress

# Work on it
# ... implement, test, commit ...

git add crates/jit/src/worktree_identity.rs
git commit -m "feat: implement worktree identity generation

Generates deterministic worktree IDs from path hash.
Resolves task 5186a98d."

# Update issue
jit issue update 5186a98d --state done
```

### Agent 3 Workflow (Worktree 2)

**Task:** 61783d2f - Implement agent identity configuration

```bash
cd ../just-in-time-worktrees/task-identity

# Claim the task
jit issue show 61783d2f
jit issue update 61783d2f --assignee agent:agent-3 --state in-progress

# Work on it
# ... implement, test, commit ...

git add crates/jit/src/agent_config.rs
git commit -m "feat: implement agent identity configuration

Supports JIT_AGENT_ID env var and config file.
Resolves task 61783d2f."

# Update issue
jit issue update 61783d2f --state done
```

## Integration: Merging Work Back

```bash
# From main worktree
cd /home/vkaskivuo/Projects/just-in-time

# Check current state
git status
git branch -a

# Pull in work from agent branches
git merge agents/foundation-work --no-ff -m "Merge: WorktreePaths detection"
git merge agents/identity-work --no-ff -m "Merge: worktree identity generation"
git merge agents/config-work --no-ff -m "Merge: agent identity configuration"

# If conflicts occur, resolve them
# git status
# git mergetool  # or manually edit
# git commit

# Run full test suite after merge
cargo test --workspace

# Run quality checks
cargo clippy --workspace --all-targets
cargo fmt --all -- --check

# Push merged result
git push origin feature/worktree-parallel-work
```

## Current Limitations (Manual Coordination)

**⚠️ What DOESN'T work yet:**
- `.jit/` is still shared across worktrees (not isolated yet)
- No automatic claim/lease system (manual communication needed)
- No enforcement preventing concurrent edits to same issue
- Queries don't filter by who's working on what
- Issue state changes from different worktrees can conflict

**✅ What DOES work:**
- Parallel git branches (agents/foundation-work, agents/identity-work, etc.)
- Isolated working directories (no file conflicts on source code)
- Independent builds/tests in each worktree
- Git merge workflow for integration
- Each agent can run `cargo test` in parallel

## Manual Coordination Protocol

Until automation exists, follow these rules:

### 1. Claim Issues Explicitly
```bash
# Before starting work
jit issue update <id> --assignee agent:your-name --state in-progress
```

### 2. Communicate Claims
- Verbally announce: "I'm working on task b69fa9a9"
- Check assignments: `jit query assignee agent:agent-1`

### 3. Avoid Conflicts
- **Never work on the same issue** as another agent
- **Minimize .jit/ edits** (issue updates only at start/end)
- **Small commits, frequent merges** to reduce conflict window

### 4. One Issue Per Worktree
Keep scope focused to avoid complex merges

## Cleanup When Done

```bash
# From main worktree
cd /home/vkaskivuo/Projects/just-in-time

# List worktrees
git worktree list

# Remove a worktree (after merging its work)
git worktree remove ../just-in-time-worktrees/task-foundation

# Or remove all
git worktree remove ../just-in-time-worktrees/task-identity
git worktree remove ../just-in-time-worktrees/task-config

# Clean up remote branches (optional)
git push origin --delete agents/foundation-work
git push origin --delete agents/identity-work
git push origin --delete agents/config-work

# Verify cleanup
git worktree list
git branch -a
```

## Recommended Tasks for Experiment

**Try this 3-agent parallel workflow:**

| Agent | Task ID | Title | Rationale |
|-------|---------|-------|-----------|
| Agent 1 (main) | `b69fa9a9` | Implement WorktreePaths detection | Core path resolution, well-scoped |
| Agent 2 (wt-1) | `5186a98d` | Implement worktree identity | Independent from paths, clear interface |
| Agent 3 (wt-2) | `61783d2f` | Implement agent identity config | Config-only, minimal conflicts |

**Why these tasks:**
- All are Phase 1 (Foundation) - relatively independent
- Different files/modules - minimal merge conflicts
- Small scope - completable in 1-2 hours each
- Clear acceptance criteria in design doc

## Success Criteria

**Experiment succeeds if:**
- [ ] All 3 agents complete tasks in parallel
- [ ] Merges complete without major conflicts (< 10 lines conflict)
- [ ] `cargo test --workspace` passes after merge
- [ ] `cargo clippy` and `cargo fmt` pass
- [ ] No data loss or corruption in `.jit/`

**Learning outcomes:**
- [ ] Document pain points encountered
- [ ] List manual coordination steps that should be automated
- [ ] Identify `.jit/` conflict scenarios
- [ ] Validate merge strategy works

## Observations to Record

During the experiment, note:

### Coordination Pain Points
- How did you coordinate who works on what?
- Did anyone accidentally work on the same issue?
- How did you know when someone finished?

### .jit/ Conflicts
- Did multiple agents edit the same issue file?
- Were there merge conflicts in `.jit/`?
- How were they resolved?

### Workflow Friction
- What felt manual/tedious?
- Where would automation help most?
- What surprised you?

### Git Worktree Experience
- Was worktree setup/cleanup easy?
- Did separate directories help or hinder?
- Any issues with git operations?

## Next Steps After Experiment

1. **Document findings** in session notes
2. **Update design** based on pain points discovered
3. **Prioritize automation** features by impact
4. **Implement coordination** starting with most painful manual steps

## References

- Epic: `ad601a15` - Enable parallel multi-agent work with git worktrees
- Design: `dev/design/worktree-parallel-work.md`
- Tasks: Phase 1 Foundation (b69fa9a9, 5186a98d, 61783d2f, 565cda12)
