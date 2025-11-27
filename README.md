# Just-In-Time Issue Tracker

A repository-local CLI issue tracker designed for agent orchestration with dependency graph enforcement and quality gating.

## Features

- **Dependency Graph Management**: Issues form a directed acyclic graph (DAG) with cycle detection
- **Quality Gates**: Enforce process requirements (tests, reviews, scans) before state transitions
- **Agent Orchestration**: Coordinator daemon dispatches work to multiple agents (Copilot, CI, custom)
- **Event Logging**: Full audit trail of all operations in append-only log
- **Priority-based Dispatch**: Intelligent work assignment based on priority and agent availability
- **Machine-first Design**: Deterministic, versionable plain-text storage for automation

## Quick Start

```bash
# Install (assuming Rust is installed)
cd cli
cargo build --release

# Initialize repository
./target/release/jit init

# Create your first issue
jit issue create --title "Setup project" --priority high

# Add gate definitions
jit registry add unit-tests --title "Unit Tests" --auto true
jit registry add review --title "Code Review" --auto false

# Add gates to an issue
jit gate add <issue-id> unit-tests
jit gate add <issue-id> review
```

## Agent Orchestration Workflow

### Scenario: Lead Agent Orchestrating Multiple Agents

This example shows a lead Copilot agent that breaks down work and orchestrates other agents to complete it.

#### 1. Setup: Initialize Coordinator with Agent Pool

```bash
# Create coordinator configuration
jit coordinator init-config

# Edit data/coordinator.json to configure your agents:
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
jit dep add $EPIC_ID --on $TASK1
jit dep add $EPIC_ID --on $TASK2
jit dep add $EPIC_ID --on $TASK3
jit dep add $EPIC_ID --on $TASK4

# Task 4 depends on tasks 1-3 being complete
jit dep add $TASK4 --on $TASK1
jit dep add $TASK4 --on $TASK2
jit dep add $TASK4 --on $TASK3
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
jit dep add $EPIC_ID --on $TASK5

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
jit issue claim $EPIC_ID --to copilot:copilot-lead

# Run final integration tests, pass gates
jit gate pass $EPIC_ID integration-tests --by "ci:github-actions"
jit gate pass $EPIC_ID review --by "human:tech-lead"

# Mark epic complete
jit issue update $EPIC_ID --state done

# Stop coordinator when done
jit coordinator stop
```

## Key Capabilities for Agent Orchestration

1. **Dynamic Issue Creation**: Agents can create new issues on-demand as they discover work
2. **Dependency Management**: Express prerequisites and blocked states automatically
3. **Quality Gates**: Enforce process (tests, reviews) before transitions
4. **Priority Queuing**: Critical work automatically dispatched first
5. **Event Sourcing**: Full audit trail of who did what when
6. **Atomic Operations**: `claim-next` ensures no race conditions
7. **Graph Queries**: Understand relationships and impact

## Architecture

- **Storage**: Plain JSON files in `data/` directory (versionable, transparent)
- **Coordinator**: Push-based daemon that monitors and dispatches work
- **Agents**: Any process that can execute `jit` commands
- **Events**: Append-only log for audit and debugging

See [docs/design.md](docs/design.md) for detailed design specifications.
