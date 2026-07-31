#!/usr/bin/env bash
# NB: NOT `set -e` — this harness inspects child exit codes on purpose.
set -uo pipefail

# generate-shipped-policy-regions-selftest — durable, re-runnable regression
# evidence for `scripts/generate-shipped-policy-regions.sh`.
#
# It proves the four properties the generator is built for:
#   (1) a run against a current tree changes nothing (idempotence);
#   (2) a corrupted region is restored and every byte outside the markers
#       survives, including authored text the harness adds;
#   (3) a target whose markers are missing or malformed, and a publication the
#       rename cannot complete, each stop the run without a partial write, and
#       each reports a failure (1) rather than a refusal (2) — the exit-code
#       boundary every entry point shares;
#   (4) the generator refuses to write unless it has positively established
#       that the binary it reads the classification from carries provenance
#       resolvable in the repository being written to and does not predate that
#       repository's sources. Each way that can fail — no build commit at all,
#       a build commit the repository does not contain, no resolvable head, a
#       binary older than the sources — is asserted separately, because the
#       first three produce the same silence from the binary's own staleness
#       report as a current binary does.
#
# The harness NEVER mutates the real repository. Every generator invocation
# runs against a fixture under a mktemp dir — a `git clone` of this repository
# for the write cases, a freshly initialized unrelated repository for the
# provenance cases — and every invocation runs from a working directory
# outside any git repository, so a generator that resolved its target from the
# current directory instead of from its own location could not reach the real
# tree either.
#
# Exit codes:
#   0 — all assertions passed
#   1 — one or more assertions failed
#   2 — environment error (missing tooling, not inside a git work tree)

command -v git >/dev/null 2>&1 || { echo "selftest: 'git' not on PATH" >&2; exit 2; }
command -v jit >/dev/null 2>&1 || { echo "selftest: 'jit' not on PATH" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "selftest: 'jq' not on PATH" >&2; exit 2; }
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
assert_rc 0 $? "generator: succeeds against a repository whose binary provenance resolves"

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

echo "== provenance must be established positively, never assumed =="
# Arm 1: the binary reports no build commit at all. An ordinary build injects
# no provenance and reports exactly this, so it is the common case, not a rare
# one. The stub answers `version --json` and nothing else: reaching any other
# subcommand would mean the generator acted before establishing currency.
stub="$scratch/stub"
mkdir -p "$stub"
cat >"$stub/jit" <<'STUB'
#!/usr/bin/env bash
if [ "${1:-}" = "version" ]; then
  printf '{"git_commit":"unknown","git_short_commit":"unknown","git_dirty":null}\n'
  exit 0
fi
echo "stub jit: reached '$*' without established provenance" >&2
exit 99
STUB
chmod +x "$stub/jit"

corrupt_region "$clone/$reference" "$md_begin" "$md_end"
cp "$clone/$reference" "$scratch/unknown-provenance.before"
(cd "$scratch" && PATH="$stub:$PATH" "$clone/scripts/generate-shipped-policy-regions.sh" >/dev/null 2>&1)
assert_rc 2 $? "generator: a binary reporting no build commit is a refusal to write"
assert_same_bytes "$scratch/unknown-provenance.before" "$clone/$reference" \
  "generator: leaves a stale region stale rather than writing on unestablished provenance"
git -C "$clone" checkout -q -- "$reference" || exit 2
echo

# Arm 2: a repository that does not contain the binary's build commit. The
# binary's own staleness report is silent here — it treats an unrelated
# repository exactly as it treats a current one — which is why the generator
# establishes resolvability itself instead of reading that silence as a pass.
alien="$scratch/alien"
mkdir -p "$alien/scripts" "$alien/docs/reference"
git -C "$alien" init -q >/dev/null 2>&1 || exit 2
cp "$generator" "$alien/scripts/" || exit 2
cp "$root/$reference" "$alien/$reference" || exit 2
cp "$root/$example" "$alien/$example" || exit 2
corrupt_region "$alien/$reference" "$md_begin" "$md_end"
cp "$alien/$reference" "$scratch/alien.before"

(cd "$alien" && JIT_GATE_RUN=1 jit version --json >/dev/null 2>&1)
assert_rc 0 $? "binary: its own staleness report is silent in a repository that lacks its build commit"

git -C "$alien" add -A >/dev/null 2>&1 || exit 2
git -C "$alien" "${git_id[@]}" commit -q -m "alien baseline" || exit 2
run_generator "$alien"
assert_rc 2 $? "generator: a build commit unresolvable in the target repository is a refusal to write"
assert_same_bytes "$scratch/alien.before" "$alien/$reference" \
  "generator: writes nothing into a repository whose history it cannot be placed in"
echo

# Arm 3: no resolvable head. A repository without a commit resolves neither
# side of the comparison, which is the same silence again.
headless="$scratch/headless"
mkdir -p "$headless/scripts" "$headless/docs/reference"
git -C "$headless" init -q >/dev/null 2>&1 || exit 2
cp "$generator" "$headless/scripts/" || exit 2
cp "$root/$reference" "$headless/$reference" || exit 2
cp "$root/$example" "$headless/$example" || exit 2
corrupt_region "$headless/$reference" "$md_begin" "$md_end"
cp "$headless/$reference" "$scratch/headless.before"
run_generator "$headless"
assert_rc 2 $? "generator: an unresolvable head in the target repository is a refusal to write"
assert_same_bytes "$scratch/headless.before" "$headless/$reference" \
  "generator: writes nothing into a repository with no resolvable head"
echo

# Arm 4: a binary that predates the repository's sources. Committing a change
# to a build input in the clone makes the installed binary stale there. This is
# the one arm the binary answers for itself, and the generator takes that
# answer rather than re-deriving which paths count as build inputs.
corrupt_region "$clone/$reference" "$md_begin" "$md_end"
cp "$clone/$reference" "$scratch/stale.before"
printf '\n// selftest build-input touch\n' >>"$clone/crates/jit/src/lib.rs"
git -C "$clone" add -A >/dev/null 2>&1 || exit 2
git -C "$clone" "${git_id[@]}" commit -q -m "touch a build input" || exit 2
run_generator "$clone"
assert_rc 2 $? "generator: a binary predating the target repository's sources is a refusal to write"
assert_same_bytes "$scratch/stale.before" "$clone/$reference" \
  "generator: writes nothing while the binary predates the sources it would describe"
echo

if [ "$fail" -eq 0 ]; then
  echo "OK: generate-shipped-policy-regions selftest passed"
  exit 0
fi
echo "FAILED: one or more generate-shipped-policy-regions assertions failed" >&2
exit 1
