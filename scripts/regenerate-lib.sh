#!/usr/bin/env bash

# regenerate-lib — the shared body of the generate-<artifact>.sh entry points
# whose artifact this crate renders in process.
#
# Sourced rather than run: each entry point states its own target, its
# authority, and its exit codes in its own header, then calls the two functions
# here. The render itself is declared in crates/jit/src/generated_artifacts.rs,
# which the artifact's drift assertion reads too, and the `regenerate` example
# supplies the I/O around it.
#
# Every entry point reports the same way: one `updated: <path>` line per file it
# rewrote, then one `OK: …` summary. Exit 0 means the artifact holds its
# rendered values, 1 that the render or the publication failed, and 2 that the
# run was refused or the environment could not support it.

# Report an environment or usage problem and stop, in the entry point's name.
regenerate_die() {
  echo "$(basename "$0"): $*" >&2
  exit 2
}

# Refuse arguments: every entry point names its own artifact.
regenerate_require_no_arguments() {
  [ "$#" -eq 0 ] || regenerate_die "takes no arguments (got: $*)"
}

# Render and publish the named artifact, passing the renderer's own report and
# exit status through. A cargo failure — the renderer could not be built or run
# at all — is an environment error rather than a failed render.
regenerate_artifact() {
  command -v cargo >/dev/null 2>&1 || regenerate_die "'cargo' not found on PATH"

  local root status=0
  root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
  cargo run --quiet --manifest-path "$root/Cargo.toml" \
    --package jit --example regenerate -- "$1" || status=$?

  case "$status" in
    0 | 1 | 2) return "$status" ;;
    *) regenerate_die "the renderer could not be built or run (cargo exited $status)" ;;
  esac
}
