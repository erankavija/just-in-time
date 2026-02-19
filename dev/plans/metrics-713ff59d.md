# Implementation Plan: Metrics Reporting System (`713ff59d`)

## Executive Summary

This plan implements `jit metrics` as a new top-level command (not a subcommand group) following the pattern established by `jit status`. The command is a single variant with category selection via positional argument and time-range/format flags. Metric calculation logic lives in `crates/jit/src/domain/metrics.rs` as pure functions (no I/O), orchestration in `crates/jit/src/commands/metrics.rs`, CLI wiring in `cli.rs`, and handler dispatch in `main.rs`.

---

## 1. CLI Subcommand Design

### Command Signature

```
jit metrics [category] [--since DATE] [--last N] [--format json|prometheus|csv] [--json]
```

Where `[category]` is an optional positional argument with possible values: `issues`, `velocity`, `gates`, `deps`, `all` (default if omitted).

### Clap Definition — in `crates/jit/src/cli.rs`

Add this variant to the `Commands` enum (around line 184, before `Recover`):

```rust
/// Metrics and reporting
///
/// Reports actionable insights into issue tracker state and workflow performance.
/// Categories: issues (state distribution), velocity (throughput, cycle time),
/// gates (pass rates), deps (graph health), all (default).
Metrics {
    /// Category: issues, velocity, gates, deps, all (default)
    #[arg(default_value = "all")]
    category: String,

    /// Only include events since this date (RFC 3339 or YYYY-MM-DD)
    #[arg(long)]
    since: Option<String>,

    /// Only include events from the last N days
    #[arg(long)]
    last: Option<u64>,

    /// Output format (json, prometheus, csv)
    #[arg(long, default_value = "json")]
    format: String,

    /// Machine-readable JSON output (alias for --format json)
    #[arg(long)]
    json: bool,
},
```

The `--json` flag is kept for consistency with every other command. When `--json` is set, it overrides `--format` to `json`.

---

## 2. Event Log Parsing Approach

### Existing Infrastructure

`IssueStore::read_events()` (defined in `crates/jit/src/storage/mod.rs` line 159) loads all events as `Vec<Event>`. The `Event` enum in `crates/jit/src/domain/types.rs` (lines 556-709) has these variants relevant to metrics:

| Event Variant | Fields of Interest |
|---|---|
| `IssueCreated` | `issue_id`, `timestamp` |
| `IssueStateChanged` | `issue_id`, `timestamp`, `from: State`, `to: State` |
| `IssueClaimed` | `issue_id`, `timestamp`, `assignee` |
| `GatePassed` | `issue_id`, `timestamp`, `gate_key`, `updated_by` |
| `GateFailed` | `issue_id`, `timestamp`, `gate_key`, `updated_by` |
| `IssueCompleted` | `issue_id`, `timestamp` |
| `IssueReleased` | `issue_id`, `timestamp`, `assignee`, `reason` |

### Time Range Filtering

Parse time bounds in the command handler, then filter events before passing to metric functions:

```rust
// In commands/metrics.rs
fn filter_events_by_time(events: &[Event], since: Option<DateTime<Utc>>) -> Vec<&Event> {
    events.iter().filter(|e| {
        if let Some(since_dt) = since {
            get_event_timestamp(e) >= since_dt
        } else {
            true
        }
    }).collect()
}

fn get_event_timestamp(event: &Event) -> DateTime<Utc> {
    match event {
        Event::IssueCreated { timestamp, .. } => *timestamp,
        Event::IssueStateChanged { timestamp, .. } => *timestamp,
        // ... all variants
    }
}
```

`--last N` is converted to `since = Utc::now() - Duration::days(N as i64)`.

---

## 3. Metric Calculation Functions

All functions live in `crates/jit/src/domain/metrics.rs`. They are pure — take `&[Issue]` and `&[Event]` as input, return computed structs. No I/O.

### 3.1 Issue State Distribution

```rust
pub struct IssueMetrics {
    pub by_state: HashMap<String, usize>,
    pub by_priority: HashMap<String, usize>,
    pub total: usize,
    pub wip: usize,   // in_progress + gated
    pub done: usize,
    pub open: usize,  // backlog + ready
}

pub fn compute_issue_metrics(issues: &[Issue]) -> IssueMetrics
```

Implementation: iterate issues, bucket by `issue.state` and `issue.priority`. WIP = count where state is `InProgress` or `Gated`.

### 3.2 Workflow Velocity Metrics

```rust
pub struct VelocityMetrics {
    pub throughput_per_day: f64,
    pub avg_cycle_time_secs: Option<f64>,   // ready→done
    pub avg_lead_time_secs: Option<f64>,    // created→done
    pub median_cycle_time_secs: Option<f64>,
    pub p95_cycle_time_secs: Option<f64>,
    pub wip_count: usize,
    pub issues_completed: usize,
    pub time_window_days: u64,
    pub blocked_time_distributions: BlockedTimeStats,
}

pub struct BlockedTimeStats {
    pub avg_blocked_time_secs: Option<f64>,
    pub issues_with_blocked_time: usize,
}
```

**Cycle time (Ready → Done):** For each issue, find the earliest `IssueStateChanged { from: Ready, to: InProgress }` event and the latest `IssueStateChanged { to: Done }` event. Group events by `issue_id` using `HashMap<String, Vec<&Event>>`.

```rust
pub fn compute_velocity_metrics(
    issues: &[Issue],
    events: &[Event],
    since: Option<DateTime<Utc>>,
    window_days: u64,
) -> VelocityMetrics

// Internal helper
struct IssueTimeline {
    created_at: Option<DateTime<Utc>>,
    first_ready_at: Option<DateTime<Utc>>,
    done_at: Option<DateTime<Utc>>,
}

fn compute_per_issue_timelines(events: &[Event]) -> HashMap<String, IssueTimeline>
```

**Throughput:** count `IssueStateChanged { to: Done }` events in time window, divide by window days.

**Blocked time:** time spent between `IssueStateChanged { to: Backlog }` (re-blocked) and next `IssueStateChanged { to: Ready }`.

### 3.3 Gate Performance Metrics

```rust
pub struct GateMetrics {
    pub by_gate: HashMap<String, GateStats>,
    pub overall_pass_rate: f64,
    pub total_evaluations: usize,
}

pub struct GateStats {
    pub gate_key: String,
    pub passed: usize,
    pub failed: usize,
    pub pass_rate: f64,
    pub avg_time_to_pass_secs: Option<f64>, // GateAdded → GatePassed
}

pub fn compute_gate_metrics(events: &[Event]) -> GateMetrics
```

Implementation: iterate events, bucket `GatePassed`/`GateFailed` by `gate_key`. For time-to-pass: find the `GateAdded` event for the same `(issue_id, gate_key)` pair.

### 3.4 Dependency Graph Health Metrics

```rust
pub struct DepsMetrics {
    pub total_issues: usize,
    pub total_edges: usize,
    pub avg_fanout: f64,
    pub max_depth: usize,
    pub root_count: usize,
    pub isolated_count: usize,
    pub blocking_issues: usize,   // issues blocking 2+ others
    pub longest_chain_ids: Vec<String>,
}

pub fn compute_deps_metrics(issues: &[Issue]) -> DepsMetrics
```

Reuse the existing `DependencyGraph` from `crates/jit/src/graph.rs`. Use `graph.get_roots()`, `graph.get_isolated_nodes()`, `graph.get_dependents()`. Max depth: BFS from each root.

---

## 4. Output Format Implementations

All formatting happens in `crates/jit/src/commands/metrics.rs`.

### 4.1 JSON Output

```rust
#[derive(Serialize)]
pub struct MetricsReport {
    pub generated_at: String,
    pub time_window: TimeWindow,
    pub issues: Option<IssueMetrics>,
    pub velocity: Option<VelocityMetrics>,
    pub gates: Option<GateMetrics>,
    pub deps: Option<DepsMetrics>,
}

#[derive(Serialize)]
pub struct TimeWindow {
    pub since: Option<String>,
    pub until: String,
    pub days: Option<u64>,
}
```

Follow the `JsonOutput::success(data, "metrics")` pattern from `crates/jit/src/output.rs` (line 120).

### 4.2 Prometheus Exposition Format

Plain text, one metric per line:

```
# HELP jit_issues_total Total issues by state
# TYPE jit_issues_total gauge
jit_issues_total{state="backlog"} 12
jit_issues_total{state="ready"} 3
jit_issues_total{state="in_progress"} 2
# HELP jit_velocity_throughput_per_day Issue throughput per day
# TYPE jit_velocity_throughput_per_day gauge
jit_velocity_throughput_per_day 1.4
# HELP jit_gate_pass_rate Gate pass rate by gate key
# TYPE jit_gate_pass_rate gauge
jit_gate_pass_rate{gate="tests"} 0.92
```

```rust
fn format_prometheus(report: &MetricsReport) -> String
```

Follow Prometheus naming conventions: `jit_` prefix, snake_case, `_total` for counters, no suffix for gauges.

### 4.3 CSV Output

Two-column format: `metric_name,value`. Categories separated by blank lines with `#` headers:

```
# issues
state_backlog,12
state_ready,3
# velocity
throughput_per_day,1.4
avg_cycle_time_secs,43200
```

```rust
fn format_csv(report: &MetricsReport) -> String
```

---

## 5. New Files and Modifications

### New files

- `crates/jit/src/domain/metrics.rs` — All pure computation structs and functions
- `crates/jit/src/commands/metrics.rs` — `CommandExecutor::compute_metrics()`, output formatters, `MetricsReport`/`TimeWindow` structs
- `crates/jit/tests/metrics_harness_tests.rs` — Harness-level tests
- `crates/jit/tests/metrics_tests.rs` — Integration tests

### Modified files

| File | Change |
|------|--------|
| `crates/jit/src/cli.rs` | Add `Metrics { ... }` variant to `Commands` enum (~line 184) |
| `crates/jit/src/commands/mod.rs` | Add `mod metrics;` (~line 34) |
| `crates/jit/src/domain/mod.rs` | Add `pub mod metrics;` and re-export key types |
| `crates/jit/src/main.rs` | Add `Commands::Metrics { ... }` arm before `Commands::Snapshot` (~line 3940) |

---

## 6. TDD Approach

### Step 1: Unit tests in `domain/metrics.rs`

Write these first, before implementation:

```rust
// test_compute_issue_metrics_empty_returns_all_zeroes
// test_compute_issue_metrics_counts_by_state_correctly
// test_compute_issue_metrics_counts_by_priority_correctly
// test_compute_issue_metrics_wip_includes_in_progress_and_gated
// test_compute_velocity_no_events_returns_none_times
// test_compute_velocity_cycle_time_from_ready_to_done
// test_compute_velocity_throughput_counts_done_in_window
// test_compute_velocity_excludes_events_before_since
// test_compute_gate_metrics_empty_events_returns_zero_evaluations
// test_compute_gate_metrics_pass_rate_calculation
// test_compute_gate_metrics_groups_by_gate_key
// test_compute_deps_metrics_counts_edges_correctly
// test_compute_deps_metrics_identifies_roots
// test_compute_deps_metrics_max_depth_linear_chain
```

Helper for constructing test events:

```rust
fn make_state_change(issue_id: &str, from: State, to: State, mins_ago: i64) -> Event {
    Event::IssueStateChanged {
        id: uuid::Uuid::new_v4().to_string(),
        issue_id: issue_id.to_string(),
        timestamp: Utc::now() - chrono::Duration::minutes(mins_ago),
        from,
        to,
    }
}
```

### Step 2: Harness tests in `tests/metrics_harness_tests.rs`

```rust
// test_metrics_all_categories_returns_report
// test_metrics_issues_category_only_returns_issue_metrics
// test_metrics_velocity_category_only
// test_metrics_gates_category_only
// test_metrics_deps_category_only
// test_metrics_since_filter_excludes_old_events
// test_metrics_last_n_days_filter
```

### Step 3: Integration tests in `tests/metrics_tests.rs`

```rust
// test_metrics_command_json_output_structure
// test_metrics_command_prometheus_format
// test_metrics_command_csv_format
// test_metrics_command_invalid_category_returns_error
// test_metrics_command_invalid_format_returns_error
// test_metrics_command_empty_repo_returns_zeroes
```

---

## 7. Step-by-Step Implementation Order

### Phase 1 — Domain (no CLI touches)

1. Create `crates/jit/src/domain/metrics.rs` with all structs defined (empty bodies)
2. Write all unit tests — they will fail
3. Implement `compute_issue_metrics()` — pass its tests
4. Implement `compute_velocity_metrics()` with `IssueTimeline` helper — pass its tests
5. Implement `compute_gate_metrics()` — pass its tests
6. Implement `compute_deps_metrics()` using existing `DependencyGraph` — pass its tests
7. Add `pub mod metrics;` to `crates/jit/src/domain/mod.rs`, verify `cargo test --lib`

### Phase 2 — Command orchestration

8. Create `crates/jit/src/commands/metrics.rs` with `MetricsReport`, `TimeWindow` and `CommandExecutor::compute_metrics()`
9. Implement `format_prometheus()` and `format_csv()`
10. Add `mod metrics;` to `crates/jit/src/commands/mod.rs`
11. Write and pass harness tests

### Phase 3 — CLI wiring

12. Add `Metrics { category, since, last, format, json }` to `Commands` in `cli.rs`
13. Add dispatch arm in `main.rs`:
    - Parse `since` date string to `DateTime<Utc>`
    - Convert `--last N` to `since` datetime
    - Return error if both `--since` and `--last` are provided
    - `--json` overrides `--format`
    - Call `executor.compute_metrics(category, since_dt, window_days)`
    - Format and print output
14. Write and pass integration tests

### Phase 4 — Quality gates

```bash
cargo clippy --workspace --all-targets
cargo fmt --all
cargo test --lib
cargo test --test metrics_harness_tests
cargo test --test metrics_tests
```

---

## Implementation Notes

**No new dependencies needed.** `chrono` is already in `Cargo.toml`. Prometheus and CSV are plain text.

**Use `IssueStateChanged` for all timeline calculations**, not `IssueCompleted`. The `IssueStateChanged { to: Done }` event is always emitted and carries the `from` state, making it suitable for all timeline math.

**Gate time-to-pass fallback:** For gates present at issue creation, there is no `GateAdded` event. Use `IssueCreated` timestamp as fallback for time-to-pass calculation.

**Velocity window when no filter specified:** Use all available events. Calculate `time_window.days` as the span from oldest event to now.

**`--since` and `--last` are mutually exclusive.** Return an error if both are provided.
