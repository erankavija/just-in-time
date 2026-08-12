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

The two arms the decision rests on, `opt0_baseline` and `opt1_candidate`, were
re-run together as a matched pair with the harness's default 3 clean + 3
rebuild samples each, at revisions `3514acf4` and `7f577e31` respectively (one
commit apart, touching only an unrelated progress-tracking file, no code and
no manifest change; see `summary.json` `.method.fixed_revision`).
`opt2_rejected_arm` retains its original 3 clean, 2 rebuild samples from a
since-merged branch revision (`14513e48d` / `4ce93b361`); it was rejected and
is carried here for context only, not as part of the accepted comparison.

| Metric | opt-level 0 | opt-level 1 | opt-level 2 | Ceiling |
| --- | ---: | ---: | ---: | ---: |
| Clean `clippy --all-targets` | 40.367 s | 45.872 s (+13.6%) | 50.178 s (+24.3%) | 50.459 s |
| Clean `test --no-run` | 51.729 s | 57.239 s (+10.6%) | 59.086 s (+14.2%) | 64.661 s |
| Representative rebuild | 18.517 s | 19.332 s (+4.4%) | 20.337 s (+9.8%) | 23.146 s |
| Largest recorded target directory | 6.740 GB | 6.816 GB | 6.929 GB | 10.737 GB |

Both arms clear both mandatory ceilings. The representative rebuild is the
metric the epic's traps single out, and it is the one the dependency graph
barely touches: the probe recompiles workspace crates and relinks, and no
dependency rlib is rebuilt.

The representative-rebuild samples are the noisiest of the three timed
metrics at this sample size: opt-level 0 recorded 17.230 s / 18.517 s /
20.640 s and opt-level 1 recorded 18.387 s / 19.332 s / 22.183 s. Even so, the
decision does not depend on averaging away that noise: opt-level 1's *worst*
individual rebuild sample (22.183 s) is still below the 23.146 s ceiling
computed from opt-level 0's median, so the metric clears the mandatory ceiling
at either arm's own extreme, not only at the median.

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
