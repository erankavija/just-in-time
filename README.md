# Just-In-Time Issue Tracker

[![CI](https://github.com/erankavija/just-in-time/actions/workflows/ci.yml/badge.svg)](https://github.com/erankavija/just-in-time/actions/workflows/ci.yml)
[![Docker](https://github.com/erankavija/just-in-time/actions/workflows/docker.yml/badge.svg)](https://github.com/erankavija/just-in-time/actions/workflows/docker.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

**Orchestrate, automate and supervise the work of AI agents.** A repository-local CLI issue tracker that enables defining complex workflows, quality control and project planning with AI agents.

## The Problem

Working with AI agents on complex projects gives rise to a coordination problem: agents need to break down work, avoid conflicts, enforce quality, and track progress—both with and without human intervention. Traditional issue trackers are not designed for AI agents, making it difficult to manage multi-agent workflows effectively. 

## Why JIT?

JIT is built from the ground up to support AI agent workflows with features that address their unique needs:

- 🔗 **Dependency DAG**: Express "Task B needs Task A" with automatic blocking and cycle detection  
- ✅ **Quality Gates**: Enforce tests, linting, reviews, scans before work can proceed
- 🔒 **Multi-Agent Safe**: File locking prevents race conditions with concurrent agents
- 📁 **Git-Friendly**: All state in plain JSON—version, diff, merge and version like code
- 🤖 **Agent-First Design** - JSON output, short hashes, atomic claims, event logs for observability
- ⚙️ **Configurable**: Customize issue hierarchies and validation rules per repository

## Use Cases

- **Multi-agent software development** - Lead agent architects, workers implement and test 
- **Research projects** - Break down analysis into parallel tasks with dependency management
- **CI/CD orchestration** - Gate progression through build → test → deploy pipeline
- **Content generation** - Manage writing, editing, review tasks with quality checks
- **Any workflow** where you want agents to discover and create work dynamically

## Quick Start

### Installation

**Pre-built binaries (Linux x64):**
```bash
wget https://github.com/erankavija/just-in-time/releases/latest/download/jit-linux-x64.tar.gz
tar -xzf jit-linux-x64.tar.gz
sudo mv jit jit-server jit-dispatch /usr/local/bin/
```

**From source:**
```bash
cargo install --path crates/jit
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

JIT's core model revolves around issues, dependencies, quality gates, and flexible organization. JIT itself is a sofware project, but it can be used to manage any type of work. Domain-specific logic and terminology is configurable.

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

# Validate links before archiving
jit doc check-links --scope issue:<issue>

# Archive completed documentation safely
jit doc archive design.md --type features
```

**Key features:**
- **Document tracking** - Link markdown files, images, and assets to issues
- **Link validation** - Ensure references stay valid when documents move
- **Safe archival** - Move completed docs with assets intact
- **Git integration** - Documents version alongside code

Documents can capture design rationale, session notes, research findings, or any context that helps agents understand the work. See the CLI reference for full `jit doc` commands.

### Organization with Labels

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

## Documentation

JIT follows the [Diátaxis](https://diataxis.fr/) framework for clear, organized documentation:

### 📚 [Concepts](docs/concepts/) - Understanding JIT
Learn core mental models: dependencies, gates, states, and the issue lifecycle.

### 🎓 [Tutorials](docs/tutorials/) - Get Started
- [Quickstart](docs/tutorials/quickstart.md) - Basic CLI usage in 10 minutes
- [First Workflow](docs/tutorials/first-workflow.md) - Complete agent orchestration walkthrough

### 🔧 [How-To Guides](docs/how-to/) - Solve Specific Problems
- [Custom Gates](docs/how-to/custom-gates.md) - Define quality checkpoints for your workflow
- [Software Development](docs/how-to/software-development.md) - TDD, CI/CD integration, code quality
- [Dependency Management](docs/how-to/dependency-management.md) - Graph strategies and patterns

### 📖 [Reference](docs/reference/) - Look Up Details
- [CLI Commands](docs/reference/cli-commands.md) - Complete command reference
- [Configuration](docs/reference/configuration.md) - Config options and customization
- [Glossary](docs/reference/glossary.md) - Term definitions

**New to JIT?** Start with [Concepts](docs/concepts/), then try the [Quickstart Tutorial](docs/tutorials/quickstart.md).

## Configuration

JIT is domain-agnostic and configurable via `.jit/config.toml`:

**Example: Research Project Configuration**
```toml
# Custom issue hierarchy for research workflows
[type_hierarchy]
types = ["program", "study", "experiment", "analysis"]
strategic_types = ["program", "study"]

# Label namespaces for organizing research
[[namespaces]]
key = "dataset"
description = "Dataset being analyzed"
unique = false

[[namespaces]]
key = "method"
description = "Analytical method"
unique = false

# Validation rules
[type_hierarchy.validation]
strictness = "loose"           # Allow flexible organization
warn_orphaned_leaves = false   # Individual analyses can stand alone
```

**Common configurations:**
- **Issue hierarchies** - Define your own type levels (milestone → epic → story → task)
- **Label namespaces** - Custom categorization (dataset, method, experiment, author, etc.)
- **Validation rules** - Enforce or relax organizational requirements
- **Strategic types** - What shows up in high-level queries

See [Configuration Reference](docs/reference/configuration.md) for complete options and examples across different domains.

## Project Status

JIT is in active development, and anything may change or break at any time. The core concepts and architecture are stable, but expect ongoing improvements, new features, and bug fixes before reaching 1.0.

## License

MIT OR Apache-2.0
