# Dependency build-profile evidence (`6d10e5d4`)

This directory is the durable evidence for the accepted
`[profile.dev.package."*"] opt-level = 1` in the workspace manifest, and for
rejecting `opt-level = 2` in its favour.

Three earlier arms in this epic — `efaec52f`, `7a2fc6f6` and `b883f916` — moved
**workspace** crates off opt-level 0 and were each rejected at a fixed 25%
build-regression ceiling, because a workspace crate is recompiled on every
source edit and the representative comment-touch rebuild pays for it. This arm
moves only the **dependency** graph, which that rebuild does not recompile, so
it is the one shape of the same idea whose cost lands on clean builds rather
than on the edit-rebuild loop. It is measured here against both mandatory
ceilings anyway.

## Why the suite pays for unoptimized dependencies

The suite's CPU profile is flat and almost none of it is workspace code:
SHA-256 over whole build artifacts, `winnow`/`toml_edit` parsing, and
`serde_json`. SHA-256 is the identity function of every content-addressed
fixture, cache entry and provenance record the suite publishes, and its inputs
include whole executables — one fixture consumer hashes the 137 MB `jit`
binary, which an unoptimized build hashes at roughly 55 MB/s.

## Build screen

`scripts/benchmark-rust-build.sh`, one freshly created `CARGO_TARGET_DIR` per
sample, every Cargo invocation holding the host-wide `CARGO_CI_BUILD_LOCK`. The
arms differ only in the presence and value of the manifest key under test.
Medians over each arm's own samples; the ceiling is 25% over the opt-level 0
median of the same metric.

| Metric | opt-level 0 | opt-level 1 | opt-level 2 | Ceiling |
| --- | ---: | ---: | ---: | ---: |
| Clean `clippy --all-targets` | 40.605 s | 45.607 s (+12.3%) | 50.178 s (+23.6%) | 50.756 s |
| Clean `test --no-run` | 52.344 s | 56.877 s (+8.7%) | 59.086 s (+12.9%) | 65.430 s |
| Representative rebuild | 19.083 s | 19.666 s (+3.1%) | 20.337 s (+6.6%) | 23.854 s |
| Largest recorded target directory | 6.740 GB | 6.816 GB | 6.929 GB | 10.737 GB |

Both arms clear both mandatory ceilings. The representative rebuild is the
metric the epic's traps single out, and it is the one the dependency graph
barely touches: the probe recompiles workspace crates and relinks, and no
dependency rlib is rebuilt.

## Suite runtime

Already-built warm runs of exactly the named suite clock of
`scripts/cargo-ci.sh` (`cargo nextest run --workspace` plus
`cargo test --doc --workspace`), with `CARGO_INCREMENTAL=0` and
`CARGO_CI_NO_SCCACHE=1`.

| Arm | nextest | doctest | Suite clock |
| --- | ---: | ---: | ---: |
| Pre-change | 40,740 ms | 5,339 ms | 46,079 ms |
| opt-level 2 | 18,517–18,594 ms | 4,483–4,540 ms | 23,037–23,077 ms |
| opt-level 1 | 18,078–18,160 ms | 4,621–4,656 ms | 22,699–22,816 ms |

## Decision

opt-level 1 is accepted. It is at least as fast at runtime as opt-level 2 and
costs about half its build regression, so the extra optimization buys nothing
the suite can observe. A package override sets no other profile key, so
`debug = "line-tables-only"`, `incremental`, `debug-assertions` and
`overflow-checks` are what they were, and test semantics are unchanged.

## Reproducing the checks

`summary.json` is the machine-readable authority and `raw/` holds the
unmodified harness output behind every number in it, one file per screen
invocation. Run
`python3 dev/benchmarks/dependency-profile-6d10e5d4/validate.py` to recompute
every regression, ceiling comparison, target-directory comparison and suite
bound from those files, and to check each recorded sample against the harness
output it came from.
