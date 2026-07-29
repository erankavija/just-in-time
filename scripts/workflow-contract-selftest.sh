#!/usr/bin/env bash
# NB: NOT `set -e` — this harness inspects child exit codes on purpose.
set -uo pipefail

# workflow-contract-selftest — durable, re-runnable evidence that the workflow
# contract harness actually rejects the defects it claims to reject
# (issue 7edd2fe8, REQ-03).
#
# Two layers of evidence:
#
#   (1) CASE VECTORS — every directory under test-vectors/workflow-contract/ is
#       a miniature repository root: `.github/workflow-contract.yml` plus the
#       `.github/workflows/` (and optional `.github/actions/`) tree it declares
#       assertions over. `expected.txt` states the exit code the verifier must
#       produce and the substrings its report must contain. The harness runs the
#       verifier against each root and compares. Adding a defect class is adding
#       a directory — no edit to this script.
#
#   (2) ACTIONLINT WIRING — the entry point (workflow-contract.sh) is run twice
#       against a scratch copy of THIS repository's `.github/` tree: pristine
#       (must pass) and with a seeded schema defect that only actionlint detects
#       (must fail). That proves the pinned linter is genuinely in the path
#       rather than declared and skipped.
#
# The real repository tree is never mutated: every seed is written into a
# mktemp scratch directory.
#
# Exit codes:
#   0 — all assertions passed
#   1 — one or more assertions failed
#   2 — environment error (missing tooling, not inside a git work tree)

root=$(git rev-parse --show-toplevel 2>/dev/null) || {
  echo "selftest: not inside a git work tree" >&2
  exit 2
}
cd "$root" || exit 2

here=$(cd "$(dirname "$0")" && pwd)
verifier="$here/workflow-contract.py"
entrypoint="$here/workflow-contract.sh"
vectors="$root/test-vectors/workflow-contract"

command -v python3 >/dev/null 2>&1 || { echo "selftest: 'python3' not on PATH" >&2; exit 2; }
python3 -c 'import yaml' >/dev/null 2>&1 || {
  echo "selftest: the PyYAML module is required (python3 -m pip install PyYAML)" >&2
  exit 2
}
[ -d "$vectors" ] || { echo "selftest: missing case-vector directory $vectors" >&2; exit 2; }

fail=0
pass_count=0
report() {
  local ok="$1" msg="$2"
  if [ "$ok" -eq 0 ]; then
    echo "PASS: $msg"
    pass_count=$((pass_count + 1))
  else
    echo "FAIL: $msg"
    fail=1
  fi
}

scratch=$(mktemp -d)
# shellcheck disable=SC2329  # invoked indirectly via the EXIT trap below
cleanup() { rm -rf "$scratch"; }
trap cleanup EXIT

echo "== case vectors =="
case_count=0
for dir in "$vectors"/*/; do
  [ -d "$dir" ] || continue
  name=$(basename "$dir")
  expected="$dir/expected.txt"
  if [ ! -f "$expected" ]; then
    report 1 "$name: case vector has no expected.txt"
    continue
  fi
  case_count=$((case_count + 1))

  want_rc=$(sed -n 's/^exit:[[:space:]]*//p' "$expected" | head -1)
  if [ -z "$want_rc" ]; then
    report 1 "$name: expected.txt declares no 'exit:' line"
    continue
  fi

  out="$scratch/$name.out"
  python3 "$verifier" --root "$dir" >"$out" 2>&1
  got_rc=$?

  if [ "$got_rc" -eq "$want_rc" ]; then
    report 0 "$name: exit $got_rc"
  else
    report 1 "$name: want exit $want_rc, got $got_rc"
    sed 's/^/      | /' "$out"
  fi

  # Every non-comment, non-`exit:` line is a substring the report must contain.
  while IFS= read -r want; do
    case "$want" in
      ''|'#'*|'exit:'*) continue ;;
    esac
    if grep -qF -- "$want" "$out"; then
      report 0 "$name: report mentions \"$want\""
    else
      report 1 "$name: report is missing \"$want\""
      sed 's/^/      | /' "$out"
    fi
  done <"$expected"
done

if [ "$case_count" -eq 0 ]; then
  echo "FAIL: no case vectors found under $vectors"
  fail=1
fi
echo

echo "== actionlint wiring (entry point over a scratch copy of .github/) =="
copy="$scratch/repo"
mkdir -p "$copy"
if cp -R "$root/.github" "$copy/.github"; then
  "$entrypoint" "$copy" >"$scratch/entry_clean.out" 2>&1
  rc_clean=$?
  if [ "$rc_clean" -eq 2 ]; then
    echo "SKIP: entry point reported an environment error — actionlint unavailable"
    sed 's/^/      | /' "$scratch/entry_clean.out"
  else
    if [ "$rc_clean" -eq 0 ]; then
      report 0 "entry point passes on the committed workflow tree"
    else
      report 1 "entry point passes on the committed workflow tree (exit $rc_clean)"
      sed 's/^/      | /' "$scratch/entry_clean.out"
    fi

    # Seed a defect that ONLY actionlint detects: a step-level key the workflow
    # schema does not define. The structural verifier ignores unknown keys, so a
    # failure here can only come from the linter — which the report substring
    # assertion below pins down, so the case cannot pass for another reason.
    seeded="$copy/.github/workflows/ci.yml"
    printf '\n  actionlint-seed:\n    runs-on: ubuntu-latest\n    steps:\n      - run: "true"\n        not-a-real-step-key: 1\n' >>"$seeded"
    "$entrypoint" "$copy" >"$scratch/entry_seeded.out" 2>&1
    rc_seeded=$?
    if [ "$rc_seeded" -eq 1 ] && grep -qF "not-a-real-step-key" "$scratch/entry_seeded.out"; then
      report 0 "seeded schema defect fails the entry point through actionlint"
    else
      report 1 "seeded schema defect fails the entry point through actionlint (want exit 1 naming the key, got $rc_seeded)"
      sed 's/^/      | /' "$scratch/entry_seeded.out"
    fi
  fi
else
  report 1 "could not copy .github/ into the scratch tree"
fi
echo

if [ "$fail" -eq 0 ]; then
  echo "SELFTEST: all $pass_count assertions passed across $case_count case vectors"
else
  echo "SELFTEST: assertions FAILED"
fi
exit "$fail"
