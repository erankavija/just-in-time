#!/usr/bin/env bash
# NB: NOT `set -e` — this harness inspects child exit codes on purpose.
set -uo pipefail

# verify-commit-builds-selftest — regression evidence for jit:45e1b7e8 REQ-03.
#
# Constructs the resurrection case in a scratch git repo and asserts the two
# facts that motivate scripts/verify-commit-builds.sh:
#
#   (A) the merged commit does NOT build, and verify-commit-builds.sh reports
#       that failure by judging the commit's sources in isolation;
#   (B) the post-wave leak check (check-leak-into-main.sh) reports SUCCESS on the
#       exact same merged tree — it compares the working tree against a
#       pre-dispatch snapshot and is structurally blind to a commit that does
#       not compile.
#
# The scenario reproduces the observed incident: a worker branch anchored BEFORE
# a module deletion, merged AFTER it. Base has an undeclared source file (cargo
# ignores a .rs file with no `mod` decl, so base builds). Main deletes the dead
# file. The worker, anchored at base, starts declaring the module (`mod
# claims_log;`). Git sees a file removed on one side and an added declaration
# line on the other, finds no textual overlap, and merges without a conflict —
# the merge commit declares a module whose file no longer exists and fails with
# `error[E0583]: file not found for module`.
#
# The self-test NEVER mutates the real repository: everything happens inside a
# mktemp scratch git repo, and the isolated build cache is redirected under that
# scratch dir.
#
# Exit codes:
#   0 — all assertions passed
#   1 — one or more assertions failed
#   2 — environment error (missing tooling, not inside a git work tree)

command -v git >/dev/null 2>&1 || { echo "selftest: 'git' not on PATH" >&2; exit 2; }

here=$(cd "$(dirname "$0")" && pwd)
verify="$here/verify-commit-builds.sh"
[ -x "$verify" ] || { echo "selftest: $verify not found or not executable" >&2; exit 2; }

# The leak check lives with the jit-execution-lead skill. Locate it from the real
# repo this script belongs to (before we cd into the scratch repo).
real_root=$(git -C "$here" rev-parse --show-toplevel 2>/dev/null) || {
  echo "selftest: not inside a git work tree" >&2
  exit 2
}
leak="$real_root/.agents/skills/jit-execution-lead/scripts/check-leak-into-main.sh"
[ -x "$leak" ] || { echo "selftest: leak check $leak not found or not executable" >&2; exit 2; }

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
# shellcheck disable=SC2329  # invoked indirectly via the EXIT trap below
cleanup() { rm -rf "$scratch"; }
trap cleanup EXIT

repo="$scratch/repo"
mkdir -p "$repo/src"
(
  cd "$repo" || exit 3
  git init -q
  git config user.email selftest@jit
  git config user.name selftest

  # Base: a minimal, dependency-free cargo binary. claims_log.rs exists but is
  # UNDECLARED, so cargo ignores it and the base builds.
  cat > Cargo.toml <<'EOF'
[package]
name = "resurrection-demo"
version = "0.0.0"
edition = "2021"

[[bin]]
name = "resurrection-demo"
path = "src/main.rs"
EOF
  cat > src/main.rs <<'EOF'
mod alpha;
mod omega;

fn main() {
    alpha::a();
    omega::o();
}
EOF
  printf 'pub fn a() {}\n' > src/alpha.rs
  printf 'pub fn o() {}\n' > src/omega.rs
  printf 'pub fn c() {}\n' > src/claims_log.rs
  git add -A
  git commit -qm "base: undeclared claims_log.rs, builds"

  git branch worker

  # Main: delete the dead, undeclared module file. Main still builds.
  git rm -q src/claims_log.rs
  git commit -qm "main: delete dead claims_log.rs"

  # Worker (anchored at base): begins declaring the module. Worker builds — the
  # file is still present on this branch.
  git checkout -q worker
  cat > src/main.rs <<'EOF'
mod alpha;
mod omega;
mod claims_log;

fn main() {
    alpha::a();
    omega::o();
}
EOF
  git add -A
  git commit -qm "worker: declare claims_log module"

  # Merge worker into main. Clean textual merge; the merge commit declares a
  # module whose file was deleted on main.
  git checkout -q main
  git merge --no-ff -q worker -m "merge worker into main" || {
    echo "selftest: expected a CLEAN merge but git reported a conflict" >&2
    exit 3
  }
) || exit 2

merge_commit=$(git -C "$repo" rev-parse HEAD)
base_commit=$(git -C "$repo" rev-parse main^)   # main's pre-merge tip = the good "delete" commit

echo "== (A) verify-commit-builds judges the COMMIT in isolation =="
# Isolate the build cache under the scratch dir so the real user cache is never
# touched, and disable the shared host lock (irrelevant for a scratch build).
export VERIFY_COMMIT_CACHE="$scratch/verify-cache"
export VERIFY_COMMIT_NO_LOCK=1

(cd "$repo" && "$verify" "$base_commit") >/dev/null 2>&1
assert_rc 0 $? "verify-commit-builds reports the good pre-merge commit builds"

(cd "$repo" && "$verify" "$merge_commit") >/dev/null 2>&1
assert_rc 1 $? "verify-commit-builds reports the resurrection merge does NOT build"

# Sanity: the failure is judged from the commit, not from any working-tree edit.
# Overwrite the working tree with a COMPENSATING fix (drop the stray decl) but do
# NOT commit it. verify-commit-builds must still report the commit as broken,
# because the archive it builds from ignores the uncommitted edit.
cat > "$repo/src/main.rs" <<'EOF'
mod alpha;
mod omega;

fn main() {
    alpha::a();
    omega::o();
}
EOF
(cd "$repo" && "$verify" "$merge_commit") >/dev/null 2>&1
assert_rc 1 $? "an uncommitted compensating edit does not mask the broken commit"

echo
echo "== (B) the leak check is blind to the broken merge commit =="
# Restore a clean checkout of the merge commit so the working tree matches HEAD.
git -C "$repo" checkout -q -- .
# The pre-dispatch snapshot the leak check compares against is the clean merged
# state: `git status -uall --porcelain` is empty after a committed clean merge.
snapshot="$scratch/pre-dispatch-snapshot.txt"
git -C "$repo" status -uall --porcelain > "$snapshot"
(cd "$repo" && "$leak" "$snapshot") >/dev/null 2>&1
assert_rc 0 $? "check-leak-into-main reports SUCCESS on the same broken-merge tree"

echo
if [ "$fail" -eq 0 ]; then
  echo "SELFTEST: all assertions passed"
else
  echo "SELFTEST: assertions FAILED"
fi
exit "$fail"
