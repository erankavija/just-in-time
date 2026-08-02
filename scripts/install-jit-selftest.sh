#!/usr/bin/env bash
# NB: NOT `set -e` — this harness inspects child behaviour on purpose.
set -uo pipefail

# install-jit-selftest — behavioural evidence for jit:32779829 REQ-11.
#
# scripts/install-jit.sh embeds a dirty flag that makes the installed binary
# report itself stale for its whole life. This asserts what that flag means:
# it is true when a DECLARED build input is uncommitted, and false otherwise,
# whatever else the working tree carries.
#
# The installer's last act is `exec cargo install`, so each case runs it with a
# stub `cargo` first on PATH that prints the provenance variables it was handed
# and exits. The assertion is therefore over the value the real installer
# computed and passed, not over a reimplementation of its logic.
#
# Each case builds a throwaway git repository holding only what the installer
# reads: the script itself and the build-input inventory it consults.
#
# Usage: install-jit-selftest.sh
#
# Exit codes:
#   0 — every case behaved as asserted
#   1 — a case failed
#   2 — environment problem (git or the files under test unavailable)

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
installer="$here/install-jit.sh"
inventory="$here/../crates/jit/src/domain/binary_build_inputs.txt"

command -v git >/dev/null 2>&1 || { echo "selftest: git not on PATH" >&2; exit 2; }
[ -r "$installer" ] || { echo "selftest: cannot read $installer" >&2; exit 2; }
[ -r "$inventory" ] || { echo "selftest: cannot read $inventory" >&2; exit 2; }

workspace="$(mktemp -d)"
trap 'rm -rf "$workspace"' EXIT

# A stub cargo that reports the dirty flag the installer handed it.
stub_bin="$workspace/stub-bin"
mkdir -p "$stub_bin"
cat >"$stub_bin/cargo" <<'STUB'
#!/usr/bin/env bash
printf 'JIT_BUILD_GIT_DIRTY=%s\n' "${JIT_BUILD_GIT_DIRTY:-<unset>}"
STUB
chmod +x "$stub_bin/cargo"

failures=0

# Build a repository carrying the installer and the inventory, committed clean.
new_repo() {
  local repo="$workspace/$1"
  mkdir -p "$repo/scripts" "$repo/crates/jit/src/domain" "$repo/dev" "$repo/.jit"
  cp "$installer" "$repo/scripts/install-jit.sh"
  cp "$inventory" "$repo/crates/jit/src/domain/binary_build_inputs.txt"
  printf 'fn main() {}\n' >"$repo/crates/jit/src/main.rs"
  printf 'notes\n' >"$repo/dev/plan.md"
  printf '{}\n' >"$repo/.jit/issue.json"
  git -C "$repo" init -q
  git -C "$repo" config user.name Test
  git -C "$repo" config user.email test@example.com
  git -C "$repo" add -A
  git -C "$repo" commit -q -m seed
  printf '%s' "$repo"
}

# Run the installer in `repo` with the stub cargo, echoing the flag it passed.
dirty_flag() {
  PATH="$stub_bin:$PATH" "$1/scripts/install-jit.sh" 2>/dev/null \
    | sed -n 's/^JIT_BUILD_GIT_DIRTY=//p'
}

expect() {
  local label="$1" expected="$2" actual="$3"
  if [ "$actual" = "$expected" ]; then
    echo "  ok   $label (dirty=$actual)"
  else
    echo "  FAIL $label: expected dirty=$expected, got dirty=${actual:-<none>}" >&2
    failures=$((failures + 1))
  fi
}

echo "== install-jit dirty flag =="

repo="$(new_repo clean)"
expect "a committed tree is clean" false "$(dirty_flag "$repo")"

# REQ-11: the case that cost this repository three pipelines.
repo="$(new_repo untracked-plan)"
printf 'progress\n' >"$repo/dev/progress.json"
expect "an untracked file feeding no build stays clean" false "$(dirty_flag "$repo")"

repo="$(new_repo modified-plan)"
printf 'edited\n' >>"$repo/dev/plan.md"
expect "an edited file feeding no build stays clean" false "$(dirty_flag "$repo")"

repo="$(new_repo tracker-data)"
printf '{"changed":true}\n' >"$repo/.jit/issue.json"
expect "the tracker's own data stays clean" false "$(dirty_flag "$repo")"

repo="$(new_repo modified-source)"
printf '// edited\n' >>"$repo/crates/jit/src/main.rs"
expect "an edited declared source is dirty" true "$(dirty_flag "$repo")"

repo="$(new_repo untracked-source)"
printf 'pub fn added() {}\n' >"$repo/crates/jit/src/added.rs"
expect "an untracked declared source is dirty" true "$(dirty_flag "$repo")"

repo="$(new_repo edited-inventory)"
printf 'docs\n' >>"$repo/crates/jit/src/domain/binary_build_inputs.txt"
expect "editing the inventory itself is dirty" true "$(dirty_flag "$repo")"

repo="$(new_repo missing-inventory)"
rm "$repo/crates/jit/src/domain/binary_build_inputs.txt"
git -C "$repo" add -A
git -C "$repo" commit -q -m "drop the inventory"
expect "an unreadable inventory fails closed" true "$(dirty_flag "$repo")"

if [ "$failures" -ne 0 ]; then
  echo "install-jit-selftest: $failures case(s) failed" >&2
  exit 1
fi
echo "install-jit-selftest: every case behaved as asserted"
