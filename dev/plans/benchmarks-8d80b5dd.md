# Implementation Plan: Performance Benchmark Suite (`8d80b5dd`)

## Overview

Establishes a criterion.rs benchmark suite for the JIT issue tracker, exercising the most latency-sensitive code paths at realistic scale (1000+ issues, complex DAGs). Four deliverables:

1. `crates/jit/benches/` — benchmark binaries
2. `crates/jit/benches/fixtures.rs` — shared fixture generation
3. `docs/performance.md` — documented baselines and targets
4. `.github/workflows/benchmarks.yml` — CI benchmark tracking

---

## 1. Cargo.toml Changes

**`crates/jit/Cargo.toml`**

Add to `[dev-dependencies]` and add new `[[bench]]` sections:

```toml
[dev-dependencies]
# ... existing ...
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "query_benchmarks"
harness = false

[[bench]]
name = "graph_benchmarks"
harness = false

[[bench]]
name = "storage_benchmarks"
harness = false

[[bench]]
name = "bulk_ops_benchmarks"
harness = false
```

`harness = false` is required for criterion to register its own entry point.

---

## 2. Fixture Generation Module

### `crates/jit/benches/fixtures.rs`

Each bench file imports via `#[path = "fixtures.rs"] mod fixtures;`.

**Key functions:**

```rust
/// n issues in Ready state, no dependencies. Baseline query performance.
pub fn make_storage_with_n_flat_issues(n: usize) -> InMemoryStorage

/// Linear chain: issue[i] depends on issue[i-1]. Returns storage + terminal ID.
pub fn make_storage_with_linear_chain(n: usize) -> (InMemoryStorage, String)

/// Wide DAG: `width` roots, `depth` layers of dependents below each root.
pub fn make_storage_with_wide_dag(width: usize, depth: usize) -> InMemoryStorage

/// n diamond patterns (A->B, A->C, B->D, C->D). For transitive reduction benchmarks.
pub fn make_storage_with_diamond_dag(n: usize) -> InMemoryStorage

/// n issues with variety of labels (type:task, type:bug, milestone:v1.0, epic:alpha).
pub fn make_storage_with_labeled_issues(n: usize) -> InMemoryStorage

/// n issues distributed ~evenly across Backlog/Ready/InProgress/Gated/Done.
pub fn make_storage_with_mixed_states(n: usize) -> InMemoryStorage

/// n issues each with `gates_per_issue` gate requirements.
pub fn make_storage_with_gates(n: usize, gates_per_issue: usize) -> InMemoryStorage
```

**Implementation notes:**
- Set `std::env::set_var("JIT_TEST_MODE", "1")` — follows `TestHarness::new()` pattern from `tests/harness.rs:19-20` to disable worktree divergence checks
- Create issues directly via `storage.save_issue(issue)`, not `CommandExecutor::create_issue`, to avoid config/event-log overhead skewing benchmark times
- Use deterministic UUIDs (`uuid::Uuid::from_u128(i as u128)`) for predictable construction and cacheability

---

## 3. Benchmark Files

### `crates/jit/benches/query_benchmarks.rs`

Sizes: `[100, 500, 1000]` issues via `BenchmarkId::new("query_ready", size)`.

**Functions under test** (from `crates/jit/src/domain/queries.rs` and `commands/query.rs`):
- `query_ready(&issues)` — `queries.rs:39`
- `query_blocked(&issues)` — `queries.rs:53`
- `query_by_state(&issues, state)` — `queries.rs:106`
- `query_by_priority(&issues, priority)` — `queries.rs:115`
- `query_by_label(&issues, pattern)` — `queries.rs:126`
- `executor.search_issues(query)` — `commands/search.rs:6`
- `storage.list_issues()` — raw baseline

**Benchmark functions:**
```rust
fn bench_list_all_issues(c: &mut Criterion)
fn bench_query_ready(c: &mut Criterion)
fn bench_query_blocked(c: &mut Criterion)
fn bench_query_by_state(c: &mut Criterion)
fn bench_query_by_priority(c: &mut Criterion)
fn bench_query_by_label(c: &mut Criterion)
fn bench_search_issues(c: &mut Criterion)
```

**Performance targets:**
- `query_ready(1000 issues)` < 10ms
- `query_blocked(1000 issues)` < 20ms (builds resolved map + checks deps)
- `search_issues(1000 issues)` < 50ms
- `list_issues()` InMemoryStorage < 5ms

---

### `crates/jit/benches/graph_benchmarks.rs`

**Functions under test** (from `crates/jit/src/graph.rs` and `commands/graph.rs`):
- `DependencyGraph::new(&issues)` — `graph.rs:43` — construction at scale
- `graph.validate_dag()` — `graph.rs:168` — full DFS cycle check
- `graph.validate_add_dependency(from, to)` — `graph.rs:55`
- `graph.get_roots()` — `graph.rs:107`
- `graph.get_transitive_dependents(node_id)` — `graph.rs:125`
- `graph.compute_transitive_reduction(node_id)` — `graph.rs:288`
- `visualization::export_dot(&graph)` — `visualization.rs:31`
- `visualization::export_mermaid(&graph)` — `visualization.rs`
- `executor.build_dependency_tree(issue_id, depth)` — `commands/graph.rs:11`

**Fixture sizes:**
- Linear chain: 100, 500, 1000 nodes
- Wide DAG: width=10/depth=5, width=10/depth=10, width=20/depth=10
- Diamond DAG: 50, 100, 200 patterns

**Performance targets:**
- `DependencyGraph::new(1000 issues)` < 5ms
- `validate_dag(1000 issues, linear)` < 10ms
- `export_dot(500 issues)` < 100ms
- `export_mermaid(500 issues)` < 100ms
- `build_dependency_tree(200-node DAG)` < 50ms

**Lifetime note:** `DependencyGraph<'a, T>` holds `&'a T` references. Benchmarks must own the `Vec<Issue>` and build the ref slice inside the closure:

```rust
let issues: Vec<Issue> = fixtures::make_flat_issues(n);
let refs: Vec<&Issue> = issues.iter().collect();
b.iter(|| {
    let graph = DependencyGraph::new(black_box(&refs));
    black_box(graph);
});
```

---

### `crates/jit/benches/storage_benchmarks.rs`

Tests both `InMemoryStorage` and `JsonFileStorage` (via `tempfile::TempDir`).

**Functions under test:**
- `storage.save_issue(issue)` — `memory.rs:74`
- `storage.list_issues()` — `memory.rs:148`
- `storage.load_issue(id)` — `memory.rs:82`
- `storage.resolve_issue_id(partial)` — `memory.rs:91`
- Same four operations for `JsonFileStorage`

**Performance targets:**
- `InMemoryStorage::list_issues(1000 issues)` < 5ms
- `JsonFileStorage::list_issues(500 issues)` < 500ms
- `JsonFileStorage::save_issue` < 10ms per write

Use `iter_batched` or `iter_with_setup` to separate fixture construction from measured operations.

---

### `crates/jit/benches/bulk_ops_benchmarks.rs`

Sizes: 10, 50, 100 issues.

**Functions under test:**
- `executor.apply_bulk_update(&filter, &ops)` — `commands/bulk_update.rs:156` — target: < 1s for 100 issues
- `domain::queries::build_issue_map(&issues)` — `domain/queries.rs:29`
- `issue.is_blocked(&resolved)` — `domain/types.rs:212`
- `issue.has_unpassed_gates()` — `domain/types.rs`

Note: `apply_bulk_update` takes `&mut self` — use `iter_batched` to clone the executor per iteration.

---

## 4. Criterion Configuration

```rust
fn configure_criterion() -> Criterion {
    Criterion::default()
        .sample_size(50)
        .warm_up_time(std::time::Duration::from_secs(1))
        .measurement_time(std::time::Duration::from_secs(5))
}

criterion_group! {
    name = query_benches;
    config = configure_criterion();
    targets = bench_list_all_issues, bench_query_ready, ...
}
```

Use `black_box()` around all inputs:

```rust
b.iter(|| {
    let result = query_ready(criterion::black_box(&issues));
    criterion::black_box(result)
});
```

---

## 5. `docs/performance.md` Structure

```markdown
# Performance

## Performance Targets

| Operation                    | Scale        | Target  | Measured |
|------------------------------|--------------|---------|----------|
| query_ready                  | 1000 issues  | < 10ms  | TBD      |
| query_blocked                | 1000 issues  | < 20ms  | TBD      |
| search_issues                | 1000 issues  | < 50ms  | TBD      |
| export_dot                   | 500 issues   | < 100ms | TBD      |
| export_mermaid               | 500 issues   | < 100ms | TBD      |
| build_dependency_tree        | 200-node DAG | < 50ms  | TBD      |
| bulk_update (state change)   | 100 issues   | < 1s    | TBD      |
| JsonFileStorage::list_issues | 500 issues   | < 500ms | TBD      |
| InMemoryStorage::list_issues | 1000 issues  | < 5ms   | TBD      |
| DependencyGraph::new         | 1000 issues  | < 5ms   | TBD      |

## Running Benchmarks Locally

cargo bench --package jit
open target/criterion/query_benchmarks/query_ready/report/index.html
```

Populate the "Measured" column from actual `cargo bench` output before merging.

---

## 6. CI Integration

**`.github/workflows/benchmarks.yml`**

```yaml
name: Benchmarks
on:
  push:
    branches: [main]
  workflow_dispatch:

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - uses: dtolnay/rust-toolchain@stable
      - name: Cache criterion baselines
        uses: actions/cache@v4
        with:
          path: target/criterion
          key: criterion-baselines-${{ github.ref }}
          restore-keys: criterion-baselines-main
      - name: Run benchmarks
        run: |
          cargo bench --package jit -- --output-format=bencher \
            2>&1 | tee output.txt
      - name: Store benchmark result
        uses: benchmark-action/github-action-benchmark@v1
        with:
          name: JIT Rust Benchmarks
          tool: cargo
          output-file-path: output.txt
          github-token: ${{ secrets.GITHUB_TOKEN }}
          auto-push: true
          alert-threshold: 125%
          comment-on-alert: true
          fail-on-alert: false
      - name: Upload criterion HTML reports
        uses: actions/upload-artifact@v4
        with:
          name: criterion-reports
          path: target/criterion/
          retention-days: 30
```

Uses `benchmark-action/github-action-benchmark` which supports criterion's bencher format natively. Alerts at >25% regression; doesn't block merges.

---

## 7. Step-by-Step Implementation Order

**Step 1:** Add criterion dependency and `[[bench]]` sections to `crates/jit/Cargo.toml`. Verify `cargo check --package jit` passes.

**Step 2:** Create `crates/jit/benches/fixtures.rs` with all fixture functions. Verify `cargo build --package jit --benches`.

**Step 3:** Implement `query_benchmarks.rs` starting with `bench_list_all_issues` and `bench_query_ready`. Run locally and record times.

**Step 4:** Implement `graph_benchmarks.rs` — graph construction and `validate_dag` first, then exports.

**Step 5:** Implement `storage_benchmarks.rs` using `tempfile::TempDir` for `JsonFileStorage` benchmarks.

**Step 6:** Implement `bulk_ops_benchmarks.rs` using `iter_batched` for mutable executor.

**Step 7:** Write `docs/performance.md` with actual measured values from local runs. Fill in the "Measured" column.

**Step 8:** Add `.github/workflows/benchmarks.yml`.

**Step 9:** Run full suite, verify all targets met or document gaps:
```bash
cargo bench --package jit
ls target/criterion/
```

---

## Key Architectural Notes

**`JIT_TEST_MODE=1` must be set** in all fixture functions — otherwise fixture setup fails outside a git worktree.

**`build_issue_map` is called inside every query.** `domain/queries.rs:29` rebuilds a `HashMap<String, &Issue>` on every `query_blocked` call — a 1000-entry allocation per call. This will be the dominant cost in `query_blocked`. Worth calling out in `docs/performance.md`.

**Use `InMemoryStorage` for most benchmarks.** `CommandExecutor::create_issue` triggers config loading, validation, and event logging — use `storage.save_issue(issue)` directly to isolate domain operation cost.

---

## Critical Files

| File | Role |
|------|------|
| `crates/jit/Cargo.toml` | Add criterion dev-dep and `[[bench]]` sections |
| `crates/jit/src/storage/memory.rs` | Core storage under test; `list_issues()` clone cost (line 148) is main variable |
| `crates/jit/src/domain/queries.rs` | Primary benchmark targets; `build_issue_map` at line 29 is key allocation hotspot |
| `crates/jit/src/graph.rs` | `DependencyGraph` lifetime constraint (lines 39-52) drives fixture structure |
| `crates/jit/tests/harness.rs` | Pattern to follow for fixture setup (`JIT_TEST_MODE`, direct `save_issue()`) |
