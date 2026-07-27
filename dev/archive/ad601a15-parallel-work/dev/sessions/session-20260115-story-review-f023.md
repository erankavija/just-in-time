# Story Review: CLI Commands for Claims and Worktrees (f0235aa4)

**Date:** 2026-01-15  
**Story:** f0235aa4 (CLI Commands for Claims and Worktrees)  
**Reviewer:** copilot:cli-session  
**Status:** In Progress

## Review Checklist

### Implementation Coverage

**10 Tasks from Story Description:**

1. ✅ **`jit claim acquire`** - Acquire lease on issue with configurable TTL
2. ✅ **`jit claim renew`** - Extend lease TTL for long-running work
3. ✅ **`jit claim release`** - Explicitly release lease before expiry
4. ✅ **`jit claim force-evict`** - Admin operation to evict stale leases
5. ✅ **`jit claim status`** - Show current claim status (single issue or all)
6. ✅ **`jit claim list`** - List all active claims across worktrees
7. ✅ **`jit worktree info`** - Display current worktree context
8. ✅ **`jit worktree list`** - List all worktrees with claim info
9. ✅ **`jit validate --divergence`** - Explicit divergence validation
10. ✅ **`jit validate --leases`** - Validate lease consistency

**All 10 commands implemented!**

### Success Criteria Verification

From story description:

- [ ] All commands support `--json` output for agent consumption
- [ ] Clear error messages with actionable remediation
- [ ] `jit claim acquire` fails gracefully if already claimed
- [ ] `jit claim status` shows remaining TTL and expiry time
- [ ] Commands work in both main worktree and secondary worktrees

### Design Document Compliance

**Commands from design doc (section 2054-2112):**

**Claim commands:**
- [x] `jit claim acquire <issue-id> [--ttl <seconds>] [--agent-id <id>]`
- [x] `jit claim renew <lease-id> [--ttl <seconds>]`
- [x] `jit claim release <lease-id>`
- [x] `jit claim force-evict <lease-id> --reason <reason>`
- [x] `jit claim status [--json]`
- [x] `jit claim status --issue <issue-id> [--json]`
- [x] `jit claim list [--json]`

**Worktree commands:**
- [x] `jit worktree info [--json]`
- [x] `jit worktree list [--json]`
- [ ] `jit worktree init` (may not be needed - check)

**Validate commands:**
- [x] `jit validate --divergence`
- [x] `jit validate --leases`
- [x] `jit validate --json` (from validate base command)
- [x] `jit validate --fix` (bonus - from existing validate command)

### Manual Testing Plan

**Test Environment Setup:**
1. Create temp git repository
2. Initialize jit
3. Create test issue
4. Test in main worktree
5. Create secondary worktree
6. Test in secondary worktree

**Claim Command Tests:**
- [ ] Acquire lease with default TTL
- [ ] Acquire lease with custom TTL
- [ ] Acquire lease with explicit agent ID
- [ ] JSON output format verification
- [ ] Error: Already claimed by another agent
- [ ] Error: Non-existent issue
- [ ] Renew lease successfully
- [ ] Renew non-existent lease (error)
- [ ] Release lease successfully
- [ ] Release non-existent lease (error)
- [ ] Force evict with reason
- [ ] Status shows TTL and expiry
- [ ] Status with --issue filter
- [ ] List shows all active leases
- [ ] List JSON format

**Worktree Command Tests:**
- [ ] Info shows current worktree
- [ ] Info in main worktree
- [ ] Info in secondary worktree
- [ ] Info JSON format
- [ ] List shows all worktrees
- [ ] List shows claim status per worktree
- [ ] List JSON format

**Validate Command Tests:**
- [ ] Divergence check passes (up-to-date)
- [ ] Divergence check fails (diverged)
- [ ] Leases validation passes
- [ ] Leases validation shows stale leases
- [ ] JSON output format

**Error Message Quality:**
- [ ] Actionable error when lease not found
- [ ] Actionable error when already claimed
- [ ] Actionable error for git failures
- [ ] Help text is clear

### Code Quality Checks

**Pre-flight checks:**
- [ ] All 492 tests passing
- [ ] Zero clippy warnings
- [ ] Code formatted with rustfmt

**Integration tests:**
- [ ] Review existing claim integration tests (claim_integration_tests.rs)
- [ ] Check coverage of error scenarios
- [ ] Verify JSON output validation

## Test Results

### Environment Setup

```bash
# Create temp test directory
cd /tmp && rm -rf test-story-f023 && mkdir test-story-f023
cd test-story-f023
git init && git config user.name "Test" && git config user.email "test@example.com"
jit init
git add -A && git commit -m "Initial commit"
```

### Claim Commands

#### 1. Acquire Lease

**Test:** Basic acquire with default TTL
```bash
ISSUE_ID=$(jit issue create --title "Test claim acquire" --json | jq -r '.data.id')
jit claim acquire "$ISSUE_ID" --agent-id "agent:test-1"
```

**Expected:**
- Success message with lease ID
- Lease stored in `.git/jit/claims.jsonl`
- Index updated in `.git/jit/claims.index.json`

**Test:** Acquire with JSON output
```bash
jit claim acquire "$ISSUE_ID" --agent-id "agent:test-2" --ttl 3600 --json
```

**Expected:**
- Valid JSON with `success: true`
- Contains `lease_id`, `issue_id`, `agent_id`, `ttl_secs`, `expires_at`

**Test:** Already claimed error
```bash
# Second acquire should fail
jit claim acquire "$ISSUE_ID" --agent-id "agent:test-3" --ttl 3600
```

**Expected:**
- Error message with "already claimed"
- Shows "Possible causes:" and "To fix:"
- Mentions `jit claim status` command

#### 2. Status and List

**Test:** Status shows details
```bash
jit claim status --json
```

**Expected:**
- Shows all leases for current agent
- Includes remaining TTL
- Includes expiry time

**Test:** List all claims
```bash
jit claim list --json
```

**Expected:**
- Shows all active leases across all agents
- Valid JSON array structure

#### 3. Renew and Release

**Test:** Renew lease
```bash
LEASE_ID=<from acquire output>
jit claim renew "$LEASE_ID" --ttl 1200
```

**Expected:**
- Success message
- New expiry time shown

**Test:** Release lease
```bash
jit claim release "$LEASE_ID"
```

**Expected:**
- Success message
- Lease removed from index

### Worktree Commands

#### 1. Info Command

**Test:** In main worktree
```bash
jit worktree info --json
```

**Expected:**
- Shows worktree root
- Shows `.jit/` path
- Shows `.git/jit/` control plane path

#### 2. Secondary Worktree

**Test:** Create and use secondary worktree
```bash
git worktree add ../test-wt-secondary test-branch
cd ../test-wt-secondary
jit worktree info --json
```

**Expected:**
- Different worktree root
- Different `.jit/` path
- Same `.git/jit/` control plane

**Test:** List worktrees
```bash
jit worktree list --json
```

**Expected:**
- Shows both worktrees
- Shows branch per worktree
- Shows any active claims per worktree

### Validate Commands

#### 1. Divergence Check

**Test:** Up-to-date branch
```bash
jit validate --divergence
```

**Expected:**
- Success message (if on main or up-to-date)

#### 2. Leases Check

**Test:** Valid leases
```bash
jit validate --leases
```

**Expected:**
- Success if all leases valid
- Shows stale leases if any exist

## Review Findings

### Positive Observations
- ✅ All 10 CLI commands are implemented and functional
- ✅ JSON output works correctly for all commands
- ✅ Actionable error messages are implemented (from 4b2cb4cd)
- ✅ Commands work in main worktree
- ✅ Claim coordination works correctly (acquire, renew, release, list, status)
- ✅ Worktree info and list commands work properly
- ✅ Force-evict includes required --reason flag
- ✅ Lease not found errors include helpful remediation steps
- ✅ Already claimed errors show expiry time and remediation options
- ✅ All 492 tests pass, zero clippy warnings

### Issues Found

**CRITICAL: Issue visibility across worktrees not implemented**

**Issue:** Secondary worktrees cannot read issues that exist in the main worktree or git, even for read-only operations.

**Expected behavior (from design doc lines 353, 464):**
- "All issue data remains readable from any worktree regardless of claims"
- "Worktrees contain write copies of claimed issues, but all issues remain readable from any worktree via git"
- Read operations should fall back through: Local `.jit/` → Git HEAD → Main `.jit/`

**Actual behavior:**
- `jit issue show <id>` in secondary worktree returns null for issues not claimed there
- `jit query all` in secondary worktree returns empty even when issues exist in git
- Only shows issues that have local write copies (claimed issues)

**Root cause:**
- `JsonFileStorage::load_issue()` only reads from local `.jit/issues/` directory
- No fallback to read from git commits
- No fallback to read from main worktree's `.jit/`

**Impact:**
- **HIGH** - Breaks fundamental design principle of read/write separation
- Agents in secondary worktrees cannot inspect dependencies
- Cannot check status of issues claimed by other worktrees
- Cannot use `jit graph show`, `jit query blocked`, etc. effectively
- Makes secondary worktrees nearly unusable for coordination

**Test case:**
```bash
# Main worktree
jit issue create --title "Test" --json  # Creates issue c389...
git add .jit/issues/c389*.json && git commit -m "Add issue"

# Secondary worktree  
cd ../secondary-worktree
jit issue show c389  # Expected: Shows issue. Actual: Returns null
jit query all        # Expected: Shows committed issues. Actual: Empty
```

### Suggestions for Improvement

**MUST FIX: Implement multi-source issue reading**

Need to implement the layered resolution strategy from design doc (line 447):

1. **Local `.jit/`** - Check for local write copy first (current implementation)
2. **Git HEAD** - Fall back to committed state in git
3. **Main worktree `.jit/`** - Fall back to uncommitted issues in main worktree

**Implementation approach:**
```rust
fn load_issue(&self, id: &str) -> Result<Issue> {
    // 1. Try local .jit/issues/
    let local_path = self.issue_path(id);
    if local_path.exists() {
        return self.read_json(&local_path);
    }
    
    // 2. Try git HEAD
    if let Ok(issue) = self.load_issue_from_git(id) {
        return Ok(issue);
    }
    
    // 3. Try main worktree .jit/ (if we're in a secondary worktree)
    if let Ok(issue) = self.load_issue_from_main_worktree(id) {
        return Ok(issue);
    }
    
    Err(anyhow!("Issue {} not found", id))
}
```

**Additional considerations:**
- `query all` should also implement multi-source aggregation
- Performance: May need caching strategy for git reads
- Must handle merge conflicts gracefully
- Document behavior in user-facing docs

## Next Steps

**Story f023 Status: BLOCKED - Cannot mark complete until critical issue is resolved**

### Immediate Action Required

1. **Create blocking issue for multi-source issue reading**
   - Title: "Implement cross-worktree issue visibility with git fallback"
   - Priority: Critical
   - Blocks: This story (f023)
   - Description: Implement layered resolution for issue reads (local → git → main worktree)

2. **Add dependency from f023 to new issue**
   - Story cannot be marked complete without this functionality
   - Violates fundamental design principle from worktree-parallel-work.md

### Success Criteria Status

From story description:

- ✅ All commands support `--json` output for agent consumption
- ✅ Clear error messages with actionable remediation (thanks to 4b2cb4cd)
- ✅ `jit claim acquire` fails gracefully if already claimed
- ✅ `jit claim status` shows remaining TTL and expiry time
- ❌ **Commands work in both main worktree and secondary worktrees** - FAILS due to issue visibility bug

**1 of 5 success criteria FAILING**

### Post-Fix Actions

Once the blocking issue is resolved:

1. [ ] Re-run full manual testing plan in secondary worktrees
2. [ ] Verify cross-worktree queries work (`jit query all`, `jit graph show`)
3. [ ] Verify dependency checking across worktrees
4. [ ] Test claim workflow: secondary worktree claiming issue from main
5. [ ] Run all pre-flight checks (tests, clippy, fmt)
6. [ ] Pass all gates
7. [ ] Mark story as complete
