#!/usr/bin/env bash
set -euo pipefail

# assemble-package — produce this repository's workflow package tree at a
# destination.
#
# The package is a directory: a manifest, the assets it declares, and a
# managed-region source. Most of those assets name a repository file as their
# target and carry that file's bytes, so the repository file is their authority
# and the tree is produced from it. What the package authors itself — its
# manifest, its install-only assets, its region source — is checked in under
# profiles/jit-dogfood, and the assembly draws each declared source from its
# owning source.
#
# WHY THIS IS NOT A BUILD STEP. No build consumes the tree it writes, so a build
# step would make every build do work no build consumes, and the directory
# watching it would need is what once relinked every test target on an unchanged
# rebuild.
# Running as an entry point also leaves the manifest one reader, the crate's own
# package model, where a build script could only have been a second one: a build
# script cannot import the crate it builds.
#
# WHY THIS IS NOT A GENERATED ARTIFACT. Its siblings scripts/generate-*.sh
# publish committed files, each guarded by a drift assertion. The tree here is
# deliberately not committed: the destination is the caller's, a run replaces it
# whole, and each run publishes a freshly staged tree, so a source the manifest
# stops declaring cannot survive into the next run. It shares those
# scripts' shape — a render in the crate, a thin script over it — and differs in
# taking a destination, because its consumer is a release job staging its own
# directory.
#
# Usage:
#   assemble-package.sh <destination>
# for example, into a disposable directory this repository already ignores:
#   assemble-package.sh target/package/jit-dogfood
# An occupied destination is replaced whole, so name a directory the run may
# take over.
#
# Exit codes:
#   0 — the tree is assembled at the destination
#   1 — the assembly or the publication failed
#   2 — a usage or environment error

die() {
  echo "${0##*/}: $*" >&2
  exit 2
}

[ "$#" -eq 1 ] || die "takes one argument, the destination (got: $*)"
command -v cargo >/dev/null 2>&1 || die "'cargo' not found on PATH"

# Resolved here so a relative destination means what the caller's shell means by
# it, whatever working directory cargo hands the renderer.
case "$1" in
  /*) destination=$1 ;;
  *) destination="$PWD/$1" ;;
esac

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
status=0
cargo run --quiet --manifest-path "$root/Cargo.toml" \
  --package jit --example assemble-package -- "$destination" || status=$?

case "$status" in
  0 | 1 | 2) exit "$status" ;;
  *) die "the assembly could not be built or run (cargo exited $status)" ;;
esac
