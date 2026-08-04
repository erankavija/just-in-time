#!/usr/bin/env bash
# NB: NOT `set -e` — this harness inspects child exit codes on purpose.
set -uo pipefail

# assemble-package-selftest — regression coverage for the wrapper's argument
# contract. The cargo stub records whether the wrapper attempted assembly and
# creates the destination only when it was invoked.
#
# Usage: assemble-package-selftest.sh
#
# Exit codes:
#   0 — every assertion passed
#   1 — a case failed
#   2 — the script under test is unavailable

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
script="$here/assemble-package.sh"
[ -r "$script" ] || { echo "selftest: cannot read $script" >&2; exit 2; }

workspace=$(mktemp -d)
trap 'rm -rf "$workspace"' EXIT

stub_bin="$workspace/stub-bin"
mkdir -p "$stub_bin"
cat >"$stub_bin/cargo" <<'STUB'
#!/usr/bin/env bash
printf '<%s>\n' "$@" >"$ASSEMBLE_PACKAGE_TEST_LOG"
destination="${!#}"
mkdir -p "$destination"
STUB
chmod +x "$stub_bin/cargo"

failures=0
assert_rc() {
  local want="$1" got="$2" message="$3"
  if [ "$got" -eq "$want" ]; then
    echo "PASS: $message (exit $got)"
  else
    echo "FAIL: $message (want exit $want, got $got)" >&2
    failures=$((failures + 1))
  fi
}

assert_absent() {
  local path="$1" message="$2"
  if [ ! -e "$path" ]; then
    echo "PASS: $message"
  else
    echo "FAIL: $message ($path exists)" >&2
    failures=$((failures + 1))
  fi
}

assert_output() {
  local pattern="$1" output="$2" message="$3"
  if grep -qF -- "$pattern" "$output"; then
    echo "PASS: $message"
  else
    echo "FAIL: $message (missing '$pattern')" >&2
    failures=$((failures + 1))
  fi
}

run_script() {
  local cwd="$1" output="$2" error="$3"
  shift 3
  (
    cd "$cwd" || exit 3
    PATH="$stub_bin:$PATH" \
      ASSEMBLE_PACKAGE_TEST_LOG="$cwd/cargo.log" \
      "$script" "$@"
  ) >"$output" 2>"$error"
  return $?
}

echo "== assemble-package.sh argument contract =="

unknown="$workspace/unknown-option"
mkdir -p "$unknown"
run_script "$unknown" "$unknown/stdout" "$unknown/stderr" --mistyped
assert_rc 2 $? "an unrecognised option is a usage error"
assert_absent "$unknown/--mistyped" "an unrecognised option does not write its would-be destination"

help="$workspace/help"
mkdir -p "$help"
run_script "$help" "$help/stdout" "$help/stderr" --help
assert_rc 0 $? "--help succeeds"
assert_output "Usage:" "$help/stdout" "--help prints usage"
assert_absent "$help/--help" "--help does not write its would-be destination"
assert_absent "$help/cargo.log" "--help does not invoke assembly"

zero="$workspace/zero"
mkdir -p "$zero"
run_script "$zero" "$zero/stdout" "$zero/stderr"
assert_rc 2 $? "no destination is a usage error"
assert_output "one destination" "$zero/stderr" "no destination names the expected argument"

multiple="$workspace/multiple"
mkdir -p "$multiple"
run_script "$multiple" "$multiple/stdout" "$multiple/stderr" first second
assert_rc 2 $? "two destinations are a usage error"
assert_absent "$multiple/first" "two destinations do not assemble the first argument"

single="$workspace/single"
mkdir -p "$single"
run_script "$single" "$single/stdout" "$single/stderr" package-output
assert_rc 0 $? "a single destination still assembles successfully"
if [ -d "$single/package-output" ]; then
  echo "PASS: a single destination is passed to the assembly command"
else
  echo "FAIL: a single destination was not assembled" >&2
  failures=$((failures + 1))
fi

separator="$workspace/separator"
mkdir -p "$separator"
run_script "$separator" "$separator/stdout" "$separator/stderr" -- --dash-destination
assert_rc 0 $? "-- permits a destination beginning with a dash"
if [ -d "$separator/--dash-destination" ]; then
  echo "PASS: -- reaches a dash-prefixed destination"
else
  echo "FAIL: -- did not reach a dash-prefixed destination" >&2
  failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
  echo "assemble-package-selftest: $failures case(s) failed" >&2
  exit 1
fi
echo "assemble-package-selftest: every case passed"
