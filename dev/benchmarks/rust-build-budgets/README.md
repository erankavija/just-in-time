# Rust build budgets: what the enforced constants rest on (jit:0708d692)

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

Raw gate output is under `raw/`.

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

The second command inherits the workspace manifest's incremental setting, as
every gate step now does: measure this budget the way the gate does or the
number is not the enforced one.

## What the tree measures

| quantity | measured | constant in force | used |
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

## The headroom rule

**Each constant is the worst warm measurement of the current tree plus 20%,
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

All three derive to the value already in force, so the post-cutover re-derivation
holds every constant. The first-run checks hold for the one that could bind:
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
consolidated tree — and the measurements behind two policies. One this document
does not re-derive and does not change: the `debug = "line-tables-only"` setting
on `[profile.dev]`/`[profile.test]`. The other, the gate's `CARGO_INCREMENTAL=0`
export, is **retired**, and the paragraph below records why. The study's
measurements stay valid as the record of what was observed then; its topology
tables describe the tree before that consolidation landed and are history.

### The gate's incremental ban, retired 2026-09-21

The ban rested on that study's measurement of 19 GiB of incremental state on a
tree of 140 integration targets and 712 test executables, and on the premise
that a gate run compiles once and exits with no later rebuild to amortize
against. Both have lapsed. Today's tree reports 10 targets and 14 executables,
and the runs that dominate the repository's build cost are repeat runs against a
warm target directory: every agent worktree run and every local re-check.

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

The workspace manifest's `[profile.dev]`/`[profile.test]` `incremental = true` is
the one declaration of the policy, and `build_profile_policy_tests.rs` asserts it.
The matched-methodology comparison produced by the same protocol,
including its fresh-target acceptance thresholds, is
[../rust-build-efficiency/report.md](../rust-build-efficiency/report.md).
