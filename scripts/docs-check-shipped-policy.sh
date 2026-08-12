#!/usr/bin/env bash
set -euo pipefail

# docs-check-shipped-policy — shipped-policy region freshness.
# Mechanical check M7 of the `docs-mechanical` gate.
#
# The adopter configuration documents carry the shipped development-area
# classification in generated regions. Generating them does not keep them
# current: a change to the shipped classification leaves the committed regions
# correct-looking and wrong until someone reruns the generator. This check
# reruns it against a scratch copy of the tree and reports a region that no
# longer matches what repository initialization produces.
#
# BOTH SIDES ARE DERIVED LIVE. One side is the working tree; the other is what
# `scripts/generate-shipped-policy-regions.sh` produces from a throwaway
# `jit init`. This script therefore states no classification value, no region
# marker, and no target path — the generator owns all three, and the drift
# reported here is the difference between the fixture's tree object before and
# after that generator ran. A generator that gains or moves a target is covered
# without editing this file.
#
# WHY A FIXTURE. The generator writes its regions in place, and a check leaves
# the tree it inspects alone. The fixture is a shared clone of the repository
# under check, moved to that repository's HEAD and then carrying its
# uncommitted tracked changes, so the comparison is against the working tree
# while every write lands under a temporary directory.
#
# Usage:
#   docs-check-shipped-policy.sh   (takes no footprint — targets are configured)
#
# Exit codes:
#   0 — the generated regions carry the shipped classification
#   1 — a generated region drifted
#   2 — environment error (missing tooling, no git work tree, or a generator
#       that could not run)

# Expanded rather than shelled out for, so nothing can fail before `die` exists
# to report it.
me=${0##*/}
die() {
  echo "$me: $*" >&2
  exit 2
}

[ "$#" -eq 0 ] || die "takes no arguments (got: $*)"

for tool in git jit; do
  command -v "$tool" >/dev/null 2>&1 || die "'$tool' not found on PATH"
done

root=$(git rev-parse --show-toplevel 2>/dev/null) ||
  die "not inside a git work tree (needed to derive the side under check)"

generator="scripts/generate-shipped-policy-regions.sh"
[ -x "$root/$generator" ] || die "$generator is missing or not executable in $root"

head=$(git -C "$root" rev-parse --verify --quiet HEAD) ||
  die "cannot compare: $root has no resolvable HEAD to place the fixture at"

# --- the fixture: this repository's working tree, somewhere writable ----------

tmp=$(mktemp -d) || die "could not create a temporary directory"
trap 'rm -rf "$tmp"' EXIT

fixture="$tmp/fixture"
git clone --shared --no-checkout --quiet "$root" "$fixture" 2>/dev/null ||
  die "could not clone $root into a scratch fixture"
git -C "$fixture" checkout --quiet --detach "$head" ||
  die "could not place the scratch fixture at $head"

patch="$tmp/worktree.patch"
git -C "$root" diff --binary HEAD >"$patch" ||
  die "could not read $root's uncommitted changes"
if [ -s "$patch" ]; then
  git -C "$fixture" apply --binary --whitespace=nowarn "$patch" ||
    die "could not carry $root's uncommitted changes into the scratch fixture"
fi

# --- regenerate, and let git say whether anything moved -----------------------

git -C "$fixture" add -A || die "could not stage the scratch fixture"
before=$(git -C "$fixture" write-tree) || die "could not record the fixture's state"

if ! report=$( (cd "$fixture" && "./$generator") 2>&1 ); then
  die "the generator could not produce the shipped classification: ${report:-(no output)}"
fi

git -C "$fixture" add -A || die "could not stage the regenerated fixture"
after=$(git -C "$fixture" write-tree) || die "could not record the regenerated fixture's state"

if [ "$before" = "$after" ]; then
  echo "OK: shipped-policy regions carry the shipped classification"
  exit 0
fi

{
  echo "DRIFT: a generated shipped-policy region no longer carries what repository"
  echo "       initialization produces. Rerun ./$generator and commit:"
  # The verdict is already settled by the two tree objects. Rendering the
  # difference is explanation, so its failure must not carry a status of its
  # own over the finding.
  git -C "$fixture" --no-pager diff "$before" "$after" ||
    echo "       (git could not render the difference)"
} >&2
exit 1
