# Validation view cost on this repository (jit:5d774061)

Wall time of the validation views and side paths on a copy of this repository, release
build, recorded as the REQ-08 evidence that plan DEC-12 names ([plan](../plan.md)). Every
figure below is copied from [`raw/validation.txt`](raw/validation.txt), the harness's
unedited output.

## Harness, build and conditions

- **Harness:** [`scripts/benchmark-mutation-cost.sh`](../../../../scripts/benchmark-mutation-cost.sh),
  validation suite, run as
  `BENCH_JIT_REPO=<worktree> BENCH_WORK_ROOT=/mnt/nvme/jit/tmp/9876-workers/bench flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock scripts/benchmark-mutation-cost.sh target/release/jit run validation`.
  Seven interleaved pairs; every other scenario is the median of five runs.
- **Commit:** `4ef3977ab2911a4a8906861e6596a100450f9069`, built and copied from the same tree.
- **Binary:** `cargo build --release -p jit` at that commit (`jit 1.0.0 (profile release)`),
  sha256 `19562138db8c3ea40a0047201720df431dc22fca34e780d4bce4cad31a882798`.
- **Fixture:** `git clone --shared` of the tree plus a `.jit` sync, on ext4, 1861 issues,
  event log 5,328,789 bytes. The fixture was deleted after the run.
- **Subjects:** `<id>` and `--explain` use epic `9876a23a`; `--scope` uses container `9876a23a`.
- **Machine:** AMD Ryzen 9 5900X, 24 logical cores, Linux 7.2.11-arch1-1, shared with other
  leads' CI; the two CI locks above kept cooperating CI runs out of the measurement.

| load threshold (1-minute) | loadavg at start | loadavg at end |
|---|---|---|
| 2.0 (harness default) | 1.98 2.48 4.56 | 2.45 2.55 4.47 |

The 1-minute load rose to 2.45 during the runs; no process isolation was applied.

## Interleaved pairs, ms

Pair *i* runs whole first when *i* is odd and per-issue first when it is even. The
difference is per-issue minus whole.

| pair | first | `jit validate` | `jit validate 9876a23a` | difference |
|---|---|---|---|---|
| 1 | whole | 3406 | 281 | −3125 |
| 2 | issue | 3413 | 286 | −3127 |
| 3 | whole | 3411 | 290 | −3121 |
| 4 | issue | 3450 | 289 | −3161 |
| 5 | whole | 3451 | 292 | −3159 |
| 6 | issue | 3442 | 295 | −3147 |
| 7 | whole | 3435 | 283 | −3152 |
| **median** | | **3435** | **289** | **−3147** |

Difference spread: −3161 to −3121 ms (range 40 ms).

**REQ-08 bound: met.** The median pair difference, −3147 ms, is at most zero.

## Other medians, ms

| command | median | runs |
|---|---|---|
| `jit validate --scope 9876a23a` | 286 | 285, 290, 286, 283, 293 |
| `jit validate 9876a23a --explain` | 280 | 280, 284, 287, 279, 276 |
| `jit validate --fix --dry-run` | 3564 | 3598, 3576, 3520, 3563, 3564 |
| `jit issue create` (human) | 126 | 143, 126, 127, 125, 123 |
| `jit issue create --json` | 125 | 125, 124, 127, 126, 122 |
| `jit issue create --quiet` | 127 | 125, 124, 129, 129, 127 |

Every timed command exited 0 and wrote nothing to stderr.
