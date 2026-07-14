# Repository Agent Instructions

This file provides guidance to AI coding agents working in this repository.

## Project Overview

Just-In-Time (JIT) is a CLI-first, repository-local issue tracker designed for AI agent workflows. It features dependency DAGs with cycle detection, quality gates, machine-consumable JSON storage in `.jit/`, event logging, and multi-agent coordination with file locking. All data is plain JSON versioned with git—no external database (`@/charter/D-1`).

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
cd web && npm test                      # Vitest unit tests
cd web && npm run build                 # Production build
cd web && npm run lint                  # ESLint
```

## Workspace Structure

Cargo workspace with two crates plus Node.js and React components:

- **`crates/jit/`** — Core CLI binary and library (see Core Architecture below).
- **`crates/server/`** — Web UI HTTP server embedding the jit library (`CommandExecutor`) in-process.
- **`mcp-server/`** — MCP (Model Context Protocol) server (Node.js). Auto-generates its tools from the CLI schema.
- **`web/`** — React + TypeScript + Vite web UI for issue visualization.
- **`docs/`** — User-facing documentation (Diataxis structure).
- **`dev/`** — Contributor/development documentation and session notes.

## Core Architecture (crates/jit)

### Layers

1. **CLI** (`cli.rs`) — Clap command definitions and argument parsing.
2. **Commands** (`commands/`) — Business logic per command: `issue.rs`, `gate.rs`, `dependency.rs`, `claim.rs`, `document.rs`, `query.rs`, `validate.rs`, etc.
3. **Domain** (`domain/types.rs`, `domain/queries.rs`, `domain/type_taxonomy.rs`) — Core types (`Issue`, `State`, `Priority`, `GateStatus`), pure query functions (`query_ready`, `query_blocked`, `query_by_assignee`), and the taxonomy of type labels and their levels (`HierarchyConfig`), which `config_manager.rs` loads from `[type_hierarchy]` in `config.toml`.
4. **Storage** (`storage/`) — `IssueStore` trait with `JsonFileStorage` (file-based, `.jit/` directory) and `InMemoryStorage` (testing). Also contains `claim_coordinator.rs` (lease system), `lock.rs` (file locking).
5. **Graph** (`graph/`) — DAG construction, cycle detection, blocking analysis, transitive reduction, and DAG-authoritative hierarchy resolution (`graph/hierarchy.rs`).
6. **Output** (`output.rs`) — JSON serialization and structured output formatting.

Adjacent subsystems include `validation/` (rules engine), `document/` (linked docs), `query_engine/`, and `search.rs`.

`commands/mod.rs` hosts the `CommandExecutor` that orchestrates commands. `main.rs` is CLI dispatch and output rendering — large and monolithic.

### Issue Lifecycle States

`Backlog → Ready → InProgress → Gated → Done`

- `Rejected` and `Archived` are reachable from any state.
- Dependencies must reach a terminal state (`Done` or `Rejected`) before an issue becomes `Ready`.
- Gates must pass before transitioning through `Gated` to `Done` (`@/inv/gate-semantics`).

### Data Storage (`.jit/` directory)

```
.jit/
├── index.json          # Repository metadata (incl. format version)
├── config.toml         # Configuration
├── gates.toml          # Gate registry (`@/charter/D-2`)
├── templates.toml      # Graph template registry (this repo declares the `plan` bracket)
├── rules.toml          # Validation rules
├── invariants.toml     # Invariants registry (rendered to the `[invariant_projection]` target; here AGENTS.md)
├── issues/{id}.json    # Individual issue files
├── events.jsonl        # Append-only event log
├── gate-runs/          # Recorded gate runs (+ structured findings)
└── schemas/            # JSON schemas
```

A live repo also carries gitignored machine-local files in `.jit/` (`worktree.json`, `server.log`, `server.pid.json`, `*.lock`, `tmp/`). Advisory work leases live in `.git/jit/`, not `.jit/`.

### Addressable Items

Structured lines in issue descriptions and project registries carry a self-id and are addressable via qualified ids: `@/<kind>/<self-id>` (project scope, e.g. `@/invariant/dag-acyclic`), `@/issue/<short-id>/<kind>/<self-id>` (issue scope), with `<short-id>/<self-id>` as input sugar. Kinds (requirement, decision, risk, invariant, …) and their aliases (`@/inv/…`) are declared in `[item_kinds]` in `.jit/config.toml`; beyond the kinds `jit init` scaffolds, this repo adds a `definition` kind over `docs/reference/glossary.md` and a `charter` kind over the v1.0 vision charter (`dev/vision/9db27a3a-charter.md`), whose decisions are citable as `@/charter/D-N` via `per:` labels (`@/charter/D-7`). Each kind declares its source of truth (`@/charter/D-6`): markdown-first for description-embedded items (requirement, decision, risk), registry-first for TOML registries (invariant, rule, gate — `jit invariant render` and `jit reference render` project the registries into markdown); the item index is always a projection. Citations like `@/inv/gate-semantics` in docs and issue text resolve through this scheme; `jit validate` flags dangling item links (`dangling-item-link`).

## Dogfooding Setup

This repository tracks jit's own development with jit: `.jit/` here is project configuration, distinct from what the product ships.

- **`jit init` ships**: `index.json`, an empty `gates.toml`, `events.jsonl`, a template-generated `config.toml` (milestone/epic/story/task hierarchy plus the namespace and item-kind registries), `rules.toml` with the default ruleset (format, registry, hierarchy, and per-namespace uniqueness checks). Gate presets (language starter bundles like `rust-tdd`, plus `minimal`, `security-audit`, and the planning-bracket trio) live in code and materialize only via `jit gate preset apply`; `templates.toml`, `invariants.toml`, and the projection tables are authored per project, never scaffolded.
- **This repo's local layer**: gates wired to repo scripts (`cargo-ci` for the Rust workspace, `npm-ci` for `web/`, `mcp-ci` for `mcp-server/`, `jit-validate`, `code-review` via `./scripts/ai-review.sh`); `planning`/`breakdown`/`bug`/`enhancement` types; `brackets:`/`satisfies:`/`per:` namespaces; the `coverage-preview` rule; the `plan` template; the `definition` and `charter` item kinds; the invariant projection into this file and the rules-gates projection into `docs/reference/rules-and-gates.md`; the `dev/` doc lifecycle.

When editing docs or config, keep this boundary explicit: adopter-facing text describes the shipped surface, repo-local values are signalled as this project's configuration.

## Agent Workflow Quick Reference

All commands support `--json` (envelope spec under Coding Conventions).

- `jit issue status <id>...` — state + gates + unmet deps, one line per issue
- `jit issue children <id>` / `jit issue progress <id>` — per-child rollup, counts by state
- `jit query available --label a:b --label c:d` — ready work; labels AND
- `jit query count --by state [--label ...]` — aggregate over a bucket
- `jit graph tree` / `jit query divergence` — resolved hierarchy; label-vs-DAG report
- `jit gate evaluate <id> <gate>` runs a checker; `jit gate status` reads results; `--findings` prints structured findings
- `jit config get <dotted.key>` — config values
- `jit graph export --format json --full` — full records incl. lifecycle timestamps
- `jit apply <template> <container>` — instantiate a graph template from `.jit/templates.toml` (plan-before-fan-out scaffold, `@/charter/D-3`)
- `jit issue batch-create --from-json <file>` — create many issues plus dependency edges from one JSON payload
- `jit item show @/inv/dag-acyclic` — resolve a qualified id; `jit item list --kind <k>` / `jit item search <text>` to discover
- `jit --schema` — JSON shapes + exit-code taxonomy

## Testing Strategy

Three-layer approach (see dev/TESTING.md for details):

- **Unit tests** — In-source `#[cfg(test)]` modules. Fast, test individual functions.
- **Harness tests** (`tests/harness_demo.rs`) — Use `TestHarness` for isolated in-process tests with `CommandExecutor` directly. Fast and reliable.
- **Integration tests** (`tests/*.rs`, e.g. `integration_test.rs`) — Spawn `jit` as subprocess, test actual CLI interface end-to-end.

Tests cover relevant success, boundary, failure, and concurrency behavior. Depending on the affected subsystem, representative cases include empty graphs, cycles, missing issues, and concurrent claims.

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
- **Public API documentation** — Public APIs have doc comments describing their purpose and material contracts, including errors or invariants where relevant. Add an example only when it materially clarifies non-obvious usage or behavior. Do not require an example for every public API; tautological examples for accessors, constants, constructors, or direct field mappings are maintenance and CI cost, not documentation value. Prefer one type- or module-level walkthrough over repetitive per-method examples.
- **CLI commands must support `--json`** for machine-readable output. List-emitting commands wrap collections in the envelope `{"count": N, "<collection>": [...]}`.
- **git is optional** — jit must work without git unless a feature strictly requires it (`@/charter/D-4`). Exception: the `jit claim` lease subcommands require a git repository for worktree identity and branch tracking; they fail with a typed `ClaimRequiresGitError` (exit 10) when run outside one.

### Domain Invariants

Each invariant is addressable at `@/inv/<name>`.

<!-- jit:invariants:begin -->
- **label-format** — Every label is namespace:value (namespace lowercase-kebab, value non-empty).
- **namespace-registry** — Every label namespace is declared in the namespace registry.
- **dag-acyclic** — Cycle detection runs before every dependency operation; the graph stays acyclic.
- **gate-semantics** — An issue cannot reach Done with pending or failed gates; unpassed gates divert completion to Gated.
- **event-log** — Every state change appends an event to events.jsonl.
- **atomic-writes** — All file replacements use the temp-file + atomic-rename pattern; new-file publication uses verified staging plus atomic no-replace publication, so an occupied destination is never overwritten.
- **pid-safety** — Process-signaling code rejects sentinel or lossy PID conversions before invoking the operating system, including the `u32::MAX as i32 == -1` case that would turn a targeted signal into `kill(-1, sig)`.
- **assignee-format** — Every assignee is {type}:{identifier} (e.g. agent:worker-1, human:alice).
- **domain-agnostic** — Engine logic is domain-agnostic: type names, label vocabularies, gate keys, templates, and workflow shapes come from repository configuration (.jit/), never from hardcoded domain assumptions.
- **single-source-prose** — Every fact with a single source of truth reaches prose by projection or citation; volatile facts (counts, enumerations, registry contents) are stated structurally or derived, and a hand-maintained copy is a staleness defect.
<!-- jit:invariants:end -->

## Commit Conventions

- Include the short ID of the relevant jit issue prefixed with `jit:` in commit messages for traceability.
- Run `cargo clippy` and `cargo fmt` before committing—zero warnings required.
