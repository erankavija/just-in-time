#!/usr/bin/env bash
set -uo pipefail

# workflow-contract — the single entry point for workflow verification.
#
# Two checkers run over the same committed tree:
#
#   1. actionlint, at a pinned version whose release archive is verified against
#      a recorded SHA-256 before it is unpacked. It covers workflow schema,
#      expression syntax, context and `needs` reference validity, and shell
#      quoting inside `run:` scripts.
#   2. workflow-contract.py, this repository's own structural verifier. It
#      checks the declarations in .github/workflow-contract.yml — triggers,
#      `workflow_call` interfaces, `needs` edges, permissions, failure escapes,
#      and commit-SHA pinning of every external `uses:` reference.
#
# Both checkers always run, so one report names every finding rather than
# hiding the second class behind the first.
#
# Usage:
#   workflow-contract.sh [ROOT]
# ROOT defaults to the enclosing git work tree, falling back to the current
# directory. It holds .github/workflows/ and .github/workflow-contract.yml —
# the self-test passes a scratch copy so it can seed defects without touching
# the real tree.
#
# The pinned linter is fetched once into a machine-local cache
# (${XDG_CACHE_HOME:-~/.cache}/jit/actionlint/<version>) and reused offline
# afterwards. ACTIONLINT_BIN overrides the lookup with an already-installed
# binary; its `-version` output must still match the pin, so an override cannot
# silently downgrade the check.
#
# Exit codes:
#   0 — both checkers pass
#   1 — one or more findings
#   2 — environment error: the pinned linter could not be obtained or verified,
#       a required tool is missing, or the contract is unusable. Never a pass.

# Pinned actionlint release. Update both the version and every checksum
# together; the checksums are the ones published in the release's
# actionlint_<version>_checksums.txt.
ACTIONLINT_VERSION="1.7.12"
actionlint_checksum() {
  case "$1" in
    linux_amd64)  echo "8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8" ;;
    linux_arm64)  echo "325e971b6ba9bfa504672e29be93c24981eeb1c07576d730e9f7c8805afff0c6" ;;
    darwin_amd64) echo "5b44c3bc2255115c9b69e30efc0fecdf498fdb63c5d58e17084fd5f16324c644" ;;
    darwin_arm64) echo "aba9ced2dee8d27fecca3dc7feb1a7f9a52caefa1eb46f3271ea66b6e0e6953f" ;;
    *)            echo "" ;;
  esac
}

here=$(cd "$(dirname "$0")" && pwd)
root="${1:-}"
if [ -z "$root" ]; then
  root=$(git rev-parse --show-toplevel 2>/dev/null) || root=$PWD
fi
[ -d "$root/.github/workflows" ] || {
  echo "workflow-contract: no .github/workflows directory under $root" >&2
  exit 2
}

command -v python3 >/dev/null 2>&1 || {
  echo "workflow-contract: 'python3' is not on PATH" >&2
  exit 2
}
python3 -c 'import yaml' >/dev/null 2>&1 || {
  echo "workflow-contract: the PyYAML module is required (python3 -m pip install PyYAML)" >&2
  exit 2
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    echo ""
  fi
}

# Resolve the pinned linter, downloading and verifying it when the cache is cold.
resolve_actionlint() {
  local platform asset expected cache archive actual
  if [ -n "${ACTIONLINT_BIN:-}" ]; then
    echo "$ACTIONLINT_BIN"
    return 0
  fi

  case "$(uname -s)" in
    Linux)  platform="linux" ;;
    Darwin) platform="darwin" ;;
    *)      echo "workflow-contract: unsupported operating system $(uname -s)" >&2; return 2 ;;
  esac
  case "$(uname -m)" in
    x86_64|amd64)  platform="${platform}_amd64" ;;
    arm64|aarch64) platform="${platform}_arm64" ;;
    *)             echo "workflow-contract: unsupported architecture $(uname -m)" >&2; return 2 ;;
  esac

  expected=$(actionlint_checksum "$platform")
  [ -n "$expected" ] || {
    echo "workflow-contract: no recorded checksum for platform $platform" >&2
    return 2
  }

  cache="${XDG_CACHE_HOME:-$HOME/.cache}/jit/actionlint/$ACTIONLINT_VERSION"
  if [ -x "$cache/actionlint" ]; then
    echo "$cache/actionlint"
    return 0
  fi

  command -v curl >/dev/null 2>&1 || {
    echo "workflow-contract: 'curl' is required to fetch actionlint $ACTIONLINT_VERSION" >&2
    return 2
  }
  asset="actionlint_${ACTIONLINT_VERSION}_${platform}.tar.gz"
  mkdir -p "$cache" || return 2
  archive=$(mktemp -d)/"$asset"
  if ! curl --silent --show-error --fail --location --retry 3 \
      "https://github.com/rhysd/actionlint/releases/download/v${ACTIONLINT_VERSION}/${asset}" \
      --output "$archive"; then
    echo "workflow-contract: could not download $asset" >&2
    return 2
  fi
  actual=$(sha256_of "$archive")
  if [ -z "$actual" ]; then
    echo "workflow-contract: no sha256 tool available to verify $asset" >&2
    return 2
  fi
  if [ "$actual" != "$expected" ]; then
    echo "workflow-contract: checksum mismatch for $asset" >&2
    echo "  expected $expected" >&2
    echo "  actual   $actual" >&2
    return 2
  fi
  if ! tar -xzf "$archive" -C "$cache" actionlint; then
    echo "workflow-contract: could not unpack $asset" >&2
    return 2
  fi
  rm -rf "$(dirname "$archive")"
  echo "$cache/actionlint"
}

actionlint_bin=$(resolve_actionlint) || exit 2
[ -x "$actionlint_bin" ] || {
  echo "workflow-contract: $actionlint_bin is not an executable actionlint binary" >&2
  exit 2
}
installed=$("$actionlint_bin" -version 2>/dev/null | head -1)
if [ "$installed" != "$ACTIONLINT_VERSION" ]; then
  echo "workflow-contract: actionlint $installed does not match the pinned $ACTIONLINT_VERSION" >&2
  exit 2
fi

status=0

echo "== actionlint $ACTIONLINT_VERSION =="
# The workflow file list is passed explicitly so the linter reads the requested
# root rather than rediscovering a repository from the current directory.
# `-shellcheck=` and `-pyflakes=` keep the result reproducible: with those tools
# present the linter would add findings that depend on the host toolbox.
workflows=()
while IFS= read -r file; do
  workflows+=("$file")
done < <(find "$root/.github/workflows" -maxdepth 1 -type f \( -name '*.yml' -o -name '*.yaml' \) | sort)
if [ "${#workflows[@]}" -eq 0 ]; then
  echo "workflow-contract: no workflow files under $root/.github/workflows" >&2
  exit 2
fi
if "$actionlint_bin" -shellcheck= -pyflakes= "${workflows[@]}"; then
  echo "actionlint: ${#workflows[@]} workflow file(s) clean"
else
  status=1
fi
echo

echo "== structural contract =="
python3 "$here/workflow-contract.py" --root "$root"
case $? in
  0) ;;
  1) status=1 ;;
  *) exit 2 ;;
esac

exit "$status"
