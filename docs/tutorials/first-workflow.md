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
jit graph deps $EPIC --depth 0

# Query what's blocked
jit query blocked
# Shows: Epic is blocked (waiting for tasks)

# Query what's ready
jit query available
# Shows: the three tasks — ready to claim (the epic stays blocked until they finish)
```

**What we did:**
- Created dependency relationships (epic ← tasks)
- Visualized the graph
- Queried to understand blocking (the epic is blocked; the tasks are ready)

## Step 4: Confirm the Tasks Are Ready

The tasks have no dependencies of their own, so JIT created them directly in the `ready` state — no manual transition is needed. Their gates are postchecks, verified at completion (Step 7), not before work starts. Only the epic is blocked, waiting on the tasks.

```bash
# Check status
jit status
jit query available
# Shows: all 3 tasks ready to claim (the epic stays blocked)
```

**What we did:**
- Confirmed the tasks are ready to claim — readiness is automatic once dependencies clear
- Confirmed the epic stays blocked until its task dependencies finish

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
jit query all --assignee "agent:worker-1"
jit query all --assignee "agent:worker-2"
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

# Add to epic dependencies (the epic now waits for this too)
jit dep add $EPIC $TASK4

# The new task is ready immediately (no dependencies of its own),
# so another agent claims it right away
jit issue claim $TASK4 agent:worker-3

echo "Added critical security task: $TASK4"
```

**What we did:**
- Agent dynamically created new issue
- Added to dependency graph
- Another agent picked it up
- Epic automatically updated (now waits for 4 tasks)

## Step 7: Pass Gates and Complete the Tasks

As agents finish work, they pass each task's postcheck gates (unit tests, review), then mark it done. A task whose gates are not yet passed diverts to `gated` instead of `done`:

```bash
# Pass each task's gates (simulating CI and human review), then complete it
jit gate evaluate $TASK1 unit-tests --by "ci:github-actions"
jit gate evaluate $TASK1 review --by "human:tech-lead"
jit issue update $TASK1 --state done

jit gate evaluate $TASK2 unit-tests --by "ci:github-actions"
jit gate evaluate $TASK2 review --by "human:tech-lead"
jit issue update $TASK2 --state done

jit gate evaluate $TASK3 unit-tests --by "ci:github-actions"
jit gate evaluate $TASK3 review --by "human:tech-lead"
jit issue update $TASK3 --state done

jit gate evaluate $TASK4 unit-tests --by "ci:github-actions"
jit gate evaluate $TASK4 review --by "human:security-team"
jit issue update $TASK4 --state done

# Check status
jit status
jit query blocked
# Epic is now unblocked (all dependencies reached a terminal state)
```

**What we did:**
- Passed each task's postcheck gates, then marked it done
- The epic automatically became unblocked once its dependencies reached a terminal state
- Ready for final integration

## Step 8: Complete the Epic

Final integration and epic completion:

```bash
# The epic became ready automatically once its task dependencies finished.
# The lead agent claims it, then passes the epic's own postcheck gates.
jit issue claim $EPIC agent:lead
jit gate evaluate $EPIC review --by "human:tech-lead"
jit gate evaluate $EPIC integration-tests --by "ci:github-actions"

# Complete the epic
jit issue update $EPIC --state done

# Final status
jit status
# Shows: 5 done
```

**What we did:**
- The epic became ready automatically once its dependencies finished
- Lead agent claimed the epic and passed its postcheck gates (review, integration tests)
- Completed the epic
- Entire feature is done!

## Review and Verification

Examine what we built:

```bash
# View complete dependency tree
jit graph deps $EPIC --depth 0

# View all auth work
jit query all --label "epic:auth"

# View event log
jit events query --issue-id $EPIC

# Check milestone progress
jit query all --label "milestone:v1.0"
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
- `jit gate evaluate` - Produce a gate verdict: run the checker (auto) or record attestation (manual)
- `jit query available/blocked` - Find available work
- `jit graph deps` - Visualize dependency trees

## Next Steps

- **[How-To: Software Development](../how-to/software-development.md)** - TDD workflows with automated gates
- **[How-To: Custom Gates](../how-to/custom-gates.md)** - Write gate checker scripts
- **[Reference: CLI Commands](../reference/cli-commands.md)** - Complete command reference
- **[Concepts: Core Model](../concepts/core-model.md)** - Deep dive into dependencies vs labels
