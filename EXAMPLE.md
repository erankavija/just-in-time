# Agent Orchestration Example

This example demonstrates a lead Copilot agent orchestrating multiple worker agents to complete a complex task.

## Setup

```bash
# Build the CLI
cd cli
cargo build --release
alias jit=./target/release/jit

# Initialize repository
mkdir my-project && cd my-project
jit init

# Setup gate definitions
jit registry add unit-tests --title "Unit Tests" --auto
jit registry add review --title "Code Review"
jit registry add integration-tests --title "Integration Tests" --auto

# Initialize coordinator with agent pool
jit coordinator init-config

# Edit .jit/coordinator.json to configure your agents
# (The default config has 2 example Copilot agents)
```

## Workflow: Lead Agent Breaks Down Work

### 1. Lead Agent Creates Epic and Tasks

```bash
# Create the epic
EPIC=$(jit issue create \
  --title "Implement user authentication" \
  --desc "Complete auth system with JWT" \
  --priority high \
  --gate review --gate integration-tests | grep -oP 'Created issue: \K.*')

# Lead agent analyzes requirements and creates sub-tasks
TASK1=$(jit issue create \
  --title "Create user model" \
  --desc "SQLAlchemy model with email, password_hash fields" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K.*')

TASK2=$(jit issue create \
  --title "Implement login endpoint" \
  --desc "POST /api/login endpoint with JWT generation" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K.*')

TASK3=$(jit issue create \
  --title "Add authentication middleware" \
  --desc "Verify JWT tokens on protected routes" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K.*')

# Setup dependency graph: epic depends on all tasks
jit dep add $EPIC $TASK1
jit dep add $EPIC $TASK2
jit dep add $EPIC $TASK3

# View the dependency tree
jit graph show $EPIC
```

### 2. Pass Gates and Mark Tasks Ready

```bash
# Pass gates for tasks (in real scenario, CI would do this automatically)
jit gate pass $TASK1 unit-tests --by "ci:github-actions"
jit gate pass $TASK1 review --by "human:tech-lead"

jit gate pass $TASK2 unit-tests --by "ci:github-actions"
jit gate pass $TASK2 review --by "human:tech-lead"

jit gate pass $TASK3 unit-tests --by "ci:github-actions"
jit gate pass $TASK3 review --by "human:tech-lead"

# Mark tasks as ready for work
jit issue update $TASK1 --state ready
jit issue update $TASK2 --state ready
jit issue update $TASK3 --state ready

# Check status
jit status
```

### 3. Agents Claim and Execute Work

```bash
# Worker agents claim tasks (could be automated by coordinator)
jit issue claim $TASK1 copilot:worker-1
jit issue claim $TASK2 copilot:worker-2

# Check active agents
jit coordinator agents

# View status
jit status
# Output shows:
#   Ready: 1
#   In Progress: 2
```

### 4. Dynamic Issue Creation

While working, an agent discovers additional work needed:

```bash
# Worker discovers security requirement
TASK4=$(jit issue create \
  --title "Add rate limiting to login" \
  --desc "Prevent brute force attacks - 5 attempts per minute" \
  --priority critical \
  --gate unit-tests --gate security-scan | grep -oP 'Created issue: \K.*')

# Add as dependency to epic
jit dep add $EPIC $TASK4

# Pass gates and mark ready
jit gate pass $TASK4 unit-tests --by "ci:github-actions"
jit gate pass $TASK4 security-scan --by "ci:snyk"
jit issue update $TASK4 --state ready

# Another agent can claim it
jit issue claim $TASK4 copilot:worker-1
```

### 5. Complete Tasks

```bash
# As agents finish work, mark tasks complete
jit issue update $TASK1 --state done
jit issue update $TASK2 --state done
jit issue update $TASK3 --state done
jit issue update $TASK4 --state done

# Check status - epic should now be unblocked
jit status
```

### 6. Lead Agent Completes Epic

```bash
# Epic dependencies are done, pass gates
jit gate pass $EPIC review --by "human:tech-lead"
jit gate pass $EPIC integration-tests --by "ci:github-actions"

# Mark epic ready and claim it
jit issue update $EPIC --state ready
jit issue claim $EPIC copilot:lead

# Lead agent does final integration and marks complete
jit issue update $EPIC --state done

# Final status
jit status
# Output shows:
#   Done: 5
```

## Monitoring and Observability

```bash
# View event log
jit events tail -n 20

# Query specific events
jit events query --event-type gate_passed --limit 10

# Check dependency relationships
jit graph show $EPIC
jit graph downstream $TASK1

# Find root issues (no dependencies)
jit graph roots

# Coordinator status
jit coordinator status
```

## Running with Coordinator Daemon

For automatic dispatch (instead of manual claiming):

```bash
# Start coordinator (dispatches ready issues to available agents)
jit coordinator start

# In another terminal, create and mark issues ready
# The coordinator will automatically assign them to agents

# Stop coordinator
jit coordinator stop
```

## Key Concepts

- **Dynamic Issue Creation**: Agents create issues as they discover work
- **Dependency Graph**: Express prerequisites, automatic blocking
- **Quality Gates**: Enforce process before transitions
- **Priority Queuing**: Critical work dispatched first
- **Event Sourcing**: Full audit trail
- **Atomic Claiming**: No race conditions with `claim`
- **Graph Queries**: Understand relationships and impact

## Advanced Patterns

### Parallel Work with Dependencies

```bash
# Create parallel tasks that don't depend on each other
FEATURE_A=$(jit issue create --title "Feature A" --priority high ...)
FEATURE_B=$(jit issue create --title "Feature B" --priority high ...)

# But both needed for integration
INTEGRATION=$(jit issue create --title "Integration tests" --priority normal ...)
jit dep add $INTEGRATION $FEATURE_A
jit dep add $INTEGRATION $FEATURE_B

# Features can be worked on in parallel
# Integration automatically blocked until both complete
```

### Claiming Next Available Work

```bash
# Agent claims highest priority ready issue
jit issue claim-next --to copilot:worker-1

# With filter (future enhancement)
jit issue claim-next --filter "priority:high|critical" --to copilot:worker-1
```

### Cross-agent Communication via Context

```bash
# Agent adds context for other agents
jit issue update $TASK1 --context "agent_notes=Remember to update docs"

# View issue with context
jit issue show $TASK1
```
