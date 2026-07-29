# Rust build artifact and compilation efficiency design

Parent issue: `73482aa1` — Bound Rust build artifacts and compilation cost.

## Goal

Keep the complete Rust validation suite while replacing the current unbounded test-crate topology with a small number of cohesive suites, compact build profiles, intentional dependency features, and reproducible build metadata. The repository must enforce budgets automatically so future test additions cannot recreate the current artifact and linking cost.

## Baseline

Measurements taken on 2026-07-14 in the repository workspace with Cargo and Rustc 1.97.0 on `x86_64-unknown-linux-gnu`:

| Metric | Baseline |
|---|---:|
| Complete `target/` directory | 92 GiB |
| `target/debug/deps` | 72 GiB |
| `target/debug/incremental` | 19 GiB |
| Cargo integration-test targets | 140 |
| Accumulated ELF executables | 712 |
| Typical integration-test executable | 200–260 MiB |
| Representative executable debug sections | 193.7 MiB of 246.7 MiB |

The accumulated counts include stale hashes, but the clean logical topology still asks Cargo to link one executable for every top-level integration-test source file. Every such executable contains the full `jit` library and most of its dependency graph.

Recent `cargo-ci` gate runs ranged from roughly 80 to 252 seconds. The variance includes execution and host load, so compilation acceptance uses an isolated, repeated protocol rather than historical gate duration.

## Constraints

- Preserve all non-ignored and ignored tests unless a removal has an independently documented behavioral justification.
- Keep line-number backtraces useful for local failures.
- Keep targeted suite selection and enough suite-level parallelism for development.
- Do not require a non-portable linker for a correct build. A faster available linker may be an optional optimization.
- Do not make `cargo-ci` perform a second cold compilation solely to enforce budgets.
- Release provenance remains accurate and reproducible when release automation supplies source metadata.

## Test suite topology

Cargo compiles each top-level `tests/*.rs` file as a separate crate. Replace the 140-target layout with no more than 12 cohesive suite entry points. Existing case files may remain separate Rust modules beneath suite directories so ownership and focused test filtering remain clear.

Candidate suite boundaries are CLI contracts, command execution, storage and concurrency, validation and schemas, documents and items, templates and planning, Git and worktree behavior, policy and golden contracts, and the server API. Shared harness code belongs under a common module and must not be auto-discovered as a test target.

The upper bound of 12 is intentional: it removes more than 90% of link targets while retaining multiple independent compilation and execution units. The exact grouping follows dependency and execution-model cohesion rather than historical issue boundaries.

## Build profiles and gate cache policy

Development and test profiles retain line tables without emitting complete debugger payloads into every executable. Interactive development may retain incremental compilation where it materially improves edit cycles. The gate disables incremental state because its repeated, broad all-target builds are the source of most accumulated incremental directories.

The policy must be explicit in the workspace manifest and gate script. A regression checker verifies the policy rather than relying on reviewer memory.

## Dependency feature surface

The default `jsonschema` feature set enables remote file and HTTP resolution. Repository schemas contain only local fragment references (`#/types/Priority` and `#/types/State`), so the default build does not need `reqwest`, Hyper, Tokio-Rustls, or AWS-LC for schema validation.

`ureq` currently enables its default Rustls backend and the explicitly requested native-TLS backend. Remote document access selects exactly one TLS implementation. Because `git2` already requires the platform OpenSSL stack, native TLS is the initial candidate, subject to feature-compatible tests.

A dependency-feature assertion checks the resolved graph so remote schema resolution or a second TLS backend cannot return unnoticed.

## Build provenance and invalidation

The package build script watches Git metadata, including `.git/index`, and emits a wall-clock build timestamp. Staging or committing can therefore alter compiler inputs and relink every package test target despite unchanged Rust sources.

Test and ordinary validation builds use stable provenance inputs that do not change merely because the index, branch reference, or clock changes. Release automation injects commit and source-date values explicitly. Tests demonstrate that unchanged source plus a Git metadata-only operation produces no dirty Rust test units, while release metadata remains available and reproducible.

## Artifact budget checker

The committed checker derives its inputs from Cargo rather than scanning every stale file in `target/`:

1. Count integration-test targets from `cargo metadata` and fail above 12.
2. Obtain the active test executable paths from `cargo test --workspace --no-run --message-format=json` after the existing gate compilation.
3. Sum unique active executable sizes and fail above 2 GiB.
4. Assert the intended profile, incremental, and dependency-feature policies.
5. Emit a concise success summary and actionable over-budget diagnostics.

Positive fixtures cover the current compliant shape. Negative fixtures independently exceed the target-count and executable-size limits and must fail. `cargo-ci` runs the checker after its normal compilation so it reuses warm Cargo outputs.

The project invariant is named `bounded-rust-build-footprint` and states that Rust test topology, build profiles, and dependency features remain within automatically enforced artifact-count and clean-build-size budgets. The invariant is registry-owned and projected into `AGENTS.md`.

## Benchmark protocol

The benchmark is committed as a repeatable script or documented command sequence and records raw machine-readable samples plus a Markdown summary.

For both baseline and optimized revisions:

1. Record Git revision, operating system, CPU count, memory, Cargo version, Rustc version, linker, and relevant environment variables.
2. Ensure no competing Cargo build is running.
3. Use a new isolated `CARGO_TARGET_DIR` for every clean sample.
4. Run zero-warning workspace all-target clippy followed by workspace test compilation without executing tests.
5. Record wall time, maximum resident memory, target-directory bytes, integration-target count, and unique active test-executable bytes.
6. Run at least three clean samples and report the median.
7. For incremental-source samples, begin from the completed clean output, make the same reversible source-only change in the `jit` library, compile tests without running them, restore the source, and report at least three medians from equivalent prepared states.

Acceptance requires:

- At most 10 GiB for the complete fresh validation target directory.
- At most 2 GiB for unique active test executables.
- At least 40% lower median clean workspace test-compilation wall time than baseline.
- At least 60% lower median rebuild wall time after the representative library source change.

The final report explains variance and retains every raw sample; a single favorable run is not sufficient.

## Delivery sequence

The deterministic inventory and baseline protocol come first because every optimization depends on trustworthy measurements. Suite consolidation, build-profile tuning, dependency pruning, and provenance stabilization can then proceed independently where their files do not overlap. The final budget integration and benchmark depend on all optimization work, followed by the contributor-policy and invariant projection sweep.

## Risks

- One monolithic test target would reduce links but create a serial compilation bottleneck. The 12-target ceiling permits cohesive parallel suites.
- Disabling all incremental compilation could harm interactive development. The gate and local policies remain distinct and measured.
- Artifact size depends on toolchain and platform. The checker measures active executables, and the clean-directory benchmark records a pinned environment.
- Dependency feature pruning can alter remote document behavior. Existing remote-link tests and explicit TLS behavior tests must pass before accepting it.
- Provenance changes can affect `jit version` contracts. Release injection and invalidation behavior both require regression tests.

## Parent success criteria

- [hard] REQ-01: Register and project the enforced build-footprint invariant.
- [hard] REQ-02: Expose no more than 12 integration-test targets.
- [hard] REQ-03: Meet the clean target and active executable size budgets.
- [hard] REQ-04: Enforce deterministic budgets in `cargo-ci` with regression fixtures.
- [hard] REQ-05: Retain useful line backtraces while bounding debug and incremental state.
- [hard] REQ-06: Demonstrate the required clean and source-rebuild compilation improvements.
- [hard] REQ-07: Eliminate Git metadata-only test invalidation while preserving release provenance.
- [hard] REQ-08: Remove unused resolver and duplicate TLS feature stacks.
- [hard] REQ-09: Preserve the complete test inventory and pass all Rust validation variants.
- [hard] REQ-10: Document test placement, budgets, benchmarking, and diagnosis for contributors.
