# Nextest worker-concurrency benchmark

This is the durable evidence for `jit:934985ca`. It compares the current
24-worker schedule with bounded 36- and 48-worker screens on one fixed,
already-built opt-level-0 target. The 64-worker arm was not run because 48
workers had already produced the shared-lock resource/order failure the issue
defines as unsafe.

The result is `no_change`. No candidate produced one qualifying run, so the
three-consecutive-run acceptance phase was not entered. `summary.json` records
the decision and reconciled metrics; `raw/` retains exact commands, nextest
JSON events, reporter output, setup boundaries, and host pressure. Run
`python3 dev/benchmarks/nextest-concurrency-934985ca/validate.py` to rederive
the identity, timing, failure, and decision invariants, and use
`sha256sum -c SHA256SUMS` from this directory to verify raw integrity.

The historical 37-second result is cited only as unmatched context. Its source
revision and setup topology differ from this experiment, so no causal claim is
made from the timing difference.
