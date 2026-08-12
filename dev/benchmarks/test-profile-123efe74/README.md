# Matched Rust test-profile benchmark

This directory is the durable evidence for issue `123efe74`. The machine-readable
authority is [`summary.json`](summary.json); it records the fixed source revisions,
matched method, acceptance thresholds, identity reconciliation, every summarized
sample, operational exclusions, decision matrix, and follow-up recommendation.

`opt0-build/` and `opt1-build/` contain the existing isolated build-harness outputs,
including all clean-build and comment-touch rebuild logs plus the compiler-derived
test inventories. `opt0-runtime/raw/` and `opt1-runtime/raw/` contain the full
nextest JSON-plus event streams, setup/runner stderr, outer wall clocks, and doctest
output for all measured samples. Failed pre-measurement doctest warmup attempts are
retained under `opt1-runtime/raw/` and are explicitly classified in `summary.json`;
they are not acceptance samples.

[`rejected-candidate.patch`](rejected-candidate.patch) is the exact candidate that
was measured. `SHA256SUMS` binds every evidence file other than the manifest itself;
verify it from this directory with `sha256sum --check SHA256SUMS`.

The summary is intentionally the only maintained statement of volatile measurements
and the decision. This file describes the evidence layout without copying those
values into a second prose authority.
