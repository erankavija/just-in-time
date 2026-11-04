#!/usr/bin/env bash
# RUSTC_WRAPPER for cargo-ci.sh: the caller's wrapper, or sccache when the gate
# selected it, with the caller's TMPDIR. A daemon the wrapper starts or restarts
# mid-run must outlive the run's own TMPDIR.
set -euo pipefail
export TMPDIR="$CARGO_CI_CALLER_TMPDIR"
exec "${CARGO_CI_CALLER_RUSTC_WRAPPER:-sccache}" "$@"
