#!/usr/bin/env bash
# NB: NOT `set -e` — this harness inspects child exit codes on purpose.
set -uo pipefail

# docs-check-selftest — durable, re-runnable regression evidence for the
# documentation mechanical checkers and their orchestrator.
#
# For EACH checker it proves both required behaviours:
#   (1) seed the checker's defect class → assert the checker exits NONZERO;
#   (2) revert the seed → assert the checker exits ZERO on a clean footprint.
# It also uses an isolated fixture to prove the orchestrator's bare default is
# the adopter docs surface and that DOCS_FOOTPRINT overrides that default.
#
# The self-test NEVER mutates the real repository index or working tree: the
# link/citation seeds are scratch files in a mktemp dir, and the projection
# check — which necessarily renders into tracked targets — runs inside a
# throwaway `git clone` of this repo under that mktemp dir. A pre-existing
# staged or unstaged change in the real repo is therefore left untouched. The
# harness exits 0 only if every assertion passes; it never commits a seed.
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
mechanical="$here/docs-mechanical.sh"

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

assert_logged_footprint() {
  local checker="$1" want="$2" log="$3" msg="$4"
  if grep -qxF "$checker:$want" "$log"; then
    echo "PASS: $msg"
  else
    echo "FAIL: $msg (missing $checker:$want)"
    fail=1
  fi
}

scratch=$(mktemp -d)
# The only cleanup needed: remove the scratch dir. Nothing in the real repo is
# ever mutated, so there is no repository state to restore.
# shellcheck disable=SC2329  # invoked indirectly via the EXIT trap below
cleanup() { rm -rf "$scratch"; }
trap cleanup EXIT

echo "== docs-mechanical.sh footprint resolution =="
# The orchestrator discovers its children beside itself, so a copied entrypoint
# plus recording child stubs exercises its real resolution logic without
# coupling this assertion to the repository's live documentation or gate env.
fixture="$scratch/orchestrator"
fixture_scripts="$fixture/scripts"
fixture_log="$scratch/orchestrator.log"
mkdir -p "$fixture_scripts"
cp "$mechanical" "$fixture_scripts/docs-mechanical.sh"
for checker in docs-check-links.sh docs-check-citations.sh docs-check-projections.sh; do
  # shellcheck disable=SC2016  # $0/$* and the log variable expand in the stub.
  printf '#!/usr/bin/env bash\nprintf "%%s:%%s\\n" "$(basename "$0")" "$*" >>"$DOCS_MECHANICAL_LOG"\n' >"$fixture_scripts/$checker"
  chmod +x "$fixture_scripts/$checker"
done

# No caller override: the entrypoint must select docs, not the archival dev/
# permanent paths it used before this regression fix.
(
  cd "$fixture" || exit 3
  DOCS_MECHANICAL_LOG="$fixture_log" "$fixture_scripts/docs-mechanical.sh" >/dev/null 2>&1
)
assert_rc 0 $? "orchestrator: bare invocation succeeds in isolated fixture"
assert_logged_footprint "docs-check-links.sh" "docs" "$fixture_log" "orchestrator: bare invocation selects adopter docs, not archival paths"
assert_logged_footprint "docs-check-citations.sh" "docs" "$fixture_log" "orchestrator: bare invocation passes adopter docs to both footprint checkers"

# An explicit environment footprint remains higher precedence than the default.
: >"$fixture_log"
(
  cd "$fixture" || exit 3
  DOCS_MECHANICAL_LOG="$fixture_log" DOCS_FOOTPRINT="fixture-explicit" "$fixture_scripts/docs-mechanical.sh" >/dev/null 2>&1
)
assert_rc 0 $? "orchestrator: explicit DOCS_FOOTPRINT invocation succeeds"
assert_logged_footprint "docs-check-links.sh" "fixture-explicit" "$fixture_log" "orchestrator: DOCS_FOOTPRINT overrides the bare default"
assert_logged_footprint "docs-check-citations.sh" "fixture-explicit" "$fixture_log" "orchestrator: explicit footprint reaches both footprint checkers"
echo

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
# Defect: a repo-rooted citation (first segment is a tracked top-level entry)
# that does not exist, extensionless on purpose — proves the extension gate is
# gone and a genuine dangling repo path is caught.
# Construct the deliberately absent path separately so this self-test source is
# not itself reported as a dangling citation when the scripts tree is scanned.
missing_path='crates/jit/does_not_exist_zzz'
# shellcheck disable=SC2016  # Markdown backticks are literal fixture content.
printf 'See `%s` for details.\n' "$missing_path" >"$scratch/cite_bad.md"
"$citations" "$scratch/cite_bad.md" >/dev/null 2>&1
assert_rc 1 $? "citations: dangling repo-rooted path is MISSING"
# Clean: a real slashed path plus a placeholder that must stay suppressed.
# shellcheck disable=SC2016  # backticks/path/brace are literal citation text, not expansion
printf 'Config `crates/jit/Cargo.toml` and template `.jit/issues/{id}.json`.\n' >"$scratch/cite_clean.md"
"$citations" "$scratch/cite_clean.md" >/dev/null 2>&1
assert_rc 0 $? "citations: real path resolves, placeholder suppressed"
echo

echo "== footprint error handling (env errors, never a false-green pass) =="
# Nonexistent footprint path → exit 2 for both footprint-taking checkers.
"$links" "$scratch/no_such_path_zzz" >/dev/null 2>&1
assert_rc 2 $? "links: nonexistent footprint path is an env error"
"$citations" "$scratch/no_such_path_zzz" >/dev/null 2>&1
assert_rc 2 $? "citations: nonexistent footprint path is an env error"
# Unreadable footprint file → exit 2. Skipped where the read bit is not enforced
# (e.g. running as root, which bypasses permission checks).
noread="$scratch/unreadable.md"
printf '# x\n' >"$noread"
chmod 000 "$noread"
if [ ! -r "$noread" ]; then
  "$links" "$noread" >/dev/null 2>&1
  assert_rc 2 $? "links: unreadable footprint file is an env error"
  "$citations" "$noread" >/dev/null 2>&1
  assert_rc 2 $? "citations: unreadable footprint file is an env error"
  # A READABLE footprint root containing an UNREADABLE nested directory must not
  # be silently scanned as complete — both checkers must exit 2.
  nest="$scratch/nest"
  mkdir -p "$nest/sub"
  printf '# ok\n' >"$nest/top.md"
  printf '# hidden\n' >"$nest/sub/inner.md"
  chmod 000 "$nest/sub"
  if [ ! -r "$nest/sub" ]; then
    "$links" "$nest" >/dev/null 2>&1
    assert_rc 2 $? "links: unreadable nested directory is an env error"
    "$citations" "$nest" >/dev/null 2>&1
    assert_rc 2 $? "citations: unreadable nested directory is an env error"
  else
    echo "SKIP: read bit not enforced on nested dir — nested-traversal assertions skipped"
  fi
  chmod 755 "$nest/sub" 2>/dev/null || true
else
  echo "SKIP: read bit not enforced here (likely root) — unreadable-path assertions skipped"
fi
chmod 644 "$noread" 2>/dev/null || true
echo

echo "== M5 docs-check-projections.sh =="
# The projection check renders into tracked targets, so it is exercised inside a
# throwaway clone of THIS repo — the real index and working tree are never
# touched. A local, non-hardlinked clone keeps the disposable object store on
# the scratch filesystem (hardlinks would fail across devices).
clone="$scratch/repo"
if git clone --local --no-hardlinks --quiet . "$clone" 2>/dev/null; then
  # Run both projection assertions in a subshell rooted at the clone.
  (
    cd "$clone" || exit 3
    # This untouched clone is the end-to-end regression for the entrypoint's
    # bare default, including all three real child checkers.
    "$mechanical" >/dev/null 2>&1
    echo "$?" >"$scratch/rc_mechanical_bare"
    # All projection targets, deduplicated; the drift is injected into the first.
    mapfile -t targets < <(jit project render --json | jq -r '.projections[].target' | sort -u)
    it="${targets[0]}"
    # Stage a freshly-rendered baseline so the index matches the live registries;
    # a genuine clean run is then observable independent of any pre-existing
    # drift committed on the branch.
    jit project render >/dev/null
    git add -- "${targets[@]}"
    "$projections" >/dev/null 2>&1
    echo "$?" >"$scratch/rc_fresh"
    # Defect: append a stray line at EOF of the first projection target, OUTSIDE
    # the rendered region, so re-render preserves it and the working copy drifts.
    printf '\n<!-- selftest projection drift -->\n' >>"$it"
    "$projections" >/dev/null 2>&1
    echo "$?" >"$scratch/rc_drift"
  )
  assert_rc 0 "$(cat "$scratch/rc_mechanical_bare")" "orchestrator: bare invocation succeeds over an unmodified repository"
  assert_rc 0 "$(cat "$scratch/rc_fresh")" "projections: fresh (rendered==staged) tree is clean"
  assert_rc 1 "$(cat "$scratch/rc_drift")" "projections: drifted target region is a finding"
else
  echo "FAIL: could not create isolated clone for the projection check"
  fail=1
fi
echo

if [ "$fail" -eq 0 ]; then
  echo "SELFTEST: all assertions passed"
else
  echo "SELFTEST: assertions FAILED"
fi
exit "$fail"
