# jit-dispatch Examples

This directory contains example orchestrators and integration patterns demonstrating how to build custom dispatchers for the jit issue tracker.

## Quick Start Examples

### 1. Bash One-Liner Orchestrator

**File:** `bash-orchestrator.sh`

A minimal bash script showing the simplest possible orchestrator:
- Polls for ready issues every 5 seconds
- Claims issues for a single agent
- Simulates work processing
- Marks issues as done

**Usage:**
```bash
chmod +x examples/bash-orchestrator.sh
./examples/bash-orchestrator.sh
```

**Good for:** Quick prototyping, understanding the basics, shell scripting enthusiasts

### 2. Simple Python Orchestrator

**File:** `simple-orchestrator.py`

A clean Python implementation with:
- Agent pool management
- Capacity tracking (max concurrent per agent)
- Priority-based dispatch (critical > high > normal > low)
- Daemon mode with configurable polling

**Usage:**
```bash
python3 examples/simple-orchestrator.py
```

**Good for:** Production use, extending with custom logic, Python ecosystems

### 3. Integration Patterns

**File:** `integration-patterns.md`

Comprehensive guide covering:
- GitHub Actions integration
- Kubernetes CronJob deployment
- Webhook-driven orchestration
- Multi-repo coordination
- Stale issue detection
- Best practices and testing strategies

**Good for:** Understanding advanced patterns, production deployments, custom integrations

## Example Comparison

| Example | Lines | Language | Features | Complexity |
|---------|-------|----------|----------|------------|
| Bash one-liner | ~50 | Bash | Basic polling & claiming | ⭐ Simple |
| Python orchestrator | ~130 | Python | Agent pool, priority dispatch | ⭐⭐ Medium |
| Built-in (jit-dispatch) | ~400 | Rust | Full-featured, production-ready | ⭐⭐⭐ Advanced |

## Architecture

All orchestrators follow the same pattern:

```
┌──────────────────┐
│  Orchestrator    │
│  (Your Code)     │
└────────┬─────────┘
         │
         │ 1. Query ready issues
         │    jit query ready --json
         ├─────────────────────────────┐
         │                             │
         │ 2. Claim issue              │
         │    jit issue claim <id> <agent>
         ├─────────────────────────────┤
         │                             │
         │ 3. Agent executes work      │
         │    (external process)       │
         ├─────────────────────────────┤
         │                             │
         │ 4. Mark complete            │
         │    jit issue update <id> --state done
         └─────────────────────────────┘
                     ▲
                     │
         Loop with poll interval
```

## Building Your Own

### Minimal Requirements

Your orchestrator needs to:
1. **Query** for ready issues: `jit query ready --json`
2. **Claim** issues for agents: `jit issue claim <id> <agent>`
3. **Loop** with appropriate interval

### Optional Enhancements

- Priority-based sorting
- Agent capacity management
- Timeout and retry logic
- Health checks
- Metrics and monitoring
- Multi-repo support

### Using jit as a Library (Rust)

If writing in Rust, you can use jit as a library instead of subprocess:

```rust
use jit::{Storage, CommandExecutor};

let storage = Storage::new(".");
let executor = CommandExecutor::new(storage);

// Query ready issues
let ready = executor.query_ready()?;

// Claim issue
executor.claim_issue(&issue_id, "agent:my-agent".to_string())?;
```

## Testing Your Orchestrator

### Setup Test Repository

```bash
# Create test jit repo
mkdir test-repo && cd test-repo
jit init

# Add test issues
jit issue create -t "Test task 1" -p high
jit issue create -t "Test task 2" -p normal
```

### Run Your Orchestrator

```bash
# Test once (single cycle)
timeout 5s ./your-orchestrator.sh

# Verify results
jit query assignee "agent:your-agent"
```

### Check Events

```bash
# View what happened
jit events tail
```

## Common Patterns

### Pattern: Single-Shot Dispatch

Run once and exit (good for cron jobs):

```python
def dispatch_once():
    ready = query_ready_issues()
    for issue in ready:
        claim_issue(issue['id'], 'agent:cron-worker')

if __name__ == "__main__":
    dispatch_once()
```

### Pattern: Daemon Mode

Run continuously with polling:

```python
while True:
    dispatch_cycle()
    time.sleep(poll_interval)
```

### Pattern: Claim-Next for Single Agent

Let jit pick the highest priority work:

```bash
#!/bin/bash
while true; do
    ISSUE_ID=$(jit issue claim-next "agent:my-worker" 2>/dev/null || echo "")
    if [ -n "$ISSUE_ID" ]; then
        process_task "$ISSUE_ID"
        jit issue update "$ISSUE_ID" --state done
    fi
    sleep 30
done
```

## Troubleshooting

### No issues being claimed

Check if issues are actually ready:
```bash
jit query ready
jit query blocked  # See what's blocked
```

### Agent at capacity

Check current assignments:
```bash
jit query assignee "agent:your-agent"
```

### Issues stuck in InProgress

Use release to reclaim:
```bash
jit issue release <id> "timeout"
```

## Real-World Examples

See `integration-patterns.md` for:
- GitHub Actions worker
- Kubernetes CronJob
- Multi-repo orchestration
- Stale issue detection
- Webhook-driven dispatch

## Contributing

Have a cool orchestrator pattern? Submit a PR!

## See Also

- **Built-in orchestrator**: `../README.md` - The production-ready jit-dispatch
- **Integration patterns**: `integration-patterns.md` - Advanced patterns and best practices
- **Core design**: `../../docs/design.md` - Architecture and API design
- **Examples**: `../../EXAMPLE.md` - End-to-end workflow examples
