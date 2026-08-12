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
active. `SHA256SUMS` covers every other file in this directory.

The raw doctest-list attempt records an operational fixture error caused by a
missing external `TMPDIR`. It is not presented as a successful candidate
doctest run. Full doctest/runtime sampling was not required after the hard
rebuild rejection; the unchanged exact doctest identity set comes from the
fresh same-revision opt-level-0 arm as described in `summary.json`.
