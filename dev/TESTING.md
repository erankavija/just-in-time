# JIT Testing Strategy

This document describes the testing approach for the Just-In-Time issue tracker. It is the
detailed elaboration of the three-layer strategy summarized in [AGENTS.md](../AGENTS.md).

## Three Layers

Tests are organized by how much of the system they exercise and how much they cost to run.

```mermaid
flowchart TD
    I["Integration tests<br/>spawn the jit binary, exercise the CLI"]
    H["Harness tests<br/>in-process CommandExecutor over InMemoryStorage"]
    U["Unit tests<br/>pure functions, in-source test modules"]

    I --> H --> U
```

Each layer answers a different question. Unit tests ask whether a function is correct.
Harness tests ask whether a workflow composes. Integration tests ask whether the CLI surface
matches the contract users and agents depend on.

Pick the cheapest layer that can observe the behavior you care about.

## 1. Unit Tests

Location: `#[cfg(test)] mod tests` blocks inside the source files under `crates/jit/src/`.

The crate's source is organized as directories, so unit tests sit next to the code they
cover:

```
crates/jit/src/
├── commands/       # Business logic per command (issue.rs, gate.rs, dependency.rs, …)
├── domain/         # Core types and pure query functions
├── graph/          # DAG construction, cycle detection, hierarchy resolution
├── storage/        # IssueStore implementations, locking, claim coordination
├── validation/     # Rules engine
├── document/       # Linked documents
└── query_engine/   # Query parsing and evaluation
```

Unit tests are the right home for edge cases: cycles, empty inputs, boundary values,
malformed configuration. They run in the same process as the code and report failures at the
exact function that broke.

Domain logic and graph algorithms are pure and free of I/O by design, so their tests need
nothing beyond the values they construct. Tests over `storage/` and over the command modules
that read repository configuration set up a `TempDir` and work against real files, which is
what those modules exist to do. When a behavior in `domain/` or `graph/` is hard to test
without a filesystem, that is a signal that side effects have leaked out of the storage
boundary.

Run them with `cargo test --lib`.

### Property-Based Tests

Property-based testing with [`proptest`](https://docs.rs/proptest) is current practice for
graph operations, identifier resolution, and concurrent claim coordination. Instead of
asserting one example, a property test states an invariant and lets proptest search for a
counterexample across generated inputs.

| Module | Properties covered |
| --- | --- |
| `crates/jit/src/graph/mod.rs` | Unlimited traversal equals the transitive dependency set; cycle detection reports none for DAGs |
| `crates/jit/src/graph/hierarchy.rs` | Resolution invariants hold on arbitrary DAGs; resolution is order-invariant |
| `crates/jit/src/domain/type_taxonomy.rs` | Type-name extraction normalizes consistently; invalid labels are rejected |
| `crates/jit/src/storage/claim_coordinator_proptests.rs` | Index rebuild is idempotent and lossless; lease counts stay consistent; sequence numbers increase monotonically; concurrent claims stay exclusive |
| `crates/jit/tests/fast_docs_templates/template_apply_tests.rs` | Template application yields an acyclic, transitively reduced graph; force-refresh is idempotent over nodes and edges |
| `crates/jit/tests/fast_issue/readiness_coherence_tests.rs` | Stored readiness agrees with the graph-derived blocked predicate after any sequence of dependency additions and removals |
| `crates/jit/tests/fast_issue/short_hash_tests.rs` | Any unique prefix resolves to its issue; shared prefixes are reported as ambiguous |

When proptest finds a counterexample it records the seed so the case is replayed on every
later run. Those seeds live in `crates/jit/proptest-regressions/` and, for the integration
target, in `crates/jit/tests/fast_issue/short_hash_tests.proptest-regressions` (beside the
suite module that owns the property). Commit them: they are regression tests.

Write a property test when you can name an invariant that must hold for all inputs. Write an
example test when you care about one specific input.

## 2. Harness Tests

Location: `crates/jit/tests/common/harness.rs` (the harness, below Cargo's auto-discovery
boundary so it is never its own test target) and the in-process suites that use it, such as
`crates/jit/tests/fast_docs_templates/harness_demo.rs`.

`TestHarness` backs a real `CommandExecutor` with `InMemoryStorage`, so a test drives command
logic end to end in the calling process, with issue state held in memory. Each harness gets
its own isolated storage. Repository configuration still lives on disk: `with_item_kinds()`
writes `config.toml` under the storage root.

### TestHarness API

```rust
pub struct TestHarness {
    pub executor: CommandExecutor<InMemoryStorage>,
    pub storage: InMemoryStorage,
}

impl TestHarness {
    // Setup
    pub fn new() -> Self;
    pub fn with_item_kinds(self) -> Self;

    // Issue creation helpers
    pub fn create_issue(&self, title: &str) -> String;
    pub fn create_issue_with_desc(&self, title: &str, desc: &str) -> String;
    pub fn create_issue_with_priority(&self, title: &str, priority: Priority) -> String;
    pub fn create_ready_issue(&self, title: &str) -> String;
    pub fn create_issue_with_gates(&self, title: &str, gates: Vec<String>) -> String;

    // Gate setup
    pub fn add_gate(&self, key: &str, title: &str, description: &str, auto: bool);

    // Queries
    pub fn all_issues(&self) -> Vec<Issue>;
    pub fn get_issue(&self, id: &str) -> Issue;
}

impl Default for TestHarness { /* delegates to new() */ }
```

`new()` initializes the storage and sets `JIT_TEST_MODE=1`. The helpers cover the common
shapes; anything else goes through `h.executor` (any command) or `h.storage` (direct
`IssueStore` calls).

`with_item_kinds()` writes the canonical `[item_kinds]` table into the harness repo's
`config.toml`. The engine bakes in no item kinds (`@/inv/domain-agnostic`), so a test that
exercises addressable items, item indexing, or link resolution must opt in. Call it on the
harness before creating issues:

```rust
let h = TestHarness::new().with_item_kinds();
```

### Usage

A suite that runs in-process declares the shared harness once in its `main.rs`
(`#[path = "../common/harness.rs"] mod harness;`); each test module inside the suite reaches
it through the crate root:

```rust
use crate::harness::TestHarness;

#[test]
fn test_harness_query_ready() {
    let h = TestHarness::new();

    let ready_id = h.create_ready_issue("Ready task");
    let assigned_id = h.create_ready_issue("Assigned task");
    h.executor
        .claim_issue(&assigned_id, "agent:worker-1".to_string())
        .unwrap();

    let ready = h.executor.query_ready().unwrap();

    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id, ready_id);
}
```

Harness tests in `fast_docs_templates/harness_demo.rs` cover queries, the issue lifecycle,
dependency blocking and cycle detection, gates, container rollups, item resolution, response
projections, and behavior at scale.

Run the whole suite with `cargo test --test fast_docs_templates`, or just the demo module
with `cargo test --test fast_docs_templates harness_demo`.

## 3. Integration Tests

Integration tests are grouped into cohesive **suites**. Each suite is one directory under
`crates/jit/tests/<suite>/` whose `main.rs` declares its test files as modules, so Cargo
links and runs one executable per suite instead of one per file. Suites divide by execution
model and subsystem:

| Suite | Execution model | Covers |
| --- | --- | --- |
| `fast_issue`, `fast_rules`, `fast_docs_templates` | in-process (`CommandExecutor`) | issue lifecycle/graph, rules/labels/config, documents/templates |
| `cli_issue`, `cli_gate`, `cli_query_graph`, `cli_item_validate`, `cli_repo_workflow` | CLI subprocess | the `jit` binary's command surface by area |
| `scratch_build` | heavyweight | tests that build scratch `cargo` projects (stale-binary checks and the merged-tree gate self-test), plus the build-footprint budget-checker fixtures |
| `provenance_contract` | `#[ignore]`d contracts | build-provenance stability, run by `scripts/cargo-ci.sh` under `--ignored` |

The `cargo-ci` gate runs `scripts/rust-build-budget.sh` as a `budget` step after
its test step, on warm Cargo artifacts, pointing it with `--root` at the
workspace that run compiled. The checker derives the integration-test
target count and unique active test-executable bytes from `cargo metadata` and
`cargo test --workspace --no-run --message-format=json`, and asserts the debug
profile, gate incremental, and dependency-feature policies, failing the gate when
a budget or policy is exceeded (jit:3f73423b). Its inputs are injectable
(`--metadata-json`, `--artifacts-json`, `--root`), so
`scratch_build/rust_build_budget_checker_tests.rs` exercises each failure mode
with synthetic JSON and sparse executables — no compilation. See "4. Build
Footprint Budget" below for the exact budgets, the build-profile and
dependency-feature policy they check, and how to diagnose a failure.

`scratch_build/merged_tree_gate_verification_tests.rs` runs
`scripts/cargo-ci-selftest.sh`, which is what keeps the `cargo-ci` gate honest as
the post-merge check (jit:3019eacd). A textually clean merge can leave `main`
broken in ways no per-issue gate saw, so the worktree dispatch protocol runs the
gate on the merged tree. The self-test seeds four merges in throwaway git repos —
one healthy, one declaring a deleted module, one whose `#[cfg(test)]` caller lost
an argument, one that compiles and fails at test time — runs the shipped
`scripts/cargo-ci.sh` against each, and asserts the verdict its build-and-test
step reports. The last two also assert that `cargo build --workspace` still
succeeds on the same tree: that is why a build-only merge check was vacuous, and
substituting one makes the self-test fail. Every fixture is a dependency-free
two-module crate with its own target directory, so the whole self-test costs
seconds and never rebuilds this workspace.

Shared helpers live below Cargo's auto-discovery boundary in `crates/jit/tests/common/`, so
they never surface as their own test targets. Add a new integration case to the file that
matches its subsystem, or add a module file plus a `mod` line in the suite's `main.rs`.

The CLI suites spawn the compiled binary through `env!("CARGO_BIN_EXE_jit")` against a
`TempDir` seeded by `jit init`, then assert on exit status, stdout, and JSON payloads. They
are the only layer that can catch argument-parsing regressions, output-format drift, and
exit-code changes.

```rust
#[test]
fn test_create_and_query() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    Command::new(jit)
        .args(["issue", "create", "-t", "Task", "--priority", "high"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(jit)
        .args(["query", "ready", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["issues"].is_array());
}
```

Reserve this layer for what only it can see: user-facing command contracts, `--json` envelope
shapes, flag combinations, and regression tests for reported CLI bugs. Workflow coverage
belongs in the harness layer, where it runs faster and fails more legibly.

`crates/server/tests/` holds the equivalent layer for the web UI server crate, organized
the same way: one `server_integration` suite whose `main.rs` declares the modules, covering
both the in-process API cases and the real-process shutdown cases that spawn the compiled
`jit-server` binary through `env!("CARGO_BIN_EXE_jit-server")`.

Run a whole suite with `cargo test --test cli_repo_workflow`, or filter to one module or case
with `cargo test --test cli_repo_workflow integration_test`.

## 4. Build Footprint Budget

The Rust workspace's test topology, build profiles, and dependency features are bounded by
the enforced `@/inv/bounded-rust-build-footprint` project invariant.

### Budgets

`cargo-ci`'s `budget` step (`scripts/rust-build-budget.sh`) enforces two budgets on every
gate run, both derived from `cargo metadata` and `cargo test --workspace --no-run
--message-format=json` rather than a `target/` directory scan (stale per-hash artifacts
there cannot describe the current build): at most 12 integration-test targets and at most
2 GiB of unique active test-executable bytes. Both constants are declared once, in the
script's own header comment (`MAX_INTEGRATION_TARGETS`, `MAX_EXECUTABLE_BYTES`); read them
there rather than assuming either has changed.

A third budget — at most 10 GiB for the complete fresh validation target directory — is the
acceptance threshold the benchmark protocol below validates against once per build-topology
change, not re-checked on every gate run: a full clean rebuild on every gate invocation would
defeat the point of the interactive incremental-build policy described next. See
[dev/archive/6eb585bc-core-maintenance/active/73482aa1-rust-build-efficiency.md](archive/6eb585bc-core-maintenance/active/73482aa1-rust-build-efficiency.md)
("Benchmark protocol") for the full acceptance criteria and
[dev/benchmarks/rust-build-efficiency/report.md](benchmarks/rust-build-efficiency/report.md)
for the measured comparison.

### Build profile and dependency-feature policy

- **Debug info** — `[profile.dev]`/`[profile.test]` in the workspace `Cargo.toml` set
  `debug = "line-tables-only"`: enough for line-number backtraces on a local failure, without
  embedding the full debugger payload (type info, macro expansions) that dominates a test
  executable's size.
- **Dependency optimization** — `[profile.dev.package."*"]` sets `opt-level = 2`, so third-party
  crates are compiled optimized while workspace crates keep the debug profile's compile times.
  The suite runs dependency code far more often than it compiles it: its CPU profile is spread
  across SHA-256 over whole build artifacts, TOML parsing, and JSON serialization, none of which
  is workspace code. A package override changes no other profile key, so `debug-assertions` and
  `overflow-checks` still hold everywhere. The measured build cost of the override, against the
  matched unoptimized arm and both fixed 25% ceilings, is in
  [dev/benchmarks/dependency-profile-6d10e5d4/](benchmarks/dependency-profile-6d10e5d4/).
- **Incremental compilation** — both profiles also state `incremental = true` explicitly, for
  ordinary interactive development, where the cost amortizes across many rebuilds of the same
  tree. `scripts/cargo-ci.sh` overrides this with `CARGO_INCREMENTAL=0` for every gate step,
  since a gate run compiles once and exits with no later rebuild to amortize against; a
  dedicated `incremental-state` gate step fails the run if a non-empty `incremental` directory
  remains under the target directory afterward.
- **Shared compiler cache** — when `sccache` is on `PATH`, `scripts/cargo-ci.sh` exports it as
  `RUSTC_WRAPPER` unless a wrapper is already set. Set `CARGO_CI_NO_SCCACHE=1` for a diagnostic
  run that must bypass the cache. Opting an existing target directory into a wrapper changes
  Cargo fingerprints, so remove its stale artifacts first if disk headroom is limited.
- **Host-wide serialization** — gate and benchmark builds default to the shared
  `${XDG_RUNTIME_DIR:-/tmp}/cargo-ci.lock`, allowing Cargo-heavy work in adjacent repositories
  to queue instead of competing for CPU and causing timeout-sensitive tests to become noisy.
  `CARGO_CI_BUILD_LOCK` overrides the path when an isolated host needs a different convention.
  Run compilation-heavy focused checks as `./scripts/cargo-ci.sh --cargo test ...` (or
  `--cargo clippy ...`) so they reuse the lock and cache without running the broad gate. Invoke
  `./scripts/cargo-ci.sh` without arguments only when the complete gate is intended.
- **Dependency features** — `jsonschema` (`crates/jit/Cargo.toml`) sets
  `default-features = false`: repository schemas only use local fragment refs
  (`#/types/Priority`, `#/types/State`), so the crate's remote `$ref`-resolution
  infrastructure is never exercised. `ureq` (the remote-document-access client) also sets
  `default-features = false` and enables exactly `rustls` and `gzip`: one deliberately chosen
  TLS backend, plus the content-decoding feature the remote-document path relies on.

### Benchmarking a build-topology change

`scripts/benchmark-rust-build.sh` is the committed benchmark harness; see its header comment
for the full contract and environment overrides. Each clean or rebuild sample runs in a
freshly created, isolated `CARGO_TARGET_DIR`, removed immediately after that sample's
measurements are recorded, so no sample inherits another's warm build state. It collects at
least three clean samples (zero-warning `cargo clippy --workspace --all-targets -- -D
warnings` followed by `cargo test --workspace --no-run`, each timed) and at least three
rebuild samples (the same sequence as setup, then one fixed, reversible, comment-only probe
line appended to `crates/jit/src/lib.rs`, a second timed `test --no-run` for the incremental
rebuild, and the file restored byte-for-byte and verified against `git show HEAD:...`), and
reports the median of each. Run it with `./scripts/benchmark-rust-build.sh`; see
[dev/benchmarks/rust-build-efficiency/report.md](benchmarks/rust-build-efficiency/report.md)
for the story's own recorded comparison and acceptance verdict.

### Diagnosing a budget-checker failure

On failure, `scripts/rust-build-budget.sh` reports the observed value, the limit, and a
corrective area for each violated check on stderr. Read that diagnostic before adding a new
test target or dependency feature:

- **Integration-test targets over budget** — a new top-level `crates/jit/tests/*.rs` file was
  added. Add the case to an existing suite's module instead (see the shared-helpers and
  case-placement guidance in "3. Integration Tests" above), or fold it into the suite whose
  execution model and subsystem it matches.
- **Active test-executable bytes over budget** — the linked executables grew past the
  ceiling. Check whether a new dependency pulled in unexpectedly large debug info, or whether
  the debug/incremental profile policy above regressed, before adding more test code to the
  same suites.
- **Profile drift** — `[profile.dev]` or `[profile.test]` in the workspace `Cargo.toml` no
  longer sets `debug = "line-tables-only"`. Restore it.
- **Incremental-policy drift** — `scripts/cargo-ci.sh` no longer exports
  `CARGO_INCREMENTAL=0` before its first step. Restore the export.
- **Remote resolver reintroduction** — `jsonschema` in `crates/jit/Cargo.toml` no longer sets
  `default-features = false`, or explicitly re-enables a `resolve-*`/`tls-*` feature. Disable
  default features (and drop the explicit feature) again.
- **Duplicate TLS backend / TLS backend drift** — `ureq` enables `native-tls` alongside
  `rustls`, or no longer enables `rustls`. Keep exactly the `rustls` feature (plus `gzip`).

## Test Environment

Two environment behaviors matter when writing tests:

- **`JIT_TEST_MODE=1`** disables the git guards that global operations enforce in a live
  repository: the branch-drift check against main history (`enforce_main_only_operations`) and the
  claims-index validation inside `jit validate`. `TestHarness::new()` sets it. Integration
  tests that call these paths set it explicitly.
- **Selected doc examples** in `crates/jit/src/` compile and run under `cargo test --doc`.
  Keep them for non-obvious workflows and material contracts, not as one-per-public-API
  coverage: each snippet carries a separate rustdoc compilation cost. When an example is
  warranted, it is part of the contract and must keep working.

## Running Tests

```bash
# Everything (unit + doc + harness + integration, all workspace crates)
cargo test

# Unit tests only, fastest feedback
cargo test --lib

# Harness tests
cargo test --test fast_docs_templates harness_demo

# One integration suite, or one module within it
cargo test --test cli_repo_workflow
cargo test --test cli_query_graph query_tests

# Doc examples
cargo test --doc

# A single test by name
cargo test test_harness_query_ready

# With stdout captured from passing tests
cargo test -- --nocapture

# Serialized, for debugging cross-test interference
cargo test -- --test-threads=1
```

### Nextest

The repository provisions cargo-nextest 0.9.133 for the workspace suite. Install that
version with:

```bash
cargo install cargo-nextest --locked --version 0.9.133
```

Run the workspace suite with nextest, then run doctests as a separate substep because
nextest does not cover them:

```bash
cargo nextest run --workspace
cargo test --doc --workspace
```

The committed nextest policy is documented in
[`.config/nextest.toml`](../.config/nextest.toml).

Lint and format alongside tests. Both must be clean before a commit:

```bash
cargo clippy --workspace --all-targets   # zero warnings required
cargo fmt --all -- --check
```

## Writing Tests

### Practice

1. **Write the test first.** TDD is the project default.
2. **Start at the cheapest layer.** Unit test the function, harness test the workflow, and
   add an integration test only when the CLI surface itself is the thing under test.
3. **Reach for a property test when you can state an invariant.** DAG operations, id
   resolution, and idempotent rebuilds are the recurring candidates.
4. **Assert specifically.** Compare against expected values rather than checking that a
   collection is non-empty.

### Naming

Test names follow `test_<function>_<scenario>` and describe the scenario in full:

```rust
// Names that tell you what broke
test_query_ready_returns_only_unassigned()
test_add_dependency_rejects_cycle()
test_gate_blocks_until_passed()

// Names that make you open the file
test_query()
test_dependencies()
test_it_works()
```

### Assertions

```rust
// Specific
assert_eq!(ready.len(), 1);
assert_eq!(ready[0].id, expected_id);

// Vague
assert!(ready.len() > 0);
assert!(ready.contains(&expected));
```

### Adding Coverage for a New Command

Take the layers in order and stop as soon as the behavior is pinned down. Sketched here for a
hypothetical `jit issue defer` that returns an issue to `Backlog`:

```rust
// 1. Unit test, in the command's module under crates/jit/src/commands/
#[test]
fn test_defer_issue_returns_issue_to_backlog() {
    let executor = CommandExecutor::new(InMemoryStorage::new());
    // ...assertions on the returned value and on storage...
}

// 2. Harness test, in crates/jit/tests/fast_docs_templates/harness_demo.rs
#[test]
fn test_harness_defer_issue() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Task");

    h.executor.defer_issue(&id).unwrap();

    assert_eq!(h.get_issue(&id).state, State::Backlog);
}

// 3. Integration test, only if the CLI surface is user-facing
#[test]
fn test_cli_defer_issue() {
    let temp = setup_test_repo();
    let output = Command::new(jit_binary())
        .args(["issue", "defer", "abc12345"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
}
```

## See Also

- [AGENTS.md](../AGENTS.md) - architecture, layer boundaries, coding conventions
- [.github/copilot-instructions.md](../.github/copilot-instructions.md) - TDD guidelines and functional style
- [docs/reference/jit-content-standards.md](../docs/reference/jit-content-standards.md) - content standards for docs and issues
- [crates/jit/tests/common/harness.rs](../crates/jit/tests/common/harness.rs) - the harness implementation
- [crates/jit/tests/fast_docs_templates/harness_demo.rs](../crates/jit/tests/fast_docs_templates/harness_demo.rs) - worked harness examples
