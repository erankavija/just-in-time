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
# WHY CURRENCY IS ESTABLISHED FIRST. The classification comes from the
# installed binary, so a binary predating the repository under check produces
# an authoritative-looking but outdated table — and the generator run here
# reads that same binary, so it agrees with a region that binary wrote. That
# agreement is neither a documentation finding nor a pass: the comparison
# cannot be made at all, which is an environment failure. The repository's
# stale-binary guard does not establish the difference on its own, because it
# identifies a repository by resolving a revision in it and stays equally
# silent for a binary with no injected provenance, an unresolvable head, and a
# build commit outside this repository's history. So currency is established
# positively here, against the repository being checked, before anything is
# compared, and anything short of a resolved, current provenance is an
# environment failure (`@/issue/e204e63d/decision/D-4`).
#
# Usage:
#   docs-check-shipped-policy.sh   (takes no footprint — targets are configured)
#
# Exit codes:
#   0 — the generated regions carry the shipped classification
#   1 — a generated region drifted
#   2 — environment error (missing tooling, no git work tree, a classification
#       that cannot be trusted, or a generator that could not run)

me=$(basename "$0")
die() {
  echo "$me: $*" >&2
  exit 2
}

[ "$#" -eq 0 ] || die "takes no arguments (got: $*)"

for tool in git jit jq; do
  command -v "$tool" >/dev/null 2>&1 || die "'$tool' not found on PATH"
done

root=$(git rev-parse --show-toplevel 2>/dev/null) ||
  die "not inside a git work tree (needed to derive the side under check)"

generator="scripts/generate-shipped-policy-regions.sh"
[ -x "$root/$generator" ] || die "$generator is missing or not executable in $root"

# --- the classification's trustworthiness, judged against the repo under check

version_json=$(jit version --json 2>/dev/null) || die "'jit version --json' failed"
build_commit=$(printf '%s' "$version_json" | jq -r '.git_commit // ""')
case "$build_commit" in
  "" | unknown)
    die "cannot compare: the jit binary on PATH reports no build commit, so the classification it carries cannot be placed in $root — install it with ./scripts/install-jit.sh, which injects build provenance"
    ;;
esac

head=$(git -C "$root" rev-parse --verify --quiet HEAD) ||
  die "cannot compare: $root has no resolvable HEAD to compare the binary against"

# The engine's own identity predicate: whether the build commit is a commit
# object this repository contains (crates/jit/src/domain/build_provenance.rs).
git -C "$root" rev-parse --verify --quiet "$build_commit^{commit}" >/dev/null ||
  die "cannot compare: the jit binary was built from $build_commit, which is not a commit in $root — its classification describes some other tree"

# The binary's own verdict on whether it predates this repository's sources.
# `JIT_GATE_RUN` puts it in gate-child mode, where it self-checks against the
# repository it is invoked in and refuses rather than serving output. Asking it
# keeps the build-input inventory in the one place that declares it.
if ! probe=$( { cd "$root" && JIT_GATE_RUN=1 jit version --json >/dev/null; } 2>&1 ); then
  die "cannot compare: the jit binary reports itself stale against $root, so the classification it produces predates the regions under check — reinstall it with ./scripts/install-jit.sh and run this again. It reported: ${probe:-(no output)}"
fi

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
  git -C "$fixture" --no-pager diff "$before" "$after"
} >&2
exit 1
