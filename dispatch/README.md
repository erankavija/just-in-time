# jit-dispatch

Orchestrator for the jit issue tracker. Coordinates multiple agents to work on ready issues based on priority.

## Features

- **Priority-based dispatch**: Assigns critical > high > normal > low
- **Agent capacity tracking**: Respects max_concurrent limits
- **Periodic polling**: Continuously monitors for ready work
- **Clean separation**: Uses jit query interface (no direct storage access)

## Usage

### Configuration

Create a `dispatch.toml` file:

```toml
poll_interval_secs = 5

[[agents]]
id = "copilot-1"
type = "copilot"
max_concurrent = 2
command = "gh copilot work {issue_id}"

[[agents]]
id = "ci-runner"
type = "ci"
max_concurrent = 5
command = "run-ci-job {issue_id}"
```

### Running

Start the daemon:
```bash
jit-dispatch start --config dispatch.toml --repo /path/to/jit/repo
```

Run one dispatch cycle:
```bash
jit-dispatch once --config dispatch.toml --repo /path/to/jit/repo
```

## Architecture

```
┌─────────────────┐
│  jit-dispatch   │  Orchestrator daemon
└────────┬────────┘
         │ Uses query interface
         ▼
┌─────────────────┐
│      jit        │  Issue tracker (CLI)
└─────────────────┘
         │ Stores data
         ▼
┌─────────────────┐
│   data/*.json   │  Repository storage
└─────────────────┘
```

The orchestrator:
1. Polls `jit query ready --json` for unassigned issues
2. Sorts by priority (critical > high > normal > low)
3. Assigns to available agents via `jit issue claim`
4. Tracks agent capacity (max_concurrent per agent)
5. Repeats every `poll_interval_secs`

## Testing

Run tests:
```bash
cargo test -p jit-dispatch
```

## Development

The orchestrator is implemented as a library (`src/lib.rs`) with a CLI frontend (`src/main.rs`). This allows for easy testing and potential embedding in other tools.

See `../ROADMAP.md` for features and `../TESTING.md` for test strategy.
