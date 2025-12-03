# Just-In-Time Issue Tracker

**Enable AI agents to orchestrate their own work.** A repository-local CLI issue tracker that lets a lead agent break down complex tasks and coordinate multiple worker agents—with dependency management, quality gates, and full observability.

## Why JIT?

Traditional issue trackers are designed for humans. JIT is designed for **AI agents**:

- 🤖 **Agent-First**: Copilot agents can create issues, claim work, and coordinate with each other
- 🔗 **Dependency DAG**: Express "Task B needs Task A" with automatic blocking and cycle detection  
- ✅ **Quality Gates**: Enforce tests, reviews, scans before work can proceed
- 📊 **Full Observability**: Event log tracks every action for debugging agent behavior
- 🎯 **Priority Dispatch**: Coordinator automatically assigns critical work first
- 📁 **Git-Friendly**: All state in plain JSON—version, diff, and merge like code

## Use Cases

- **Multi-agent software development**: Lead agent architects, workers implement, another tests
- **CI/CD orchestration**: Gate progression through build → test → deploy pipeline
- **Research projects**: Break down analysis into parallel tasks with dependencies
- **Any workflow** where you want agents to discover and create work dynamically

## Quick Start

```bash
# 1. Install (requires Rust)
cd cli && cargo build --release

# 2. Initialize in your project (creates .jit/ directory)
./target/release/jit init

# 3. Create your first issue
jit issue create --title "Implement login feature" --priority high

# 4. Setup coordinator for agent orchestration  
jit coordinator init-config

# That's it! Now agents can create issues, claim work, and coordinate.
```

**Note:** All data is stored in `.jit/` directory (similar to `.git/`). Override with `JIT_DATA_DIR` environment variable if needed.

See [EXAMPLE.md](EXAMPLE.md) for complete workflows.

## How It Works

### 1. Lead Agent Breaks Down Work

```bash
# Lead agent creates an epic
EPIC=$(jit issue create --title "User authentication" --priority high)

# Analyzes and creates sub-tasks
TASK1=$(jit issue create --title "Create user model" --priority high)
TASK2=$(jit issue create --title "Implement login endpoint" --priority high)

# Defines dependencies
jit dep add $EPIC $TASK1
jit dep add $EPIC $TASK2
```

### 2. Worker Agents Execute Tasks

```bash
# Workers claim ready tasks
jit issue claim $TASK1 copilot:worker-1
jit issue claim $TASK2 copilot:worker-2

# Work on them, pass quality gates
jit gate pass $TASK1 unit-tests
jit issue update $TASK1 --state done
```

### 3. Dynamic Issue Creation

```bash
# Agent discovers new work while executing
NEW=$(jit issue create --title "Add rate limiting" --priority critical)
jit dep add $EPIC $NEW  # Epic now waits for this too
```

### 4. Monitor Everything

```bash
jit status                  # Overview of work state
jit coordinator agents      # See what each agent is doing
jit events tail             # Full audit trail
jit graph show $EPIC        # Visualize dependencies
```

## Complete Example Workflow

#### 1. Setup: Initialize Coordinator with Agent Pool

```bash
# Create coordinator configuration
jit coordinator init-config

# Edit .jit/coordinator.json to configure your agents:
{
  "agent_pool": [
    {
      "id": "copilot-lead",
      "agent_type": "copilot",
      "command": "copilot-agent-lead",
      "max_concurrent": 1
    },
    {
      "id": "copilot-worker-1",
      "agent_type": "copilot",
      "command": "copilot-agent-worker",
      "max_concurrent": 2
    },
    {
      "id": "copilot-worker-2",
      "agent_type": "copilot",
      "command": "copilot-agent-worker",
      "max_concurrent": 2
    }
  ],
  "dispatch_rules": {
    "priority_order": ["critical", "high", "normal", "low"],
    "stall_timeout_minutes": 30
  },
  "poll_interval_secs": 5
}
```

#### 2. Lead Agent: Break Down Epic into Tasks

The lead agent creates an epic and dynamically creates sub-issues:

```bash
# Lead agent claims the epic
EPIC_ID=$(jit issue create --title "Implement user authentication" \
  --desc "Complete auth system with tests" \
  --priority high \
  --gate review --gate integration-tests)

# Lead agent analyzes and creates sub-tasks dynamically
TASK1=$(jit issue create --title "Create user model" \
  --desc "SQLAlchemy model with fields" \
  --priority high \
  --gate unit-tests --gate review)

TASK2=$(jit issue create --title "Implement login endpoint" \
  --desc "POST /api/login with JWT" \
  --priority high \
  --gate unit-tests --gate review)

TASK3=$(jit issue create --title "Add authentication middleware" \
  --desc "Verify JWT on protected routes" \
  --priority high \
  --gate unit-tests --gate review)

TASK4=$(jit issue create --title "Write integration tests" \
  --desc "E2E tests for auth flow" \
  --priority normal \
  --gate review)

# Set up dependencies: epic depends on all tasks
jit dep add $EPIC_ID $TASK1
jit dep add $EPIC_ID $TASK2
jit dep add $EPIC_ID $TASK3
jit dep add $EPIC_ID $TASK4

# Task 4 depends on tasks 1-3 being complete
jit dep add $TASK4 $TASK1
jit dep add $TASK4 $TASK2
jit dep add $TASK4 $TASK3
```

#### 3. Worker Agents: Execute Tasks

```bash
# Mark tasks as ready for work (assuming no blockers)
jit issue update $TASK1 --state ready
jit issue update $TASK2 --state ready
jit issue update $TASK3 --state ready

# Start coordinator daemon (dispatches to available agents)
jit coordinator start

# Coordinator automatically:
# - Finds ready issues (TASK1, TASK2, TASK3)
# - Assigns them to available worker agents
# - Logs events for audit trail
```

#### 4. Agent Work Simulation

Each worker agent:
1. Receives assignment via coordinator
2. Does the work (code, tests, etc.)
3. Passes gates as work completes
4. Marks issue complete

```bash
# Worker agent workflow (automated by coordinator)
# Agent copilot-worker-1 working on TASK1:
jit gate pass $TASK1 unit-tests --by "copilot-worker-1"
jit gate pass $TASK1 review --by "human:tech-lead"
jit issue update $TASK1 --state done

# Agent copilot-worker-2 working on TASK2:
jit gate pass $TASK2 unit-tests --by "copilot-worker-2"
jit gate pass $TASK2 review --by "human:tech-lead"
jit issue update $TASK2 --state done
```

#### 5. Dynamic Issue Creation During Execution

While working, an agent discovers new work needed:

```bash
# Worker agent discovers missing security requirement
TASK5=$(jit issue create --title "Add rate limiting to login" \
  --desc "Prevent brute force attacks" \
  --priority critical \
  --gate unit-tests --gate security-scan)

# Add as dependency to epic
jit dep add $EPIC_ID $TASK5

# Mark ready and coordinator will dispatch it
jit issue update $TASK5 --state ready
```

#### 6. Monitoring Progress

```bash
# Check coordinator status
jit coordinator status
# Output:
# Status: Running (PID: 12345)
# Agent pool: 3 agents
#   - copilot-lead (type: copilot, max_concurrent: 1)
#   - copilot-worker-1 (type: copilot, max_concurrent: 2)
#   - copilot-worker-2 (type: copilot, max_concurrent: 2)
# 
# Work queue:
#   Ready: 2
#   In Progress: 3

# List active agents and assignments
jit coordinator agents
# Output:
# Active agents:
# 
# copilot:copilot-worker-1
#   - abc123 | Implement login endpoint
#   - def456 | Add rate limiting to login
# 
# copilot:copilot-worker-2
#   - ghi789 | Add authentication middleware

# View dependency graph
jit graph show $EPIC_ID
# Shows full tree of dependencies

# Check overall status
jit status
# Output:
# Open: 2  Ready: 1  In Progress: 3  Blocked: 0  Done: 5
# 
# Recent completions:
#   - Create user model (3m ago)
#   - Implement login endpoint (1m ago)

# Query event log
jit events tail -n 20
# Shows recent events: issue.created, issue.claimed, gate.passed, issue.completed
```

#### 7. Completion and Cleanup

```bash
# Once all tasks done, epic becomes unblocked
# Lead agent can claim and finalize
jit issue claim $EPIC_ID copilot:copilot-lead

# Run final integration tests, pass gates
jit gate pass $EPIC_ID integration-tests --by "ci:github-actions"
jit gate pass $EPIC_ID review --by "human:tech-lead"

# Mark epic complete
jit issue update $EPIC_ID --state done

# Stop coordinator when done
jit coordinator stop
```

**See the full step-by-step workflow in [EXAMPLE.md](EXAMPLE.md)** showing:
- Lead agent orchestrating multiple workers
- Dynamic issue creation during execution  
- Dependency management and gate passing
- Monitoring and observability

## Key Features

| Feature | Description |
|---------|-------------|
| **Dependency DAG** | Issues form a directed acyclic graph with automatic cycle detection |
| **Quality Gates** | Tests, reviews, scans must pass before state transitions |
| **Coordinator Daemon** | Automatically dispatches ready work to available agents |
| **Priority Dispatch** | Critical work gets assigned first |
| **Event Log** | Full audit trail—every action logged to `.jit/events.jsonl` |
| **Git-Friendly Storage** | Plain JSON files you can version, diff, and merge |
| **Atomic Operations** | `claim` ensures no race conditions between agents |
| **Graph Queries** | Understand upstream/downstream dependencies |

## Commands

```bash
# Issue Management
jit issue create --title "..." --priority high
jit issue list --state ready
jit issue claim <id> agent:worker-1
jit issue update <id> --state done

# Dependencies
jit dep add <from-issue> <to-dependency>
jit graph show <issue>

# Quality Gates  
jit gate add <issue> unit-tests
jit gate pass <issue> unit-tests

# Coordination
jit coordinator start
jit coordinator agents
jit coordinator status

# Monitoring
jit status
jit events tail -n 20
```

## Project Status

✅ **Phase 0**: Design and architecture  
✅ **Phase 1**: Core issue management with dependency graph  
✅ **Phase 2**: Quality gates and coordinator daemon  
🚧 **Phase 3**: Advanced observability (graph export, webhooks)  
📋 **Phase 4**: Production readiness (locking, plugins, metrics)

See [ROADMAP.md](ROADMAP.md) for details.

## Documentation

- [EXAMPLE.md](EXAMPLE.md) - Complete agent orchestration walkthrough
- [docs/design.md](docs/design.md) - Detailed design specifications
- [ROADMAP.md](ROADMAP.md) - Development phases and progress
- [TESTING.md](TESTING.md) - Testing strategy and best practices
- [docs/storage-abstraction.md](docs/storage-abstraction.md) - Pluggable backend design (next priority)

## Architecture

```
┌─────────────┐
│ Coordinator │ ← Monitors ready issues, dispatches to agents
└──────┬──────┘
       │
   ┌───┴────┬────────┬─────────┐
   │        │        │         │
┌──▼───┐ ┌─▼────┐ ┌─▼────┐ ┌──▼────┐
│Agent1│ │Agent2│ │Agent3│ │CI/CD  │
└──┬───┘ └──┬───┘ └──┬───┘ └───┬───┘
   │        │        │         │
   └────────┴────────┴─────────┘
            │
    ┌───────▼────────┐
    │  .jit/         │
    │  ├─ issues/    │ ← One JSON file per issue
    │  ├─ gates.json │ ← Gate definitions
    │  └─ events.jsonl│ ← Append-only audit log
    └────────────────┘
```

## Contributing

Built with Rust. Requires Rust 1.70+.

```bash
cargo build
cargo test
cargo clippy
cargo fmt
```

## License

MIT
