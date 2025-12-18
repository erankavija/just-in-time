# Just-In-Time Issue Tracker

[![CI](https://github.com/erankavija/just-in-time/actions/workflows/ci.yml/badge.svg)](https://github.com/erankavija/just-in-time/actions/workflows/ci.yml)
[![Docker](https://github.com/erankavija/just-in-time/actions/workflows/docker.yml/badge.svg)](https://github.com/erankavija/just-in-time/actions/workflows/docker.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

**Enable AI agents to orchestrate their own work.** A repository-local CLI issue tracker that lets a lead agent break down complex tasks and coordinate multiple worker agents—with dependency management, quality gates, and full observability.

## Why JIT?

Traditional issue trackers are designed for humans. JIT is designed for **AI agents**:

- 🤖 **Agent-First**: Copilot agents can create issues, claim work, and coordinate with each other
- 🔗 **Dependency DAG**: Express "Task B needs Task A" with automatic blocking and cycle detection  
- ✅ **Quality Gates**: Enforce tests, reviews, scans before work can proceed
- 🏗️ **Issue Hierarchy**: Organize work with epics, milestones, and strategic/tactical views
- ⚙️ **Configurable**: Customize type hierarchies and validation rules per repository
- 🔒 **Multi-Agent Safe**: File locking prevents race conditions with concurrent agents
- 📊 **Full Observability**: Event log tracks every action for debugging agent behavior
- 🎯 **Priority Dispatch**: Coordinator automatically assigns critical work first
- 📁 **Git-Friendly**: All state in plain JSON—version, diff, and merge like code

## Use Cases

- **Multi-agent software development**: Lead agent architects, workers implement, another tests
- **CI/CD orchestration**: Gate progression through build → test → deploy pipeline
- **Research projects**: Break down analysis into parallel tasks with dependencies
- **Any workflow** where you want agents to discover and create work dynamically

## Quick Start

### Installation

**Pre-built binaries (Linux x64):**
```bash
wget https://github.com/erankavija/just-in-time/releases/latest/download/jit-linux-x64.tar.gz
tar -xzf jit-linux-x64.tar.gz
sudo mv jit jit-server jit-dispatch /usr/local/bin/
```

**Docker:**
```bash
docker-compose up -d
# Access Web UI at http://localhost:8080
# API at http://localhost:3000
```

**From source:**
```bash
cargo build --release --workspace
```

See [INSTALL.md](INSTALL.md) for all installation options.

### Usage

```bash
# 1. Initialize in your project (creates .jit/ directory)
jit init

# 2. Create your first issue (labels optional!)
jit issue create --title "Implement login feature" --priority high

# OR with labels for hierarchy and organization
jit issue create --title "Implement login feature" \
  --label "type:epic" --label "milestone:v1.0" --priority high

# 3. That's it! Start tracking work
jit status
jit issue list
```

**Note:** 
- All data is stored in `.jit/` directory (similar to `.git/`)
- **Labels are optional** - use them when you need organizational structure
- Override storage location with `JIT_DATA_DIR` environment variable

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
| **Issue Hierarchy** | Labels define epics, milestones, and organizational structure |
| **Strategic Views** | Filter and visualize high-level planning vs tactical work |
| **Configurable Rules** | Customize type hierarchies and validation per repository |
| **Coordinator Daemon** | Automatically dispatches ready work to available agents |
| **Priority Dispatch** | Critical work gets assigned first |
| **Event Log** | Full audit trail—every action logged to `.jit/events.jsonl` |
| **Git-Friendly Storage** | Plain JSON files you can version, diff, and merge |
| **Atomic Operations** | `claim` ensures no race conditions between agents |
| **Graph Queries** | Understand upstream/downstream dependencies |

## Issue Lifecycle

Issues progress through states with automated quality checks:

```
backlog → ready → in_progress → gated → done
           ↑         ↓            ↓
           └──── prechecks    postchecks
              (validate)    (verify quality)
```

**States:**
- **backlog**: Has incomplete dependencies
- **ready**: Dependencies done, available to start
- **in_progress**: Work actively happening
- **gated**: Work complete, awaiting quality gate approval
- **done**: All gates passed, complete

**Quality Gates:**
- **Prechecks** run before work starts (e.g., TDD: verify tests exist)
- **Postchecks** run after work completes (e.g., run tests, lint, security scans)
- Gates can be **manual** (human approval) or **automated** (script execution)

See [docs/ci-gate-integration-design.md](docs/ci-gate-integration-design.md) for gate details.
| **Strategic Views** | Filter and visualize high-level planning vs tactical work |
| **Configurable Rules** | Customize type hierarchies and validation per repository |
| **Coordinator Daemon** | Automatically dispatches ready work to available agents |
| **Priority Dispatch** | Critical work gets assigned first |
| **Event Log** | Full audit trail—every action logged to `.jit/events.jsonl` |
| **Git-Friendly Storage** | Plain JSON files you can version, diff, and merge |
| **Atomic Operations** | `claim` ensures no race conditions between agents |
| **Graph Queries** | Understand upstream/downstream dependencies |

## Issue Hierarchy & Organization

JIT supports organizing work with **labels-based hierarchies**:

```bash
# Create a milestone
jit issue create --title "Version 1.0 Release" \
  --label "type:milestone" --label "milestone:v1.0"

# Create an epic under that milestone
jit issue create --title "User Authentication" \
  --label "type:epic" --label "epic:auth" --label "milestone:v1.0"

# Create tasks that belong to the epic
jit issue create --title "Implement login endpoint" \
  --label "type:task" --label "epic:auth"

# Query strategic issues
jit query strategic        # Shows only milestones and epics
jit query label "epic:*"   # Shows all issues belonging to any epic
```

**Benefits:**
- **Strategic/Tactical Views**: Toggle between high-level planning and detailed work
- **Progress Tracking**: See downstream task counts and completion status
- **Flexible Naming**: Use your own terminology (themes, features, etc.)
- **Validation**: Warns about orphaned tasks or missing strategic labels

See [docs/label-conventions.md](docs/label-conventions.md) for detailed usage patterns.

## Configuration

Customize validation and type hierarchies with `.jit/config.toml`:

```toml
[type_hierarchy]
# Define your type hierarchy (lower numbers = more strategic)
types = { milestone = 1, epic = 2, story = 3, task = 4 }

# Map types to label namespaces
[type_hierarchy.label_associations]
epic = "epic"
milestone = "milestone"

[validation]
# Control validation behavior
warn_orphaned_leaves = true       # Warn on tasks without parent labels
warn_strategic_consistency = true # Warn on strategic types missing labels
```

See [docs/example-config.toml](docs/example-config.toml) for more examples.

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

# Labels & Organization
jit issue create --label "type:epic" --label "milestone:v1.0"
jit query label "epic:*"
jit query strategic

# Quality Gates  
jit gate add <issue> unit-tests
jit gate pass <issue> unit-tests

# Search
jit search "authentication"                    # Search all files
jit search "bug" --glob "*.json"               # Search only issues
jit search "API" --glob "*.md"                 # Search only documents
jit search "auth(entication|orization)" --regex # Regex search
jit search "login" --json                      # JSON output

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
✅ **Phase 2**: Quality gates, web UI, and REST API  
✅ **Phase 3.1**: Full-text search with ripgrep  
✅ **Phase 4**: Issue hierarchy, labels, and configuration support  
🚧 **Phase 5**: Advanced knowledge management features (in progress)  

See [ROADMAP.md](ROADMAP.md) for details.

## Advanced Configuration

### Environment Variables

```bash
# Data directory (default: .jit/)
export JIT_DATA_DIR=/path/to/custom/dir

# Lock timeout for concurrent operations (default: 5 seconds)
export JIT_LOCK_TIMEOUT=10  # Increase for high-contention scenarios
```

### Concurrent Agent Usage

JIT is safe for concurrent access by multiple agents or processes:

```bash
# Terminal 1: Agent creating issues
jit issue create --title "Task 1"

# Terminal 2: Agent listing issues (concurrent, no conflicts)
jit issue list

# Terminal 3: Agent updating different issue (concurrent)
jit issue update <issue-id> --state done
```

File locking ensures data consistency. See [docs/file-locking-usage.md](docs/file-locking-usage.md) for details.

## Documentation

### Getting Started
- [INSTALL.md](INSTALL.md) - Installation guide (binaries, Docker, from source)
- [EXAMPLE.md](EXAMPLE.md) - Complete agent orchestration walkthrough
- [DEPLOYMENT.md](DEPLOYMENT.md) - Production deployment guide

### Core Features
- [docs/design.md](docs/design.md) - Detailed design specifications
- [docs/label-conventions.md](docs/label-conventions.md) - Issue hierarchy and labels usage
- [docs/example-config.toml](docs/example-config.toml) - Configuration examples
- [docs/file-locking-usage.md](docs/file-locking-usage.md) - Multi-agent concurrency guide

### Development
- [ROADMAP.md](ROADMAP.md) - Development phases and progress
- [TESTING.md](TESTING.md) - Testing strategy and best practices
- [docs/storage-abstraction.md](docs/storage-abstraction.md) - Pluggable backend design

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
