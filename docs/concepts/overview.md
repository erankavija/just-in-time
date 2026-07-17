# Overview

> **Diátaxis Type:** Explanation

## What is JIT?

JIT (Just-In-Time) is a **CLI-first issue tracker** designed for **AI agent orchestration** and **human collaboration**. Unlike traditional issue trackers built for web browsers, JIT is optimized for programmatic access, enabling AI agents to coordinate their own work through dependency graphs and quality gates.

**Core Philosophy:**
- **Machine-readable by default** - JSON output, atomic operations, clear exit codes
- **Domain-agnostic** - Works for software, research, knowledge work, any workflow
- **Dependency-driven** - Explicit DAG (directed acyclic graph) controls work order
- **Quality-enforced** - Gates integrate validation directly into workflow
- **Repository-local** - Plain JSON files in `.jit/` (like `.git/`)

## Who is JIT For?

**AI Agents** - Primary audience
- Coordinate multi-agent work distribution
- Query available work programmatically
- Use advisory leases when an agent needs exclusive work ownership
- Enforce quality through automated gates
- Access via MCP (Model Context Protocol) tools

**Human Developers** - Collaborative users
- Track personal projects with CLI
- Manage team workflows with dependencies
- Integrate with existing tools (git, CI/CD)
- Visualize work through web UI

**Mixed Teams** - Agents + humans
- Agents handle routine work (tests, builds, documentation)
- Humans handle complex work (design, review, architecture)
- Shared dependency graph coordinates both

## Key Characteristics

**CLI-First Design**
- All operations via command line (`jit issue create`, `jit dep add`)
- JSON output for machine consumption (`--json` flag)
- Short hashes for convenience (`jit issue show 01abc` works)
- Quiet mode for scripting (`--quiet` suppresses headers)

**Agent-Friendly Architecture**
- Atomic file replacement guarded by advisory locks (no database, no server)
- MCP tool integration for AI frameworks
- Stateless coordination through shared file state
- Event logs for observability and audit trail

**Domain-Agnostic Terminology**
- **Issue** (not ticket, not task) - Universal unit of work
- **Gate** (not check, not test) - Quality checkpoint
- **Dependency** (not blocker) - Work order constraint
- **Assignee** (not owner) - Who's working on it (human or agent)

**Repository-Local Storage**
- Issue and configuration data in `.jit/` (version controlled)
- Machine-local runtime state also under `.jit/`, gitignored
- Lease coordination in `.git/jit/`, shared across worktrees
- Plain JSON files (no database, no server)
- Git-optional (works standalone or with version control)
- Easy backup, export, and migration

## Why JIT Exists

**Problem: Traditional issue trackers don't support AI agents**
- Web-first UIs require browser automation (slow, brittle)
- GraphQL/REST APIs are paginated and rate-limited
- No repository-local advisory lease coordination
- No explicit dependency graphs (only implicit parent/child)
- Quality checks happen externally (CI/CD separate from tracking)

**Solution: JIT integrates coordination and quality**
- CLI-first with JSON output (fast, reliable)
- Atomic file writes plus advisory leases for exclusive coordination
- Explicit dependency DAG (clear work order)
- Gates integrate quality into workflow (not external)
- MCP tools for agent frameworks (standardized interface)

**Design Principles:**
1. **Programmatic-first** - Optimize for agents, humans benefit too
2. **Simple storage** - JSON files, no database complexity
3. **Explicit relationships** - Dependencies, not magic
4. **Quality-aware** - Gates are first-class, not bolted-on
5. **Domain-neutral** - Software, research, knowledge work all fit

## How It Works (High Level)

**Mental Model:**
```
.jit/                          # Like .git/ - repository-local
├── index.json                 # Issue index for fast queries; carries the format version
├── config.toml                # Configuration
├── gates.toml                 # Gate registry definitions
├── rules.toml                 # Validation rules
├── events.jsonl               # Append-only event log
├── issues/                    # One JSON file per issue
│   └── <uuid>.json            # Issue state, dependencies, gates
├── gate-runs/                 # Recorded gate runs
└── schemas/                   # JSON Schema files referenced by rules.toml
```

See [Storage Format](../reference/storage-format.md) for the authoritative layout, including per-project files.

**Workflow:**
1. **Create issues** with dependencies and gates
2. **Query for ready work** (unblocked, unassigned)
3. **Acquire a lease when exclusivity matters, then claim**
4. **Do work** with quality gates guiding process
5. **Validate and complete** when gates pass
6. **Repeat** - next agent/human picks up ready work

**Coordination:**
- Issues move through states: `backlog → ready → in_progress → gated → done`
- Dependencies block issues until prerequisites become effectively terminal
- Gates block completion until quality standards met
- Assignees record ownership; advisory leases provide exclusive coordination
- The event log records issue lifecycle changes (state transitions and a defined event set), not every repository mutation

**Example:**
```bash
# Agent queries ready work
jit query available --json

# Record workflow ownership (acquire a lease first when exclusivity matters)
jit issue claim abc123 agent:worker-1

# Agent does work...

# Agent validates quality (an automated gate runs its checker; a manual
# gate's evaluate requires --by <attestor>)
jit gate evaluate abc123 tests
jit gate evaluate abc123 code-review --by "human:reviewer"

# Inspect gate statuses, then explicitly request completion
jit gate status-all abc123
jit issue update abc123 --state done
```

**Key Insight:** JIT has no central coordination server. Agents share repository
state; atomic writes keep files whole, and `jit claim acquire` provides advisory
leases when exclusive coordination is needed.

## Domain Examples

**Software Development:**
- Issue = Feature/Bug, Gate = CI check, Dependency = Prerequisite feature
- Workflow: TDD → Tests → Linting → Code Review → Deploy

**Research Projects:**
- Issue = Experiment, Gate = Peer review, Dependency = Literature review
- Workflow: Literature → Design → Data Collection → Analysis → Publication

**Knowledge Work:**
- Issue = Document section, Gate = Approval, Dependency = Previous chapter
- Workflow: Outline → Draft → Review → Edit → Publish

**See Also:**
- [Core Model](core-model.md) - Issues, dependencies, gates, states in detail
- [Quickstart Tutorial](../tutorials/quickstart.md) - Get started in 10 minutes
- [How-To Guides](../how-to/) - Domain-specific workflows
