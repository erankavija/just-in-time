# Your First Workflow

> **Diátaxis Type:** Tutorial  
> **Time:** 30 minutes  
> **Goal:** Learn agent orchestration with epic → tasks → gates → completion

## What You'll Build

A complete user authentication system managed with JIT:
- 1 Epic (high-level feature)
- 4 Tasks (concrete work items)
- Quality gates (tests, code review)
- Dependency graph (epic depends on tasks)
- Agent claiming and coordination

This tutorial demonstrates the full power of JIT for multi-agent workflows.

## Prerequisites

- Completed [Quickstart](quickstart.md)
- JIT initialized in a project directory
- Understanding of short hashes and basic commands

## Setup: Define Quality Gates

First, define the gates we'll use for quality control:

```bash
# Automated gate - runs a checker script
jit gate define unit-tests \
  --title "Unit Tests" \
  --description "Run test suite" \
  --mode auto \
  --checker-command "cargo test"

# Manual gate - requires human judgment
jit gate define review \
  --title "Code Review" \
  --description "Peer review required" \
  --mode manual

# Automated integration tests
jit gate define integration-tests \
  --title "Integration Tests" \
  --description "End-to-end test suite" \
  --mode auto \
  --checker-command "cargo test --test integration"

# List defined gates
jit gate list
```

## Step 1: Create the Epic

Create a high-level epic to organize the work:

```bash
# Create epic with labels for organization
EPIC=$(jit issue create \
  --title "Implement user authentication" \
  --description "Complete auth system with JWT tokens" \
  --label "type:epic" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --priority high \
  --gate review \
  --gate integration-tests \
  --json | jq -r '.id')

echo "Created epic: $EPIC"

# View the epic
jit issue show $EPIC
```

**What we did:**
- Created an epic (high-level feature)
- Added labels for organization (type, epic name, milestone)
- Added quality gates (review and integration tests required)
- Used `--json` output for scripting (extract ID with jq)

## Step 2: Break Down into Tasks

Create concrete tasks that implement the epic:

```bash
# Task 1: User model
TASK1=$(jit issue create \
  --title "Create user model" \
  --description "SQLAlchemy model with email and password_hash fields" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests \
  --gate review \
  --json | jq -r '.id')

# Task 2: Login endpoint
TASK2=$(jit issue create \
  --title "Implement login endpoint" \
  --description "POST /api/login endpoint with JWT generation" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests \
  --gate review \
  --json | jq -r '.id')

# Task 3: Auth middleware
TASK3=$(jit issue create \
  --title "Add authentication middleware" \
  --description "Verify JWT tokens on protected routes" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests \
  --gate review \
  --json | jq -r '.id')

echo "Created tasks: $TASK1, $TASK2, $TASK3"
```

**What we did:**
- Created 3 concrete tasks (actual implementation work)
- Each task labeled with same epic and milestone
- Each task has component label (backend)
- Quality gates on each task (unit tests, review)

## Step 3: Build the Dependency Graph

Express that the epic depends on all tasks:

```bash
# Epic cannot complete until all tasks are done
jit dep add $EPIC $TASK1
jit dep add $EPIC $TASK2
jit dep add $EPIC $TASK3

# Visualize the dependency tree
jit graph show $EPIC

# Query what's blocked
jit query blocked
# Shows: Epic is blocked (waiting for tasks)

# Query what's ready
jit query ready
# Shows: Nothing ready (tasks have unpassed gates)
```

**What we did:**
- Created dependency relationships (epic ← tasks)
- Visualized the graph
- Queried to understand blocking

## Step 4: Pass Prechecks and Mark Ready

In a real workflow, these gates would be passed automatically or by team members:

```bash
# Pass gates for Task 1
jit gate pass $TASK1 unit-tests --by "ci:github-actions"
jit gate pass $TASK1 review --by "human:tech-lead"
jit issue update $TASK1 --state ready

# Pass gates for Task 2
jit gate pass $TASK2 unit-tests --by "ci:github-actions"
jit gate pass $TASK2 review --by "human:tech-lead"
jit issue update $TASK2 --state ready

# Pass gates for Task 3
jit gate pass $TASK3 unit-tests --by "ci:github-actions"
jit gate pass $TASK3 review --by "human:tech-lead"
jit issue update $TASK3 --state ready

# Check status
jit status
jit query ready
# Shows: All 3 tasks are now ready to claim
```

**What we did:**
- Passed required gates (simulating CI and human review)
- Transitioned tasks to ready state
- Tasks are now available for agents to claim

## Step 5: Agents Claim and Work on Tasks

Simulate multiple agents working in parallel:

```bash
# Agent 1 claims first task
jit issue claim $TASK1 agent:worker-1

# Agent 2 claims second task
jit issue claim $TASK2 agent:worker-2

# Check status
jit status
# Shows: 2 in_progress, 1 ready

# View who's working on what
jit issue list --assignee "agent:worker-1"
jit issue list --assignee "agent:worker-2"
```

**What we did:**
- Multiple agents claimed tasks atomically
- Tasks transitioned to in_progress
- One task still ready for another agent

## Step 6: Dynamic Discovery - Add More Work

While working, an agent discovers additional requirements:

```bash
# Worker discovers security requirement
TASK4=$(jit issue create \
  --title "Add rate limiting to login" \
  --description "Prevent brute force attacks - 5 attempts per minute" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:security" \
  --priority critical \
  --gate unit-tests \
  --gate review \
  --json | jq -r '.id')

# Add to epic dependencies (epic now waits for this too)
jit dep add $EPIC $TASK4

# Pass gates and make ready
jit gate pass $TASK4 unit-tests --by "ci:github-actions"
jit gate pass $TASK4 review --by "human:security-team"
jit issue update $TASK4 --state ready

# Another agent claims it
jit issue claim $TASK4 agent:worker-3

echo "Added critical security task: $TASK4"
```

**What we did:**
- Agent dynamically created new issue
- Added to dependency graph
- Another agent picked it up
- Epic automatically updated (now waits for 4 tasks)

## Step 7: Complete the Tasks

As agents finish work, mark tasks complete:

```bash
# Agents complete their work
jit issue update $TASK1 --state done
jit issue update $TASK2 --state done
jit issue update $TASK3 --state done
jit issue update $TASK4 --state done

# Check status
jit status
jit query blocked
# Epic should now be unblocked (all dependencies done)
```

**What we did:**
- Marked all tasks complete
- Epic automatically became unblocked
- Ready for final integration

## Step 8: Complete the Epic

Final integration and epic completion:

```bash
# Epic is unblocked, but still needs its own gates
jit gate pass $EPIC review --by "human:tech-lead"
jit gate pass $EPIC integration-tests --by "ci:github-actions"

# Epic is now ready
jit issue update $EPIC --state ready

# Lead agent claims and completes
jit issue claim $EPIC agent:lead
jit issue update $EPIC --state done

# Final status
jit status
# Shows: 5 done
```

**What we did:**
- Passed epic-level gates (review, integration tests)
- Lead agent claimed the epic
- Completed the epic
- Entire feature is done!

## Review and Verification

Examine what we built:

```bash
# View complete dependency graph
jit graph show $EPIC

# View all auth work
jit query label "epic:auth"

# View event log
jit events query --issue-id $EPIC

# Check milestone progress
jit query label "milestone:v1.0"
jit status
```

## What You Learned

### Core Concepts
- **Epics**: High-level features that organize tasks
- **Tasks**: Concrete work items with clear deliverables
- **Dependencies**: Express "A blocks B" (epic ← tasks)
- **Labels**: Organize work (type, epic, milestone, component)

### Workflow Patterns
- **Quality Gates**: Enforce process (tests, review)
- **Agent Claiming**: Atomic assignment, no conflicts
- **Dynamic Discovery**: Add work as you learn
- **Parallel Execution**: Multiple agents work simultaneously

### Commands Mastered
- `jit gate define` - Create quality gates
- `jit issue create` with labels and gates
- `jit dep add` - Build dependency graph
- `jit issue claim` - Atomic agent assignment
- `jit gate pass` - Mark gates as passed
- `jit query ready/blocked` - Find available work
- `jit graph show` - Visualize relationships

## Next Steps

- **[How-To: Software Development](../how-to/software-development.md)** - TDD workflows with automated gates
- **[How-To: Custom Gates](../how-to/custom-gates.md)** - Write gate checker scripts
- **[Reference: CLI Commands](../reference/cli-commands.md)** - Complete command reference
- **[Concepts: Core Model](../concepts/core-model.md)** - Deep dive into dependencies vs labels
