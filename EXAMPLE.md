# Agent Orchestration Example

This example demonstrates a lead Copilot agent orchestrating multiple worker agents to complete a complex task.

## Quick Note: Labels are Optional

**JIT works perfectly fine without labels!** You can use it as a simple issue tracker:

```bash
# Simple usage - no labels required
jit issue create --title "Fix login bug"
jit issue create --title "Add dark mode" --priority high
jit dep add <issue1> <issue2>

# Use short hashes for convenience (min 4 chars, case-insensitive)
jit issue show 9db27a3a      # Full: 9db27a3a-86c5-4d79-9582-9ad68364ea36
jit gate pass 003f tests     # Full: 003f9f83-4e8a-4a5f-8e48-44f6f48a7c17
jit dep add abc123 def456    # Works with short prefixes
```

**Labels add organizational power when you need it:**
- Small teams or simple projects: Labels optional
- Complex projects or multi-agent coordination: Labels help organize work
- You can add labels gradually as your project grows

This example shows the **full power** of label hierarchies for complex projects. Start simple and add structure when needed!

---

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
# Create the epic with labels for hierarchy
EPIC=$(jit issue create \
  --title "Implement user authentication" \
  --desc "Complete auth system with JWT" \
  --label "type:epic" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --priority high \
  --gate review --gate integration-tests | grep -oP 'Created issue: \K.*')

# Lead agent analyzes requirements and creates sub-tasks
TASK1=$(jit issue create \
  --title "Create user model" \
  --desc "SQLAlchemy model with email, password_hash fields" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K.*')

TASK2=$(jit issue create \
  --title "Implement login endpoint" \
  --desc "POST /api/login endpoint with JWT generation" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K.*')

TASK3=$(jit issue create \
  --title "Add authentication middleware" \
  --desc "Verify JWT tokens on protected routes" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K.*')

# Setup dependency graph: epic depends on all tasks
jit dep add $EPIC $TASK1
jit dep add $EPIC $TASK2
jit dep add $EPIC $TASK3

# View the dependency tree
jit graph show $EPIC

# Query strategic view (shows only epics and milestones)
jit query strategic

# Query all tasks in this epic
jit query label "epic:auth"
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

---

## Troubleshooting

### Common Issues

#### Issue: "Repository not initialized"
```bash
Error: .jit directory not found
```
**Solution**: Run `jit init` in your project directory first

#### Issue: "Cycle detected"
```bash
Error: Adding dependency would create a cycle
```
**Solution**: Check your dependency graph with `jit graph show` and remove circular references

#### Issue: "Invalid label format"
```bash
Error: Invalid label format: 'milestone-v1.0'
Expected format: 'namespace:value'
```
**Solution**: Use colon separator: `--label "milestone:v1.0"`

#### Issue: "Missing type label"
```bash
Warning: Issue created without type label
```
**Solution**: Add type label: `jit issue update $ISSUE --label "type:task"`

#### Issue: "Orphaned task"
```bash
Warning: Issue is an orphaned task (no epic:* or milestone:* label)
```
**Solution**: Add parent label: `jit issue update $ISSUE --label "epic:auth"`

### Validation

```bash
# Check repository health
jit validate

# Automatically fix issues
jit validate --fix

# Preview fixes without applying
jit validate --fix --dry-run
```

### Getting Help

```bash
# Command-specific help
jit issue create --help
jit dep add --help

# List available commands
jit --help

# Check label namespaces
jit label namespaces

# View existing label values
jit label values milestone
```

---

## Label Hierarchy Best Practices

### Creating a Milestone

```bash
# 1. Create milestone issue
MILESTONE=$(jit issue create \
  --title "Release v1.0" \
  --desc "First production release" \
  --label "type:milestone" \
  --label "milestone:v1.0" \
  --priority critical)

# 2. Create epics under milestone
EPIC_AUTH=$(jit issue create \
  --title "Authentication System" \
  --label "type:epic" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --priority high)

EPIC_UI=$(jit issue create \
  --title "User Interface" \
  --label "type:epic" \
  --label "epic:ui" \
  --label "milestone:v1.0" \
  --priority high)

# 3. Set up dependencies
jit dep add $MILESTONE $EPIC_AUTH
jit dep add $MILESTONE $EPIC_UI

# 4. View strategic view
jit query strategic
jit graph show $MILESTONE
```

### Quick Task Breakdown

```bash
# Break down epic into tasks automatically
jit breakdown $EPIC_AUTH \
  --subtask "Implement JWT generation" \
  --subtask "Create login endpoint" \
  --subtask "Add password reset" \
  --subtask "Write integration tests"

# All tasks automatically get:
# - type:task label
# - epic:auth label (inherited)
# - Dependencies configured (epic → tasks)
```

### Querying the Hierarchy

```bash
# Show all strategic issues (milestones + epics)
jit query strategic

# Show all issues in a milestone
jit query label "milestone:v1.0"

# Show all tasks in an epic
jit query label "epic:auth"

# Show specific components
jit query label "component:backend"
```

---

## Next Steps

- **Read the complete guide**: See [docs/getting-started-complete.md](docs/getting-started-complete.md)
- **Label conventions**: See [docs/label-conventions.md](docs/label-conventions.md)
- **Label quick reference**: See [docs/label-quick-reference.md](docs/label-quick-reference.md)
- **Design documentation**: See [docs/design.md](docs/design.md)
- **Web UI**: Start `jit-server` and access http://localhost:5173
- **MCP integration**: Connect to Claude/GPT-4 with the MCP server

**Happy tracking!** 🚀

---

## Understanding Labels and Warnings

### When You Don't Use Labels

Issues created without labels work perfectly fine:

```bash
# Completely valid - no warnings
jit issue create --title "Fix navbar bug"
jit issue create --title "Refactor auth code" --priority high

# JIT treats these as standalone tasks
# No hierarchy, no warnings - just simple issue tracking
```

### When Warnings Appear

Warnings only appear when you **mix** labeled and unlabeled issues in ways that might be inconsistent:

```bash
# Create an epic
jit issue create --title "Auth System" --label "type:epic" --label "epic:auth"

# Create a task WITHOUT linking it to the epic
jit issue create --title "JWT implementation" --label "type:task"
# Warning: Orphaned task (has type:task but no epic:* or milestone:* label)
```

**Why the warning?** You're using the type system (`type:task`) but not the grouping system (`epic:*`). This might be intentional, but JIT warns you in case you forgot.

### Suppressing Warnings

If you intend to create standalone tasks (not part of any epic), use `--orphan`:

```bash
jit issue create \
  --title "Quick hotfix" \
  --label "type:task" \
  --orphan
# No warning - you explicitly acknowledged it's standalone
```

Or configure strictness in `.jit/config.toml`:

```toml
[type_hierarchy.validation]
strictness = "loose"                 # Don't warn about orphaned tasks
warn_orphaned_leaves = false         # Disable orphan warnings
warn_strategic_consistency = false   # Disable strategic label warnings
```

### Label Philosophy

**JIT's approach:**
1. **No labels**: Simple, flat issue tracking (like GitHub Issues without labels)
2. **Some labels**: Add structure where it helps (e.g., `component:frontend`)
3. **Full hierarchy**: Complete organizational structure (milestones → epics → tasks)

**You choose the level of structure you need!**

