#!/usr/bin/env bash
# Format 3 window over four repositories. Stops at the first failure.
# Usage: format3-window.sh <gf2-head-sha-from-the-quiet-check> <green-sha>
# <green-sha> is a commit whose cargo-ci passed. The code at HEAD must match it.
#
# What it changes, in order:
#   1. ~/.cargo/bin/jit and ~/.cargo/bin/jit-server, built from this checkout's HEAD.
#      The old binaries are copied to $ROLLBACK first. A running jit-server keeps
#      running on its old image.
#   2. .jit/index.json and .jit/events.jsonl in each repository, by format3-rewrite.py,
#      followed by one commit of those two paths.
# It pushes nothing, stops no process, and touches no branch but the checked-out one.
# Refuses to run if a validate before the install fails, or if gf2 moved past the
# quiet check. After a failed rewrite the repository is left uncommitted, and the
# message says so. A stop part-way names what committed; a repository already on
# format 3 stays there, because the rollback binary refuses it. The rewrite takes no
# jit write lock and relies on quiet: run it only with no tracker writer in any of
# these repositories.
set -uo pipefail

[ $# -eq 2 ] || { echo "usage: $0 <gf2-head-sha-from-the-quiet-check> <green-sha>"; exit 2; }
GF2_HEAD=$1
GREEN=$2
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(git -C "$HERE" rev-parse --show-toplevel)
PROJECTS=$(dirname "$ROOT")
REWRITE=$HERE/format3-rewrite.py
REPOS=(just-in-time mealie-fi inflected-messages gf2)
ROLLBACK=$(mktemp -d "$HOME/.cache/jit-format3-rollback.XXXXXX")
LOGS=$ROLLBACK/logs
mkdir -p "$LOGS"
JIT=$HOME/.cargo/bin/jit

COMMITTED=()
die() { echo "WINDOW STOPPED: $*"; echo "committed on format 3: ${COMMITTED[*]:-none}"; exit 1; }

[ "$(git -C "$PROJECTS/gf2" rev-parse HEAD)" = "$(git -C "$PROJECTS/gf2" rev-parse "$GF2_HEAD")" ] ||
  die "gf2 main moved past the quiet check"
git -C "$ROOT" diff --quiet "$GREEN" HEAD -- crates Cargo.toml Cargo.lock ||
  die "code at HEAD differs from the green commit $GREEN"
[ "$(df --output=avail -B1G "$ROOT" | tail -1 | tr -d ' ')" -ge 60 ] || die "below the 60 GiB guard"

for repo in "${REPOS[@]}"; do
  [ -z "$(git -C "$PROJECTS/$repo" status --short -- .jit)" ] || die "$repo has uncommitted .jit paths"
  echo "== before: $repo $(git -C "$PROJECTS/$repo" rev-parse --short HEAD) $(date +%T)"
  (cd "$PROJECTS/$repo" && timeout 280 "$JIT" validate) >"$LOGS/before-$repo.out" 2>&1
  rc=$?
  tail -1 "$LOGS/before-$repo.out" | cut -c1-120
  [ "$rc" -eq 0 ] || die "$repo validate before: rc=$rc; nothing was changed"
done

[ -z "$(git -C "$ROOT" status --short | grep -v ' \.jit/')" ] || die "this checkout has non-tracker changes"
cp "$JIT" "$ROLLBACK/jit" && cp "$HOME/.cargo/bin/jit-server" "$ROLLBACK/jit-server" || die "rollback copy failed"
echo "== install from $(git -C "$ROOT" rev-parse --short HEAD), code of green $GREEN, $(date +%T); rollback in $ROLLBACK"
export CARGO_INCREMENTAL=0
cd "$ROOT" || die "cd"
flock -w 900 -E 76 -o /tmp/jit-ci-turnstile.lock flock -w 900 -E 76 -o -s /tmp/jit-full-ci.lock \
  bash -c 'cargo install --quiet --force --path crates/jit && cargo install --quiet --force --path crates/server' ||
  die "install failed, rc=$?; repositories unchanged; old binaries in $ROLLBACK"

for repo in "${REPOS[@]}"; do
  cd "$PROJECTS/$repo" || die "cd $repo"
  echo "== rewrite: $repo $(date +%T)"
  python3 -I "$REWRITE" . || die "$repo rewrite failed"
  timeout 280 "$JIT" validate >"$LOGS/after-$repo.out" 2>&1
  rc=$?
  tail -1 "$LOGS/after-$repo.out" | cut -c1-140
  [ "$rc" -eq 0 ] || die "$repo validate after: rc=$rc; rewrite left uncommitted in $repo; log $LOGS/after-$repo.out"
  duplicates=$(jq -r '.id // empty' .jit/events.jsonl | sort | uniq -d | wc -l)
  echo "duplicate event ids: $duplicates; schema_version: $(jq -r .schema_version .jit/index.json)"
  [ "$duplicates" -eq 0 ] || die "$repo has $duplicates duplicate event ids; rewrite left uncommitted"
  git add -- .jit/index.json .jit/events.jsonl || die "$repo add"
  git commit -q -m "chore: rewrite the jit store to format 3

One explicit rewrite by a script outside the product: the index holds only
deleted_ids and schema_version, retired event tags are dropped, and
released_by is null on lines recorded before the field existed.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>" -- .jit/index.json .jit/events.jsonl || die "$repo commit"
  COMMITTED+=("$repo")
  echo "$repo SHA $(git rev-parse --short HEAD)"
done
echo "WINDOW DONE $(date +%T); rollback binaries in $ROLLBACK"
