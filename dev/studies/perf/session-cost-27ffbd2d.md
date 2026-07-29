# jit session-cost profile

Companion summary for `dev/archive/1cc809de-repository-state-quality/dev/studies/perf/session-cost-27ffbd2d.json`. That
artifact is the schema template for subsequent harness runs. Every timing below
cites its artifact path, command name, and statistic.

> **Historical profile:** These measurements describe commit `27ffbd2d` before
> the lock-hygiene change. Issue `fc744df6` resolved the sidecar finding on
> 2026-07-24: current issue reads create no per-issue locks, the surviving lock
> set is repository-scoped and constant in issue count, and `list_issues`
> performs a one-time cleanup of legacy empty UUID sidecars.

## Machine and corpus

- jit 0.2.1, commit 27ffbd2d, release
- AMD Ryzen 9 5900X (24 logical CPUs), 32 GiB RAM, Linux 7.1.3-arch1-3
- Corpus: the jit repo's own `.jit/` (665 issues), copied with `cp -a`
- **fs caveat:** measured on **tmpfs**; the real repo is **ext4**. Lock-file / rename / fsync
  costs are understated here. The dominant cost below (minor page faults) is fs-independent.

## Timing

Warm, 25-sample measurements. Sample count source:
`dev/archive/1cc809de-repository-state-quality/dev/studies/perf/session-cost-27ffbd2d.json > method > samples_per_command`.

| command | class | median | p95 | locks created | timing source |
|---|---|---:|---:|---:|---|
| `jit --version` | baseline | 3.0 ms | 3.2 ms | 0 | `dev/archive/1cc809de-repository-state-quality/dev/studies/perf/session-cost-27ffbd2d.json > version > median_ms,p95_ms` |
| `jit issue show <id> --json` | single read | 50.6 ms | 52.8 ms | 1 | `dev/archive/1cc809de-repository-state-quality/dev/studies/perf/session-cost-27ffbd2d.json > issue_show_single_read > median_ms,p95_ms` |
| `jit query available --json` | read-all | 23.4 ms | 25.0 ms | 665 | `dev/archive/1cc809de-repository-state-quality/dev/studies/perf/session-cost-27ffbd2d.json > query_available > median_ms,p95_ms` |
| `jit issue list --json` | read-all | 26.9 ms | 28.9 ms | 665 | `dev/archive/1cc809de-repository-state-quality/dev/studies/perf/session-cost-27ffbd2d.json > issue_list > median_ms,p95_ms` |
| `jit issue update <id> --priority … --json` | mutation | **3720.6 ms** | **3747.5 ms** | 1 | `dev/archive/1cc809de-repository-state-quality/dev/studies/perf/session-cost-27ffbd2d.json > issue_update_mutation > median_ms,p95_ms` |

## Where the mutation cost goes (perf stat, one mutation)

- task-clock 3430 ms: **user 1.14 s, sys 2.70 s**
  (`dev/archive/1cc809de-repository-state-quality/dev/studies/perf/session-cost-27ffbd2d.json > mutation_syscall_summary.command > task_clock_ms,user_s,sys_s`)
- **545,219 minor page faults** (~2.1 GiB touched), context-switches 0
- System-time / page-fault dominated → memory materialization, not I/O.
- context-switches 0 → no blocking git subprocess.
- The sampled `issue_update_mutation` result points to base two-full-capture
  materialization, not lock I/O (one sidecar).

The p95 measurement reproducibly corroborates the audit's previously
single-sourced figure (`dev/archive/1cc809de-repository-state-quality/dev/studies/perf/session-cost-27ffbd2d.json >
issue_update_mutation > p95_ms`).

## Lock mechanism

- Per-issue sidecar lock created at `crates/jit/src/storage/json.rs:918` (`load_issue`
  takes a shared lock on `<id>.lock`), via `FileLocker::open_or_create` (`lock.rs:286`, `O_CREAT`).
- Never removed: `LockGuard::drop` (`lock.rs:66`) unlocks and deletes only `.lock.meta`.
- `cleanup_stale_locks` (`lock_cleanup.rs:67`) only sweeps `.git/jit/locks`, never `.jit/issues/*.lock`.
- Result: one zero-byte `.lock` per distinct issue ever read; **692 today**, unbounded.
- Reads are already safe without it: issue JSON is published via atomic temp+rename
  (`atomic_write.rs:365`), writers serialize on `.repo-write.lock`, and `list_issues`
  already holds `.index.lock` shared (`json.rs:1029`) around the whole load loop.
