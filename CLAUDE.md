# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Just-In-Time (JIT) is a CLI-first, repository-local issue tracker designed for AI agent workflows. It features dependency DAGs with cycle detection, quality gates, machine-consumable JSON storage in `.jit/`, event logging, and multi-agent coordination with file locking. All data is plain JSON versioned with git—no external database.

## Build & Test Commands

```bash
# Build
cargo build                          # Debug build (all workspace crates)
cargo build --release                # Release build
cargo install --path crates/jit      # Install jit binary to PATH

# Test
cargo test                           # All tests (unit + harness + integration)
cargo test --lib                     # Unit tests only (fast)
cargo test --test harness_demo       # Harness tests only (fast, in-process)
cargo test --test integration_test   # CLI integration tests (subprocess)
cargo test test_name                 # Single test by name
cargo test -- --nocapture            # With stdout output

# Lint & Format
cargo clippy --workspace --all-targets  # Zero warnings required
cargo fmt --all                         # Format all code
cargo fmt --all -- --check              # Check formatting only

# MCP Server (Node.js)
cd mcp-server && npm install && npm test

# Web UI (React + Vite)
cd web && npm install && npm run dev    # Dev server
cd web && npm run build                 # Production build
cd web && npm run lint                  # ESLint
```

## Workspace Structure

Cargo workspace with three crates plus Node.js and React components:

- **`crates/jit/`** — Core CLI binary and library. Contains all domain logic, storage, graph algorithms, and command implementations.
- **`crates/server/`** — Web UI HTTP server wrapping the jit CLI.
- **`mcp-server/`** — MCP (Model Context Protocol) server (Node.js). Auto-generates 60+ tools from CLI schema.
- **`web/`** — React + TypeScript + Vite web UI for issue visualization.
- **`docs/`** — User-facing documentation (Diataxis structure).
- **`dev/`** — Contributor/development documentation and session notes.

## Core Architecture (crates/jit)

### Layers

1. **CLI** (`cli.rs`) — Clap command definitions and argument parsing.
2. **Commands** (`commands/`) — Business logic per command: `issue.rs`, `gate.rs`, `dependency.rs`, `claim.rs`, `document.rs`, `query.rs`, `validate.rs`, etc.
3. **Domain** (`domain/types.rs`, `domain/queries.rs`) — Core types (`Issue`, `State`, `Priority`, `GateStatus`) and pure query functions (`query_ready`, `query_blocked`, `query_by_assignee`).
4. **Storage** (`storage/`) — `IssueStore` trait with `JsonFileStorage` (file-based, `.jit/` directory) and `InMemoryStorage` (testing). Also contains `claim_coordinator.rs` (lease system), `lock.rs` (file locking).
5. **Graph** (`graph/`) — DAG construction, cycle detection, blocking analysis, transitive reduction, and DAG-authoritative hierarchy resolution (`graph/hierarchy.rs`).
6. **Output** (`output.rs`) — JSON serialization and structured output formatting.

`commands/mod.rs` hosts the `CommandExecutor` that orchestrates commands. `main.rs` is CLI dispatch and output rendering — large and monolithic.

### Issue Lifecycle States

`Backlog → Ready → InProgress → Gated → Done`

- `Rejected` and `Archived` are reachable from any state.
- Dependencies must complete before an issue becomes `Ready`.
- Gates must pass before transitioning through `Gated` to `Done` (`@/inv/gate-semantics`).

### Data Storage (`.jit/` directory)

```
.jit/
├── index.json          # Repository metadata (incl. format version)
├── config.toml         # Configuration
├── gates.toml          # Gate registry
├── templates.toml      # Graph templates (plan bracket)
├── rules.toml          # Validation rules
├── invariants.toml     # Invariants registry (rendered into CLAUDE.md)
├── issues/{id}.json    # Individual issue files
├── events.jsonl        # Append-only event log
├── gate-runs/          # Recorded gate runs (+ structured findings)
└── schemas/            # JSON schemas
```

Advisory work leases live in `.git/jit/`, not `.jit/`.

## Agent Workflow Quick Reference

All commands support `--json`; list output uses `{"count": N, "<collection>": [...]}`.

- `jit issue status <id>...` — state + gates + unmet deps, one line per issue
- `jit issue children` / `jit issue progress <id>` — per-child rollup, counts by state
- `jit query available --label a:b --label c:d` — ready work; labels AND
- `jit query count --by state [--label ...]` — aggregate over a bucket
- `jit graph tree [--json]` / `jit query divergence` — resolved hierarchy; label-vs-DAG report
- `jit gate evaluate <id> <gate>` runs a checker; `jit gate status` reads results; `--findings` prints structured findings
- `jit config get <dotted.key>` — config values
- `jit graph export --format json --full` — full records incl. lifecycle timestamps
- `jit schema` — JSON shapes + exit-code taxonomy

## Testing Strategy

Three-layer approach (see TESTING.md for details):

- **Unit tests** — In-source `#[cfg(test)]` modules. Fast, test individual functions.
- **Harness tests** (`tests/harness_demo.rs`) — Use `TestHarness` for isolated in-process tests with `CommandExecutor` directly. Fast and reliable.
- **Integration tests** (`tests/integration_test.rs`, 50+ test files) — Spawn `jit` as subprocess, test actual CLI interface end-to-end.

Test naming: `test_<function>_<scenario>` (e.g., `test_query_ready_returns_unassigned`).

## Key Design Principles

### Separation of Concerns

Each layer has a clear responsibility and should not reach into another's domain:

- **Domain logic** (`domain/`, `graph.rs`) must be pure and free of I/O — testable without a filesystem.
- **Storage** (`storage/`) owns all persistence — other layers interact through the `IssueStore` trait, never touching files directly.
- **Commands** (`commands/`) orchestrate domain + storage but should not contain CLI parsing or output formatting.
- **CLI** (`cli.rs`) and **Output** (`output.rs`) handle user-facing concerns only.

New code should respect these boundaries. Prefer adding a domain function over embedding logic in a command handler.

### Testability

- **TDD** — Write tests first. Property-based tests (`proptest`) for graph operations.
- **Pure functions** are preferred because they're trivially testable — push side effects to the boundaries.
- **`InMemoryStorage`** exists specifically so domain and command logic can be tested without file I/O.
- **`TestHarness`** provides isolated in-process testing with `CommandExecutor` — use this for new command tests before writing CLI integration tests.

### Coding Conventions

- **Functional style** — Prefer iterators/combinators over imperative loops, immutability over mutation, expression-oriented code over statements.
- **No unsafe code** — `#![deny(unsafe_code)]` enforced.
- **Result-based errors** — `thiserror` custom types with descriptive messages. No panics in library code.
- **Naming** — Verbs for actions (`add_dependency`, `claim_issue`), `is_`/`has_` for predicates (`is_blocked`, `has_passing_gates`).
- **CLI commands must support `--json`** for machine-readable output. List-emitting commands wrap collections in the envelope `{"count": N, "<collection>": [...]}`.
- **git is optional** — jit must work without git unless a feature strictly requires it. Exception: claim and lease commands (`jit claim acquire/release/renew/heartbeat/status/list`) require a git repository for worktree identity and branch tracking; they fail with a typed `ClaimRequiresGitError` (exit 10) when run outside one.

### Domain Invariants

<!-- jit:invariants:begin -->
- **label-format** — Every label is namespace:value (namespace lowercase-kebab, value non-empty).
- **namespace-registry** — Every label namespace is declared in the namespace registry.
- **dag-acyclic** — Cycle detection runs before every dependency operation; the graph stays acyclic.
- **gate-semantics** — An issue cannot reach Ready or Done with pending or failed gates.
- **event-log** — Every state change appends an event to events.jsonl.
- **atomic-writes** — All file writes use the temp-file + atomic-rename pattern.
- **assignee-format** — Every assignee is {type}:{identifier} (e.g. agent:worker-1, human:alice).
- **domain-agnostic** — Engine logic is domain-agnostic: type names, label vocabularies, gate keys, templates, and workflow shapes come from repository configuration (.jit/), never from hardcoded domain assumptions.
<!-- jit:invariants:end -->

## Commit Conventions

- Include the short ID of the relevant jit issue prefixed with `jit:` in commit messages for traceability.
- Run `cargo clippy` and `cargo fmt` before committing—zero warnings required.
