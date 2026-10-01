# Label-update publication cost, before and after the redo record (jit:722878d4)

Wall time of one `jit issue update <id> --label team:perf-bench` before and after
the redo-record work, on the adopter-scale repository (gf2), on this repository,
and on this repository under a ten-times event log, with the work's Rust line
balance. Every figure below is copied from a file under `raw/`.

## Harness, builds, and conditions

- **Harness:** [`dev/active/f1dc843a-redo-mutations/bench.sh`](../../active/f1dc843a-redo-mutations/bench.sh)
  at commit `b85bdf018`, run from a copy in `/dev/shm`:
  `zsh ./bench.sh <binary> base > raw/base.txt`, then the same with `tip`.
  Each scenario is three runs; the reported figure is their median.
- **Before (`base`):** `34d37f8be6ea3ab1f45153e64974f05d2b9445f2`, built in a detached
  worktree with `cargo build --release --locked -p jit`; binary sha256
  `ca0356668de217ea520e6df3b9a8cfe41b9c4096c3f2ad5e9384dc6ae14d83b2`.
- **After (`tip`):** `c0b1b4e2e6794a95184cd6cc1d9d8650b811a1d0`, built the same way;
  sha256 `65892a8f9eb537840554848ed4a5ac8266cefebbac2143dc1e2e626713a0d122`.
  The release profile is the one `scripts/install-jit.sh` (`cargo install`) uses.
- **Fixture sources:** gf2 at `6bf634098a7f`, this repository at `c0b1b4e2e679`.
- **Machine:** AMD Ryzen 9 5900X, 24 logical cores, Linux 7.2.6-arch2-1, fixtures on
  tmpfs. The harness waits for a 1-minute load below 2.0 before its runs.

| run | loadavg at start | loadavg at end |
|---|---|---|
| base | 0.87 6.70 24.21 | 0.88 6.60 24.08 |
| tip | 0.90 6.42 23.84 | 0.90 6.42 23.84 |

The 5- and 15-minute figures are decaying load from an earlier job on the shared
host; nothing ran beside the measurements.

## Fixtures

| fixture | issues | event log bytes | lease enforcement |
|---|---|---|---|
| gf2 | 1002 | 3,300,903 | `strict`, appended to the copy |
| jit | 1136 | 3,627,204 | this repository's `[worktree]` section (`strict`) |
| jit-10x | 1136 | 36,272,154 (10.00x) | as jit |

gf2 declares no `[worktree]` section, so leases are off there; the harness appends
`enforce_leases = "strict"` to its fixture copy so the lease-refused scenario exists
on every fixture. No label migration was needed: the tip accepts label updates on
gf2 with its unqualified `satisfies:` labels, so the copy is otherwise unmodified.

## Median wall time, ms

| scenario | gf2 before | gf2 after | jit before | jit after | jit-10x before | jit-10x after |
|---|---|---|---|---|---|---|
| changing label | 118 | **49** | 122 | **49** | 441 | **50** |
| idempotent label | 73 | 45 | 75 | 48 | 134 | 49 |
| lease refused | 73 | 43 | 73 | 46 | 131 | 47 |

- Changing-update median after the change is under 100 ms on both repositories.
- jit-10x against jit, after: 50 against 49 ms for the changing update (+2%), and
  `1.02x` for each scenario in the harness's ratio table. Before, the same ratio
  was `3.61x`.
- On the base build at 10x, `jit claim acquire` exits 1 (`captured 36274974 bytes
  but the maximum is 16777216`); the lease still lands, so the timed runs proceed,
  as the harness records. The tip acquires the claim.
- Every lease-refused run exits 1 with `No active lease` (`raw/*-stderr.log`); every
  changing and idempotent run exits 0.

## Rust line balance from `34d37f8be`

`git diff --shortstat 34d37f8be -- '*.rs'` at this artifact's commit:

```
118 files changed, 11728 insertions(+), 14235 deletions(-)
```

Net −2,507 lines: the work removes more Rust than it adds.

Attributed balance excluding non-redo-record work landed in the same window, from
[`rust-balance.py`](rust-balance.py) (`raw/rust-balance.txt`):

```console
$ python3 dev/benchmarks/publication-cost-722878d4/rust-balance.py 34d37f8be \
    --exclude-tasks ce053478,19e7298a,2d4fda72,48e4f65c,7226f00b,b7c99dcb,434a60b7,adb5e9df \
    --exclude-commits 1fce64b78
```

The script attributes each first-parent step's `'*.rs'` numstat to its `jit:` tag, so
a task's merge is excluded with it, and checks that the step nets sum to the
shortstat net. Excluded steps total −200; the redo-record remainder is **−2,307**
(`7226f00b` touched no Rust). The output names `b85bdf018`; later commits on this
branch change no Rust file.
