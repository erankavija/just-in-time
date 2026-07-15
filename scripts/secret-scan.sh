#!/usr/bin/env bash
# Secret scan over the repository's tracked content (jit:3c4d6fe8).
#
# Copies the working-tree version of every tracked file into a temporary
# directory and scans that copy, so the scan covers exactly what the
# repository ships — including uncommitted edits to tracked files — while
# generated and operational trees (.agents/worktrees, target/, node_modules,
# .git, machine-local .jit files) are excluded by construction: they are not
# tracked, so they never enter the copy.
#
# Fail-closed: a missing scanner, a failed copy, or any finding exits nonzero.
set -euo pipefail

command -v gitleaks >/dev/null 2>&1 || {
  echo "secret-scan: gitleaks not found on PATH" >&2
  exit 2
}

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

# Same scratch-space convention as scripts/cargo-ci.sh: /tmp on this class of
# host is shared and quota-bound, so stage the copy under the user cache.
scan_base="${XDG_CACHE_HOME:-$HOME/.cache}/jit-secret-scan-tmp"
mkdir -p "$scan_base"
tmp=$(mktemp -d "$scan_base/scan.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

# Working-tree versions of all tracked files. tar fails if a tracked file is
# missing from the working tree (e.g. deleted but not staged) — that is
# fail-closed on purpose: scan what you cannot enumerate is not a guarantee.
git ls-files -z | tar --null --files-from=- -cf - | tar -xf - -C "$tmp"

gitleaks detect --no-git --source "$tmp" --exit-code 1
