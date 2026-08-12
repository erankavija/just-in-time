# Ordinary JIT library artifact wrapper screen

This directory is the durable `no_change` evidence for `jit:b883f916`. The
candidate used a SHA-256-addressed `RUSTC_WORKSPACE_WRAPPER` to append
`-C opt-level=1` only to the ordinary non-test `jit` library. The ordinary CLI
main and every test, server, dependency, build-script, probe, and unknown
compiler shape passed through unchanged. The candidate was rejected at the
mandatory one-sample build screen: its representative library comment-touch
rebuild exceeded the fixed 25-percent regression ceiling.

[`summary.json`](summary.json) is the machine-readable decision authority.
[`compiler-audit.json`](compiler-audit.json) contains the authoritative audit
from one fresh `cargo test --workspace --no-run`: exactly one optimized library
invocation and 24 passthrough invocations. The broader screen emitted 70 audit
records across clean, inventory, setup, and rebuild phases; those are not used
to assert a per-command eligible count. `screen/` retains the matched fresh
baseline and candidate harness outputs; `raw/` contains compiler output,
wrapper self-test, line-table, backtrace, and post-screen host state.
[`rejected-candidate.patch`](rejected-candidate.patch) is the exact measured
implementation and integration and is intentionally absent from the repository
tree.

Run
`python3 dev/benchmarks/jit-library-artifact-wrapper-b883f916/validate.py` to
recompute the decision, exact identity, audit, debug, restoration, and integrity
assertions. Runtime sampling was not authorized after the hard rebuild failure.
The standalone digest check is
`cd dev/benchmarks/jit-library-artifact-wrapper-b883f916 && sha256sum -c SHA256SUMS`.
