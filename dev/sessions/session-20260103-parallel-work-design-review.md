# Session Notes: Parallel Work Design Review

**Date:** 2026-01-03  
**Epic:** ad601a15 - Enable parallel multi-agent work with git worktrees  
**Participants:** User, GitHub Copilot  
**Focus:** Design review, TTL=0 leases, workflow clarifications

## Session Overview

Comprehensive review of the parallel work design document with focus on:
1. TTL=0 indefinite lease semantics
2. Sequential vs parallel workflow impact
3. Worktree usage for non-parallel scenarios
4. Information access and context availability across worktrees
5. Critical gaps in the design document

## Key Decisions and Updates

### 1. TTL=0 Indefinite Lease Support Added

**Context:** Need for long-running manual oversight scenarios (complex refactors, schema migrations, global operations).

**Decision:** Implement TTL=0 with operational guardrails:
- **Semantics:** `ttl_secs = 0` means no time-based expiry, `expires_at = null`
- **Heartbeats required:** Track `last_beat` timestamp, update via heartbeat operation
- **Staleness detection:** Mark lease as `stale: true` if `now - last_beat > stale_threshold_secs`
- **Policy enforcement:**
  - Mandatory `--reason` flag when acquiring TTL=0
  - Per-agent limit: `max_indefinite_leases_per_agent` (default: 2)
  - Per-repository limit: `max_indefinite_leases_per_repo` (default: 10)
  - Pre-commit/pre-push hooks reject structural edits with `stale: true`

**Changes made:**
- ✅ Updated design doc: `dev/design/worktree-parallel-work.md`
  - Added "Indefinite (TTL=0) Leases" section
  - Updated "Automatic Expiration and Staleness" section
  - Updated lease operations (heartbeat logic)
  - Updated claims JSONL and index schemas
  - Added config options for staleness threshold and limits
  - Updated hook examples to check `stale != true`
- ✅ Created task: `608ae1e8` - "Implement TTL=0 indefinite lease support with staleness detection"
- ✅ Updated 8 existing tasks for TTL=0 compatibility:
  - `e66da7c0` - Automatic lease expiration (finite + staleness)
  - `1a4c1f79` - Lease renewal/release/force-evict (heartbeat logic)
  - `1a5e737d` - Claims index (schema with stale fields)
  - `ee20327b` - jit claim acquire (TTL=0 constraints)
  - `77206cf6` - jit claim renew (heartbeat for TTL=0)
  - `3f2cf966` - jit claim status (highlight indefinite leases)
  - `bc3742d9` - Pre-commit hook (check stale != true)
  - `62ff2fd9` - Pre-push hook (check stale != true)

### 2. Time Model in Parallel Workflows

**Q: What is the concept of time in parallel workflows?**

**A: Dual-clock approach for determinism:**
- **Monotonic time** (`std::time::Instant`): For TTL expiry checks, immune to system clock changes (NTP, manual adjustments)
- **Wall-clock time** (RFC3339 timestamps): For audit trail (`acquired_at`, `expires_at`, `last_beat`)
- **Sequence numbers** (optional): Monotonic counters for total ordering of claim operations

**Q: Can parallel work streams stay idle?**

**A: Yes, with automatic recovery:**
- **Finite leases (TTL > 0):** Auto-expire after TTL, freeing the issue
- **Indefinite leases (TTL = 0):** Marked stale if idle too long, require renewal or force-eviction
- **Heartbeat daemon (optional):** Background thread auto-renews leases for long-running work
- **Lease renewal warnings:** CLI warns when lease <threshold% remaining (configurable)

### 3. Sequential Workflow Impact

**Q: How will parallel work affect usual sequential workflows?**

**A: Non-breaking, opt-in design:**

**Default behavior (sequential):**
```toml
[worktree]
mode = "auto"              # Auto-detect worktrees
enforce_leases = "off"     # No lease enforcement
```
- ✅ No changes required for existing workflows
- ✅ No claims needed for sequential work
- ✅ No hooks required
- ✅ Backward compatible

**Three enforcement modes:**
1. `enforce_leases = "off"` - No lease checks (default for sequential)
2. `enforce_leases = "warn"` - Warn but allow operations without lease (migration mode)
3. `enforce_leases = "strict"` - Require lease for structural edits (multi-agent parallel)

**Migration path:**
- Phase 1 (Non-Breaking): Worktree detection added, `.jit/` works normally
- Phase 2 (Opt-In): `jit claim` commands available but optional
- Phase 3 (Gradual): Teams enable enforcement via config when ready

### 4. Non-Parallel Worktree Usage

**Q: What if I use worktrees for other purposes?**

**A: No conflicts with personal worktree usage:**

**Common scenarios supported:**
- ✅ Personal feature branch worktrees (avoid branch switching)
- ✅ Code review worktrees (read-only review)
- ✅ Build/test worktrees (run tests while developing)
- ✅ Emergency hotfix worktrees (urgent fixes while feature incomplete)

**Key insight:** Each worktree has isolated `.jit/` directory. With `enforce_leases = "off"`, worktrees work independently without coordination overhead.

**When claims are needed:**
- ✅ Multiple agents/people working simultaneously on same repository
- ✅ Enforcement enabled: `enforce_leases = "strict"`
- ✅ Concurrent edits to same issue possible

**Configuration matrix:**

| Scenario | `enforce_leases` | Claims Needed? |
|----------|------------------|----------------|
| Solo dev, personal worktrees | `off` | ❌ No |
| Code review worktree | `off` | ❌ No |
| Build/test worktree | `off` | ❌ No |
| Emergency hotfix worktree | `off` | ❌ No |
| Multi-agent parallel work | `strict` | ✅ Yes |

### 5. Information Access Across Worktrees

**Q: How can agents with different worktrees query information?**

**A: Read vs Write separation model:**

**CRITICAL CLARIFICATION:** Worktrees only contain WRITE copies of claimed issues. All issues remain readable from any worktree.

**Read operations (no lease required):**
```bash
# From any worktree
jit issue show <any-issue-id>   # ✅ Works via git/main .jit/
jit query available                 # ✅ Coordination-aware queries
jit graph show <issue-id>       # ✅ Full dependency tree
jit dep show <issue-id>         # ✅ Dependencies
jit gate list                   # ✅ Global configuration
jit claim status                # ✅ Who has what
```

**Write operations (require active lease):**
```bash
jit issue update <issue-id> --state done   # ❌ Requires lease
jit dep add <from> <to>                    # ❌ Requires lease
jit issue delete <issue-id>                # ❌ Requires lease
```

**Data access patterns:**
1. **Git HEAD:** Committed issue state (canonical for merged issues)
2. **Main `.jit/`:** Uncommitted issues in main worktree (fallback)
3. **Claims index:** Real-time coordination state (`.git/jit/claims.index.json`)

**Example - Dependency checking:**
```bash
# Agent-1 in worktree-1 (working on issue-A)
# issue-A depends on issue-B (being worked by agent-2)

jit graph show issue-A          # ✅ Shows full tree including issue-B
jit issue show issue-B          # ✅ Read issue-B details (via git)
jit claim status --issue issue-B # ✅ See who's working on it
jit query blocked               # ✅ Check if blocked by issue-B
```

## Critical Gaps Identified in Design Document

### Gap 1: Read Access Not Explicitly Documented ❌

**Issue:** Design doesn't clearly state that reads don't require leases.

**Missing:** "Read vs Write Operations" section specifying:
- What operations require leases (writes)
- What operations don't require leases (reads)
- How reads access data (git HEAD, main `.jit/`, claims index)

**Impact:** Implementers might incorrectly restrict reads or create confusion about data access.

**Recommendation:** Add explicit section after "Single-Writer Policy" documenting read/write separation.

### Gap 2: Issue Creation - Chicken-and-Egg Problem ❌

**Issue:** Design says "Creating issue" requires lease (line 291), but you can't claim a non-existent issue.

**Ambiguity:**
```
Create issue → Requires lease?
Lease → Requires issue ID
Issue ID → Doesn't exist yet (chicken-and-egg)
```

**Possible interpretations:**

**Option A: Create is Exempted (most logical)**
```bash
# Create doesn't require lease (allocates new ID, no conflict)
jit issue create --title "New feature" --priority high
# => issue-E created in local .jit/ (state: ready)

# Optionally claim to work on it
jit claim acquire issue-E --ttl 600
```

**Option B: Create-in-Main-Only (stricter)**
```bash
# Issue creation restricted to main worktree
# Config: worktree.allow_issue_creation = "main-only"
```

**Impact:** Unclear whether agents in secondary worktrees can create issues. Needs design decision.

**Recommendation:** 
1. Clarify that issue creation is exempted from lease requirement (new ID allocation can't conflict)
2. OR specify main-worktree-only restriction with config option
3. Document create-then-claim workflow pattern

### Gap 3: Query Scope Ambiguity ❌

**Issue:** Design shows worktrees with "only claimed issues" (line 84), but doesn't explain how queries work across worktrees.

**Missing specification:**
- `jit query all` behavior in main vs secondary worktrees
- `jit query available` - how does it filter claimed issues?
- `jit issue show <id>` - can it show unclaimed issues from secondary worktree?
- `jit graph show <id>` - full graph or only local issues?

**Impact:** Unclear whether worktrees have "local-only" view or "global with local workspace" view.

**Recommendation:** Add "Query Behavior in Worktrees" section specifying:

```markdown
### Query Behavior in Worktrees

**Local queries** (`jit query all`):
- Main worktree: Shows all issues in `.jit/issues/`
- Secondary worktree: Shows only claimed issues in local `.jit/issues/`

**Global queries** (coordination-aware):
- `jit query available`: Filters out claimed issues (consults `.git/jit/claims.index.json`)
- `jit query blocked`: Shows all blocked issues (reads dependencies from git)
- Access to full issue graph regardless of worktree

**Direct issue access:**
- `jit issue show <id>` works for ANY issue from ANY worktree
- Reads from: git HEAD or main `.jit/` (fallback)
```

## Parallel Workflow Detailed Model

### Storage Architecture

```
Main Worktree (.jit/)
├── All issues (source of truth before claims)
│   
Per-Worktree (.jit/)  
├── Only WRITE copies of claimed issues
├── Isolated workspace per agent
│
Shared Control Plane (.git/jit/)
├── Claims index (who has what)
├── Coordination state (global visibility)
```

### Coordination Flow

1. **Claim** → Issue file copied from main `.jit/` to worktree `.jit/`
2. **Work** → Agent modifies local copy in worktree
3. **Commit** → Changes staged in git (worktree branch)
4. **Push + Merge** → Git merge brings changes back to main
5. **Release** → Lease released, issue accessible to others

### Read vs Write Model

```
┌────────────────────────────────────────────────┐
│ READS: Always accessible from any worktree    │
│ - Issue metadata (via git)                    │
│ - Dependencies (via git)                      │
│ - Global config (gates, hierarchy, config)    │
│ - Claims status (coordination layer)          │
│ - Query results (aggregated view)             │
└────────────────────────────────────────────────┘
                     │
                     ▼ Claims coordinator checks
┌────────────────────────────────────────────────┐
│ WRITES: Require active, non-stale lease       │
│ - Update issue state                          │
│ - Modify dependencies                         │
│ - Change labels/assignees                     │
│ - Add/remove gates                            │
└────────────────────────────────────────────────┘
```

## Action Items for Next Session

### Design Document Updates Needed

1. **Add "Read vs Write Operations" section** (Priority: High)
   - List operations that require leases vs don't
   - Document read access patterns (git, main `.jit/`, claims index)
   - Clarify that reads are unrestricted across worktrees

2. **Clarify Issue Creation** (Priority: High)
   - Decide: Create exempted from lease requirement OR main-only
   - Document create-then-claim workflow
   - Add configuration option if restricting to main worktree

3. **Add "Query Behavior in Worktrees" section** (Priority: High)
   - Specify `jit query all` behavior (main vs secondary)
   - Document coordination-aware queries (`jit query available`)
   - Clarify global visibility (dependency graphs, issue show)

4. **Add "Data Access Patterns" section** (Priority: Medium)
   - Document how reads access git HEAD
   - Explain fallback to main `.jit/`
   - Describe claims index consultation

### Task/Issue Creation

Consider creating issues for:
- Design clarification: Read access model
- Design clarification: Issue creation in worktrees
- Design clarification: Query scope and behavior
- Documentation: Update design doc with missing sections

### Testing Considerations

When implementing, ensure tests cover:
- Read operations work from any worktree without lease
- Issue creation workflow (create-then-claim)
- Query coordination (filtering claimed issues)
- Dependency graph access across worktrees
- Global configuration visibility

## Summary

**Achievements:**
- ✅ TTL=0 indefinite lease design added and propagated to tasks
- ✅ Sequential workflow impact understood (non-breaking, opt-in)
- ✅ Non-parallel worktree usage clarified (no conflicts)
- ✅ Information access model clarified (read vs write separation)

**Critical findings:**
- ❌ Design document missing explicit read access specification
- ❌ Issue creation chicken-and-egg problem needs resolution
- ❌ Query behavior across worktrees needs documentation

**Next steps:**
- Update design document with missing sections (read/write ops, issue creation, query behavior)
- Create clarification tasks/issues
- Continue implementation with clear read vs write model
- Test read access, issue creation, and query coordination thoroughly

## References

- Design document: `dev/design/worktree-parallel-work.md`
- Epic: `ad601a15` - Enable parallel multi-agent work with git worktrees
- New task: `608ae1e8` - Implement TTL=0 indefinite lease support
- Updated tasks: `e66da7c0`, `1a4c1f79`, `1a5e737d`, `ee20327b`, `77206cf6`, `3f2cf966`, `bc3742d9`, `62ff2fd9`
