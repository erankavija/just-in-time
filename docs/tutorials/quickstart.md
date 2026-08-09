# Quickstart

> **Diátaxis Type:** Tutorial  
> **Time:** 10 minutes  
> **Goal:** Get started with JIT in 10 minutes

## For AI Agents: Quick Orientation

**If you're an AI agent**, you want to get productive fast. Here's the 2-minute version:

**Core Concepts:**
- **Issues** = Units of work. A dependency-free issue starts at `ready`; `backlog` holds issues with unmet dependencies. Claiming a ready issue moves it to `in_progress`; completing it lands in `done`, or diverts to `gated` when a required gate has not passed. The full state machine (also `rejected`, `archived`) is in [Core Model → States](../concepts/core-model.md#states).
- **Dependencies** = DAG controlling work order (FROM depends on TO)
- **Gates** = Quality checkpoints that must pass
- **Labels** = `namespace:value` format for organization
- **Assignees** = `{type}:{identifier}` (e.g., `agent:copilot-session-1`)

**Essential Commands:**
```bash
# Find ready work
jit query available --json

# Claim records assignment; acquire a lease first when exclusivity matters
jit issue claim <short-hash> agent:your-id

# Check status
jit issue show <short-hash> --json

# Evaluate gates, inspect their statuses, then complete explicitly. A manual
# gate's evaluate requires --by <attestor>; an automated gate runs its checker.
jit gate evaluate <short-hash> tests
jit gate evaluate <short-hash> <manual-gate> --by "human:reviewer"
jit gate status-all <short-hash>

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
jit gate evaluate 003f tests     # Full: 003f9f83-4e8a-4a5f-8e48-44f6f48a7c17
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

# Place the workflow package the release archive carries, then initialize with it
cp -R <extracted-archive>/packages .
jit init --profile path:packages/jit-dogfood

# Check initial status
jit status
```

This is the preferred setup for new repositories. It installs JIT's portable
planning, validation, gate, projection, and agent workflow, reading the placed
package and needing no Git repository, network access, or source checkout.
Plain `jit init` remains the minimal, methodology-neutral alternative. The
[Repository Profiles reference](../reference/profiles.md) is the canonical
reference for obtaining a package, the commands, the guarantees, and the
lifecycle.

## Create Your First Issues

```bash
# Create a simple task
TASK1=$(jit issue create \
  --title "Fix login bug" \
  --priority high \
  -q)

# Create another task
TASK2=$(jit issue create \
  --title "Add dark mode" \
  --priority normal \
  -q)

# List all issues
jit query all
```

## Try the Dependency Graph

```bash
# Make dark mode depend on login fix
jit dep add $TASK2 $TASK1

# View the dependency tree for TASK2
jit graph deps $TASK2

# Check what's ready to work on
jit query available
# Only TASK1 shows up (TASK2 is blocked)

# Mark TASK1 done
jit issue update $TASK1 --state done

# Check ready again
jit query available
# Now TASK2 shows up (TASK1 is done, so it's unblocked)
```

## Add a Quality Gate

```bash
# Create a new issue with a gate requirement
TASK3=$(jit issue create \
  --title "Implement user profile" \
  --priority high \
  -q)

# Define a separate manual gate for this tutorial
jit gate define tutorial-review \
  --title "Tutorial Review" \
  --description "Peer review required" \
  --mode manual

# Add gate to the issue
jit gate add $TASK3 tutorial-review

# Try to mark it done (will fail - gate not passed)
jit issue update $TASK3 --state done
# Transitions to 'gated' instead

# Record manual approval; --by names the attestor. A manual pass may
# complete an already gated issue.
jit gate evaluate $TASK3 tutorial-review --by "human:reviewer"

# Inspect status; if it is still gated, retry explicit completion.
jit gate status-all $TASK3
jit issue update $TASK3 --state done
```

## Next Steps

You've learned the basics! Now explore:

- **[First Workflow](first-workflow.md)** - Complete example with labels and agent orchestration
- **[How-To: Custom Gates](../how-to/custom-gates.md)** - Set up automated quality gates
- **[How-To: Software Development](../how-to/software-development.md)** - TDD workflow with gates
- **[Reference: CLI Commands](../reference/cli-commands.md)** - Complete command reference

## Key Concepts Learned

- **Issues**: Units of work. A dependency-free issue starts at `ready`; claiming moves it to `in_progress`, and completing lands in `done` (or `gated` if a required gate has not passed). See [Core Model → States](../concepts/core-model.md#states) for the full state machine (incl. `backlog`, `rejected`, `archived`)
- **Dependencies**: Express "A blocks B" relationships (directed acyclic graph)
- **Short hashes**: id prefixes for convenience (like git). See [Storage Record Layout → Issue Identifiers](../reference/storage-records.md#issue-identifiers) for the id shape and how a prefix resolves
- **Gates**: Quality checkpoints that must pass before completion
- **Labels**: Optional organizational power (namespace:value format)
