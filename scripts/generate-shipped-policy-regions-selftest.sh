#!/usr/bin/env bash
# NB: NOT `set -e` — this harness inspects child exit codes on purpose.
set -uo pipefail

# generate-shipped-policy-regions-selftest — durable, re-runnable regression
# evidence for `scripts/generate-shipped-policy-regions.sh`.
#
# It proves the three properties the generator is built for:
#   (1) a run against a current tree changes nothing (idempotence);
#   (2) a corrupted region is restored and every byte outside the markers
#       survives, including authored text the harness adds;
#   (3) a target whose markers are missing or malformed, and a publication the
#       rename cannot complete, each stop the run without a partial write, and
#       each reports a failure (1) rather than a refusal (2) — the exit-code
#       boundary every entry point shares.
#
# The harness NEVER mutates the real repository. Every generator invocation
# runs against a `git clone` of this repository under a mktemp dir, and every
# invocation runs from a working directory outside any git repository, so a
# generator that resolved its target from the current directory instead of
# from its own location could not reach the real tree either.
#
# Exit codes:
#   0 — all assertions passed
#   1 — one or more assertions failed
#   2 — environment error (missing tooling, not inside a git work tree)

command -v git >/dev/null 2>&1 || { echo "selftest: 'git' not on PATH" >&2; exit 2; }
command -v jit >/dev/null 2>&1 || { echo "selftest: 'jit' not on PATH" >&2; exit 2; }
root=$(git rev-parse --show-toplevel 2>/dev/null) || {
  echo "selftest: not inside a git work tree" >&2
  exit 2
}

here=$(cd "$(dirname "$0")" && pwd)
generator="$here/generate-shipped-policy-regions.sh"
[ -x "$generator" ] || { echo "selftest: $generator is not executable" >&2; exit 2; }

reference="docs/reference/configuration.md"
example="docs/reference/example-config.toml"
region="shipped-documentation-policy"
md_begin="<!-- jit:$region:begin -->"
md_end="<!-- jit:$region:end -->"
toml_begin="# jit:$region:begin"
toml_end="# jit:$region:end"

scratch=$(mktemp -d) || { echo "selftest: mktemp failed" >&2; exit 2; }
trap 'rm -rf "$scratch"' EXIT

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

assert_contains() {
  local file="$1" needle="$2" msg="$3"
  if grep -qF -- "$needle" "$file"; then
    echo "PASS: $msg"
  else
    echo "FAIL: $msg (missing: $needle)"
    fail=1
  fi
}

assert_missing() {
  local file="$1" needle="$2" msg="$3"
  if grep -qF -- "$needle" "$file"; then
    echo "FAIL: $msg (still present: $needle)"
    fail=1
  else
    echo "PASS: $msg"
  fi
}

assert_same_bytes() {
  local a="$1" b="$2" msg="$3"
  if cmp -s "$a" "$b"; then
    echo "PASS: $msg"
  else
    echo "FAIL: $msg (files differ)"
    fail=1
  fi
}

# Replace a marked region's body with a single stale line, leaving the markers
# and every byte outside them in place.
corrupt_region() {
  local file="$1" begin="$2" end="$3"
  awk -v begin="$begin" -v end="$end" '
    $0 == begin { print; print "STALE-REGION-BODY"; inside = 1; next }
    $0 == end { inside = 0; print; next }
    !inside { print }
  ' "$file" >"$file.seed" && mv "$file.seed" "$file"
}

# Every invocation runs from outside any repository: the generator must resolve
# its target from its own location.
run_generator() { (cd "$scratch" && "$1/scripts/generate-shipped-policy-regions.sh" >/dev/null 2>&1); }

git_id=(-c user.email=selftest@invalid -c user.name=selftest)

echo "== fixture: a clone of this repository carrying the working-tree scripts and targets =="
clone="$scratch/clone"
git clone --quiet --shared --local "$root" "$clone" || {
  echo "selftest: could not clone $root" >&2
  exit 2
}
cp "$generator" "$clone/scripts/" || exit 2
cp "$root/$reference" "$clone/$reference" || exit 2
cp "$root/$example" "$clone/$example" || exit 2
git -C "$clone" add -A >/dev/null 2>&1 || exit 2
git -C "$clone" "${git_id[@]}" commit -q -m "selftest baseline" --allow-empty || exit 2
echo

echo "== idempotence and containment =="
run_generator "$clone"
assert_rc 0 $? "generator: succeeds against a current repository"

git -C "$clone" diff --quiet
assert_rc 0 $? "generator: a run against a current tree rewrites no tracked byte"

[ -z "$(git -C "$clone" status --porcelain)" ]
assert_rc 0 $? "generator: leaves the working tree it runs in otherwise unchanged"

cp "$clone/$reference" "$scratch/reference.first"
cp "$clone/$example" "$scratch/example.first"
run_generator "$clone"
assert_rc 0 $? "generator: a second run immediately after the first succeeds"
assert_same_bytes "$scratch/reference.first" "$clone/$reference" \
  "generator: the second run leaves the configuration reference byte-identical"
assert_same_bytes "$scratch/example.first" "$clone/$example" \
  "generator: the second run leaves the example configuration byte-identical"
echo

echo "== region restoration and byte preservation outside the markers =="
corrupt_region "$clone/$reference" "$md_begin" "$md_end"
corrupt_region "$clone/$example" "$toml_begin" "$toml_end"
printf '\n<!-- selftest authored tail -->\n' >>"$clone/$reference"
printf '\n# selftest authored tail\n' >>"$clone/$example"

run_generator "$clone"
assert_rc 0 $? "generator: succeeds against corrupted regions"
assert_missing "$clone/$reference" "STALE-REGION-BODY" \
  "generator: replaces a stale body in the configuration reference"
assert_missing "$clone/$example" "STALE-REGION-BODY" \
  "generator: replaces a stale body in the example configuration"
assert_contains "$clone/$reference" "<!-- selftest authored tail -->" \
  "generator: preserves authored bytes after the region in the configuration reference"
assert_contains "$clone/$example" "# selftest authored tail" \
  "generator: preserves authored bytes after the region in the example configuration"

# Removing exactly the authored tails must leave the committed tree: nothing
# outside the markers moved, and the regions hold the shipped classification.
head -n -2 "$clone/$reference" >"$scratch/reference.trimmed" && mv "$scratch/reference.trimmed" "$clone/$reference"
head -n -2 "$clone/$example" >"$scratch/example.trimmed" && mv "$scratch/example.trimmed" "$clone/$example"
git -C "$clone" diff --quiet
assert_rc 0 $? "generator: restores both targets to their committed bytes"
echo

echo "== a target without usable markers fails the splice =="
grep -vF -- "$md_begin" "$clone/$reference" >"$scratch/unmarked" && mv "$scratch/unmarked" "$clone/$reference"
cp "$clone/$reference" "$scratch/unmarked.before"
run_generator "$clone"
assert_rc 1 $? "generator: a target missing its begin marker fails the splice"
assert_same_bytes "$scratch/unmarked.before" "$clone/$reference" \
  "generator: writes nothing into a target it cannot splice"
git -C "$clone" checkout -q -- "$reference" || exit 2
echo

echo "== a publication the rename cannot complete fails rather than refuses =="
# The splice has already produced complete bytes by the time the rename runs, so
# this is the one failure that is unambiguously post-render. A stub `mv` earlier
# on PATH isolates it: the generator's write path calls `mv` exactly once, to
# publish. The harness's own `mv` calls run outside the stubbed environment.
mv_stub="$scratch/mv-stub"
mkdir -p "$mv_stub"
cat >"$mv_stub/mv" <<'STUB'
#!/usr/bin/env bash
echo "stub mv: refusing to rename" >&2
exit 1
STUB
chmod +x "$mv_stub/mv"

corrupt_region "$clone/$reference" "$md_begin" "$md_end"
cp "$clone/$reference" "$scratch/unpublished.before"
(cd "$scratch" && PATH="$mv_stub:$PATH" "$clone/scripts/generate-shipped-policy-regions.sh" >/dev/null 2>&1)
assert_rc 1 $? "generator: a publication the rename cannot complete is a failure, not a refusal"
assert_same_bytes "$scratch/unpublished.before" "$clone/$reference" \
  "generator: leaves the target as it found it when the publication fails"
[ ! -e "$clone/$reference.jit-region" ]
assert_rc 0 $? "generator: leaves no staged file behind when the publication fails"
git -C "$clone" checkout -q -- "$reference" || exit 2
echo

if [ "$fail" -eq 0 ]; then
  echo "OK: generate-shipped-policy-regions selftest passed"
  exit 0
fi
echo "FAILED: one or more generate-shipped-policy-regions assertions failed" >&2
exit 1
