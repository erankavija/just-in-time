# Just-In-Time Issue Tracker

[![CI](https://github.com/erankavija/just-in-time/actions/workflows/ci.yml/badge.svg)](https://github.com/erankavija/just-in-time/actions/workflows/ci.yml)
[![Docker](https://github.com/erankavija/just-in-time/actions/workflows/docker.yml/badge.svg)](https://github.com/erankavija/just-in-time/actions/workflows/docker.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

**A repository-local issue tracker designed for AI agent coordination.** Define workflows with dependency graphs and quality gates, then let agents discover and execute work autonomously.

## The Problem

AI agents working on complex projects face a coordination problem: they need to break down work, avoid conflicts, enforce quality, and track progress—all without human intervention. Traditional issue trackers force agents into human-centric workflows that don't support programmatic coordination.

## Why JIT?

**JIT treats issue tracking as code orchestration, not project management.**

- 🔗 **Dependency DAG** - Express "Task B needs Task A done first" with automatic cycle detection and blocking
- ✅ **Quality Gates** - Enforce tests, linting, reviews before work proceeds (automated or manual)
- 🔒 **Multi-Agent Safe** - Atomic operations prevent race conditions when agents claim work concurrently
- 📁 **Git-Friendly** - All state in plain JSON files—diff, merge, and version like code
- 🤖 **Agent-First Design** - JSON output, short hashes, atomic claims, event logs for observability
- ⚙️ **Configurable** - Customize issue hierarchies and validation rules per repository

## Who It's For

- **Multi-agent software development** - Lead agent architects, workers implement, tester validates
- **AI-driven CI/CD** - Gate progression through build → test → deploy → release pipeline
- **Research projects** - Break down analysis into parallel tasks with dependency management
- **Any autonomous system** where agents need to coordinate work dynamically

## Quick Start

### Installation

**From pre-built binaries (Linux x64):**
```bash
wget https://github.com/erankavija/just-in-time/releases/latest/download/jit-linux-x64.tar.gz
tar -xzf jit-linux-x64.tar.gz
sudo mv jit /usr/local/bin/
```

**From source:**
```bash
cargo install --path crates/jit
```

**Via Docker:**
```bash
docker-compose up -d  # Web UI at http://localhost:8080
```

See [INSTALL.md](INSTALL.md) for all installation options.

### Basic Usage

```bash
# Initialize in your project
jit init

# Create work with dependencies
EPIC=$(jit issue create --title "User authentication" --priority high)
TASK1=$(jit issue create --title "Create user model" --priority high)
TASK2=$(jit issue create --title "Implement login endpoint" --priority high)

# Define dependencies (EPIC waits for both tasks)
jit dep add $EPIC $TASK1
jit dep add $EPIC $TASK2

# Agent claims and executes
jit issue claim $TASK1 agent:worker-1
# ... do work ...
jit issue update $TASK1 --state done

# Check what's ready to work on
jit query ready
```

**See [Quickstart Tutorial](docs/tutorials/quickstart.md) and [Complete Workflow Example](docs/tutorials/first-workflow.md) for full walkthroughs.**

## Core Concepts

### Dependencies Form a DAG

Issues can depend on other issues. An issue is **blocked** until all its dependencies complete.

```bash
jit dep add <blocked-issue> <dependency-issue>
jit graph show <issue>           # Visualize dependency tree
jit query blocked                # Find what's blocked and why
```

JIT automatically detects cycles and prevents them. Dependencies flow: if A depends on B, and B depends on C, then A is transitively blocked until C completes.

### Quality Gates Enforce Standards

Gates are checkpoints that must pass before an issue can progress or complete.

```bash
# Define a gate (once per project)
jit gate define unit-tests \
  --title "Unit Tests Pass" \
  --mode auto \
  --checker-command "cargo test --lib"

# Require it on an issue
jit issue create --title "Add feature" --gate unit-tests

# Gate automatically checks when issue completes
# Or manually pass/fail:
jit gate pass <issue> unit-tests
```

**Gate types:**
- **Automated** - Run a command, gate passes if exit code is 0
- **Manual** - Require human or agent approval

**Gate stages:**
- **Precheck** - Must pass before work can start (e.g., "TDD: write tests first")
- **Postcheck** - Must pass before marking done (e.g., tests, linting, reviews)

### Agents Coordinate Safely

Multiple agents can work concurrently without conflicts.

```bash
# Atomic claim (succeeds for exactly one agent)
jit issue claim $TASK agent:worker-1

# Or claim next available work
jit issue claim-next agent:worker-2

# Check what each agent is doing
jit query assignee agent:worker-1

# Event log tracks everything
jit events tail
```

File locking ensures consistency. The event log provides full observability into agent behavior.

## What Makes JIT Different

| Feature | JIT | GitHub Issues | Jira | Linear |
|---------|-----|---------------|------|--------|
| **Dependency DAG** | ✅ Built-in, enforced | ❌ None | ⚠️ Limited | ⚠️ Limited |
| **Quality Gates** | ✅ Automated + manual | ❌ None | ⚠️ Manual only | ❌ None |
| **Atomic Claims** | ✅ Race-free | ❌ Manual | ❌ Manual | ❌ Manual |
| **Git-Friendly Storage** | ✅ JSON files | ❌ Cloud DB | ❌ Cloud DB | ❌ Cloud DB |
| **Agent-First API** | ✅ Designed for it | ⚠️ Via GraphQL | ⚠️ REST API | ⚠️ REST API |
| **Self-Hosted** | ✅ Local-first | ❌ Cloud only | ⚠️ $$$$ | ❌ Cloud only |
| **Event Audit Log** | ✅ Built-in | ⚠️ Partial | ⚠️ Enterprise | ⚠️ Partial |

## How It Works

### 1. Lead Agent Plans Work

```bash
# Break down a feature into tasks
jit issue breakdown $EPIC \
  --subtask "Create database schema" \
  --subtask "Implement API endpoints" \
  --subtask "Write integration tests"

# Automatically creates 3 tasks with EPIC depending on all of them
```

### 2. Workers Execute in Parallel

```bash
# Coordinator dispatches ready work to available agents
jit coordinator start

# Or agents self-assign
jit issue claim-next agent:worker-1
```

### 3. Gates Enforce Quality

```bash
# Agent completes work, gates run automatically
jit issue update $TASK --state done
# ✓ Gate 'unit-tests' passed
# ✓ Gate 'clippy' passed
# ✗ Gate 'code-review' failed: Needs approval
```

### 4. Progress Flows Up

Once all tasks complete, the EPIC becomes unblocked and ready for final integration.

```bash
jit status
# Ready: 3  In Progress: 2  Done: 15  Blocked: 1
```

## Issue Lifecycle

```
backlog → ready → in_progress → gated → done
           ↑          ↓           ↓
           └─── prechecks    postchecks
```

**States:**
- `backlog` - Has incomplete dependencies
- `ready` - Dependencies done, can start
- `in_progress` - Work happening
- `gated` - Work complete, awaiting gate approval
- `done` - All gates passed

Agents can also mark issues as `rejected` with a reason.

## Organization with Labels

Labels provide flexible organization without rigid hierarchies.

```bash
# Create strategic structure
jit issue create --title "Q1 2024" --label "type:milestone"
jit issue create --title "Auth System" --label "type:epic" --label "milestone:q1-2024"
jit issue create --title "Login API" --label "type:task" --label "epic:auth"

# Query by labels
jit query strategic              # Only milestones and epics
jit query label "epic:auth"      # All tasks in auth epic
jit query label "milestone:*"    # All milestone-tagged work
```

Labels are **optional** - use them when you need organizational structure. Dependencies are always required.

## Configuration

Customize with `.jit/config.toml`:

```toml
[type_hierarchy]
# Define issue type levels (1 = most strategic)
types = { milestone = 1, epic = 2, task = 3 }

[validation]
warn_orphaned_leaves = true  # Warn on tasks without parent labels
```

See [Configuration Reference](docs/reference/configuration.md) for full options.

## Common Commands

```bash
# Issue management
jit issue create --title "..." [--priority high] [--gate <gate>]
jit issue list [--state ready] [--priority high]
jit issue update <id> --state done
jit issue claim <id> agent:worker-1
jit issue claim-next agent:worker-1

# Dependencies
jit dep add <from> <to>          # from depends on to
jit dep rm <from> <to>
jit graph show <id>              # Visualize tree
jit graph roots                  # Find top-level work

# Quality gates
jit gate define <key> --title "..." --mode auto --checker-command "..."
jit gate add <issue> <gate-key>
jit gate pass <issue> <gate-key>
jit gate check <issue> <gate-key>  # Run automated gate

# Queries
jit query ready                  # Unassigned, unblocked work
jit query blocked                # Blocked issues with reasons
jit query assignee agent:worker-1
jit status                       # Overview

# Coordination
jit coordinator start            # Auto-dispatch to agents
jit coordinator agents           # See assignments
jit events tail -n 20            # Audit log

# Search
jit search "authentication"      # Full-text across issues and docs
```

**Use short hashes everywhere** - like git, you can use short prefixes: `jit issue show 003f9f8`

All data stored in `.jit/` directory (like `.git/`). Override with `JIT_DATA_DIR` environment variable.

## Project Status

✅ **Core Features Complete**
- Issue management with dependency DAG
- Quality gates (automated + manual)
- Multi-agent coordination
- Full-text search
- Web UI and REST API
- Label-based hierarchies

🚧 **In Progress**
- Document lifecycle management
- Knowledge graph features
- Advanced reporting

See [ROADMAP.md](ROADMAP.md) for detailed progress and upcoming features.

## Documentation

- **[Tutorials](docs/tutorials/)** - Get started with step-by-step guides
- **[How-To Guides](docs/how-to/)** - Solve specific problems (custom gates, workflows)
- **[Concepts](docs/concepts/)** - Understand the core model and design philosophy
- **[Reference](docs/reference/)** - CLI commands, configuration, storage format

**For Contributors:**
- [CONTRIBUTOR-QUICKSTART.md](CONTRIBUTOR-QUICKSTART.md) - Get productive in 5 minutes
- [Development Docs](dev/index.md) - Architecture and active designs
- [TESTING.md](TESTING.md) - Testing strategy and TDD approach

## Architecture

```
┌─────────────┐
│ Coordinator │ ← Monitors ready issues, dispatches to agent pool
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
    │  ├─ issues/    │ ← JSON per issue
    │  ├─ gates.json │ ← Gate definitions
    │  └─ events.jsonl│ ← Audit log
    └────────────────┘
```

**Design principles:**
- **Git-friendly** - Plain JSON, version controlled
- **Atomic operations** - File locking prevents races
- **Observable** - Event log tracks everything
- **Simple** - No database, no server required (optional)

See [Core System Design](dev/architecture/core-system-design.md) for details.

## Contributing

Built with Rust. Requires Rust 1.80+.

```bash
git clone https://github.com/erankavija/just-in-time
cd just-in-time
cargo build
cargo test
cargo clippy
cargo fmt
```

We use JIT to track JIT development! Check `.jit/` for issues and run `jit status` to see what's being worked on.

See [CONTRIBUTOR-QUICKSTART.md](CONTRIBUTOR-QUICKSTART.md) for development workflow.

## License

MIT OR Apache-2.0
