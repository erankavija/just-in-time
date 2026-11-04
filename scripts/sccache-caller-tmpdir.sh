#!/usr/bin/env bash
# RUSTC_WRAPPER for cargo-ci.sh: sccache, with the caller's TMPDIR. A server
# this starts or restarts mid-run must outlive the run's own TMPDIR.
set -euo pipefail
export TMPDIR="$CARGO_CI_CALLER_TMPDIR"
exec sccache "$@"
