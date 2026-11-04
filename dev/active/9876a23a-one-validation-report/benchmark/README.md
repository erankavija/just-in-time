# Validation view cost on this repository (jit:4b66bc6b)

Wall time of the validation views and side paths on a copy of this repository, release
build, recorded as the REQ-08 evidence that plan DEC-12 names ([plan](../plan.md)). Every
figure below is copied from [`raw/validation.txt`](raw/validation.txt), the harness's
unedited output.

## Harness, build and conditions

- **Harness:** [`scripts/benchmark-mutation-cost.sh`](../../../../scripts/benchmark-mutation-cost.sh),
  validation suite, run as
  `BENCH_JIT_REPO=<worktree> BENCH_WORK_ROOT=/mnt/nvme/jit/tmp/9876-workers/bench flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock scripts/benchmark-mutation-cost.sh target/release/jit run validation`.
  Seven interleaved pairs; every other scenario is the median of five runs.
- **Commit:** `94c8254677174379343fed750f5191851d243f95` (main at the run), built and copied
  from the same tree.
- **Binary:** `cargo build --release -p jit` at that commit (`jit 1.0.0 (profile release)`),
  sha256 `8062b23a064cd2e26c937532adf5b943ec14784ff0849dd439d43f448d26c5c0`.
- **Fixture:** `git clone --shared` of the tree plus a `.jit` sync, on ext4, 1871 issues,
  event log 5,395,424 bytes. The fixture was deleted after the run.
- **Subjects:** `<id>` and `--explain` use epic `9876a23a`; `--scope` uses container `9876a23a`
  (the harness defaults).
- **Machine:** AMD Ryzen 9 5900X, 24 logical cores, Linux 7.2.11-arch1-1, shared with other
  leads' CI; the two CI locks above kept cooperating CI runs out of the measurement.

| load threshold (1-minute) | loadavg at start | loadavg at end |
|---|---|---|
| 2.0 (harness default) | 1.85 4.43 6.15 | 1.75 4.00 5.91 |

The harness waited five 30 s intervals (1-minute load 6.07, 4.81, 5.12, 3.57, 2.56) before the
load fell below 2.0. No process isolation was applied.

## Interleaved pairs, ms

Pair *i* runs whole first when *i* is odd and per-issue first when it is even. The
difference is per-issue minus whole.

| pair | first | `jit validate` | `jit validate 9876a23a` | difference |
|---|---|---|---|---|
| 1 | whole | 3632 | 269 | −3363 |
| 2 | issue | 3619 | 267 | −3352 |
| 3 | whole | 3637 | 268 | −3369 |
| 4 | issue | 3629 | 266 | −3363 |
| 5 | whole | 3638 | 269 | −3369 |
| 6 | issue | 3629 | 268 | −3361 |
| 7 | whole | 4026 | 268 | −3758 |
| **median** | | **3632** | **268** | **−3363** |

Difference spread: −3758 to −3352 ms (range 406 ms; pair 7's whole run, 4026 ms, is the
outlier, and −3369 to −3352 ms without it).

**REQ-08 bound: met.** The median pair difference, −3363 ms, is at most zero.

## Other medians, ms

| command | median | runs |
|---|---|---|
| `jit validate --scope 9876a23a` | 272 | 276, 267, 274, 272, 268 |
| `jit validate 9876a23a --explain` | 266 | 267, 266, 264, 265, 266 |
| `jit validate --fix --dry-run` | 3743 | 3787, 3741, 3772, 3743, 3741 |
| `jit issue create` (human) | 119 | 133, 119, 119, 122, 118 |
| `jit issue create --json` | 120 | 119, 120, 119, 123, 121 |
| `jit issue create --quiet` | 123 | 120, 124, 124, 123, 123 |

Every timed command exited 0.
