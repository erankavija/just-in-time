#!/usr/bin/env bash
# Format 4 window, one step per invocation. Stops at the first failure.
#
# Usage:
#   format4-window.sh install <green-sha>
#       Checks <green-sha> out into a temporary worktree under target/, runs
#       ./scripts/cargo-ci.sh there under the exclusive full-CI lock, and only
#       when it passes copies ~/.cargo/bin/jit and jit-server to a rollback
#       directory and installs both from that tree. A running jit-server keeps
#       running on its old image.
#   format4-window.sh rewrite <repository> <head-sha-from-the-quiet-check>
#       Rewrites the repository's .jit and <common-dir>/jit with
#       format4-rewrite.py, validates with the installed jit, and commits the
#       tracked .jit paths in one commit on the checked-out branch. Ignored paths
#       (gf2 and research ignore .jit/gate-runs) and <common-dir>/jit stay
#       uncommitted, and nothing outside .jit is staged.
#
# Refuses a rewrite when the repository has uncommitted .jit paths or its HEAD
# moved past the quiet check. A failed validate leaves the rewrite uncommitted
# and says so; rerunning the rewrite converges, since the script is idempotent.
# It pushes nothing and stops no process. The rewrite takes no jit write lock
# and relies on quiet: run it only with no tracker writer in the repository.
# Install first: the format 3 binary refuses a rewritten repository, and the
# installed format 4 binary refuses one not yet rewritten.
set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(git -C "$HERE" rev-parse --show-toplevel)
REWRITE=$HERE/format4-rewrite.py
BIN=$HOME/.cargo/bin
COMMITTED=none

die() { echo "WINDOW STOPPED: $*"; echo "committed: $COMMITTED"; exit 1; }

install() {
  local green
  green=$(git -C "$ROOT" rev-parse --verify "$1^{commit}") || die "$1 names no commit; nothing was changed"
  [ "$(df --output=avail -B1G "$ROOT" | tail -1 | tr -d ' ')" -ge 60 ] ||
    die "below the 60 GiB guard; nothing was changed"
  local tree=$ROOT/target/format4-install-${green:0:12}
  [ ! -e "$tree" ] || die "$tree exists; remove it with git worktree remove; nothing was changed"
  git -C "$ROOT" worktree add --quiet --detach "$tree" "$green" || die "worktree add $green; nothing was changed"
  cd "$tree" || die "cd $tree"
  echo "== cargo-ci at $(git rev-parse --short HEAD) $(date +%T) in $tree"
  CARGO_INCREMENTAL=0 flock -w 900 -E 76 -x /tmp/jit-full-ci.lock ./scripts/cargo-ci.sh >"$tree.cargo-ci.out" 2>&1 ||
    die "cargo-ci at $green failed, rc=$?; log $tree.cargo-ci.out; worktree $tree kept; nothing was installed"
  tail -3 "$tree.cargo-ci.out"
  local rollback
  rollback=$(mktemp -d "$HOME/.cache/jit-format4-rollback.XXXXXX") || die "rollback directory; nothing was installed"
  { cp "$BIN/jit" "$rollback/jit" && cp "$BIN/jit-server" "$rollback/jit-server"; } ||
    die "rollback copy failed; nothing was installed"
  echo "== install from $green, $(date +%T); rollback in $rollback"
  CARGO_INCREMENTAL=0 flock -w 900 -E 76 -o /tmp/jit-ci-turnstile.lock \
    flock -w 900 -E 76 -o -s /tmp/jit-full-ci.lock \
    bash -c 'cargo install --quiet --force --locked --path crates/jit && cargo install --quiet --force --locked --path crates/server' ||
    die "install failed, rc=$?; old binaries in $rollback (restore both with cp); worktree $tree kept"
  cd "$ROOT" && git worktree remove --force "$tree" || echo "note: worktree $tree left in place"
  echo "INSTALLED $green $(date +%T); rollback binaries in $rollback; cargo-ci log $tree.cargo-ci.out"
}

rewrite() {
  local repo=$1 quiet_head=$2
  cd "$repo" || die "cd $repo"
  local name
  name=$(basename "$(git rev-parse --show-toplevel)") || die "$repo is not a git checkout"
  [ "$(git rev-parse HEAD)" = "$(git rev-parse --verify "$quiet_head^{commit}")" ] ||
    die "$name moved past the quiet check $quiet_head; nothing was changed"
  [ -z "$(git status --short -- .jit)" ] || die "$name has uncommitted .jit paths; nothing was changed"
  local logs
  logs=$(mktemp -d "$HOME/.cache/jit-format4-$name.XXXXXX") || die "log directory"

  echo "== rewrite: $name $(git rev-parse --short HEAD) $(date +%T)"
  python3 -I "$REWRITE" . ||
    die "$name rewrite stopped; a refusal changed nothing, any other failure left a partial rewrite uncommitted that a rerun converges"
  timeout 600 "$BIN/jit" validate >"$logs/validate.out" 2>&1
  local rc=$?
  tail -1 "$logs/validate.out" | cut -c1-140
  [ "$rc" -eq 0 ] || die "$name validate after: rc=$rc; rewrite left uncommitted; log $logs/validate.out"
  "$BIN/jit" claim list --json >"$logs/claims.out" 2>&1 ||
    die "$name claim list failed; rewrite left uncommitted; log $logs/claims.out"

  git add -A -- .jit || die "$name add; rewrite left uncommitted"
  if git diff --cached --quiet -- .jit; then
    echo "$name: no tracked .jit path changed"
    return
  fi
  git commit -q -m "chore: rewrite the jit store to format 4

One explicit rewrite by a script outside the product: gate runs live under
their issue, gate states name their latest run, reused verdicts name their
source issue, verdict records and archive evidence are seeded, and
profile-lifecycle lines carry no torn-tail marker.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>" -- .jit || die "$name commit; rewrite left uncommitted"
  COMMITTED="$name $(git rev-parse --short HEAD)"
  echo "WINDOW STEP DONE $(date +%T): $COMMITTED; logs in $logs"
}

case "${1:-}" in
  install) [ $# -eq 2 ] || die "usage: $0 install <green-sha>"; install "$2" ;;
  rewrite) [ $# -eq 3 ] || die "usage: $0 rewrite <repository> <head-sha-from-the-quiet-check>"; rewrite "$2" "$3" ;;
  *) echo "usage: $0 install <green-sha> | rewrite <repository> <head-sha-from-the-quiet-check>"; exit 2 ;;
esac
