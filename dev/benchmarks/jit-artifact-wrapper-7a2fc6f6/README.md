# Ordinary JIT artifact wrapper screen

This directory is the durable `no_change` evidence for `jit:7a2fc6f6`. The
candidate used a SHA-256-addressed `RUSTC_WORKSPACE_WRAPPER` to append
`-C opt-level=1` only to the ordinary `jit` library and CLI compiler units.
It was rejected by the mandatory one-sample screen before representative
rebuild or runtime sampling because the completed candidate clean test build
exceeded the fixed 25-percent regression ceiling.

[`summary.json`](summary.json) is the machine-readable decision authority.
[`compiler-audit.json`](compiler-audit.json) preserves every real workspace
compiler classification from the fresh candidate no-run build. `screen/`
contains the unmodified baseline and candidate build-harness outputs;
`raw/` contains the wrapper contract self-test, line-table, backtrace, and
early-stop session evidence. [`rejected-candidate.patch`](rejected-candidate.patch)
is the exact measured implementation and integration, which is not retained
in the repository tree.

Run `python3 dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/validate.py` to
recompute the decision, identity, compiler-audit, line-table, restoration, and
integrity assertions. `record-screen.py` documents how the host-local audit and
retained build target were reduced into durable evidence; reproducing that
collection requires applying the rejected patch in an isolated worktree.

