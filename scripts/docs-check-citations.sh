#!/usr/bin/env bash
set -euo pipefail

# docs-check-citations — source-path + `@/…` citation-existence check.
# Mechanical check M3 of the `docs-mechanical` gate (epic 2d109173).
#
# Two comparisons, both derived live so the check encodes no product facts:
#   1. File-path tokens inside backtick spans must exist on disk. A token is
#      treated as a file citation only when (a) its first segment is a real
#      tracked top-level repository entry (enumerated live), and (b) its last
#      segment carries an extension (contains a dot) — a pattern class that
#      admits files while skipping directory/conceptual references. This
#      rejects prose, flags, and MIME types (`application/json`) without any
#      hardcoded path or extension list.
#   2. `@/<kind>/<self-id>` addressable-item citations must resolve through
#      `jit item show`, but only for a `<kind>` that is a live registered item
#      kind or alias (derived from `item_kinds` config). This skips the
#      reserved `issue` scope segment — `@/issue/REQ-01` and friends are
#      documented parse-error examples, not project-scope citations.
#
# Placeholder filter: a pattern CLASS, never a file allowlist (REQ-01). Path
# notation containing `<…>`, `{…}`, `}` or a glob `*` (e.g. `.jit/issues/{id}.json`,
# `.jit/schemas/*.json`) is placeholder syntax and is suppressed mechanically.
# A mis-cited real path that is NOT placeholder notation is reported.
#
# Usage:
#   docs-check-citations.sh [PATH ...]
# With no arguments it defaults to the full adopter-facing documentation
# surface. Area audits pass their own space-separated file/dir footprint.
#
# Exit codes:
#   0 — no dangling items and no missing file citations
#       (prints "OK: all cited paths and @/ items resolve")
#   1 — one or more MISSING: / DANGLING: findings
#   2 — usage/environment error

if [ "$#" -eq 0 ]; then
  set -- docs README.md INSTALL.md mcp-server/README.md web/README.md
fi

command -v jit >/dev/null 2>&1 || {
  echo "docs-check-citations: 'jit' not found on PATH (needed to resolve @/ items)" >&2
  exit 2
}
command -v jq >/dev/null 2>&1 || {
  echo "docs-check-citations: 'jq' not found on PATH (needed to enumerate item kinds)" >&2
  exit 2
}

# Live set of top-level repository roots a real path citation can begin with.
# Prefer tracked entries (excludes VCS metadata and gitignored machine-local
# files); fall back to the working-tree listing minus .git outside a repo.
if tracked=$(git ls-files 2>/dev/null) && [ -n "$tracked" ]; then
  top_entries=$(printf '%s\n' "$tracked" | sed 's#/.*##' | sort -u)
else
  top_entries=""
  for e in * .[!.]* ..?*; do
    { [ -e "$e" ] || [ -L "$e" ]; } || continue
    [ "$e" = ".git" ] && continue
    top_entries+="$e"$'\n'
  done
fi

# Live set of addressable-item leaders (registered kind names + their aliases).
# The reserved `issue` scope segment is absent by construction.
leaders=$(jit config get item_kinds | jq -r 'to_entries[] | ([.key] + (.value.aliases // [])) | .[]' | sort -u)

status=0

# 1. File-path citations in backtick spans.
# shellcheck disable=SC2016  # the backticks in the grep regex are literal, not expansion
while IFS= read -r token; do
  [ -n "$token" ] || continue
  # Strip an optional `:line` / `:line-range` suffix before testing existence.
  path=${token%%:[0-9]*}
  # Placeholder / glob notation is a pattern class, not a defect.
  case "$path" in
    *'<'* | *'{'* | *'}'* | *'*'*) continue ;;
  esac
  # First segment must be a real tracked repo root.
  first=${path%%/*}
  printf '%s\n' "$top_entries" | grep -qxF "$first" || continue
  # Last segment must carry an extension — a file citation, not a directory.
  last=${path##*/}
  case "$last" in
    *.*) ;;
    *) continue ;;
  esac
  [ -e "$path" ] || {
    echo "MISSING: $token"
    status=1
  }
done < <(grep -rhoE '`[^`]+`' "$@" | tr -d '`' | grep '/' | sort -u)

# 2. Addressable-item citations under a registered kind.
while IFS= read -r item; do
  [ -n "$item" ] || continue
  kind=${item#@/}
  kind=${kind%%/*}
  printf '%s\n' "$leaders" | grep -qxF "$kind" || continue
  jit item show "$item" >/dev/null 2>&1 || {
    echo "DANGLING: $item"
    status=1
  }
done < <(grep -rhoE '@/[a-z][a-z-]*/[a-zA-Z0-9-]+' "$@" | sort -u)

if [ "$status" -eq 0 ]; then
  echo "OK: all cited paths and @/ items resolve"
fi
exit "$status"
