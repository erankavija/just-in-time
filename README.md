# Just-In-Time Issue Tracker

[![CI](https://github.com/erankavija/just-in-time/actions/workflows/ci.yml/badge.svg)](https://github.com/erankavija/just-in-time/actions/workflows/ci.yml)
[![Docker](https://github.com/erankavija/just-in-time/actions/workflows/docker.yml/badge.svg)](https://github.com/erankavija/just-in-time/actions/workflows/docker.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

**Orchestrate, automate and supervise the work of AI agents.** A repository-local CLI issue tracker that enables defining complex workflows, quality control and project planning with AI agents.

## The Problem

Working with AI agents on complex projects gives rise to a coordination problem: agents need to break down work, avoid conflicts, enforce quality, and track progress—both with and without human intervention. Traditional issue trackers are not designed for AI agents, making it difficult to manage multi-agent workflows effectively. 

## Why JIT?

JIT is built from the ground up to support AI agent workflows with features that address their unique needs:

- ✅ **Quality Gates**: Enforce tests, linting, reviews, scans before work can proceed
- 📝 **Document Lifecycle**: Link design docs, session notes, and context to issues with safe archival
- 🔗 **Dependency DAG**: Express "Task B needs Task A" with automatic blocking and cycle detection  
- 📁 **Git-Friendly**: All state in plain JSON—version, diff, merge and version like code
- 🤖 **Agent-First Design** - JSON output, short hashes, atomic claims, event logs for observability
- 🔒 **Multi-Agent Safe**: File locking prevents race conditions with concurrent agents
- ⚙️ **Configurable**: Customize issue hierarchies and validation rules per repository

All issue data lives in `.jit/` directory within your project, versioned with git like code. No external database, no cloud service, no API dependencies.

## Use Cases

- **Multi-agent software development** - Lead agent plans work and breaks it to smaller tasks, workers claim ready tasks, quality gates enforce tests before merge. 
- **Research projects** - Break down analysis into parallel tasks, gate on peer review before publishing, preserve research context in linked documents.
- **Content generation** - Writing tasks depend on outline approval, editing tasks depend on writing completion, publication gate requires editor review.
- **Any workflow** where you want agents to discover and create work dynamically

## Quick Start

### Installation

**Pre-built binaries (Linux x64):**
```bash
wget https://github.com/erankavija/just-in-time/releases/latest/download/jit-linux-x64.tar.gz
tar -xzf jit-linux-x64.tar.gz
sudo mv jit /usr/local/bin/    # Core CLI tool
```

**From source:**
```bash
cargo install --path crates/jit
```

**Optional components:**
- `jit-server` - Web UI server (provides visualization at http://localhost:8080)
- `jit-dispatch` - Agent dispatcher for autonomous coordination
- **MCP Server** - Model Context Protocol server for AI agents like Claude (see [mcp-server/](mcp-server/))

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

# Link design document for context
jit doc add $EPIC auth-design.md --label "Design Document"

# Agent claims and executes
jit issue claim $TASK1 agent:worker-1
# ... do work ...
jit issue update $TASK1 --state done

# Check what's ready to work on
jit query available
```

**See [Quickstart Tutorial](docs/tutorials/quickstart.md) and [Complete Workflow Example](docs/tutorials/first-workflow.md) for full walkthroughs.**


## Core Concepts

JIT's workflow revolves around **issues** (units of work) that progress through **states** (lifecycle stages) with **dependencies** (execution order) and **quality gates** (checkpoints). Labels provide optional organization.

### Issue Lifecycle

Issues progress through states with automated quality checks:

```
backlog → ready → in_progress → gated → done
           ↑         ↓            ↓
           └──── prechecks    postchecks
              (validate)    (verify quality)

From any state: → rejected (terminal, bypasses gates)
```

**States:**
- **backlog**: Has incomplete dependencies
- **ready**: Dependencies done, available to claim
- **in_progress**: Work actively happening
- **gated**: Work complete, awaiting quality gate approval
- **done**: All gates passed, complete (terminal)
- **rejected**: Closed without implementation (terminal, bypasses gates)

See [Core Model - States](docs/concepts/core-model.md#states) for detailed transition rules.

### Dependencies Form a DAG

Issues can depend on other issues. An issue is **blocked** until all its dependencies complete.

```bash
jit dep add <blocked-issue> <dependency-issue>
jit graph show <issue>           # Visualize dependency tree
jit query blocked                # Find what's blocked and why
```

JIT automatically detects cycles and prevents them.

### Quality Gates Enforce Standards

Gates are checkpoints that must pass before an issue can progress or complete.

```bash
# Require a gate on an issue
jit issue create --title "Add feature" --gate unit-tests

# Gate automatically checks when issue completes
# Or manually pass/fail:
jit gate pass <issue> unit-tests
```

**Gate types:**
- **Automated** - Run a command, gate passes if exit code is 0
- **Manual** - Requires manual passing, works like a checklist item

**Gate stages:**
- **Precheck** - Must pass before work can start (e.g., "Acknowledge TDD: write tests first")
- **Postcheck** - Must pass before marking done (e.g., tests, linting, reviews)

### Document Management

JIT provides built-in document lifecycle management to preserve context and decisions alongside issues.

```bash
# Attach design docs to issues
jit doc add <issue> design.md --label "Design Document"

# Link session notes for work-in-progress
jit doc add <issue> notes/session-2024-01-02.md --label "Session Notes"

# Discover documents linked to an issue
jit doc list <issue>

# Validate links before archiving
jit doc check-links --scope issue:<issue>

# Archive completed documentation safely
jit doc archive design.md --type features
```

**Key features:**
- **Document tracking** - Link markdown files, images, and assets to issues
- **Discovery** - Query documents by issue to find relevant context
- **Link validation** - Ensure references stay valid when documents move
- **Safe archival** - Move completed docs with assets intact
- **Git integration** - Documents version alongside code

**Why this matters:** Agents can discover context from previous work, understand design decisions, and maintain institutional knowledge without relying on external systems.

See [Document Commands Reference](docs/reference/cli-commands.md#document-commands) for full `jit doc` usage.

### Organization with Labels

Labels provide flexible organization without rigid hierarchies.

```bash
# Create strategic structure
jit issue create --title "Q1 2024" --label "type:milestone"
jit issue create --title "Auth System" --label "type:epic" --label "milestone:q1-2024"
jit issue create --title "Login API" --label "type:task" --label "epic:auth"

# Query by labels
jit query strategic              # Only milestones and epics
jit query all --label "epic:auth"      # All tasks in auth epic
jit query all --label "milestone:*"    # All milestone-tagged work
```

Labels are **optional** - use them when you need organizational structure. Dependencies are always required.

## Documentation

**→ [Full Documentation](docs/index.md)** - Tutorials, how-to guides, concepts, and reference.

Quick links:
- [Quickstart](docs/tutorials/quickstart.md) - Get started in 10 minutes
- [CLI Commands](docs/reference/cli-commands.md) - Complete command reference
- [Configuration](docs/reference/configuration.md) - Customization options

## Configuration

JIT is configurable via `.jit/config.toml`. Key options:

- **Issue hierarchies** - Define type levels (milestone → epic → story → task)
- **Validation rules** - Enforce or relax organizational requirements
- **Strategic types** - Control what shows in high-level queries
- **Documentation lifecycle** - Configure archival paths and categories

```toml
[version]
schema = 2

[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4, bug = 4 }
strategic_types = ["milestone", "epic"]

[validation]
strictness = "loose"  # "strict", "loose", or "permissive"
```

See [Configuration Reference](docs/reference/configuration.md) for complete options and [Example Config](docs/reference/example-config.toml) for a full template.

## Project Status

JIT is in active development, and anything may change or break at any time. The core concepts and architecture are stable, but expect ongoing improvements, new features, and bug fixes before reaching 1.0.

## License

MIT OR Apache-2.0
