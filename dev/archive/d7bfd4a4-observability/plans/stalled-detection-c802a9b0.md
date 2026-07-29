# Implementation Plan: Stalled Work Detection (`c802a9b0`)

## Executive Summary

This plan implements `jit query stalled` — a new subcommand under `QueryCommands` that detects five stall patterns by combining issue state data with event log timestamps. The feature follows the established three-layer architecture: pure domain function in `domain/queries.rs`, thin `CommandExecutor` orchestration wrapper in `commands/query.rs`, CLI variant in `cli.rs`, and dispatch in `main.rs`.

Configuration will extend `config.toml` with a new `[stall_detection]` TOML section (using the existing `JitConfig`/`ConfigManager` machinery), avoiding a separate JSON file for consistency with all other existing configuration patterns.

---

## 1. Where `jit query stalled` Fits

### CLI Position

`jit query stalled` belongs as a new variant of `QueryCommands` in `crates/jit/src/cli.rs` at line ~1079 (after the `Closed` variant):

```rust
// In QueryCommands enum (cli.rs ~line 981)
/// Query stalled issues (stuck, abandoned, or coordination failures)
Stalled {
    /// Override in-progress stall threshold in seconds (default: 604800 = 7 days)
    #[arg(long)]
    in_progress_threshold_secs: Option<u64>,

    /// Override orphaned assignment threshold in seconds (default: 259200 = 3 days)
    #[arg(long)]
    orphaned_threshold_secs: Option<u64>,

    /// Override unclaimed high-priority threshold in seconds (default: 86400 = 1 day)
    #[arg(long)]
    unclaimed_threshold_secs: Option<u64>,

    /// Override gated-no-activity threshold in seconds (default: 86400 = 1 day)
    #[arg(long)]
    gated_threshold_secs: Option<u64>,

    /// Filter by label pattern (exact match or wildcard)
    #[arg(short = 'l', long)]
    label: Option<String>,

    /// Filter by priority
    #[arg(short = 'p', long)]
    priority: Option<String>,

    /// Return full issue objects instead of minimal summaries
    #[arg(long)]
    full: bool,

    #[arg(long)]
    json: bool,
},
```

### Main.rs Dispatch

Add a `QueryCommands::Stalled { ... }` arm inside the existing `Commands::Query(query_cmd) => match query_cmd { ... }` block in `crates/jit/src/main.rs` around line ~2537 (just before the closing brace of the query match).

---

## 2. Stall Pattern Detection Logic

### Five Patterns

#### Pattern 1: `InProgressTooLong`
- **Condition**: `issue.state == State::InProgress`
- **Trigger**: Duration since last activity > threshold (default 7 days = 604,800 seconds)
- **Last activity**: Latest event timestamp for this issue from `events.jsonl`, falling back to `issue.updated_at`
- **Suggested remediation**: `"jit issue release {id} timeout"`

#### Pattern 2: `OrphanedAssignment`
- **Condition**: `issue.assignee.is_some()` AND `issue.state` is `Ready` or `InProgress`
- **Trigger**: Duration since last activity > threshold (default 3 days = 259,200 seconds)
- **Suggested remediation**: `"jit issue release {id} inactive-assignee"`

#### Pattern 3: `BlockedByStalled`
- **Condition**: `issue.is_blocked(&resolved)` AND at least one blocking dependency is itself stalled (matching Pattern 1 or 2)
- **Detection**: Check direct dependencies only (depth=1) against the stalled_ids set computed from patterns 1, 2, 4, 5 — no recursive traversal to avoid infinite loops
- **Suggested remediation**: `"jit query stalled"` on each blocker

#### Pattern 4: `UnclaimedHighPriority`
- **Condition**: `issue.state == State::Ready` AND `issue.assignee.is_none()` AND `issue.priority` is `High` or `Critical`
- **Trigger**: Issue has been in Ready state (unassigned) for longer than threshold (default 1 day = 86,400 seconds)
- **Last activity**: Timestamp of the `IssueStateChanged { to: Ready }` event or `issue.updated_at`
- **Suggested remediation**: `"jit issue claim {id} <agent>"`

#### Pattern 5: `GatedWithoutActivity`
- **Condition**: `issue.state == State::Gated`
- **Trigger**: No `GatePassed` or `GateFailed` events within threshold (default 1 day = 86,400 seconds)
- **Last activity**: Latest gate-related event (GatePassed, GateFailed, GateAdded) or `issue.updated_at`
- **Suggested remediation**: `"jit gate check-all {id}"`

### Core Algorithm

```rust
pub fn query_stalled(
    issues: &[Issue],
    events: &[Event],
    config: &StallDetectionConfig,
    now: DateTime<Utc>,
) -> Vec<StalledIssue>
```

1. Build event activity index: `HashMap<issue_id, DateTime<Utc>>` — max timestamp per issue, O(n)
2. Build issue map for dependency resolution (reuse `build_issue_map` from `domain/queries.rs`)
3. For each issue, apply pattern checks 1→5; collect all matching patterns
4. Compute `BlockedByStalled` by checking if any dependency's issue_id is in the stalled_ids set
5. Sort results by: priority DESC, then stall_duration DESC

---

## 3. Configuration Schema

### Extend `config.toml` via `JitConfig`

Add new field to `JitConfig` in `crates/jit/src/config.rs`:

```rust
pub stall_detection: Option<StallDetectionConfig>,
```

New structs (in `config.rs`):

```rust
#[derive(Debug, Clone, Deserialize, Default)]
pub struct StallDetectionConfig {
    pub in_progress_threshold_secs: Option<u64>,
    pub orphaned_threshold_secs: Option<u64>,
    pub unclaimed_high_priority_threshold_secs: Option<u64>,
    pub gated_no_activity_threshold_secs: Option<u64>,
    pub label_overrides: Option<HashMap<String, StallLabelOverride>>,
    pub enabled_patterns: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StallLabelOverride {
    pub in_progress_threshold_secs: Option<u64>,
    pub orphaned_threshold_secs: Option<u64>,
    pub unclaimed_high_priority_threshold_secs: Option<u64>,
    pub gated_no_activity_threshold_secs: Option<u64>,
}
```

Accessor methods with defaults:

```rust
impl StallDetectionConfig {
    pub fn in_progress_threshold_secs(&self) -> u64 {
        self.in_progress_threshold_secs.unwrap_or(604_800) // 7 days
    }
    pub fn orphaned_threshold_secs(&self) -> u64 {
        self.orphaned_threshold_secs.unwrap_or(259_200) // 3 days
    }
    pub fn unclaimed_high_priority_threshold_secs(&self) -> u64 {
        self.unclaimed_high_priority_threshold_secs.unwrap_or(86_400) // 1 day
    }
    pub fn gated_no_activity_threshold_secs(&self) -> u64 {
        self.gated_no_activity_threshold_secs.unwrap_or(86_400) // 1 day
    }
}
```

Example TOML:

```toml
[stall_detection]
in_progress_threshold_secs = 604800
orphaned_threshold_secs = 259200

[stall_detection.label_overrides."sprint:current"]
in_progress_threshold_secs = 172800   # 2 days for current sprint
```

---

## 4. Output Format

### New Domain Types (in `domain/types.rs`)

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StallType {
    InProgressTooLong,
    OrphanedAssignment,
    BlockedByStalled,
    UnclaimedHighPriority,
    GatedWithoutActivity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StalledIssue {
    pub issue_id: String,
    pub short_id: String,
    pub title: String,
    pub state: State,
    pub priority: Priority,
    pub assignee: Option<String>,
    pub stall_type: StallType,
    pub stall_duration_secs: u64,
    pub last_activity_at: String,
    pub stall_description: String,
    pub remediation: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub stalled_dependency_ids: Vec<String>,
}
```

### JSON Output Shape

```json
{
  "count": 2,
  "stalled_issues": [...],
  "summary": {
    "total": 2,
    "by_type": {
      "in_progress_too_long": 1,
      "unclaimed_high_priority": 1
    }
  }
}
```

### Human-Readable Output

```
Stalled issues (2 found):

  STALLED c802a9b0 | Implement authentication [critical/in_progress]
    Type: in_progress_too_long — in-progress for 8 days (threshold: 7 days)
    Last activity: 2026-02-11T10:00:00Z
    Assignee: agent:worker-1
    Remediation:
      jit issue release c802a9b0 timeout
```

---

## 5. Determining "Last Activity"

### Strategy: Event Log Primary, `updated_at` Fallback

```rust
pub fn build_event_activity_index(events: &[Event]) -> HashMap<String, DateTime<Utc>> {
    // Build HashMap<issue_id, max_event_timestamp>
}

fn compute_last_activity(
    issue_id: &str,
    activity_index: &HashMap<String, DateTime<Utc>>,
    issue: &Issue,
) -> DateTime<Utc> {
    activity_index
        .get(issue_id)
        .copied()
        .or_else(|| DateTime::parse_from_rfc3339(&issue.updated_at).ok().map(|dt| dt.with_timezone(&Utc)))
        .or_else(|| DateTime::parse_from_rfc3339(&issue.created_at).ok().map(|dt| dt.with_timezone(&Utc)))
        .unwrap_or_else(Utc::now)
}
```

The domain function must take `now: DateTime<Utc>` as a parameter (not call `Utc::now()` internally) for test determinism.

---

## 6. TDD Approach

### Tests to Write First

#### Unit tests in `domain/queries.rs`

```rust
#[test]
fn test_query_stalled_empty_when_no_issues() { ... }

#[test]
fn test_query_stalled_in_progress_too_long_detected() {
    // InProgress issue, last event 8 days ago, 7-day threshold → detected
}

#[test]
fn test_query_stalled_in_progress_not_stalled_within_threshold() {
    // InProgress issue, last event 2 days ago → not detected
}

#[test]
fn test_query_stalled_orphaned_assignment_detected() { ... }

#[test]
fn test_query_stalled_unclaimed_high_priority_detected() { ... }

#[test]
fn test_query_stalled_unclaimed_normal_priority_not_detected() { ... }

#[test]
fn test_query_stalled_gated_without_activity_detected() { ... }

#[test]
fn test_query_stalled_blocked_by_stalled_detected() { ... }

#[test]
fn test_query_stalled_custom_threshold_respected() { ... }

#[test]
fn test_compute_last_activity_uses_latest_event() { ... }

#[test]
fn test_compute_last_activity_falls_back_to_updated_at() { ... }
```

#### Config tests in `config.rs`

```rust
#[test]
fn test_stall_detection_config_defaults() { ... }

#[test]
fn test_stall_detection_config_parse_from_toml() { ... }

#[test]
fn test_stall_detection_label_override_parse() { ... }
```

#### Integration tests (`tests/stall_detection_tests.rs`)

Use `--in-progress-threshold-secs 0` CLI flag to trigger stalls immediately without sleeping:

```rust
#[test]
fn test_cli_query_stalled_json_output_format() { ... }

#[test]
fn test_cli_query_stalled_empty_returns_count_zero() { ... }

#[test]
fn test_cli_query_stalled_with_priority_filter() { ... }

#[test]
fn test_cli_query_stalled_with_label_filter() { ... }
```

---

## 7. Step-by-Step Implementation Order

### Step 1: Config types (with tests)
**File**: `crates/jit/src/config.rs`
- Add `StallDetectionConfig` and `StallLabelOverride` structs
- Add `stall_detection: Option<StallDetectionConfig>` field to `JitConfig` (~line 13), following the `CoordinationConfig` pattern (~line 212)
- Verify `cargo test --lib`

### Step 2: Domain types
**File**: `crates/jit/src/domain/types.rs`
- Add `StallType` enum after `GateRunStatus` (~line 553)
- Add `StalledIssue` struct, following `MinimalBlockedIssue` pattern (~line 331)

### Step 3: Pure domain query function
**File**: `crates/jit/src/domain/queries.rs`
- Write all failing unit tests first
- Implement `build_event_activity_index`, `compute_last_activity`, `query_stalled`
- `query_stalled` takes `now: DateTime<Utc>` as parameter
- Verify `cargo test --lib`

### Step 4: CommandExecutor wrapper
**File**: `crates/jit/src/commands/query.rs`
- Add `query_stalled(override_config, priority_filter, label_filter)` method
- Load config, merge CLI overrides, call domain function, apply filters

### Step 5: CLI variant
**File**: `crates/jit/src/cli.rs`
- Add `Stalled { ... }` variant to `QueryCommands` enum (~line 1079)
- Follow exact shape of `Blocked` variant (~line 1027)

### Step 6: Dispatch in main.rs
**File**: `crates/jit/src/main.rs`
- Add arm in `Commands::Query` match block (~line 2537)

### Step 7: Export new types
**File**: `crates/jit/src/lib.rs`
- Re-export `StallType` and `StalledIssue` from domain module

### Step 8: Integration tests
**File**: `crates/jit/tests/stall_detection_tests.rs` (new)
- Follow subprocess pattern from `tests/query_tests.rs`
- Use threshold=0 trick for time-based tests

### Step 9: Run all gates
```bash
cargo clippy --workspace --all-targets
cargo fmt --all
cargo test --lib
cargo test --test harness_demo
cargo test --test stall_detection_tests
```

---

## Potential Challenges

**Clock injection**: Domain function must take `now: DateTime<Utc>` as parameter, never call `Utc::now()` internally.

**`BlockedByStalled` circularity**: Only check direct dependencies (depth=1) against the pre-computed stalled_ids set. No recursive traversal.

**`StallDetectionConfig::Default`**: Required for `unwrap_or_default()`. Use `#[derive(Default)]`.

**InMemoryStorage event injection**: Harness tests must inject events with past timestamps directly via `storage.append_event(...)`.

---

## Critical Files

| File | Purpose |
|------|---------|
| `crates/jit/src/domain/queries.rs` | Core logic: `query_stalled`, `build_event_activity_index` (follow `query_ready` at line 39) |
| `crates/jit/src/domain/types.rs` | `StallType` enum + `StalledIssue` struct |
| `crates/jit/src/config.rs` | `StallDetectionConfig` + field on `JitConfig` |
| `crates/jit/src/cli.rs` | `Stalled` variant in `QueryCommands` |
| `crates/jit/src/main.rs` | Dispatch arm in `Commands::Query` match |
| `crates/jit/src/commands/query.rs` | `CommandExecutor::query_stalled` method |
| `crates/jit/tests/stall_detection_tests.rs` | Integration tests (new file) |
