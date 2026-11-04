# Repository Agent Instructions

This file provides guidance to AI coding agents working in this repository.

## Project Overview

Just-In-Time (JIT) is a CLI-first, repository-local issue tracker designed for AI agent workflows. It features dependency DAGs with cycle detection, quality gates, machine-consumable JSON storage in `.jit/`, event logging, and multi-agent coordination with file locking. All data is plain JSON versioned with git—no external database (`@/charter/D-1`).

**This is a greenfield project. Do not plan for backward compatibility (`@/inv/canonical-cutover`). Breaking changes are not a problem.**

## Validation

Run checks for the workspace you changed:

- **Rust:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` (zero warnings), and `cargo test`.
- **MCP server:** `cd mcp-server && npm test`.
- **Web UI:** `cd web && npm test && npm run lint && npm run build`.

See `dev/TESTING.md` for focused Rust test commands. Configured gates in `.jit/gates.toml` are authoritative for completion. Install the dogfood binary with `./scripts/install-jit.sh`.

## Workspace Structure

The Rust workspace contains the core CLI/library in `crates/jit/` and the HTTP server in `crates/server/`. `mcp-server/` is the Node.js MCP bridge generated from the CLI schema, and `web/` is the React UI. User documentation lives in `docs/`; contributor material lives in `dev/`.

## Core Architecture (crates/jit)

### Separation of Concerns

Layer boundaries are mandatory:

- **CLI and output** (`cli.rs`, `main.rs`, `dispatch/`, `output.rs`) own parsing, dispatch, and rendering; `dispatch/` holds the library-side command handlers.
- **Commands** (`commands/`, `CommandExecutor`) orchestrate domain and storage; they do not parse CLI arguments or format output, and they do not import `dispatch/`.
- **Domain and graph** (`domain/`, `graph/`) own types, rules, queries, and DAG behavior; they must remain pure and I/O-free.
- **Storage** (`storage/`) owns persistence behind `IssueStore`, including JSON-file and in-memory implementations.

Adjacent subsystems include `validation/`, `document/`, `query_engine/`, and `search.rs`. New code must preserve these boundaries: put reusable logic in pure domain functions and push side effects outward.

### Issue Lifecycle States

`backlog → ready → in_progress → gated → done`

- `Rejected` and `Archived` are reachable from any state.
- Dependencies must reach a terminal state (`Done` or `Rejected`) before an issue becomes `Ready`.
- Gates must pass before transitioning through `Gated` to `Done` (`@/inv/gate-semantics`).

### Storage

Repository storage is specified in [the storage format reference](docs/reference/storage-format.md). Treat `.jit/` as repository-owned data; machine-local state is gitignored, and advisory work leases live in `.git/jit/`.

### Addressable Items

Project knowledge is cited through qualified IDs rather than copied prose:

- `@/<kind>/<self-id>` addresses a project item, such as `@/inv/dag-acyclic` or `@/charter/D-1`.
- `@/issue/<short-id>/<kind>/<self-id>` addresses an item embedded in an issue; `<short-id>/<self-id>` is accepted input sugar.
- Use `jit item show <id>` to resolve an item, `jit item search <text>` to discover one, and `jit item list --kind <kind>` to browse a kind.

Kinds, aliases, and their source-of-truth mode are declared under `[item_kinds]` in `.jit/config.toml`. Markdown-first items are edited in their source document or issue description; registry-first items are edited in their TOML registry and projected with `jit project render`. Never edit a generated projection as the authority. `jit validate` reports dangling item links. See [the item-address reference](docs/reference/item-addresses.md) for the full syntax.

## Dogfooding Boundary

This repository tracks its own development with jit, but its `.jit/` configuration is not the product's shipped default.

- Derive shipped behavior from initialization code and templates, not from this repository's live `.jit/` contents.
- Treat gates, types, namespaces, rules, templates, item kinds, and projections declared under `.jit/` as repository-local policy unless their production source says otherwise.
- Edit registry or markdown sources, then run `jit project render`; do not hand-edit generated regions.

Adopter-facing documentation describes the shipped surface. Repository-local examples must be identified as this project's configuration.

## JIT Interface

Use `jit <command-path> --help` for scoped command discovery. Use the larger `jit --schema` only when the authoritative machine contract for commands, JSON shapes, or exit codes is needed. Commands support `--json` for machine-readable output.

- Search issue titles, descriptions, and IDs with `jit issue search <text> --json`; combine it with `--state`, `--assignee`, `--priority`, or repeatable `--label` filters as needed.
- Find actionable work with `jit query available --json`, optionally narrowed by repeatable `--label` filters.
- Inspect work with `jit issue status <id>... --json`. Inspect gate history with `jit gate status <id> --all --json`; run a required gate with `jit gate evaluate <id> <gate>`.

## Testing

Develop changes test-first. Graph operations require property-based coverage with `proptest`. Use the cheapest layer that can observe the property; [dev/TESTING.md](dev/TESTING.md) is the detailed testing guide.

- **Unit:** pure behavior and edge cases.
- **Harness:** command workflows through `CommandExecutor` and `InMemoryStorage`.
- **CLI integration:** argument parsing, process status, repository discovery, and serialized output boundaries.

Test names follow `test_<function>_<scenario>` and describe the scenario in full. Cover relevant success, boundary, failure, and concurrency behavior. Reuse shared fixtures and conformance suites, and assert semantic properties rather than copied constants or field inventories (`@/inv/shared-test-contracts`, `@/inv/semantic-test-assertions`). Give each property one owning test that passes alone, in any order, and in parallel; a consumer or higher layer tests only what it adds (`@/inv/orthogonal-tests`).

## Coding Conventions

- **Functional style** — Prefer iterators/combinators over imperative loops, immutability over mutation, expression-oriented code over statements.
- **No unsafe code** — `unsafe_code = "deny"` in the root `Cargo.toml`'s `[workspace.lints.rust]`, inherited by every crate.
- **Result-based errors** — `thiserror` custom types with descriptive messages. No panics in library code.
- **Naming** — Verbs for actions (`add_dependency`, `claim_issue`), `is_`/`has_` for predicates (`is_blocked`, `has_passing_gates`).
- **Comments and rustdoc** — Terse, and limited to what is essential to understand the code (`@/inv/essential-code-prose`): purpose and material contracts, including relevant errors and invariants. Add an example only when it clarifies non-obvious behavior; prefer one type- or module-level walkthrough over repetitive method examples.
- **CLI commands must support `--json`** for machine-readable output. List-emitting commands wrap collections in the envelope `{"count": N, "<collection>": [...]}`.
- **Git is optional** — Core jit commands must work without Git; claims and leases are the documented exception (`@/charter/D-4`).

## Charter Decisions

The v1.0 vision charter's decision log is projected from `dev/vision/9db27a3a-charter.md` by `jit project render`. Cite entries as addressable items, for example `@/charter/D-1`; edit the charter, not the region below.

<!-- jit:charter:begin -->
- **D-1** — Repository-local git-versioned JSON storage, not an external database
- **D-2** — Quality gates declared in .jit/gates.toml, not baked into the binary
- **D-3** — Plan-before-fan-out bracket gates a breakable container before implementation
- **D-4** — git optional for core commands, required only for claims and leases
- **D-5** — A milestone-tier steward skill sits above the epic-level execution lead
- **D-6** — Each item kind declares its own source of truth (markdown-first or registry-first)
- **D-7** — Charter decisions are project-addressable items over the vision charter
- **D-8** — Apply profile packages once and let git keep their history, with no ownership records to keep in sync
- **D-9** — Remove redundant release surfaces without removing product capabilities
- **D-10** — Support one Docker topology that serves the API and web UI from a repository mount
- **D-11** — Release v1.0 with no known dependency advisories and blocking security audits
- **D-12** — Keep the v1.0 MSRV on a current stable Rust release and enforce it in CI
- **D-13** — Give each adopter-facing fact one canonical documentation home
- **D-14** — Gate the v1.0 tag on every epic under the v1.0 milestone, as the dependency graph states it
- **D-16** — Ship v1.0 through one tag-triggered release workflow publishing one GitHub release
- **D-17** — Publish each mutation through one redo record over the files it writes, so its cost never scales with project history
- **D-18** — Prefer removing, unifying, or deriving mechanism over adding it
- **D-19** — Reads never scale with project history; the storage layout addresses per-key data directly
- **D-20** — One computation per capability; previews, scoped views, and checkers are views over it
- **D-21** — Reach git only through the git CLI, from one storage-owned module
- **D-22** — Reads never wait on writers; a lock-free capture is admitted only when no commit crossed it
<!-- jit:charter:end -->

<!-- jit:dogfood-guidance:begin -->
## Project Invariants

<!-- jit:invariants:begin -->
- **label-format** — Every label is namespace:value (namespace lowercase-kebab, value non-empty).
- **namespace-registry** — Every label namespace is declared in the namespace registry.
- **dag-acyclic** — Cycle detection runs before every dependency operation; the graph stays acyclic.
- **event-log** — Every state change appends an event to events.jsonl.
- **atomic-writes** — File replacements use the temp-file and atomic-rename pattern, and the event log grows by a positioned append at the committed length the mutation read. A mutation commits when its redo record is synced, and a session opening over a leftover record applies it idempotently. No-replace publication covers repository content: initialization publishes an absent data root from a synced stage directory by a no-replace rename, refusing an occupied destination.
- **derived-state-coherence** — Every semantic mutation derives its coupled materializations from one read of the repository taken under the repository write lock, fails with a retryable conflict when a path it creates, writes, or removes changed since that read, and publishes through one synced redo record, so an interruption leaves either the prior state or a record the next session completes.
- **pid-safety** — Process-signaling code rejects sentinel or lossy PID conversions before invoking the operating system, including the `u32::MAX as i32 == -1` case that would turn a targeted signal into `kill(-1, sig)`.
- **assignee-format** — Every assignee is {type}:{identifier} (e.g. agent:worker-1, human:alice).
- **domain-agnostic** — Engine logic is domain-agnostic: type names, label vocabularies, gate keys, item kinds, templates, workflow shapes, and the paths at which the engine writes its own projections come from repository configuration (.jit/) or from an applied profile package, never from hardcoded domain assumptions. A site is mechanism when its assertion and its membership both derive from what the repository declares and it produces nothing when the repository declares nothing; a site producing content the repository did not declare is an instance, and a specific workflow — its gates, its templates, and the rules carrying its sequencing opinion — reaches a repository as a profile package. Two classes are mechanism vocabulary: a name a mechanism resolves through declared bindings is part of the mechanism, default included, since an unoverridden default only names one parameter of a mechanism that runs regardless; and a rule deriving both its assertion and its membership from the repository's own registry, emitting nothing for an empty one, is mechanism over that registry. A declaration naming a particular gate, template, or node type is an instance.
- **semantic-test-assertions** — Tests assert observable semantic properties or relationships; exact field-name inventories and literal-value assertions are confined to one canonical suite for an intentionally stable external contract.
- **shared-test-contracts** — Frequently repeated setup and interface behavior use shared fixtures and conformance suites; every implementation of an interface runs the same behavioral contract, while implementation-specific tests cover only implementation-specific behavior.
- **semantic-types** — Every identity, constrained token, closed vocabulary, and protocol sentinel has one canonical semantic type; raw strings exist only at parsing and serialization boundaries.
- **canonical-cutover** — Superseded aliases, fields, and representations are removed after cutover; compatibility code is allowed only in a named, versioned migration boundary with a tracked removal condition.
- **architecture-dependency-direction** — Dependencies point inward: domain and graph remain pure and I/O-free; storage owns persistence; commands orchestrate domain and storage; CLI, output, and server layers adapt inputs and outputs without pushing their concerns into inner layers.
- **convention-convergence** — A shared convention or abstraction has one form: work that finds it harmful or ill-fitting changes it at its source, or reports the mismatch as a blocking concern before proceeding. A local parallel variant, a private helper duplicating a shared mechanism, or a bypass around an abstraction is a defect unless it is a named, cited exception with a tracked convergence condition.
- **essential-code-prose** — Comments and rustdoc carry only what the code leaves non-obvious: a contract, an invariant, the reason behind a choice, or a pointer to evidence, each in the shortest form that conveys it. History lives in git and jit, and a module-level orientation is at most one short paragraph.
- **orthogonal-tests** — Each property has one owning test, at the cheapest layer that can observe it, and each test owns one property. A mechanism's behavior matrix (failure points, backends, edge cases) is proven once against the mechanism; each consumer proves only that it goes through the mechanism, and a higher layer tests only what it adds: wiring, composition, and its own boundary. A test that re-proves a property owned elsewhere is a duplication defect. Every test passes alone, in any order, and in parallel with the rest: it builds the state it reads in storage it owns, never changes process-wide state (working directory, environment) or claims a fixed port or path, and passes any value it varies explicitly.
- **gate-semantics** — An issue cannot reach Done with pending or failed gates; unpassed gates divert completion to Gated.
- **bounded-rust-build-footprint** — Rust test topology stays bounded to a small number of cohesive suites rather than one Cargo target per test file, build profiles stay compact rather than embedding a full debugger payload in every test executable, dependency features stay intentional rather than pulling in unused remote-resolution or duplicate TLS infrastructure, and integration-test target count and active test-executable bytes remain within automatically enforced budgets.
- **no-ignored-tests** — Every test in the workspace runs in the default suite, so a test that exists can fail: the Rust sources declare no ignored test and no ignored doctest. A test held out of the default run holds the name of a property nobody checks, and the exclusion outlives whatever unfinished state motivated it.
- **single-source-prose** — Every fact with a single source of truth reaches prose by projection or citation; volatile facts (counts, enumerations, registry contents) are stated structurally or derived, and a hand-maintained copy is a staleness defect.
<!-- jit:invariants:end -->

## JIT workflow

- Treat `.jit/` as repository-owned workflow configuration and issue data.
- Read `.jit/reference/content-standards.md` before authoring issues or planning documents.
- Derive hierarchy, templates, gates, namespaces, and documentation paths from repository configuration.
- A label means what its `[namespaces.<ns>]` declaration in `.jit/config.toml` says it means; read that declaration before judging what a label on an issue claims.
- Use `jit issue status`, `jit query available`, and the dependency graph to select and sequence work.
- Run the configured gates before completing an issue; a passing review placeholder is advisory evidence only.
<!-- jit:dogfood-guidance:end -->

## Commit Conventions

- Include the short ID of the relevant jit issue prefixed with `jit:` in commit messages for traceability.
