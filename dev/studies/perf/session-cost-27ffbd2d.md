# jit session-cost profile

Companion summary for `session-cost-profile.json`. This is the template shape for the epic's checked-in artifact.

## Machine and corpus

- jit 0.2.1, commit 27ffbd2d, release
- AMD Ryzen 9 5900X (24 logical CPUs), 32 GiB RAM, Linux 7.1.3-arch1-3
- Corpus: the jit repo's own `.jit/` (665 issues), copied with `cp -a`
- **fs caveat:** measured on **tmpfs**; the real repo is **ext4**. Lock-file / rename / fsync
  costs are understated here. The dominant cost below (minor page faults) is fs-independent.

## Timing (warm, n=25)

| command | class | median | p95 | locks created |
|---|---|---:|---:|---:|
| `jit --version` | baseline | 3.0 ms | 3.2 ms | 0 |
| `jit issue show <id> --json` | single read | 50.6 ms | 52.8 ms | 1 |
| `jit query available --json` | read-all | 23.4 ms | 25.0 ms | 665 |
| `jit issue list --json` | read-all | 26.9 ms | 28.9 ms | 665 |
| `jit issue update <id> --priority … --json` | mutation | **3720.6 ms** | **3747.5 ms** | 1 |

## Where the mutation cost goes (perf stat, one mutation)

- task-clock 3430 ms: **user 1.14 s, sys 2.70 s**
- **545,219 minor page faults** (~2.1 GiB touched), context-switches 0
- System-time / page-fault dominated → memory materialization, not I/O.
- context-switches 0 → no blocking git subprocess.
- A title-only mutation costs the same (~3.67 s) → it is the base two-full-capture
  materialization, not the auto-transition recursion and not lock I/O (1 lock).

This reproducibly corroborates the audit's previously single-sourced **3.747 s** figure.

## Lock mechanism

- Per-issue sidecar lock created at `crates/jit/src/storage/json.rs:918` (`load_issue`
  takes a shared lock on `<id>.lock`), via `FileLocker::open_or_create` (`lock.rs:286`, `O_CREAT`).
- Never removed: `LockGuard::drop` (`lock.rs:66`) unlocks and deletes only `.lock.meta`.
- `cleanup_stale_locks` (`lock_cleanup.rs:67`) only sweeps `.git/jit/locks`, never `.jit/issues/*.lock`.
- Result: one zero-byte `.lock` per distinct issue ever read; **692 today**, unbounded.
- Reads are already safe without it: issue JSON is published via atomic temp+rename
  (`atomic_write.rs:365`), writers serialize on `.repo-write.lock`, and `list_issues`
  already holds `.index.lock` shared (`json.rs:1029`) around the whole load loop.
