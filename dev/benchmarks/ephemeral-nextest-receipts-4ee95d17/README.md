# Ephemeral nextest receipt publication evidence

This directory records the quantitative decision for `jit:4ee95d17`. The
candidate retains the four independent setup recipes and their committed
filters. Each accepted sample selects one existing consumer from each recipe,
uses the shared warm target, and sums the four `SETUP PASS` durations reported
inside nextest's clock.

The matched before-source reference is 10,615 ms. Acceptance required every
one of three candidate samples to report all four distinct recipes, total at
most 5,500 ms, and save at least 5,115 ms. The candidate reported 10,556,
10,533, and 10,507 ms, saving only 59, 82, and 108 ms. The reviewed decision is
therefore `no_change`, and the Rust candidate was reverted.

`rejected-candidate.patch.gz` preserves the exact candidate. `validate.py`
reapplies it to a clean source clone, formats it, verifies its
canonical diff identity, rederives the failed bounds from the raw nextest
stderr logs, and checks their digests plus the benchmark harness, committed
nextest configuration, selected consumers, and decision identities. The
initial isolated-target attempt exhausted `/tmp` capacity before setup and is
explicitly excluded; `raw/excluded-isolated-target.stderr` retains that
failure. `raw/candidate-binaries.txt` binds the executable hashes, marker
observations, and mtimes that precede the first captured sample.

Run `python3 dev/benchmarks/ephemeral-nextest-receipts-4ee95d17/validate.py`
to validate the completed evidence.
