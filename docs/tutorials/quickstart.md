# Quickstart

> **Diátaxis Type:** Tutorial  
> **Time:** 10 minutes  
> **Goal:** Get started with JIT in 10 minutes

# Quickstart

> **Diátaxis Type:** Tutorial  
> **Time:** 10 minutes  
> **Goal:** Get started with JIT in 10 minutes

## For AI Agents: Quick Orientation

**If you're an AI agent**, you want to get productive fast. Here's the 2-minute version:

**Core Concepts:**
- **Issues** = Units of work (states: backlog → ready → in_progress → done)
- **Dependencies** = DAG controlling work order (FROM depends on TO)
- **Gates** = Quality checkpoints that must pass
- **Labels** = `namespace:value` format for organization (REQUIRED: `type:*`)
- **Assignees** = `{type}:{identifier}` (e.g., `agent:copilot-session-1`)

**Essential Commands:**
```bash
# Find ready work
jit query available --json

# Claim atomically (race-safe)
jit issue claim <short-hash> agent:your-id

# Check status
jit issue show <short-hash> --json

# Pass gates
jit gate check <short-hash> tests
jit gate pass <short-hash> code-review

# Complete
jit issue update <short-hash> --state done
```

**Use MCP Tools** - Don't fall back to CLI for efficiency. MCP provides structured responses.

See [MCP Tools Reference](../reference/cli-commands.md#mcp-tools-reference) for complete tool catalog.

**Continue with human tutorial below for detailed examples...**

---

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

This tutorial shows the basics. For the full power of label hierarchies, see [First Workflow](first-workflow.md).

## Prerequisites

- JIT installed (see [INSTALL.md](../../INSTALL.md))
- Basic command line knowledge
- A project directory (we'll create one)

## Initialize Your First Tracker

```bash
# Create a new project
mkdir my-project && cd my-project

# Initialize JIT (creates .jit/ directory)
jit init

# Check initial status
jit status
```

## Create Your First Issues

```bash
# Create a simple task
TASK1=$(jit issue create \
  --title "Fix login bug" \
  --priority high \
  --json | jq -r '.id')

# Create another task
TASK2=$(jit issue create \
  --title "Add dark mode" \
  --priority normal \
  --json | jq -r '.id')

# List all issues
jit query all
```

## Try the Dependency Graph

```bash
# Make dark mode depend on login fix
jit dep add $TASK2 $TASK1

# View the dependency graph
jit graph show

# Check what's ready to work on
jit query available
# Only TASK1 shows up (TASK2 is blocked)

# Mark TASK1 done
jit issue update $TASK1 --state done

# Check ready again
jit query available
# Now TASK2 shows up (no longer blocked)
```

## Add a Quality Gate

```bash
# Create a new issue with a gate requirement
TASK3=$(jit issue create \
  --title "Implement user profile" \
  --priority high \
  --json | jq -r '.id')

# Define a gate (manual review)
jit gate define code-review \
  --title "Code Review" \
  --description "Peer review required" \
  --mode manual

# Add gate to the issue
jit gate add $TASK3 code-review

# Try to mark it done (will fail - gate not passed)
jit issue update $TASK3 --state done
# Transitions to 'gated' instead

# Pass the gate
jit gate pass $TASK3 code-review

# Now mark done (succeeds)
jit issue update $TASK3 --state done
```

## Next Steps

You've learned the basics! Now explore:

- **[First Workflow](first-workflow.md)** - Complete example with labels and agent orchestration
- **[How-To: Custom Gates](../how-to/custom-gates.md)** - Set up automated quality gates
- **[How-To: Software Development](../how-to/software-development.md)** - TDD workflow with gates
- **[Reference: CLI Commands](../reference/cli-commands.md)** - Complete command reference

## Key Concepts Learned

- **Issues**: Units of work with states (backlog → ready → in_progress → done)
- **Dependencies**: Express "A blocks B" relationships (directed acyclic graph)
- **Short hashes**: 4-8 character prefixes for convenience (like git)
- **Gates**: Quality checkpoints that must pass before completion
- **Labels**: Optional organizational power (namespace:value format)
