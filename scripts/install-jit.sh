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
# The dirty flag makes the resulting binary report itself stale for its whole
# life (`assess_binary_provenance`, crates/jit/src/domain/build_provenance.rs),
# so it means "sources the binary is built from are uncommitted" and nothing
# wider. An uncommitted plan note, changelog entry or web asset feeds no build,
# and must leave an installed binary current.
#
# The paths that do feed the binary are declared once, in the inventory below,
# which crates/jit/src/domain/build_provenance.rs compiles in and matches with
# the same covering rule a quality gate's declared inputs use. This script
# restates none of them: it reads that file and hands its lines to git as
# pathspecs. A conformance test
# (`test_binary_build_inputs_select_the_same_files_as_the_installer_pathspecs`)
# holds the two matchers to selecting the same files, so neither question is
# answered by an inventory that can drift from the other.
#
# Fails closed: an unreadable or empty inventory records the tree as dirty,
# because a binary wrongly believed current is the costlier error.
build_inputs_file="$repo_root/crates/jit/src/domain/binary_build_inputs.txt"
build_input_paths=()
if [ -r "$build_inputs_file" ]; then
  while IFS= read -r line; do
    line="${line%%#*}"
    line="$(printf '%s' "$line" | tr -d '[:space:]')"
    [ -n "$line" ] && build_input_paths+=("$line")
  done <"$build_inputs_file"
fi

if [ "${#build_input_paths[@]}" -eq 0 ]; then
  echo "install-jit: cannot read build inputs from $build_inputs_file; recording the tree as dirty" >&2
  git_dirty=true
elif [ -n "$(git -C "$repo_root" status --porcelain --untracked-files=normal -- "${build_input_paths[@]}")" ]; then
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
