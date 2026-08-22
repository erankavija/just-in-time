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
and 3.1 GiB recorded below. "Provenance" states what that record still holds.

Raw gate output is under `raw/`.

## Environment

Commit `a3c2f597a` on `worktree-agent-0708d692`, 2026-08-13. 24 cores, 31 GiB
RAM, SATA disk, `sccache` on PATH, `TMPDIR` at the host default (`/tmp`, tmpfs),
idle host, nothing else compiling. All durations in milliseconds.

## How each quantity is measured

Every number below is produced by the code that enforces it rather than by a
reconstruction of it. One `./scripts/cargo-ci.sh` run reports all three on its
`budget` line; the commands below isolate one quantity each.

| quantity | command |
| --- | --- |
| integration-test targets | `cargo metadata --no-deps --format-version=1 \| jq '[.packages[].targets[] \| select(.kind[] == "test")] \| length'` |
| active test-executable bytes | `CARGO_INCREMENTAL=0 cargo test --workspace --no-run --message-format=json \| jq -r 'select(.reason == "compiler-artifact" and .profile.test == true and .executable != null) \| .executable' \| sort -u \| xargs stat -c %s \| awk '{s+=$1} END {print s}'` |
| suite clock | `./scripts/cargo-ci.sh`, `suite-clock` line |

`CARGO_INCREMENTAL=0` in the second command is load-bearing. `scripts/cargo-ci.sh`
exports it for every gate step, and the same tree relinked incrementally
measured 1,600,239,368 bytes against the gate's 1,596,001,608: measure this
budget the way the gate does or the number is not the enforced one.

## What the tree measures

| quantity | measured | constant in force | used |
| --- | --- | --- | --- |
| integration-test targets | 10 | 12 | 83% |
| active test-executable bytes | 1,596,001,608 (1.49 GiB) across 14 executables | 2,147,483,648 (2 GiB) | 74% |
| suite clock, warm | 22,560 / 22,571 / 22,581 | 30,000 | 75% |
| suite clock, first run in a fresh target directory | 26,832 | 30,000 | 89% |

The three warm figures are consecutive complete gate runs (`raw/warm-1.out` to
`raw/warm-3.out`), each reporting 4,514 tests passed, 0 failed, 7 skipped and 63
doctests. The fresh-target figure is the `after, cold` run recorded on this
branch over the same 4,514-test suite; see
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
| `MAX_EXECUTABLE_BYTES` | 1,596,001,608 B | 1,915,201,930 B (1.78 GiB) | whole GiB | **2 GiB** | 2 GiB |
| `MAX_TEST_SUITE_SECONDS` | 22,581 ms | 27,097 ms | five seconds | **30 s** | 30 s |

All three derive to the value already in force, so this re-derivation changes no
constant. The fresh-target check holds for the one that could bind: 26,832 ms is
inside 30,000 ms.

### Why 20%

It is the smallest headroom that absorbs the largest movement each quantity
makes without a deliberate topology change, while still failing the regression
`@/inv/bounded-rust-build-footprint` exists to catch:

- **Targets.** Two more cohesive suite entry points. The shape the invariant
  bans — one Cargo target per test file — is dozens of targets, so it fails on
  the commit that reintroduces it instead of drifting toward the limit.
- **Bytes.** The 14 executables average 108.7 MiB, so 20% is about three more of
  them and the rounding to 2 GiB allows about five. The regression this catches
  is a per-executable debug payload restored to full debug info, which
  multiplies the total rather than nudging it.
- **Duration.** 20% of the warm clock is 4.5 s, and the suite's own first-run
  fixture construction measures about 4.3 s. The headroom is sized to the one
  state-dependent cost the suite carries, which is what lets a fresh target
  directory and a warm one reach the same verdict.

## Cold margin

Accepted by the owner on 2026-08-13: the first run in a fresh target directory
measures 26,832 ms against the 30,000 ms budget, a margin of 3,168 ms, of which
about 4.3 s is the suite's own first-run fixture construction paid inside the
measured clock. Moving that construction outside the clock is a follow-up if a
cold failure recurs, not v1.0 work.

## Provenance

The archived `73482aa1` study holds the pre-optimization baseline, the original
derivation of these budgets — including the 12-target figure, chosen there as a
design ceiling for the consolidation it proposed rather than measured from a
consolidated tree — and the measurements behind two policies this document does
not re-derive and does not change: the `debug = "line-tables-only"` setting on
`[profile.dev]`/`[profile.test]`, and the gate's `CARGO_INCREMENTAL=0` export.
Those measurements stay valid as the record of what was observed then; the
study's topology tables describe the tree before that consolidation landed and
are history. The matched-methodology comparison produced by the same protocol,
including its fresh-target acceptance thresholds, is
[../rust-build-efficiency/report.md](../rust-build-efficiency/report.md).
