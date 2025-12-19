# Production Observability Design

## Overview

This document outlines observability features for production deployments: metrics reporting and stalled work detection. These features provide visibility into system health, workflow efficiency, and potential issues requiring intervention.

## Motivation

Production systems require observability to:
- **Monitor health**: Identify bottlenecks and inefficiencies
- **Track progress**: Measure velocity and cycle times
- **Detect issues**: Find stalled work and orphaned assignments
- **Inform decisions**: Data-driven prioritization and resource allocation

For AI agent orchestration, observability is critical to detect coordination failures and ensure agents make continuous progress.

## 1. Metrics Reporting

### Design Goals

Provide actionable metrics about issue tracker state and workflow performance through simple CLI commands and JSON output for integration with monitoring systems.

### Metrics Categories

#### Issue State Metrics

Basic distribution statistics:
- Count by state (backlog, ready, in-progress, gated, done, blocked)
- Count by priority (critical, high, normal, low)
- Count by assignee (including unassigned)
- Count by label (strategic labels: milestone, epic)

#### Workflow Velocity Metrics

Measure progress and cycle times:
- **Throughput**: Issues completed per time period (day/week/month)
- **Cycle time**: Average time from ready → done
- **Lead time**: Average time from creation → done
- **Work in progress (WIP)**: Count of in-progress issues
- **Blocking time**: Average time issues spend blocked

#### Gate Performance Metrics

Quality gate effectiveness:
- **Gate pass rate**: % of gate checks that pass
- **Gate failure rate**: % of gate checks that fail by gate type
- **Precheck effectiveness**: % of issues failing prechecks before work starts
- **Postcheck effectiveness**: % of issues failing postchecks
- **Gate execution time**: Average time per gate checker

#### Dependency Metrics

Graph health indicators:
- **Dependency depth**: Average/max depth of dependency chains
- **Blocking issues**: Count of issues blocking others
- **Root issues**: Count of issues with no dependencies
- **Leaf issues**: Count of issues blocking nothing

### Command Interface

```bash
# Overall summary
jit metrics

# Specific metric category
jit metrics --category workflow
jit metrics --category gates
jit metrics --category dependencies

# Time range filtering
jit metrics --since "2025-01-01"
jit metrics --range "last-7-days"
jit metrics --range "last-30-days"

# Output formats
jit metrics --json
jit metrics --format prometheus  # For Prometheus scraping
jit metrics --format csv
```

### JSON Output Format

```json
{
  "timestamp": "2025-12-19T22:00:00Z",
  "range": {
    "start": "2025-12-01T00:00:00Z",
    "end": "2025-12-19T22:00:00Z"
  },
  "issue_state": {
    "backlog": 15,
    "ready": 8,
    "in_progress": 3,
    "gated": 1,
    "done": 42,
    "blocked": 2,
    "total": 71
  },
  "workflow": {
    "throughput": {
      "last_7_days": 12,
      "last_30_days": 42
    },
    "cycle_time": {
      "average_hours": 18.5,
      "median_hours": 12.0,
      "p95_hours": 48.0
    },
    "lead_time": {
      "average_hours": 72.3,
      "median_hours": 48.0
    },
    "wip_count": 3,
    "blocked_time_hours": {
      "average": 6.2,
      "median": 2.0,
      "max": 48.0
    }
  },
  "gates": {
    "total_checks": 156,
    "passed": 142,
    "failed": 14,
    "pass_rate": 0.91,
    "by_gate": {
      "tests": {"passed": 45, "failed": 2, "rate": 0.96},
      "clippy": {"passed": 47, "failed": 0, "rate": 1.0},
      "code-review": {"passed": 38, "failed": 12, "rate": 0.76}
    }
  },
  "dependencies": {
    "avg_depth": 2.3,
    "max_depth": 5,
    "root_issues": 12,
    "leaf_issues": 28,
    "blocking_issues": 8
  }
}
```

### Prometheus Export Format

```prometheus
# HELP jit_issues_total Total number of issues
# TYPE jit_issues_total gauge
jit_issues_total{state="backlog"} 15
jit_issues_total{state="ready"} 8
jit_issues_total{state="in_progress"} 3

# HELP jit_cycle_time_seconds Average cycle time in seconds
# TYPE jit_cycle_time_seconds gauge
jit_cycle_time_seconds 66600

# HELP jit_gate_checks_total Total gate checks
# TYPE jit_gate_checks_total counter
jit_gate_checks_total{gate="tests",result="passed"} 45
jit_gate_checks_total{gate="tests",result="failed"} 2
```

### Implementation Approach

1. **Event Log Analysis**: Parse `.jit/data/events.jsonl` for historical data
2. **Issue Snapshot**: Current state from issue files
3. **Gate Results**: Parse `.jit/gate-runs/` for gate metrics
4. **Time Calculations**: Extract timestamps from events and calculate durations
5. **Caching**: Cache metrics for recent time ranges (invalidate on events)

### Use Cases

**Development Team**:
- Daily standup: Check WIP and blocked issues
- Retrospective: Review cycle times and gate pass rates
- Planning: Analyze throughput for capacity planning

**AI Agents**:
- Self-monitoring: Check personal assignment metrics
- Coordination: Identify blocking issues to prioritize
- Quality: Monitor gate pass rates to adjust testing

**Operations**:
- Prometheus integration for dashboards and alerts
- Trend analysis for system health
- Capacity planning based on throughput

## 2. Stalled Work Detection

### Design Goals

Automatically identify issues that are stuck or abandoned, enabling intervention before they become critical blockers.

### Stall Detection Criteria

#### In-Progress Too Long

Issues in `in-progress` state exceeding configurable threshold:
- **Default threshold**: 7 days
- **Configurable**: Per-label or global
- **Indicators**: No recent events, no gate activity

#### Orphaned Assignments

Assigned issues with inactive assignees:
- **Detection**: Assignee has no recent activity across all assigned issues
- **Threshold**: 3+ days of inactivity
- **Use case**: Agent crashes or network issues

#### Blocked Too Long

Issues in `blocked` state with no progress on dependencies:
- **Detection**: Blocking dependencies show no recent events
- **Threshold**: 7+ days
- **Indicators**: Dependency chain might be abandoned

#### Ready But Never Claimed

Issues stuck in `ready` state for extended periods:
- **Detection**: High priority issues not claimed
- **Threshold**: 7+ days for critical, 14+ for high, 30+ for normal
- **Indicators**: Work backlog or capacity issues

#### Gated Without Resolution

Issues in `gated` state with no gate check attempts:
- **Detection**: No gate check events in recent period
- **Threshold**: 3+ days
- **Indicators**: Unclear gate requirements or agent confusion

### Command Interface

```bash
# List all stalled issues
jit query stalled

# Specific stall types
jit query stalled --type in-progress
jit query stalled --type orphaned
jit query stalled --type blocked
jit query stalled --type ready-unclaimed
jit query stalled --type gated

# Custom thresholds
jit query stalled --threshold 3d  # 3 days instead of default 7
jit query stalled --type in-progress --threshold 14d

# With reasons
jit query stalled --show-reason

# JSON output
jit query stalled --json
```

### JSON Output Format

```json
{
  "timestamp": "2025-12-19T22:00:00Z",
  "stalled_issues": [
    {
      "id": "01ABC",
      "title": "Implement feature X",
      "state": "in-progress",
      "stall_type": "in_progress_too_long",
      "stalled_since": "2025-12-05T10:00:00Z",
      "stall_duration_hours": 350,
      "assignee": "agent:worker-1",
      "last_activity": "2025-12-05T15:30:00Z",
      "reason": "No activity for 14.6 days (threshold: 7 days)",
      "suggested_action": "Check agent status or reassign"
    },
    {
      "id": "02DEF",
      "title": "Fix bug Y",
      "state": "blocked",
      "stall_type": "blocked_too_long",
      "stalled_since": "2025-12-01T08:00:00Z",
      "stall_duration_hours": 434,
      "blocking_issues": ["03GHI"],
      "reason": "Blocked on 03GHI which has no activity for 18 days",
      "suggested_action": "Review blocking issue or remove dependency"
    }
  ],
  "summary": {
    "total_stalled": 2,
    "by_type": {
      "in_progress_too_long": 1,
      "blocked_too_long": 1
    }
  }
}
```

### Implementation Approach

1. **Event Log Scanning**: Parse recent events for activity indicators
2. **State Analysis**: Check current issue states against thresholds
3. **Dependency Traversal**: Analyze dependency chains for blocked issues
4. **Assignee Activity**: Track per-assignee activity across all issues
5. **Configurable Thresholds**: Support per-label thresholds via config file

### Intervention Workflows

**Automated**:
- Daily report: Email or Slack notification of stalled issues
- Metrics integration: Count of stalled issues exposed via `jit metrics`
- CI/CD integration: Fail build if critical issues stalled

**Manual**:
- Review stalled issues in daily standup
- Reassign orphaned issues
- Remove or update blocking dependencies
- Add notes or context to gated issues

### Configuration

`.jit/config/stall-detection.json`:
```json
{
  "thresholds": {
    "in_progress_days": 7,
    "blocked_days": 7,
    "orphaned_days": 3,
    "gated_days": 3,
    "ready_unclaimed_days": {
      "critical": 7,
      "high": 14,
      "normal": 30,
      "low": 60
    }
  },
  "per_label_overrides": {
    "epic:critical-path": {
      "in_progress_days": 3,
      "blocked_days": 2
    }
  }
}
```

## Testing Strategy

### Metrics Reporting

- Unit tests: Metric calculations from known event logs
- Integration tests: Full workflow with events → metrics
- Performance tests: Metrics calculation on large event logs (10k+ events)
- Format tests: JSON and Prometheus output validation

### Stalled Work Detection

- Unit tests: Stall detection logic with various scenarios
- Time tests: Mock current time to test threshold logic
- Integration tests: End-to-end stall detection with realistic data
- Edge cases: Empty repos, no events, all states

## Documentation

- User guide: Interpreting metrics and responding to stalled work
- Operations guide: Prometheus integration and alerting setup
- Configuration guide: Customizing stall thresholds

## Implementation Plan

1. **Metrics reporting** (2-3 days): Event log parsing, metric calculations, output formats
2. **Stalled work detection** (2-3 days): Stall detection logic, configuration, query command
3. **Testing & documentation** (1-2 days): Comprehensive tests, user guides

**Total effort**: 5-8 days of focused development
