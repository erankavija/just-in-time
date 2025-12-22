# Orchestrator Separation Design

**Status:** ✅ Completed (2025-11-30)  
**Follow-up:** ✅ CLI Improvements & Auto-transitions (2025-12-01)

## Executive Summary

The original jit design included a built-in coordinator daemon for agent orchestration. This document describes the architectural decision to extract the orchestrator into a separate `jit-dispatch` crate, creating a cleaner separation of concerns and more flexible deployment model.

## Original Architecture (Before Separation)

```
jit (single binary)
├── Core issue tracker (CRUD, dependencies, gates)
├── Query interface (ready, blocked, assignee)
└── Built-in coordinator daemon
    ├── Config loading (.jit/coordinator.json)
    ├── Agent pool management
    ├── Polling & dispatch logic
    └── Capacity tracking
```

**Problems with this approach:**
1. **Tight coupling** - Coordinator logic mixed with issue tracker core
2. **Single process** - Daemon must run alongside CLI operations
3. **Testing complexity** - Hard to test coordinator independently
4. **Deployment inflexibility** - Can't run tracker without coordinator
5. **Code bloat** - 732 lines of coordinator code in core binary

## New Architecture (After Separation)

```
┌─────────────────────────────────────────────────────────┐
│ Workspace: just-in-time                                 │
├─────────────────────────────────────────────────────────┤
│                                                          │
│  ┌─────────────────┐        ┌──────────────────┐       │
│  │ jit             │        │ jit-dispatch     │       │
│  │ (Core Tracker)  │◄───────│ (Orchestrator)   │       │
│  ├─────────────────┤        ├──────────────────┤       │
│  │ • CRUD ops      │        │ • Config loader  │       │
│  │ • Dependencies  │        │ • Agent pool     │       │
│  │ • Gates         │        │ • Polling loop   │       │
│  │ • State mgmt    │        │ • Priority queue │       │
│  │ • Event log     │        │ • Work tracker   │       │
│  │ • Query API     │        │ • CLI daemon     │       │
│  └─────────────────┘        └──────────────────┘       │
│         ▲                            │                  │
│         │                            │                  │
│         │  jit query ready           │                  │
│         │  jit issue claim <id>      │                  │
│         └────────────────────────────┘                  │
│                                                          │
│  Shared: .jit/ directory (issues, gates, events, index) │
└─────────────────────────────────────────────────────────┘
```

## Implementation Plan (Executed)

### Phase 1: Extract Core Domain ✅

**Objective:** Move coordinator-specific code out of jit core

**Changes:**
- ✅ Created `crates/dispatch/` for orchestrator
- ✅ Moved 732 lines of coordinator code
- ✅ Kept `jit` focused on issue tracking primitives
- ✅ Updated workspace `Cargo.toml` to include both crates

**Files moved:**
- Config types → `dispatch/src/lib.rs`
- Orchestrator logic → `dispatch/src/lib.rs`
- Daemon CLI → `dispatch/src/main.rs`
- Coordinator tests → `dispatch/tests/`

### Phase 2: Define Clean Interface ✅

**Objective:** Orchestrator uses jit as a subprocess, not a library

**Design decision:**
```rust
// dispatch/src/lib.rs
impl Orchestrator {
    pub fn query_ready_issues(&self) -> Result<Vec<Issue>> {
        // Spawn: jit query ready --json
        // Parse JSON output
        // Return structured data
    }
    
    pub fn claim_issue_for_agent(&self, issue_id: &str, agent: &str) -> Result<()> {
        // Spawn: jit issue claim <id> <agent>
        // Check exit status
    }
}
```

**Benefits:**
- Clear process boundary
- Works with any jit version (forward compatible)
- No shared memory or locks needed
- Easy to test with mocked subprocess calls

### Phase 3: Configuration ✅

**Files:**
- `dispatch/dispatch.toml.example` - Sample configuration
- Runtime: Load from `./dispatch.toml` or path specified via CLI

**Config structure:**
```toml
poll_interval_secs = 30

[[agents]]
id = "copilot-1"
type = "github-copilot"
capacity = 3
tags = ["rust", "python"]

[[agents]]
id = "ci-runner-1"  
type = "github-actions"
capacity = 5
tags = ["build", "test"]
```

### Phase 4: Testing Strategy ✅

**Test organization:**
- `crates/jit/tests/` - Core tracker tests (123 tests)
- `crates/dispatch/tests/` - Orchestrator tests (9 tests)
- `crates/jit/tests/test_no_coordinator.rs` - Verify removal (6 tests)

**Test coverage:**
- ✅ Orchestrator query ready issues
- ✅ Orchestrator assigns by priority
- ✅ Orchestrator claims issues for agents
- ✅ Dispatch cycle logic
- ✅ Agent capacity tracking
- ✅ Multi-agent coordination

### Phase 5: Documentation ✅

**Updated files:**
- `dispatch/README.md` - Orchestrator usage guide
- `ROADMAP.md` - Marked coordinator extraction complete
- `TESTING.md` - Updated test structure
- Root `README.md` - Updated architecture section

## Build Your Own Orchestrator

The separation enables building custom orchestrators in any language. See `crates/dispatch/examples/` for:

- **Bash one-liner** (`bash-orchestrator.sh`) - 50 lines, minimal polling orchestrator
- **Python orchestrator** (`simple-orchestrator.py`) - 130 lines, agent pool + priority dispatch
- **Integration patterns** (`integration-patterns.md`) - GitHub Actions, Kubernetes, webhooks, multi-repo

**Key insight:** jit's CLI-based API makes it trivial to build orchestrators in any language that can spawn processes and parse JSON.

## Usage Examples

### Running the Core Tracker (jit)

```bash
# Initialize repository
jit init

# Create issues
jit issue create -t "Implement parser" -p high
jit issue create -t "Write tests" -p normal

# Add dependencies
jit dep add <test-id> <parser-id>

# Query available work
jit query ready --json
```

### Running the Orchestrator (jit-dispatch)

```bash
# One-time dispatch cycle
jit-dispatch once

# Run as daemon
jit-dispatch start

# With custom config
jit-dispatch start --config ./my-dispatch.toml
```

### Agent Integration Pattern

```python
# Example Python agent
import subprocess
import json

def get_next_task(agent_id):
    # Orchestrator has claimed issue for us
    result = subprocess.run(
        ["jit", "query", "assignee", agent_id, "--json"],
        capture_output=True
    )
    tasks = json.loads(result.stdout)
    return tasks[0] if tasks else None

def complete_task(issue_id):
    subprocess.run(["jit", "issue", "update", issue_id, "--state", "done"])
```

## Benefits Realized

### 1. Clean Separation of Concerns
- **jit**: Pure issue tracker (no orchestration logic)
- **jit-dispatch**: Pure orchestrator (no issue storage)
- Each tool has a single, clear responsibility

### 2. Independent Evolution
- Can update jit without touching orchestrator
- Can swap out orchestrator for different implementation
- Version jit and dispatch separately

### 3. Flexible Deployment
- Run jit standalone (manual workflow)
- Run jit + dispatch (automated workflow)
- Run multiple dispatchers against one jit repo
- Deploy dispatch on different machine than jit

### 4. Better Testing
- Test jit without mocking coordination
- Test dispatch without full issue tracker
- Integration tests use real subprocess calls
- Each crate has focused test suite

### 5. Simpler Onboarding
- New users start with just `jit` commands
- Add `jit-dispatch` when ready for automation
- Clear mental model: tracker vs. orchestrator

## Migration Path (Completed)

**Before (single binary):**
```bash
jit coord start  # Built-in daemon
```

**After (separate binaries):**
```bash
jit-dispatch start  # External orchestrator
```

**Breaking changes:**
- ✅ Removed `jit coord` subcommand (732 lines)
- ✅ Moved `.jit/coordinator.json` → `dispatch.toml`
- ✅ Config format slightly different (agent capacity added)

## Current Features (Implemented)

The `jit-dispatch` orchestrator includes:

✅ **Polling for ready issues** - Configurable interval (default 30s)  
✅ **Agent pool management** - Track capacity and assignments  
✅ **Priority-based dispatch** - Critical > High > Normal > Low  
✅ **Capacity tracking** - Respect `max_concurrent` per agent  
✅ **Daemon mode** - Continuous polling with `start` command  
✅ **One-shot mode** - Single cycle with `once` command  
✅ **Auto-promotion** - Handled by jit core auto-transitions (2025-12-01)

**Example minimal orchestrator usage:**

```bash
# Create dispatch.toml
cat > dispatch.toml <<EOF
poll_interval_secs = 30

[[agents]]
id = "copilot-1"
type = "github-copilot"
max_concurrent = 3
command = "copilot-agent"

[[agents]]
id = "ci-runner-1"
type = "github-actions"  
max_concurrent = 5
command = "ci-agent"
EOF

# Run daemon
jit-dispatch start

# Or run once
jit-dispatch once
```

## Future Enhancements

### Orchestrator Improvements
- [ ] **Stalled work detection** - Timeout and reclaim stuck issues
- [ ] **Agent health checks** - Ping/heartbeat for agent status
- [ ] **Work stealing** - Rebalance between agents
- [ ] **Priority escalation** - Age-based priority boost
- [ ] **Agent specialization** - Match by tags/labels

### Alternative Orchestrators
- [ ] Kubernetes operator (deploy as CRD)
- [ ] GitHub Actions dispatcher
- [ ] AWS Lambda orchestrator
- [ ] Custom Go/Python orchestrators

### Advanced Patterns
- [ ] Multi-repo orchestration
- [ ] Hierarchical agents (supervisors + workers)
- [ ] Federated dispatch (multiple coordinators)
- [ ] Real-time websocket updates

## Lessons Learned

1. **CLI as API** - Using subprocess calls as the integration point was brilliant:
   - No version coupling
   - No shared libraries
   - Process isolation
   - Easy debugging

2. **Test Coverage** - Having comprehensive tests before refactoring was critical:
   - 123 tests caught regressions immediately
   - Could refactor confidently
   - Tests documented behavior

3. **Workspace Structure** - Rust workspaces made this refactor smooth:
   - Shared dependencies
   - Single `cargo test --workspace`
   - Clear module boundaries

4. **JSON Output** - The `--json` flag on all commands enabled orchestration:
   - Machine-readable output
   - Structured data
   - Easy parsing

## Follow-up Work (Completed 2025-12-01)

After the orchestrator separation, we implemented major UX improvements:

### CLI Consistency Redesign ✅
- Changed all commands to use intuitive positional arguments
- Before: `jit issue claim <id> --to <assignee>`
- After: `jit issue claim <id> <assignee>`
- Applied to: claim, assign, release, dep add, gate operations
- Added comma-separated support: `--gate review,tests`

### Auto-Transition to Ready ✅
- Issues without blockers auto-transition Open → Ready
- Adding dependencies/gates transitions Ready → Open
- Completing dependencies auto-transitions dependents to Ready
- Passing gates auto-transitions to Ready when unblocked
- Much more intuitive than manual state management

### Comprehensive Workflow Tests ✅
- Added 8 end-to-end workflow integration tests
- Updated 140+ existing tests for new behavior
- All 199 tests passing across 15 test suites
- Zero clippy warnings

## References

- **Design doc:** `docs/design.md` - Original architecture
- **Roadmap:** `ROADMAP.md` - Phase tracking
- **Testing:** `TESTING.md` - Test strategy and coverage
- **Dispatch README:** `crates/dispatch/README.md` - Orchestrator usage
- **Example config:** `crates/dispatch/dispatch.toml.example`

## Conclusion

The orchestrator separation was a successful refactoring that:
- ✅ Improved code quality (cleaner boundaries)
- ✅ Enhanced testability (focused test suites)
- ✅ Increased flexibility (independent deployment)
- ✅ Simplified core (removed 732 lines)
- ✅ Enabled future extensibility (alternative orchestrators)

The subsequent CLI improvements and auto-transition implementation further enhanced the system's usability and intuitiveness.

**Total impact:** 
- Removed 732 lines from core
- Added 9 orchestrator tests
- Added 8 workflow tests
- Updated 140+ tests for new behavior
- Zero regressions, all 199 tests passing
- Production-ready, well-documented system
