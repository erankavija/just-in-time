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

A property of the `IssueStore` contract is a case in
`crates/jit/src/storage/issue_store_conformance.rs`, which runs every case against every
backend; a backend's own tests cover only what that backend adds.

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
| `crates/jit/src/storage/claim_coordinator_proptests.rs` | Lease counts stay consistent; sequence numbers increase monotonically; concurrent claims stay exclusive; staleness and recovery agree |
| `crates/jit/tests/fast_docs_templates/template_apply_tests.rs` | Template application yields an acyclic, transitively reduced graph; force-refresh is idempotent over nodes and edges |
| `crates/jit/tests/fast_issue/readiness_coherence_tests.rs` | Stored readiness agrees with the graph-derived blocked predicate after any sequence of dependency additions and removals |
| `crates/jit/tests/fast_issue/short_hash_tests.rs` | Any unique prefix resolves to its issue; shared prefixes are reported as ambiguous |

When proptest finds a counterexample it records the seed so the case is replayed on every
later run. Those seeds live in `crates/jit/proptest-regressions/` and, for the integration
target, in `crates/jit/tests/fast_issue/short_hash_tests.proptest-regressions` (beside the
suite module that owns the property). Commit them: they are regression tests.

Write a property test when you can name an invariant that must hold for all inputs. Write an
example test when you care about one specific input.

### Claim Coordination Coverage

`crates/jit/src/storage/claim_coordinator_proptests.rs` owns these properties:

- `test_lease_count_invariant` and `test_sequence_numbers_monotonic` check stored
  lease counts and audit sequence order.
- `test_concurrent_claims_exclusive` and `test_concurrent_different_issues_succeed`
  check that competing agents get one lease per issue and can claim distinct issues.
- `test_staleness_consistent_with_threshold`,
  `test_evict_lapsed_reclaims_an_indefinite_lease_iff_it_is_stale`, and
  `test_finite_leases_never_stale` check heartbeat staleness and reclamation.

Coordinator tests in `crates/jit/src/storage/claim_coordinator.rs` cover per-issue
storage (`test_lease_lock_apply_stores_each_lease_as_its_issue_record_only`),
history-independent reads
(`test_lease_lock_loads_lists_and_applies_without_the_claims_index_or_log_history`),
and expiry and reclamation
(`test_evict_lapsed_logs_one_record_per_lease_and_counts_each_cause_apart`).
`crates/jit/src/storage/claim_coordinator_interruption_tests.rs` owns the failure
matrix in `test_lease_lock_apply_interrupted_at_any_step_leaves_the_prior_or_the_next_lease`.
Listing behavior is covered in `crates/jit/src/commands/claim.rs` by
`test_list_and_status_omit_expired_and_mark_stale_leases_without_a_lock_or_a_write`.

## 2. Harness Tests

Location: `crates/jit/src/test_harness.rs` (the harness, a `jit` library module gated on
`cfg(any(test, feature = "test-support"))` so it compiles once for every target) and the
in-process suites that use it, such as
`crates/jit/tests/fast_issue/test_no_coordinator.rs`.

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
    pub fn create_issue(&self, title: &str) -> IssueId;
    pub fn create_issue_with_desc(&self, title: &str, desc: &str) -> IssueId;
    pub fn create_issue_with_priority(&self, title: &str, priority: Priority) -> IssueId;
    pub fn create_ready_issue(&self, title: &str) -> IssueId;
    pub fn create_issue_with_gates(&self, title: &str, gates: Vec<GateKey>) -> IssueId;

    // Gate setup
    pub fn add_gate(&self, key: &GateKey, title: &str, description: &str, mode: GateMode, checker: Option<GateChecker>);

    // Queries
    pub fn all_issues(&self) -> Vec<Issue>;
    pub fn get_issue(&self, id: &IssueId) -> Issue;
}

impl Default for TestHarness { /* delegates to new() */ }
```

`new()` initializes the storage. The helpers cover the common
shapes; anything else goes through `h.executor` (any command) or `h.storage` (direct
`IssueStore` calls).

`with_item_kinds()` writes the shipped `[item_kinds]` table into the harness repo's
`config.toml`, derived from `profiles/jit-default/manifest.toml` by
`test_utils::shipped_item_kinds_toml()`; a test never restates a shipped kind. The engine bakes in no item kinds (`@/inv/domain-agnostic`), so a test that
exercises addressable items, item indexing, or link resolution must opt in. Call it on the
harness before creating issues:

```rust
let h = TestHarness::new().with_item_kinds();
```

### Usage

A suite that runs in-process imports the shared harness from the `jit` library:

```rust
use jit::test_harness::TestHarness;
```

`test_issue_lifecycle_works_without_coordinator` in `fast_issue/test_no_coordinator.rs` is a
worked example: it creates, updates, claims and releases an issue through `h.executor`, and
asserts the stored result.

Run the whole suite with `cargo test --test fast_issue`, or one module with
`cargo test --test fast_issue test_no_coordinator`.

## 3. Integration Tests

Integration tests are grouped into cohesive **suites**. Each suite is one directory under
`crates/jit/tests/<suite>/` whose `main.rs` declares its test files as modules, so Cargo
links and runs one executable per suite instead of one per file. Suites divide by execution
model and subsystem:

| Suite | Execution model | Covers |
| --- | --- | --- |
| `fast_issue`, `fast_rules`, `fast_docs_templates` | in-process (`CommandExecutor`) | issue lifecycle/graph, rules/labels/config, documents/templates |
| `cli_issue`, `cli_gate`, `cli_query_graph`, `cli_item_validate`, `cli_repo_workflow` | CLI subprocess | the `jit` binary's command surface by area |
| `scratch_build` | policy checks | manifest/source policy checks |

The `cargo-ci` gate runs `scripts/rust-build-budget.sh` as a `budget` step after
its measured suite and its `excluded-tests` step, on warm Cargo artifacts,
pointing it with `--root` at the workspace that run compiled. The checker derives the integration-test
target count and unique active test-executable bytes from `cargo metadata` and
`cargo test --workspace --no-run --message-format=json`, and asserts the debug
profile and dependency-feature policies, failing the gate when
a budget or policy is exceeded (jit:3f73423b). Its inputs are injectable
(`--metadata-json`, `--artifacts-json`, `--root`), so
the script's `--self-test` exercises the checker with synthetic JSON, policy
manifests and sparse executables, without compilation. This repository runs it
through the `build-budget-selftest` gate, separately from the default Rust suite;
missing tools fail the self-test. See "4. Build
Footprint Budget" below for the exact budgets, the build-profile and
dependency-feature policy they check, and how to diagnose a failure.

This repository's `cargo-ci-selftest` gate runs `scripts/cargo-ci-selftest.sh`
separately from the default Rust suite. Any non-zero self-test exit fails the
gate, including exit 2 for missing tools. The gate checks that `cargo-ci`
rejects broken merged trees (jit:3019eacd). A textually clean merge can leave `main`
broken in ways no per-issue gate saw, so the worktree dispatch protocol runs the
gate on the merged tree. The self-test seeds merges in throwaway git repos, runs
the shipped `scripts/cargo-ci.sh` against each, and asserts the verdict it
reports. Three break the build or the tests — a declaration of a deleted module,
a `#[cfg(test)]` caller that lost an argument, a tree that compiles and fails at
test time — and two hold a test out of the default run, by an ignore attribute
and by an ignored doctest, so the `test` step passes and the `excluded-tests`
step is what catches them. A healthy merge is the control every one of those is
read against. The signature and stale-expect trees also assert that `cargo build
--workspace` still succeeds on them: that is why a build-only merge check was
vacuous, and substituting one makes the self-test fail. Every fixture is a
dependency-free two-module crate with its own target directory, so the whole
self-test costs seconds and never rebuilds this workspace.

Shared in-process helpers live in the `jit` library's `test-support` surface (for example
`crates/jit/src/test_harness.rs`), so they never surface as their own test targets. Add a new integration case to the file that
matches its subsystem, or add a module file plus a `mod` line in the suite's `main.rs`.

New and migrated CLI tests spawn `jit` through `jit::test_cli::JitCli`, the one shared
subprocess fixture. It owns a temporary repository (`uninitialised()`, `initialised()` by
`jit init`, or `with_repository(TempDir)` for one built by shared test support) and removes it
on drop. `command(args)` yields a `std::process::Command` for the target's `jit` with the
repository as working directory and every inherited `JIT_*` variable removed;
`command_with_env` sets a variable for that process alone, so tests keep `assert_cmd`'s
assertion traits. `run` and `run_json` return stdout or parsed `--json` output,
`create_issue` returns the created id from `--json`, and `init_git` (backed by
`init_git_repository`, which in-process tests also call) initialises git with a fixed author
identity and no dependence on global git configuration. `git(args)` runs git under that same
isolation and returns stdout, and `commit(message)` stages the working tree and commits it so
`HEAD` resolves. `program_command(program, args)` spawns another program, such as a copy of
the binary `jit_binary()` locates, under the environment `command` gives `jit`.
`JitCli::linked_checkouts()` answers a `(primary, linked)` pair: a primary checkout
initialised by git and `jit init` with one base commit and machine-local `.jit/` runtime
state git-ignored, and a `git worktree add` checkout of it; both share one temporary
directory, removed when the last of them drops. A test that uses the fixture names no binary path,
`CARGO_BIN_EXE_jit` or `cargo_bin!`, and scrapes no human-readable output for ids.

```rust
#[test]
fn test_create_and_query() {
    let cli = JitCli::initialised().unwrap();
    cli.create_issue(["--title", "Task", "--priority", "high"]).unwrap();

    cli.command(["query", "ready", "--json"]).assert().success();
    let json = cli.run_json(["query", "ready"]).unwrap();
    assert!(json["issues"].is_array());
}
```

Reserve this layer for what only it can see: user-facing command contracts, `--json` envelope
shapes, flag combinations, and regression tests for reported CLI bugs. Workflow coverage
belongs in the harness layer, where it runs faster and fails more legibly.

`crates/jit/tests/cli_query_graph/query_tests.rs` shows the reduced shape: each `query`
subcommand and flag has one wiring case that proves the command reaches the domain query
and applies its filter. Selection, priority order and blocked reasons belong to the pure
functions in `crates/jit/src/domain/queries.rs`, including `query all` filter conjunction
and readiness with unpassed required gates. The CLI cases use `JitCli`, the shared
`TaxonomyRepo`, and issue ids from JSON output; alias and flag-placement cases cover parsing
boundaries. Claim and release commands live in `cli_repo_workflow/claim_integration_tests.rs`.

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

`cargo-ci`'s `budget` step (`scripts/rust-build-budget.sh`) enforces three budgets on every
gate run. Two are derived from `cargo metadata` and `cargo test --workspace --no-run
--message-format=json` rather than a `target/` directory scan (stale per-hash artifacts
there cannot describe the current build): a bounded count of integration-test targets and
a bounded total of unique active test-executable bytes. The third is derived from the measured
nextest-plus-doctest suite duration, which `scripts/cargo-ci.sh` passes in as
`--test-suite-ms`: the suite must finish under `MAX_TEST_SUITE_SECONDS`. All three
constants are declared once, in the script's header comment, and derived from a measurement
of this tree and a stated headroom rule in
[dev/benchmarks/rust-build-budgets/README.md](benchmarks/rust-build-budgets/README.md); see
[dev/benchmarks/suite-enforcement-4b7c06d0/README.md](benchmarks/suite-enforcement-4b7c06d0/README.md)
for the measured margin this budget leaves on an idle and a contended host. "5. Inherent
Test Costs" below attributes what the measured suite duration is spent on.

The measurement is taken over a target the `suite-build` step has already built and read
into the page cache, so a fresh target directory and a warm one reach the same verdict and
a duration failure is about the tree (jit:0708d692). The suite runs no nested Cargo build:
`.config/nextest.toml` declares no setup script, which `scratch_build/build_profile_policy_tests.rs`
asserts. Profiled-repository fixtures and the recorded-failure corpus are built inside the suite
on first use from a cache keyed by their inputs. Their first-run construction contributes to
the duration margin the budgets README records; see
[dev/benchmarks/cold-warm-verdict-0708d692/README.md](benchmarks/cold-warm-verdict-0708d692/README.md)
for the cold and warm figures behind this paragraph. Re-measuring needs `--force`, since
`jit gate evaluate` reuses a recorded verdict over unchanged declared inputs, `target/` not
among them.

A fourth budget — at most 10 GiB for the complete fresh validation target directory — is the
acceptance threshold the benchmark protocol below validates against once per build-topology
change, not re-checked on every gate run: a full clean rebuild on every gate invocation would
defeat the point of the incremental-build policy described next. See
[dev/benchmarks/rust-build-efficiency/report.md](benchmarks/rust-build-efficiency/report.md)
("Comparison and acceptance thresholds") for the acceptance criteria and the measured
comparison they were applied to; "Benchmarking a build-topology change" below describes the
harness that produces both.

### Build profile and dependency-feature policy

- **Debug info** — `[profile.dev]`/`[profile.test]` in the workspace `Cargo.toml` set
  `debug = "line-tables-only"`: enough for line-number backtraces on a local failure, without
  embedding the full debugger payload (type info, macro expansions) that dominates a test
  executable's size.
- **Dependency optimization** — `[profile.dev.package."*"]` sets `opt-level = 1`, so third-party
  crates are compiled optimized while workspace crates keep the debug profile's compile times.
  Level 1 is the measured selection over level 2, which ran no faster and cost about twice the
  build regression.
  The suite runs dependency code far more often than it compiles it: its CPU profile is spread
  across SHA-256 over whole build artifacts, TOML parsing, and JSON serialization, none of which
  is workspace code. A package override changes no other profile key, so `debug-assertions` and
  `overflow-checks` still hold everywhere. The measured build cost of the override, against the
  matched unoptimized arm and both fixed 25% ceilings, is in
  [dev/benchmarks/dependency-profile-6d10e5d4/](benchmarks/dependency-profile-6d10e5d4/).
- **Incremental compilation** — both profiles state `incremental = true` explicitly rather
  than leaving Cargo's default undocumented, and that setting governs gate runs too:
  `scripts/cargo-ci.sh` overrides it nowhere. The premise that it once did — that a gate run
  compiles once and exits with no later rebuild to amortize against — does not hold for the
  runs that dominate the repository's build cost. Every agent worktree run and every local
  re-check reruns the gate against a warm target directory, where one edited crate recompiles
  instead of the workspace, at no cost to a cold run. What it costs is disk, which the reclaim
  path in the worktree dispatch protocol prunes. The measurements are under "Provenance" in
  [dev/benchmarks/rust-build-budgets/README.md](benchmarks/rust-build-budgets/README.md).
- **Shared compiler cache** — when `sccache` is on `PATH` and no wrapper is set,
  `scripts/cargo-ci.sh` selects it as `RUSTC_WRAPPER`. Every wrapper the run uses, selected or
  explicit, runs through `scripts/caller-tmpdir-wrapper.sh`, which restores the caller's `TMPDIR`
  so a daemon it starts outlives the run's temporary directory. Set `CARGO_CI_NO_SCCACHE=1` for a
  diagnostic run that must bypass the cache. Opting an existing target directory into a wrapper
  changes Cargo fingerprints, so remove its stale artifacts first if disk headroom is limited.
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
  the debug profile policy above regressed, before adding more test code to the same suites.
- **Profile drift** — `[profile.dev]` or `[profile.test]` in the workspace `Cargo.toml` no
  longer sets `debug = "line-tables-only"`. Restore it.
- **Remote resolver reintroduction** — `jsonschema` in `crates/jit/Cargo.toml` no longer sets
  `default-features = false`, or explicitly re-enables a `resolve-*`/`tls-*` feature. Disable
  default features (and drop the explicit feature) again.
- **Duplicate TLS backend / TLS backend drift** — `ureq` enables `native-tls` alongside
  `rustls`, or no longer enables `rustls`. Keep exactly the `rustls` feature (plus `gzip`).

## 5. Inherent Test Costs

`dev/benchmarks/suite-profile.json` (contract `suite-timing-evidence`) records a warm
per-test duration, `warm_duration_ms`, for every test nextest ran in the default
`cargo nextest run --workspace` invocation — 4,481 tests at revision `cb8d42c3`, cache
state warm, one warmup run ahead of the measured run, `ceil(exec_time_seconds * 1000)`.
It predates the redo-record cutover and is stale against the current suite: refresh it with
`./scripts/profile-test-suite.sh` before adding a row below or changing a figure.
Its slowest test sits well under the per-test bound in
[`.config/nextest.toml`](../.config/nextest.toml); the budgets README above records the whole
run's margin under `MAX_TEST_SUITE_SECONDS`. The tables below attribute cost, not risk: for every test whose
measured cost is a direct consequence of the property it proves — a real subprocess, a
real redo-record round trip through storage, real filesystem publication,
genuine concurrency, or a property-based search over many generated cases — this names it,
its warm cost, and the mechanism. A cost this attribution does not find a mechanism for
belongs to a reducibility issue, not to this table.

### Real subprocess timing and signal semantics

Each of these spawns a real child process or binds a real socket and proves a bounded-time
property that cannot resolve faster than the real timeout, signal, or process teardown it
exercises: `test_execute_command_timeout` spawns `sleep 10` under a shorter enforced
timeout; `test_timeout_kills_process_group_and_does_not_deadlock` spawns a background
grandchild that would otherwise hold a pipe open for 60s; `test_start_server_bounds_a_child_that_neither_serves_nor_exits`
bounds a real child to a 2s startup window; `test_jit_server_shutdown_force_closes_a_stalled_connection_and_exits_zero`
and `test_open_registered_stalled_connection_waits_for_the_stopped_accept_loop` drive a
real `jit-server` process through a stalled connection and its shutdown deadline;
`test_command_exit_codes_broken_pipe_emits_141` writes a 2 MiB description so a real child
`jit` process is still writing to stdout when the parent closes its read end after the
first byte, forcing a real `SIGPIPE` — twice, once plain and once `--json`;
`test_unrelated_process_still_respects_bootstrap_lock` holds a real file lock in-process
while a competing real `jit` subprocess is denied it; `test_is_serving_on_port_reports_a_bound_socket_nobody_serves_as_not_serving`
must wait out a real connect-without-accept before it can conclude no one answered.

| Test | Warm (ms) |
| --- | ---: |
| `graceful_shutdown_tests::test_jit_server_shutdown_force_closes_a_stalled_connection_and_exits_zero` | 5,324 |
| `commands::serve::tests::test_start_server_bounds_a_child_that_neither_serves_nor_exits` | 2,012 |
| `command_exit_code_projection_tests::test_command_exit_codes_broken_pipe_emits_141` | 1,219 |
| `graceful_shutdown_tests::test_open_registered_stalled_connection_waits_for_the_stopped_accept_loop` | 1,121 |
| `nested_checker_recovery_test::test_unrelated_process_still_respects_bootstrap_lock` | 1,057 |
| `gate_execution::tests::test_execute_command_timeout` | 1,018 |
| `gate_execution::tests::test_timeout_kills_process_group_and_does_not_deadlock` | 1,014 |
| `commands::serve::tests::test_is_serving_on_port_reports_a_bound_socket_nobody_serves_as_not_serving` | 519 |

### Property-based tests over real production code paths

proptest's default is 256 generated cases per property. `crates/jit/src/storage/claim_coordinator_proptests.rs`
runs its coordination properties against a real on-disk store, with real file
locks and, for concurrency cases, real threads. It sets `with_fsync(false)`:
these properties verify lease counts, sequence order, exclusivity, and reclamation
rather than crash durability, so the cost is the file and lock work
itself. It bounds filesystem work with fewer generated cases while leaving pure-logic
properties at the default case count. `crates/jit/tests/fast_docs_templates/template_apply_tests.rs`
and `crates/jit/tests/fast_issue/readiness_coherence_tests.rs` run each case through a
fresh in-memory `TestHarness`/`CommandExecutor` pipeline rather than a synthetic
data structure, and cap at 48 cases for the same reason. Both caps trade case count for
staying inside the per-test budget without dropping to example-based coverage; see those
files' own configuration comments for the exact rationale.

| Test | Warm (ms) |
| --- | ---: |
| `storage::claim_coordinator::proptests::test_concurrent_different_issues_succeed` | 3,008 |
| `storage::claim_coordinator::proptests::test_concurrent_claims_exclusive` | 2,931 |
| `template_apply_tests::prop_apply_yields_transitively_reduced_graph` | 2,430 |
| `template_apply_tests::prop_force_refresh_is_idempotent_on_node_and_edge_set` | 2,420 |
| `template_apply_tests::prop_apply_always_yields_acyclic_dag` | 2,262 |
| `readiness_coherence_tests::prop_stored_readiness_agrees_with_the_blocked_predicate_after_dependency_edits` | 1,732 |
| `storage::claim_coordinator::proptests::test_sequence_numbers_monotonic` | 1,247 |
| `storage::claim_coordinator::proptests::test_lease_count_invariant` | 571 |

### CLI subprocess publishing durable state

Each of these spawns the compiled `jit` binary, once or several times in the same test
(a dry run, an apply, then a second apply to prove the no-op; a plain-text and a `--json`
report of the same drift), to pack, apply, validate, or repair a profile package on disk.
`test_profile_pack_and_add_carry_a_package_at_the_model_limits` carries the packaging
fixture at its model limits directly through the CLI; the rest of this table pays the same
durable-publication machinery at smaller scale, often several times per test. Publication
cost at repository scale is measured by [`scripts/benchmark-mutation-cost.sh`](../scripts/benchmark-mutation-cost.sh);
its committed run is [dev/benchmarks/publication-cost-722878d4/](benchmarks/publication-cost-722878d4/README.md).

| Test | Warm (ms) |
| --- | ---: |
| `profile_cli_tests::test_profile_reapply_repairs_missing_and_stale_default_schemas_before_no_op` | 4,095 |
| `profile_acceptance_tests::test_profile_fresh_init_and_existing_apply_are_equivalent_without_git` | 2,894 |
| `profile_cli_tests::test_profile_apply_dry_run_is_read_only_then_apply_is_exact_no_op` | 2,828 |
| `derived_state_repair_tests::test_cli_validate_fix_repairs_each_owned_class_and_preserves_authored_bytes` | 2,635 |
| `profile_acceptance_tests::test_offline_public_cli_profile_reaches_implementation_ready_breakdown` | 2,129 |
| `profile_cli_tests::test_profile_pack_and_add_carry_a_package_at_the_model_limits` | 2,058 |
| `derived_state_repair_tests::test_cli_validate_fix_repairs_mode_only_profile_drift` | 2,002 |
| `profile_cli_tests::test_validate_plain_and_json_report_installed_profile_drift` | 1,575 |
| `derived_state_repair_tests::test_cli_validate_fix_preserves_permission_error_classification` | 1,370 |
| `profile_cli_tests::test_profiled_init_publishes_valid_repo_and_applied_inventory` | 1,328 |
| `profile_acceptance_tests::test_profile_application_contributes_workflow_invariants_to_scaffolded_registry` | 1,245 |
| `profile_cli_tests::test_existing_partial_profiled_init_atomically_completes_neutral_scaffold` | 1,224 |
| `repo_discovery_tests::test_explicit_non_ancestor_data_root_keeps_worktree_assets_at_cwd` | 980 |
| `repo_discovery_tests::test_nested_profile_init_keeps_data_and_assets_in_child` | 944 |
| `repo_discovery_tests::test_relative_data_root_override_keeps_worktree_assets_at_discovered_root` | 939 |
| `cross_substrate_generality_tests::test_all_four_kinds_through_one_generic_path` | 621 |
| `derived_state_repair_tests::test_cli_validate_fix_profile_provenance_failures_are_zero_write` | 543 |
| `failure_probe_fixture::test_failure_probe_fixture_reports_nonzero_status_for_each_setup_class` | 532 |

### Command-module tests publishing a full profile package's owned surface

These stay below the CLI subprocess boundary — unit tests over `commands::`, `storage::`,
and `profile::` modules that, per "1. Unit Tests" above, work against a real
`TempDir`-backed store because that is what those modules exist to do. Each applies,
composes, or repairs a real, complete profile package's owned surface (schemas, registries,
rules, generated docs) rather than a one-file fixture, so the cost scales with the
package's real owned-materialization count. `test_pack_package_archive_stays_within_its_bounds_for_a_package_at_the_model_limits`
packs the same `write_package_tree_at_model_limits` fixture, directly through the library
rather than the CLI. `test_validate_fix_repairs_owned_materializations_in_memory` is the one
exception to the real-filesystem description above — it seeds an equivalently complete
in-memory repository fixture instead — but the property proven, and the cost, is the same:
repair correctness across every owned class of a real profile package.

| Test | Warm (ms) |
| --- | ---: |
| `commands::validate::tests::test_validate_fix_repairs_every_owned_materialization_and_preserves_unowned_files` | 1,967 |
| `commands::validate::tests::test_validate_fix_repairs_owned_materializations_in_memory` | 1,923 |
| `commands::profile::tests::test_apply_profile_selection_recovers_an_obsolete_default_schema_atomically` | 1,856 |
| `commands::profile::tests::test_apply_profile_package_jit_dogfood_composes_what_applying_both_packages_composes` | 1,830 |
| `derived_state_repair_tests::test_harness_validate_fix_repairs_each_owned_class_and_preserves_authored_bytes` | 1,568 |
| `profile::repository_package::tests::test_profile_applies_to_neutral_repo_and_ordinary_renderers_consume_config` | 1,460 |
| `derived_state_repair_tests::test_harness_validate_fix_repairs_mode_only_profile_drift` | 1,444 |
| `commands::init::tests::test_fresh_profile_init_records_every_package_of_the_selected_closure` | 1,222 |
| `commands::init::tests::test_fresh_profile_init_publishes_complete_valid_repo_without_git` | 1,199 |
| `commands::init::tests::test_reinit_profiled_over_existing_root_is_idempotent_unchanged` | 996 |
| `commands::profile::tests::test_apply_workflow_profile_to_bare_repository_contributes_hierarchy_rules` | 909 |
| `profile::package_archive::tests::test_pack_package_archive_stays_within_its_bounds_for_a_package_at_the_model_limits` | 529 |

### Genuine concurrency: real threads, timers, and watchers

`test_concurrent_fresh_profile_init_publishes_one_coherent_repository` races real threads
through real initialization; `test_concurrent_writer_observes_failed_apply_preimage_then_publishes`
does the same across a real write barrier. `test_start_watching_ignores_events_jsonl` starts a real
filesystem watcher and sleeps past its own real debounce window; it is compiled once into
the `jit-server` crate's library test target and once into its binary target, so it runs —
and is measured — twice.

| Test | Warm (ms) |
| --- | ---: |
| `commands::init::tests::test_concurrent_fresh_profile_init_publishes_one_coherent_repository` | 1,213 |
| `template_apply_atomicity_tests::test_concurrent_writer_observes_failed_apply_preimage_then_publishes` | 610 |
| `watcher::tests::test_start_watching_ignores_events_jsonl` (library target) | 513 |
| `watcher::tests::test_start_watching_ignores_events_jsonl` (binary target) | 512 |

### Data-driven tests bundling many real scenarios

`test_all_steering_scenarios` drives every scenario fixture under
`crates/jit/tests/fixtures/steering/` from one test function by design — its own doc
comment records the choice, made so adding a scenario needs a fixture, not a Rust change —
and each scenario runs multiple real `jit` subprocess invocations.

| Test | Warm (ms) |
| --- | ---: |
| `steering_scenarios::test_all_steering_scenarios` | 2,519 |

### Deliberate at-scale tests

`test_resolve_ambiguous` creates real issues through the full command pipeline until
two land on the same 4-character id prefix — the property under test, ambiguous-prefix
rejection, cannot be observed without a real collision.

| Test | Warm (ms) |
| --- | ---: |
| `short_hash_tests::test_resolve_ambiguous` | 1,969 |

### Repeated real gate execution

`commands::gate_check::tests` runs gates defined with `GateChecker::Exec`, so every gate
check in these tests spawns a real shell command. Proving history compaction,
re-execution-on-change, and priority ordering takes several such checks per test — seven in
the slowest of the group, which repeats a failing gate to exercise history compaction.

| Test | Warm (ms) |
| --- | ---: |
| `commands::gate_check::tests::test_run_history_is_compacted_to_one_latest_run` | 1,848 |
| `commands::gate_check::tests::test_check_gate_run_history_keeps_only_latest_legacy_run` | 547 |
| `commands::gate_check::tests::test_check_all_gates_respects_priority_order` | 541 |
| `commands::gate_check::tests::test_check_all_gates_stable_sort_same_priority` | 537 |
| `commands::gate_check::tests::test_check_gate_executes_again_after_declared_inputs_change` | 515 |

### Default run coverage

What the default run covers is fixed by `@/invariant/no-ignored-tests`. The `excluded-tests`
step of `scripts/cargo-ci.sh` enforces it from the counts the run itself reports — nextest's
skipped tally and the doctest step's ignored tally — so the verdict describes the suite that
ran rather than what the sources appear to declare.

## Test Environment

Two environment behaviors matter when writing tests:

- **Process-wide state** (environment variables, working directory) is never changed, so tests
  read it and run in parallel without a lock. A test that varies a value passes it explicitly
  (`Command::env`, `Command::current_dir`, a function parameter, an absolute path).
  `scratch_build/process_state_policy_tests.rs` rejects any call that changes this state.
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

# Harness tests, one module
cargo test --test fast_issue test_no_coordinator

# One integration suite, or one module within it
cargo test --test cli_repo_workflow
cargo test --test cli_query_graph query_tests

# Doc examples
cargo test --doc

# A single test by name
cargo test test_issue_lifecycle_works_without_coordinator

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
[`.config/nextest.toml`](../.config/nextest.toml), which also pins a per-test ceiling: a
slow-timeout of 10s, terminated after a second 10s grace period, so any single test that
runs past 20s fails the run rather than hanging it.

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
   resolution, and lease exclusivity are recurring candidates.
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

// 2. Harness test, in an in-process workflow suite such as
//    crates/jit/tests/fast_issue/test_no_coordinator.rs
#[test]
fn test_defer_issue_returns_ready_issue_to_backlog() {
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
- [crates/jit/src/test_harness.rs](../crates/jit/src/test_harness.rs) - the harness implementation
- [crates/jit/tests/fast_issue/test_no_coordinator.rs](../crates/jit/tests/fast_issue/test_no_coordinator.rs) - worked harness examples
