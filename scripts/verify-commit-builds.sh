#!/usr/bin/env bash
set -euo pipefail

# verify-commit-builds — does a NAMED COMMIT compile, judged from that commit's
# sources in isolation from the working tree?
#
# Motivation (jit:45e1b7e8): gate checkers compile the working tree of whoever
# invokes them. When that tree carries a compensating uncommitted edit, the
# checker passes on a state that exists in no commit — so a merge can leave the
# mainline non-compiling while every gate reports passed. The observed case:
# a worker branch anchored before a module deletion, merged after it, cleanly
# re-added `pub mod claims_log;` for a file that no longer existed; a full test
# suite still reported green because it ran against a working tree that differed
# from the merge commit.
#
# This script judges the COMMIT, not the tree. It resolves the named commit's
# tracked sources with `git archive` into a throwaway directory and builds
# there. The archive is the tree of the named commit and nothing else: staged
# changes, unstaged edits, and untracked files never enter it, so no
# working-tree state can mask a broken commit. The extracted directory has no
# `.git` and no git-worktree registry entry, so the build is coupled to no other
# tree. Both workspace build scripts (crates/jit/build.rs, crates/server/build.rs)
# degrade gracefully when git metadata is absent, so the archived tree builds.
#
# NOTE (jit:45e1b7e8): resolving this by stashing or cleaning the caller's
# working tree would be a regression — agents and humans legitimately hold
# uncommitted state, and a checker that mutates the tree it is judging is worse
# than the gap it closes. This script never touches the caller's tree.
#
# "Builds" here means `cargo build --workspace` — every workspace crate compiles.
# This is compile integrity of the merged commit, distinct from the test-run
# evidence that cargo-ci records.
#
# Usage:
#   scripts/verify-commit-builds.sh [<commit-ish>]
#     commit-ish defaults to HEAD. Operates on whatever git repo the current
#     working directory belongs to.
#
# The build runs in a FRESH target directory each invocation. Sharing one target
# directory across different commits of the same package name and version is
# unsafe: cargo can reuse a previously-built artifact and report success without
# recompiling the changed sources — a false PASS of exactly the kind this script
# exists to prevent. A cold build is the price of a trustworthy verdict.
#
# Environment overrides:
#   VERIFY_COMMIT_CACHE       disk-backed base dir for the extracted sources and
#                             the ephemeral build target (default:
#                             $XDG_CACHE_HOME/jit-verify-commit). Both are removed
#                             on exit; only the base dir persists.
#   VERIFY_COMMIT_TARGET_DIR  CARGO_TARGET_DIR override. Only safe when every
#                             invocation names the SAME commit (e.g. repeatedly
#                             re-verifying one merge); reusing it across commits
#                             can mask a broken one. Left unset, a fresh target is
#                             used per run and removed afterwards.
#   VERIFY_COMMIT_NO_LOCK=1   skip the host-wide build serialization lock.
#   CARGO_CI_BUILD_LOCK       lock path shared with cargo-ci so a merge-verify
#                             and a gate build queue rather than oversubscribe.
#
# Exit codes:
#   0 — the named commit builds
#   1 — the named commit does NOT build (diagnostics printed)
#   2 — environment problem (not a git repo, bad commit-ish, or no real cargo)

# Host-wide build serialization, shared with cargo-ci: a full workspace build
# fans out to every core, so a merge-verify running alongside a gate build would
# oversubscribe the host. Re-exec under a blocking flock so the two queue.
# VERIFY_COMMIT_LOCKED guards against infinite re-exec.
if [ -z "${VERIFY_COMMIT_NO_LOCK:-}" ] && [ -z "${VERIFY_COMMIT_LOCKED:-}" ]; then
  BUILD_LOCK="${CARGO_CI_BUILD_LOCK:-${XDG_RUNTIME_DIR:-/tmp}/jit-cargo-ci.lock}"
  if command -v flock >/dev/null 2>&1; then
    exec env VERIFY_COMMIT_LOCKED=1 flock "$BUILD_LOCK" "$0" "$@"
  fi
  echo "verify-commit-builds: flock not found; running without host-wide build lock" >&2
fi

# Resolve the real cargo binary. Some local setups place a debugging shim at
# ~/.cargo/bin/cargo that exits 0 for every invocation; without this guard the
# build below would silently succeed, recording a false-positive PASS on a
# commit that does not compile. Detect a stub via the canonical version-probe,
# then fall back to a rustup toolchain binary. (Pattern from scripts/cargo-ci.sh.)
ensure_real_cargo() {
  local probe
  probe=$(cargo --version 2>&1 || true)
  if [[ "$probe" =~ ^cargo[[:space:]][0-9]+\.[0-9]+\.[0-9]+ ]]; then
    return 0
  fi
  for tc_dir in "$HOME/.rustup/toolchains"/stable-*; do
    [[ -d "$tc_dir" && -x "$tc_dir/bin/cargo" ]] || continue
    export PATH="$tc_dir/bin:$PATH"
    probe=$(cargo --version 2>&1 || true)
    if [[ "$probe" =~ ^cargo[[:space:]][0-9]+\.[0-9]+\.[0-9]+ ]]; then
      echo "verify-commit-builds: cargo on PATH was a stub; using $tc_dir/bin/cargo" >&2
      return 0
    fi
  done
  echo "ERROR: no real cargo on PATH and no usable rustup stable toolchain found." >&2
  echo "       cargo --version output: $probe" >&2
  exit 2
}

commitish="${1:-HEAD}"

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) || {
  echo "ERROR: not inside a git work tree." >&2
  exit 2
}
cd "$repo_root"

# Resolve the commit-ish to a concrete object; a bad ref is an environment error,
# not a build failure.
commit=$(git rev-parse --verify --quiet "${commitish}^{commit}") || {
  echo "ERROR: '$commitish' does not name a commit in this repository." >&2
  exit 2
}

ensure_real_cargo

cache_base="${VERIFY_COMMIT_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/jit-verify-commit}"
mkdir -p "$cache_base/tmp"

# Extract ONLY the named commit's tracked sources. git archive emits the tree of
# <commit>; nothing from the working tree, the index, or untracked files can
# enter it.
src=$(mktemp -d "$cache_base/tmp/src.XXXXXX")
build_out=$(mktemp "$cache_base/tmp/build.XXXXXX.out")
trap 'rm -rf "$src" "$build_out"' EXIT

# Fresh target per run (inside the throwaway src tree, so it is removed with it)
# unless the caller pins one. A shared target across commits can mask a broken
# build; see the header note.
target_dir="${VERIFY_COMMIT_TARGET_DIR:-$src/target}"

git archive --format=tar "$commit" | tar -x -C "$src"

short=$(git rev-parse --short=8 "$commit")

# Build the isolated sources. Deprioritize under contention so an interactive
# shell preempts the build (best-effort; a missing binary degrades to normal).
NICE_PREFIX=()
command -v nice   >/dev/null 2>&1 && NICE_PREFIX+=(nice -n 19)
command -v ionice >/dev/null 2>&1 && NICE_PREFIX+=(ionice -c2 -n7)

if (cd "$src" && CARGO_TARGET_DIR="$target_dir" "${NICE_PREFIX[@]}" \
      cargo build --workspace) >"$build_out" 2>&1; then
  echo "verify-commit-builds: commit $short builds (cargo build --workspace)"
  exit 0
fi

echo "verify-commit-builds: commit $short does NOT build (cargo build --workspace)" >&2
echo "--- build diagnostics ---" >&2
grep -E '^error' "$build_out" | head -40 >&2 || true
echo "--- tail of build output ---" >&2
tail -20 "$build_out" >&2
exit 1
