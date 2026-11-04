# Rust build budgets: what the enforced constants rest on (jit:75b46be9, jit:0708d692)

`scripts/rust-build-budget.sh` fails the `cargo-ci` gate on three quantities: the
number of integration-test targets, the bytes of active test executables, and
the measured suite clock. This document is where those three limits come from —
a measurement of the tree they are enforced against, and the rule that turns a
measurement into a limit. The limits themselves are declared once, in that
script, and this is the derivation they cite.

It replaces the derivation in
`dev/archive/6eb585bc-core-maintenance/active/73482aa1-rust-build-efficiency.md`,
whose baseline measured a build topology this tree no longer has: 140
integration-test targets and a 92 GiB target directory, against the 10 targets
and 3.1 GiB recorded at `a3c2f597a`. "Provenance" states what that record still holds.

Historical gate output is under `raw/`.

## Environment

Gate runs at `1e9d168c9` (Rust identical to `69be36193`), 2026-10-01, 1-minute load 1.35
at the first; fresh-target and target-directory figures at `a3c2f597a`, 2026-08-13, idle
host. 24 cores, 31 GiB RAM, `sccache` on PATH, `TMPDIR` on tmpfs; durations in ms.

## How each quantity is measured

Every number below is produced by the code that enforces it rather than by a
reconstruction of it. One `./scripts/cargo-ci.sh` run reports all three on its
`budget` line; the commands below isolate one quantity each.

| quantity | command |
| --- | --- |
| integration-test targets | `cargo metadata --no-deps --format-version=1 \| jq '[.packages[].targets[] \| select(.kind[] == "test")] \| length'` |
| active test-executable bytes | `cargo test --workspace --no-run --message-format=json \| jq -r 'select(.reason == "compiler-artifact" and .profile.test == true and .executable != null) \| .executable' \| sort -u \| xargs stat -c %s \| awk '{s+=$1} END {print s}'` |
| suite clock | `./scripts/cargo-ci.sh`, `suite-clock` line |

The second command inherits the workspace test-profile policy, as every gate
step does; see "Test-profile incremental state" below.

## Unpacked debug information (jit:75b46be9)

Measured on 2026-11-04, starting at `efc34b1c5338bbcb48b5f5c3cd225cf56e1d8d9f`,
with Rust 1.97.0 (`2d8144b78`, LLVM 22.1.6), `x86_64-unknown-linux-gnu`, on the
same host and worktree. The before build had no tracked changes. The after
build adds `split-debuginfo = "unpacked"` to both workspace profiles and the
policy regression test; both builds retain the existing debug level and
`[profile.test].incremental = false`.

These are the commands used, with the profile edit between the two builds.
Cargo's artifact streams identify the active executables; stale binaries in
`target/` do not enter either measurement.

```sh
export CARGO_INCREMENTAL=0 CARGO_CI_NO_SCCACHE=1
export TMPDIR=/mnt/nvme/jit/tmp/df129d9e-workers/home/vkaskivuo/Projects/just-in-time/agent-75b46be9
flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock ./scripts/cargo-ci.sh --cargo test --workspace --no-run --message-format=json > /tmp/jit-75b46be9-before-artifacts.jsonl
flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock ./scripts/rust-build-budget.sh --artifacts-json /tmp/jit-75b46be9-before-artifacts.jsonl
# Add the profile setting and its policy test, then build and measure again.
flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock ./scripts/cargo-ci.sh --cargo test --workspace --no-run --message-format=json > /tmp/jit-75b46be9-after-artifacts.jsonl
flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock ./scripts/rust-build-budget.sh --artifacts-json /tmp/jit-75b46be9-after-artifacts.jsonl
```

| build | active executables | executable bytes |
| --- | --- | --- |
| before | 14 | 1,716,674,768 |
| unpacked | 14 | 1,177,006,296 |

The reduction is `1 - 1,177,006,296 / 1,716,674,768 = 31.436850%`.
This meets the user-approved 30% reduction floor (@/issue/75b46be9/decision/D-01).
Unpacked debug information still leaves line and range tables in each linked
executable on this compiler. The representative `cli_issue` executable shrank
from 121,231,336 to 82,249,880 bytes. Section evidence comes from:

```sh
readelf -SW target/debug/deps/cli_issue-3bc741823e65d0c7
readelf -SW target/debug/deps/cli_issue-f703e0db7d2b86eb
```

| section | before bytes | unpacked bytes |
| --- | --- | --- |
| `.debug_info` | 20,384,553 | 1,911,118 |
| `.debug_str` | 26,943,049 | 2,999,607 |
| `.debug_line` | 19,238,662 | 19,181,302 |
| `.debug_ranges` | 9,784,896 | 9,788,704 |
| `.debug_addr` | absent | 3,642,696 |

The executable-byte budget now uses **at most 10% headroom, rounded down to a
whole MiB**: `floor(1,177,006,296 × 1.10 / 1,048,576) = 1,234 MiB`, or
1,293,942,784 bytes (9.935078% headroom). This replaces the historical byte
budget and its rounding rule below. The suite consolidation measurement below
supersedes this byte limit and the earlier target-count derivation; the suite-clock
budget retains its existing derivation.

### Panic backtrace reproduction

With the same environment, the existing expected-panic test ran successfully:

```sh
RUST_BACKTRACE=1 flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock ./scripts/cargo-ci.sh --cargo test --workspace --lib test_await_refusals_reports_an_unreachable_target_rather_than_waiting_out_the_bound -- --nocapture
```

Its backtrace retained workspace function names and source lines (paths below
are relative to `crates/jit`):

```text
   2: <jit::storage::contention_probe::Contenders>::await_refusals
             at ./src/storage/contention_probe.rs:209:13
   3: test_await_refusals_reports_an_unreachable_target_rather_than_waiting_out_the_bound
             at ./src/storage/contention_probe.rs:333:20
```

The test passed with its expected panic. Keep the generated `.dwo` files in the
build tree when using these executables for source-line backtraces.

## Two integration suites (jit:b22aabce)

Measured on 2026-11-04 in the same isolated worktree, starting from the unchanged
`795c63acad811ddc027502f3813d2c0b17763d92` tree with unpacked debug information.
The after tree changes test topology and its focused commands and budgets;
production Rust is unchanged. Both artifact captures use the same Rust toolchain,
`CARGO_INCREMENTAL=0`, `CARGO_CI_NO_SCCACHE=1`, the worktree's `target/` cache,
and disk-backed scratch at
`/mnt/nvme/jit/tmp/df129d9e-workers/home/vkaskivuo/Projects/just-in-time/agent-b22aabce`.
The final full `cargo-ci` validation uses RAM-backed scratch at
`/dev/shm/jit-ci/b22aabce`; build and fixture caches remain in the worktree.
These are executable-size measurements, not a comparison of suite timings.

| build | workspace integration targets | active executables | executable bytes |
| --- | ---: | ---: | ---: |
| before | 10 | 14 | 1,177,002,600 |
| two suites | 3 | 7 | 673,455,960 |

The executable reduction is **42.782118%**. The integration-target budget is the
new workspace count: two `jit` suites plus `server_integration`, with no spare
entry point. The byte limit follows the existing at-most-10%-headroom rule:
`floor(673,455,960 × 1.10 / 1,048,576) = 706 MiB`, or 740,294,656 bytes
(9.924732% headroom). The checker self-test exercises the exact target/byte
boundaries and one beyond each using its declared limits.

`cli.rs` and `contracts.rs` retain every former target as a module. Their entry
files move from `main.rs` to `mod.rs` within the same directories, preserving
fixture paths and property-test regression files. For each row, a former test
`<path>` becomes `<module>::<path>` under the named target; these are the only
test-identity changes:

| former target/module | new target | renamed tests |
| --- | --- | ---: |
| `cli_gate` | `cli` | 146 |
| `cli_issue` | `cli` | 309 |
| `cli_item_validate` | `cli` | 114 |
| `cli_query_graph` | `cli` | 133 |
| `cli_repo_workflow` | `cli` | 478 |
| `fast_docs_templates` | `contracts` | 187 |
| `fast_issue` | `contracts` | 142 |
| `fast_rules` | `contracts` | 143 |
| `scratch_build` | `contracts` | 32 |

Before and after `cargo nextest list --workspace --message-format=json`
inventories compare **exactly equal** after applying only those moves to the
before inventory. Both contain 5,038 tests: 1,684 renamed and 3,354 unchanged,
with zero missing, unexpected, ignored, or filtered tests. The normalized rows
include binary identity, full test name, and unchanged testcase metadata. Their
SHA-256 (sorted rows, JSON with sorted keys and compact separators) is
`68f2947cb00b9fc7453f05e53f056ed7b45b6a9baec590a90b6c12c1f5f10f8b`.

To capture each phase, run these commands before and after the topology edit
with `phase=before` or `phase=after`, under the environment above:

```sh
flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock ./scripts/cargo-ci.sh --cargo test --workspace --no-run --message-format=json > "$phase-artifacts.jsonl"
flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock ./scripts/cargo-ci.sh --cargo nextest list --workspace --message-format=json > "$phase-inventory.json"
flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock ./scripts/rust-build-budget.sh --artifacts-json "$phase-artifacts.jsonl"
```

The comparison normalizes no names within a former module:

```python
import hashlib, json
from pathlib import Path

moves = {name: "cli" for name in (
    "cli_gate", "cli_issue", "cli_item_validate", "cli_query_graph", "cli_repo_workflow")}
moves.update({name: "contracts" for name in (
    "fast_docs_templates", "fast_issue", "fast_rules", "scratch_build")})

def inventory(phase):
    data = json.loads(Path(f"{phase}-inventory.json").read_text())
    rows = []
    for binary, suite in data["rust-suites"].items():
        assert suite["status"] == "listed"
        module = binary.removeprefix("jit::")
        moved = phase == "before" and module in moves
        for name, case in suite["testcases"].items():
            assert not case["ignored"]
            assert case["filter-match"] == {"status": "matches"}
            rows.append([f"jit::{moves[module]}" if moved else binary,
                         f"{module}::{name}" if moved else name, case])
    assert len(rows) == data["test-count"]
    assert len({tuple(row[:2]) for row in rows}) == len(rows)
    return sorted(rows, key=lambda row: row[:2])

before, after = inventory("before"), inventory("after")
assert before == after
print(len(before), hashlib.sha256(json.dumps(
    before, sort_keys=True, separators=(",", ":")).encode()).hexdigest())
```

The normal libtest invocation `cargo test -p jit --test cli --test contracts`
also passes all 1,180 CLI and 504 contract tests with default thread parallelism,
so former targets coexist within each suite process. The existing process-state
policy test still scans all Rust sources recursively. The path audit found no
process environment or working-directory mutations, target-name nextest filters,
or fixture paths requiring relocation. CI's focused profile-adoption command now
selects the `cli` target and `cli_repo_workflow::profile_acceptance_tests::`.

## Historical gate measurements (2026-10-01)

| quantity | measured | constant then | used |
| --- | --- | --- | --- |
| integration-test targets | 10 | 12 | 83% |
| active test-executable bytes | 1,739,334,912 (1.62 GiB) across 14 executables | 2,147,483,648 (2 GiB) | 81% |
| suite clock, warm | 24,171 / 22,437 / 21,841 | 30,000 | 81% |
| suite clock, first run after clearing the fixture cache | 29,726 | 30,000 | 99% |
| suite clock, first run in a fresh target directory (`a3c2f597a`) | 26,832 | 30,000 | 89% |

The three warm figures are consecutive complete gate runs (`raw/warm-1.out` to
`raw/warm-3.out`), each passing every test and doctest; the third waited for another
build's host lock first. The fixture-cache figure is the gate run immediately before
them (`raw/first-run.out`), after `target/jit-profiled-repository-fixtures` was
removed. The fresh-target figure is the `after, cold` run of the pre-cutover suite; see
[../cold-warm-verdict-0708d692/README.md](../cold-warm-verdict-0708d692/README.md)
for what separates it from a warm one.

The same tree's target directory measures 3,313,861,767 bytes (3.1 GiB) by
`du -sb target` after these runs. That is an observation, not one of the three
enforced budgets: the fresh-validation target-directory threshold is an
acceptance criterion checked by `scripts/benchmark-rust-build.sh` once per
build-topology change, and this figure comes from a gate-warm tree rather than
that harness's clean-room protocol.

## Historical headroom rule

**Each constant was the worst warm measurement of that tree plus 20%,
rounded up to the next whole unit of its own quantity — a whole target, a whole
GiB, five seconds.** Two conditions apply to the result: it must still pass the
worst measurement observed in any condition, including a first run in a fresh
target directory; and a re-derivation may hold a constant or tighten it, never
loosen it. A derivation that points looser is an escalation, not an edit.

| constant | worst warm | +20% | rounded up to | derived | in force |
| --- | --- | --- | --- | --- | --- |
| `MAX_INTEGRATION_TARGETS` | 10 targets | 12.0 | whole target | **12** | 12 |
| `MAX_EXECUTABLE_BYTES` | 1,739,334,912 B | 2,087,201,894 B (1.94 GiB) | whole GiB | **2 GiB** | 2 GiB |
| `MAX_TEST_SUITE_SECONDS` | 24,171 ms | 29,005 ms | five seconds | **30 s** | 30 s |

All three derived to the value then in force, so the post-cutover re-derivation
held every constant. The first-run checks hold for the one that could bind:
29,726 ms and 26,832 ms are inside 30,000 ms.

### Why 20%

It is the smallest headroom that absorbs the largest movement each quantity
makes without a deliberate topology change, while still failing the regression
`@/inv/bounded-rust-build-footprint` exists to catch:

- **Targets.** Two more cohesive suite entry points. The shape the invariant
  bans — one Cargo target per test file — is dozens of targets, so it fails on
  the commit that reintroduces it instead of drifting toward the limit.
- **Bytes.** The 14 executables average 118.5 MiB, so 20% is about three more of
  them, which the rounding to 2 GiB still allows. The regression this catches
  is a per-executable debug payload restored to full debug info, which
  multiplies the total rather than nudging it.
- **Duration.** 20% of the warm clock is 4.8 s, and the suite's own first-run
  fixture construction measured about 4.3 s at `a3c2f597a`. The headroom is sized to the one
  state-dependent cost the suite carries, which is what lets a fresh target
  directory and a warm one reach the same verdict.

## Cold margin

Accepted by the owner on 2026-08-13: the fresh-target margin of 3,168 ms, of which
about 4.3 s is first-run fixture construction paid inside the clock. Moving that
construction outside the clock is a follow-up if a cold failure recurs, not v1.0
work. The post-cutover fixture-cache margin of 274 ms was accepted by the owner on
2026-10-01.

## Provenance

The archived `73482aa1` study holds the pre-optimization baseline, the original
derivation of these budgets — including the 12-target figure, chosen there as a
design ceiling for the consolidation it proposed rather than measured from a
consolidated tree — and historical build-profile measurements. The study's
measurements stay valid as the record of what was observed then; its topology
tables describe the tree before that consolidation landed and are history.

### Historical measurement of the gate's incremental ban (retired 2026-09-21)

The ban rested on that study's measurement of 19 GiB of incremental state on a
tree of 140 integration targets and 712 test executables, and on the then-current
premise that a gate run compiles once and exits with no later rebuild to amortize
against. That premise was later replaced by warm-target measurements on the
smaller tree; this comparison records the historical basis for the policy change.

Measured on this tree at commit `498bd2449`, 24 cores, isolated target
directories on NVMe, the gate's two compile steps after touching one file under
`crates/jit/src/storage/`:

| step | ban in force | incremental |
| --- | --- | --- |
| `clippy` | 29,276 ms | 16,691 ms |
| `suite-build` | 25,133 ms | 8,608 ms |
| subtotal | **54,409 ms** | **25,299 ms** |

A cold run pays nothing for it (`clippy` 43,731 ms against 44,159 ms;
`suite-build` 55,783 ms against 54,959 ms). What it costs is disk: the target
directory measures 3.3 GB without incremental state and 7.1 GB with it.

At the time of this historical comparison, both workspace profiles enabled incremental
compilation.
The matched-methodology comparison produced by the same protocol,
including its fresh-target acceptance thresholds, is
[../rust-build-efficiency/report.md](../rust-build-efficiency/report.md).

## Test-profile incremental state

The workspace `[profile.test]` sets `incremental = false`. Workspace test builds therefore
do not accumulate `target/*/incremental` state even when invoked without a
`CARGO_INCREMENTAL` override. `[profile.dev]` remains incremental for interactive
development builds. The policy test is `test_dev_profile_enables_incremental_but_test_profile_disables_it`
in `crates/jit/tests/scratch_build/build_profile_policy_tests.rs`.

REQ-02 was measured on 2026-11-04 from a clean `target` directory. With
`CARGO_CI_NO_SCCACHE=1`, a disk-backed `TMPDIR`, and the shared exclusive Cargo
locks, both commands ran with `CARGO_INCREMENTAL` unset. The commands below preserve
the former target name at measurement time; current focused commands select
`contracts` with the `scratch_build::` module prefix:

```sh
env -u CARGO_INCREMENTAL ./scripts/cargo-ci.sh --cargo test --workspace --no-run
env -u CARGO_INCREMENTAL ./scripts/cargo-ci.sh --cargo test -p jit --test scratch_build test_dev_profile_enables_incremental_but_test_profile_disables_it
python3 -c 'from pathlib import Path; print(sum(p.stat().st_size for p in Path("target").rglob("*") if p.is_file() and "incremental" in p.relative_to("target").parts))'
```

The targeted test passed (1 test), and the measurement printed `0` bytes after
both runs. The target directory did not exist before the workspace no-run build.
