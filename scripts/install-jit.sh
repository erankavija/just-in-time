#!/usr/bin/env bash
set -euo pipefail

# install-jit — install the `jit` CLI with real build provenance embedded.
#
# `crates/jit/build.rs` reads NO ambient git or wall clock (jit:5d862134): an
# ordinary `cargo install --path crates/jit` therefore embeds "unknown" for the
# commit and dirty flag. That is correct for reproducible test builds, but the
# stale-binary guard (jit:7446af34) needs the INSTALLED binary to know the
# commit it was built from — otherwise it can never tell that a PATH `jit`
# predates the tree a gate is judging, and the guard silently stops firing.
#
# This wrapper resolves the current HEAD, short hash, dirty flag, and commit
# timestamp and injects them through the four release-provenance variables the
# build script honours, so the installed binary reports accurate provenance and
# the guard stays effective. SOURCE_DATE_EPOCH is the commit time (a stable,
# reproducible value tied to the commit), never the wall clock.
#
# Usage:
#   scripts/install-jit.sh [extra cargo install args]
# e.g.
#   scripts/install-jit.sh              # install crates/jit to ~/.cargo/bin
#   scripts/install-jit.sh --force      # forwarded to cargo install
#
# Runs from anywhere inside the repository; resolves the crate relative to this
# script, not the caller's working directory.

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

if ! git -C "$repo_root" rev-parse --git-dir >/dev/null 2>&1; then
  echo "install-jit: $repo_root is not a git repository; installing without provenance" >&2
  exec cargo install --path "$repo_root/crates/jit" "$@"
fi

git_hash="$(git -C "$repo_root" rev-parse HEAD)"
git_short_hash="$(git -C "$repo_root" rev-parse --short=8 HEAD)"
if [ -n "$(git -C "$repo_root" status --porcelain --untracked-files=normal)" ]; then
  git_dirty=true
else
  git_dirty=false
fi
source_date_epoch="$(git -C "$repo_root" show -s --format=%ct HEAD)"

echo "install-jit: embedding commit $git_short_hash (dirty=$git_dirty)" >&2

exec env \
  JIT_BUILD_GIT_HASH="$git_hash" \
  JIT_BUILD_GIT_SHORT_HASH="$git_short_hash" \
  JIT_BUILD_GIT_DIRTY="$git_dirty" \
  SOURCE_DATE_EPOCH="$source_date_epoch" \
  cargo install --path "$repo_root/crates/jit" "$@"
