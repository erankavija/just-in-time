#!/usr/bin/env bash
# NB: NOT `set -e` — this harness inspects child exit codes on purpose.
set -uo pipefail

# docs-check-selftest — durable, re-runnable REQ-02 evidence for the three
# documentation mechanical checkers (epic 2d109173, issue 99f4a2b4).
#
# For EACH checker it proves both required behaviours:
#   (1) seed the checker's defect class → assert the checker exits NONZERO;
#   (2) revert the seed → assert the checker exits ZERO on a clean footprint.
#
# All seeds are reversible: scratch inputs live in a mktemp dir, and the
# projection seed edits tracked targets/index which a trap restores via git.
# The harness exits 0 only if every assertion passes and leaves the tree clean;
# it never commits a seed.
#
# Exit codes:
#   0 — all assertions passed
#   1 — one or more assertions failed
#   2 — environment error (missing tooling, not inside a git work tree)

command -v jit >/dev/null 2>&1 || { echo "selftest: 'jit' not on PATH" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "selftest: 'jq' not on PATH" >&2; exit 2; }
root=$(git rev-parse --show-toplevel 2>/dev/null) || {
  echo "selftest: not inside a git work tree" >&2
  exit 2
}
cd "$root" || exit 2

here=$(cd "$(dirname "$0")" && pwd)
links="$here/docs-check-links.sh"
citations="$here/docs-check-citations.sh"
projections="$here/docs-check-projections.sh"

fail=0
assert_rc() {
  local want="$1" got="$2" msg="$3"
  if [ "$got" -eq "$want" ]; then
    echo "PASS: $msg (exit $got)"
  else
    echo "FAIL: $msg (want exit $want, got $got)"
    fail=1
  fi
}

scratch=$(mktemp -d)

# Projection targets (derived live) plus a restore trap covering scratch and any
# tracked-file/index mutation the projection seed performs.
inv_target=$(jit invariant render --json | jq -r '.target')
ref_target=$(jit reference render --json | jq -r '.target')
# shellcheck disable=SC2329  # invoked indirectly via the EXIT trap below
restore() {
  # Path-scoped restore of BOTH index and working tree to HEAD (never switches
  # branches — safe under the worktree-dispatch protocol).
  git restore --staged --worktree --source=HEAD -- "$inv_target" "$ref_target" 2>/dev/null || true
  rm -rf "$scratch"
}
trap restore EXIT

echo "== M2 docs-check-links.sh =="
# Defect: an intra-repo link whose target does not exist.
printf '# Broken\n[gone](./no-such-file-zzz.md)\n' >"$scratch/links_bad.md"
"$links" "$scratch/links_bad.md" >/dev/null 2>&1
assert_rc 1 $? "links: seeded missing-target link is a finding"
# Clean: heading + a resolving intra-document anchor.
printf '# Title\n[ok](#title)\n' >"$scratch/links_clean.md"
"$links" "$scratch/links_clean.md" >/dev/null 2>&1
assert_rc 0 $? "links: clean footprint resolves"
echo

echo "== M3 docs-check-citations.sh =="
# Defect: a file citation under a misspelled/nonexistent root (proves F2 — no
# tracked-root filter suppresses it).
# shellcheck disable=SC2016  # backticks/path are literal citation text, not expansion
printf 'See `crate/jit/src/does_not_exist_zzz.rs` for details.\n' >"$scratch/cite_bad.md"
"$citations" "$scratch/cite_bad.md" >/dev/null 2>&1
assert_rc 1 $? "citations: dangling path under bogus root is MISSING"
# Clean: a real slashed path plus a placeholder that must stay suppressed.
# shellcheck disable=SC2016  # backticks/path/brace are literal citation text, not expansion
printf 'Config `crates/jit/Cargo.toml` and template `.jit/issues/{id}.json`.\n' >"$scratch/cite_clean.md"
"$citations" "$scratch/cite_clean.md" >/dev/null 2>&1
assert_rc 0 $? "citations: real path resolves, placeholder suppressed"
echo

echo "== M5 docs-check-projections.sh =="
# Stage a freshly-rendered baseline so the index matches the live registries;
# this makes a genuine clean run observable independent of any pre-existing
# drift owned by other tasks.
jit invariant render >/dev/null
jit reference render >/dev/null
git add -- "$inv_target" "$ref_target"
"$projections" >/dev/null 2>&1
assert_rc 0 $? "projections: fresh (rendered==staged) tree is clean"
# Defect: append a stray line at EOF of the invariant target, OUTSIDE the
# rendered region, so re-render preserves it and the working copy drifts.
printf '\n<!-- selftest projection drift -->\n' >>"$inv_target"
"$projections" >/dev/null 2>&1
assert_rc 1 $? "projections: drifted target region is a finding"
# restore() (trap) reverts working tree + index to HEAD.
echo

if [ "$fail" -eq 0 ]; then
  echo "SELFTEST: all assertions passed"
else
  echo "SELFTEST: assertions FAILED"
fi
exit "$fail"
