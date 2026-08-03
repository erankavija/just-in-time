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
# link/citation seeds are scratch files in a mktemp dir, and the checks whose
# defects can only be seeded into tracked files — the projection check, which
# renders into its targets, and the shipped-policy check, whose seeds are a
# stale region and a committed build-input change — run inside throwaway
# `git clone`s of this repo under that mktemp dir. A pre-existing staged or
# unstaged change in the real repo is therefore left untouched. The harness
# exits 0 only if every assertion passes; it never commits a seed into the real
# repository.
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
canonical="$here/docs-check-canonical.sh"
policy="$here/docs-check-shipped-policy.sh"
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
for checker in docs-check-links.sh docs-check-citations.sh docs-check-projections.sh docs-check-canonical.sh docs-check-shipped-policy.sh; do
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
assert_logged_footprint "docs-check-shipped-policy.sh" "" "$fixture_log" "orchestrator: the fan-out reaches the shipped-policy check, with no footprint"

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

# Package content: the exclusion splits a package directory by what each file
# is for. The fixture reproduces that layout — a manifest beside the three
# asset source prefixes a manifest declares — and seeds ONE unresolvable item
# id into every one of them, so the two runs below differ only in which class
# of package file they scan. The id is assembled from a variable so this
# self-test source is not itself reported when the scripts tree is scanned.
missing_id='does-not-exist-zzz'
pkg="$scratch/pkgfixture/profiles/jit-fixture"
mkdir -p "$pkg/assets/install/.jit/reference" "$pkg/assets/regions" "$pkg/assets/live/docs"
printf 'enforced-by = "@/rule/%s"\n' "$missing_id" >"$pkg/manifest.toml"
printf -- '- **@/rule/%s** — projected into the destination registry.\n' "$missing_id" \
  >"$pkg/assets/install/.jit/reference/rules-and-gates.md"
printf -- '- The @/rule/%s rule applies once the package is applied.\n' "$missing_id" \
  >"$pkg/assets/regions/guidance.md"
printf -- 'Guide text citing @/rule/%s.\n' "$missing_id" \
  >"$pkg/assets/live/docs/guide.md"
"$citations" "$pkg/assets" >/dev/null 2>&1
assert_rc 0 $? "citations: destination-scoped ids in package assets are excluded"
"$citations" "$pkg/manifest.toml" >/dev/null 2>&1
assert_rc 1 $? "citations: the same id in a package manifest is reported"
# The consequence on live content: what the exclusion leaves scanned in this
# repository's own packages resolves.
"$citations" profiles >/dev/null 2>&1
assert_rc 0 $? "citations: this repository's scanned package content resolves"
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

echo "== M6 docs-check-canonical.sh =="
# An isolated fixture repository: one canonical home stating one fact, the
# automation that makes it true, a navigation page linking the home, a second
# scanned page that must not restate the fact, and a manifest binding them. Each
# defect class is seeded into that fixture and then reverted, so every assertion
# runs against the real checker without touching this repository's own pages.
canon="$scratch/canonical"
mkdir -p "$canon/pages"
write_canonical_fixture() {
  # shellcheck disable=SC2016  # Markdown backticks are literal fixture content.
  printf '# Home\n\n## Section\n\nRun `deploy --now` to publish.\n' >"$canon/pages/home.md"
  printf '# Other\n\nThis page links [Home](home.md) rather than restating it.\n' \
    >"$canon/pages/other.md"
  printf '# Navigation\n\n- [Home](pages/home.md)\n' >"$canon/index.md"
  printf 'steps:\n  - run: deploy --now\n' >"$canon/automation.yml"
  cat >"$canon/manifest.toml" <<'TOML'
scan_roots = ["pages"]
navigation = ["index.md"]

[[fact]]
id = "fixture-fact"
home = "pages/home.md#section"

  [[fact.binding]]
  doc = "deploy --now"
  sources = ["automation.yml"]
TOML
}
run_canonical() { (cd "$canon" && "$canonical" manifest.toml >/dev/null 2>&1); }

write_canonical_fixture
run_canonical
assert_rc 0 $? "canonical: home states a live, unique, navigable fact"

# Defect: the home no longer states the fact it owns.
printf '# Home\n\n## Section\n\nNothing to run here.\n' >"$canon/pages/home.md"
run_canonical
assert_rc 1 $? "canonical: home that stopped stating its fact is a finding"

# Defect: the declared anchor no longer resolves.
write_canonical_fixture
# shellcheck disable=SC2016  # Markdown backticks are literal fixture content.
printf '# Home\n\n## Renamed\n\nRun `deploy --now` to publish.\n' >"$canon/pages/home.md"
run_canonical
assert_rc 1 $? "canonical: home whose declared anchor is gone is a finding"

# Defect: the automation no longer carries what the home states.
write_canonical_fixture
printf 'steps:\n  - run: something-else\n' >"$canon/automation.yml"
run_canonical
assert_rc 1 $? "canonical: source that stopped carrying the stated command is a finding"

# Defect: a second scanned page states the same fact, so it has two homes.
write_canonical_fixture
# shellcheck disable=SC2016  # Markdown backticks are literal fixture content.
printf '# Other\n\nRun `deploy --now` to publish.\n' >"$canon/pages/other.md"
run_canonical
assert_rc 1 $? "canonical: fact restated on a second page is a finding"

# Defect: navigation stops linking the canonical home.
write_canonical_fixture
printf '# Navigation\n\nNo links here.\n' >"$canon/index.md"
run_canonical
assert_rc 1 $? "canonical: home no navigation page links is a finding"

# Clean again, so the reverted fixture is proven to pass rather than assumed to.
write_canonical_fixture
run_canonical
assert_rc 0 $? "canonical: reverted fixture is clean"

# A scan root naming a single file is scanned like a directory root, which is
# what lets the manifest reach entry-point and component pages that sit outside
# the documentation root.
write_canonical_fixture
# shellcheck disable=SC2016  # Markdown backticks are literal fixture content.
printf '# Entry\n\nRun `deploy --now` to publish.\n' >"$canon/entry.md"
cat >"$canon/filescan.toml" <<'TOML'
scan_roots = ["pages", "entry.md"]
navigation = ["index.md"]

[[fact]]
id = "fixture-fact"
home = "pages/home.md#section"

  [[fact.binding]]
  doc = "deploy --now"
  sources = ["automation.yml"]
TOML
(cd "$canon" && "$canonical" filescan.toml >/dev/null 2>&1)
assert_rc 1 $? "canonical: fact restated in a file scan root is a finding"

# A declared exemption carries the reason one page states the same literal for a
# different purpose, and suppresses the duplicate — and only that one.
cat >"$canon/exempt.toml" <<'TOML'
scan_roots = ["pages", "entry.md"]
navigation = ["index.md"]

[[fact]]
id = "fixture-fact"
home = "pages/home.md#section"

  [[fact.binding]]
  doc = "deploy --now"
  sources = ["automation.yml"]

    [[fact.binding.exempt]]
    path = "entry.md"
    reason = "the contributor entry point runs the same command as one validation step"
TOML
(cd "$canon" && "$canonical" exempt.toml >/dev/null 2>&1)
assert_rc 0 $? "canonical: declared exemption suppresses that page's duplicate"

# Defect: the exempt page stops stating the literal, so the exemption has
# outlived its cause and must not linger in the table unnoticed.
printf '# Entry\n\nNothing to run here.\n' >"$canon/entry.md"
(cd "$canon" && "$canonical" exempt.toml >/dev/null 2>&1)
assert_rc 1 $? "canonical: exemption that outlived its cause is a finding"

# An exemption with no stated reason is an unexplained hole in the uniqueness
# rule; the manifest cannot express one.
sed '/^    reason = /d' "$canon/exempt.toml" >"$canon/exempt_noreason.toml"
(cd "$canon" && "$canonical" exempt_noreason.toml >/dev/null 2>&1)
assert_rc 2 $? "canonical: exemption without a reason is an env error"

# A manifest that cannot be read is an environment error, never a false-green pass.
(cd "$canon" && "$canonical" no_such_manifest_zzz.toml >/dev/null 2>&1)
assert_rc 2 $? "canonical: unreadable manifest is an env error"
# So is a manifest that parses but does not conform.
printf 'scan_roots = ["pages"]\n' >"$canon/empty.toml"
(cd "$canon" && "$canonical" empty.toml >/dev/null 2>&1)
assert_rc 2 $? "canonical: manifest declaring no facts is an env error"
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

echo "== M7 docs-check-shipped-policy.sh =="
# This checker regenerates the shipped-policy regions in a scratch fixture of
# its own and never writes into the tree it inspects, so the throwaway clone
# below exists only so a stale region can be seeded and a build input committed
# without touching the real repository.
policy_clone="$scratch/policy-repo"
policy_target="docs/reference/configuration.md"
policy_begin="<!-- jit:shipped-documentation-policy:begin -->"
policy_end="<!-- jit:shipped-documentation-policy:end -->"
# A binary reporting no build commit — the state an ordinary build with no
# injected provenance is in. The stub answers `version --json` and nothing
# else, so reaching any other subcommand would mean the checker compared
# something before establishing that the classification can be trusted.
policy_stub="$scratch/policy-stub"
mkdir -p "$policy_stub"
cat >"$policy_stub/jit" <<'STUB'
#!/usr/bin/env bash
if [ "${1:-}" = "version" ]; then
  printf '{"git_commit":"unknown","git_short_commit":"unknown","git_dirty":null}\n'
  exit 0
fi
echo "stub jit: reached '$*' without established provenance" >&2
exit 99
STUB
chmod +x "$policy_stub/jit"
# A binary whose version report succeeds but cannot be read. The classification
# is unobtainable rather than untrusted, and the check owes the same
# environment failure for it — reading the report must not be the one step that
# escapes with a status of its own.
policy_unreadable="$scratch/policy-unreadable-stub"
mkdir -p "$policy_unreadable"
cat >"$policy_unreadable/jit" <<'STUB'
#!/usr/bin/env bash
if [ "${1:-}" = "version" ]; then
  printf '{"git_commit":\n'
  exit 0
fi
echo "stub jit: reached '$*' without a readable version report" >&2
exit 99
STUB
chmod +x "$policy_unreadable/jit"

if git clone --local --no-hardlinks --quiet . "$policy_clone" 2>/dev/null; then
  (
    cd "$policy_clone" || exit 3
    "$policy" >/dev/null 2>&1
    echo "$?" >"$scratch/rc_policy_fresh"
    # The tree under check is left exactly as it was found.
    if [ -z "$(git status --porcelain)" ]; then
      echo 0
    else
      echo 1
    fi >"$scratch/rc_policy_untouched"

    # Defect: replace a generated region's body, leaving the markers and every
    # byte outside them in place.
    awk -v begin="$policy_begin" -v end="$policy_end" '
      $0 == begin { print; print "STALE-REGION-BODY"; inside = 1; next }
      $0 == end { inside = 0; print; next }
      !inside { print }
    ' "$policy_target" >"$scratch/policy_seeded" &&
      cp "$scratch/policy_seeded" "$policy_target"
    "$policy" >/dev/null 2>&1
    echo "$?" >"$scratch/rc_policy_drift"
    git checkout -q -- "$policy_target"

    # An untrustworthy classification, arm 1: provenance that does not resolve
    # against the repository under check.
    PATH="$policy_stub:$PATH" "$policy" >/dev/null 2>&1
    echo "$?" >"$scratch/rc_policy_unresolvable"

    # An untrustworthy classification, arm 1b: a version report that parses
    # into nothing, so no provenance can be read out of it at all.
    PATH="$policy_unreadable:$PATH" "$policy" >/dev/null 2>&1
    echo "$?" >"$scratch/rc_policy_unreadable"

    # An untrustworthy classification, arm 2: a binary that predates the
    # repository under check. Committing a change to a build input in the clone
    # makes the installed binary older than the sources there, so the table it
    # would produce describes an earlier tree — and a rerun of the generator
    # would agree with it, since both read the same binary.
    printf '\n// selftest build-input touch\n' >>crates/jit/src/lib.rs
    git add -A >/dev/null 2>&1
    git -c user.email=selftest@invalid -c user.name=selftest commit -q -m "touch a build input"
    "$policy" >/dev/null 2>&1
    echo "$?" >"$scratch/rc_policy_stale"
  )
  assert_rc 0 "$(cat "$scratch/rc_policy_fresh")" "shipped policy: regions carrying the shipped classification pass"
  assert_rc 0 "$(cat "$scratch/rc_policy_untouched")" "shipped policy: a run leaves the tree it checks unmodified"
  assert_rc 1 "$(cat "$scratch/rc_policy_drift")" "shipped policy: a seeded stale region is a finding"
  assert_rc 2 "$(cat "$scratch/rc_policy_unresolvable")" "shipped policy: a classification whose provenance does not resolve is an environment failure"
  assert_rc 2 "$(cat "$scratch/rc_policy_unreadable")" "shipped policy: a version report that cannot be read is an environment failure"
  assert_rc 2 "$(cat "$scratch/rc_policy_stale")" "shipped policy: a classification from a binary predating the tree under check is an environment failure"
else
  echo "FAIL: could not create isolated clone for the shipped-policy check"
  fail=1
fi
echo

if [ "$fail" -eq 0 ]; then
  echo "SELFTEST: all assertions passed"
else
  echo "SELFTEST: assertions FAILED"
fi
exit "$fail"
