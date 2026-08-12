# Package-scoped test-profile benchmark

This directory is the durable evidence for `jit:efaec52f`. The candidate was
`[profile.test.package.jit] opt-level = 1`; it was rejected by the authorized
one-sample build screen before runtime sampling because its representative
comment-touch rebuild exceeded the fixed build ceiling.

`summary.json` records the method, exact threshold comparison, compiler scope,
identity accounting, byte budget, early-stop decision, and restoration.
`opt0-build-screen/` and `scoped-build-screen/` are the unmodified matched
outputs of `scripts/benchmark-rust-build.sh` at the same source revision,
including raw Cargo compiler messages and command logs.
`rejected-candidate.patch` preserves the exact candidate without leaving it
active. `SHA256SUMS` covers every other file in this directory, and
`validate.py` recomputes the decision and identity comparisons.

The original candidate inventory retains two excluded doctest-list failures
caused by a missing external `TMPDIR`. `candidate-doctest-recheck/` corrects
that evidence gap: the exact rejected manifest was reconstructed at revision
`11f09637`, a disk-backed external `TMPDIR` was created, and both workspace
doctest list commands succeeded against one fresh isolated target. Their raw
output proves that all 63 candidate identities, including ignored status,
exactly match the baseline. This discovery is not a doctest runtime sample;
full runtime sampling remains unauthorized after the hard rebuild rejection.
