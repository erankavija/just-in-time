#!/usr/bin/env bash
set -euo pipefail

# docs-check-citations — source-path + `@/…` citation-existence check.
# Mechanical check M3 of the `docs-mechanical` gate (epic 2d109173).
#
# Two comparisons, both derived live so the check encodes no product facts:
#   1. File-path tokens inside backtick spans must exist on disk. A token is
#      treated as a file citation — a pattern class, not an allowlist — when
#      (a) it contains a `/`, (b) its final segment carries an extension
#      (a `.<ext>` suffix), and (c) it is not placeholder/glob notation. Any
#      such token that does not exist on disk is a MISSING finding, regardless
#      of whether its leading segment names a tracked repo root: a citation
#      under a misspelled/nonexistent root (e.g. `crate/jit/src/foo.rs`) is
#      exactly the dangling reference REQ-01 requires us to surface. The
#      extension requirement is deliberate — an extensionless token is a
#      directory / conceptual reference, mechanically out of reach, so those
#      are deferred to the doc-review reviewer rather than flagged here. This
#      rejects prose, flags, and MIME types (`application/json`) without any
#      hardcoded path or extension list. Broad flagging that surfaces
#      auditor-adjudicated lines is intended (plan §2 M3).
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
#   docs-check-citations.sh PATH [PATH ...]
# The footprint is a REQUIRED space-separated list of files/dirs — the checker
# encodes no default path list (that would be a product fact, REQ-01). The gate
# entrypoint (docs-mechanical.sh) derives the whole-surface footprint live and
# passes it in; area audits pass their own.
#
# Exit codes:
#   0 — no dangling items and no missing file citations
#       (prints "OK: all cited paths and @/ items resolve")
#   1 — one or more MISSING: / DANGLING: findings
#   2 — usage/environment error

if [ "$#" -eq 0 ]; then
  echo "usage: docs-check-citations.sh PATH [PATH ...]" >&2
  exit 2
fi

command -v jit >/dev/null 2>&1 || {
  echo "docs-check-citations: 'jit' not found on PATH (needed to resolve @/ items)" >&2
  exit 2
}
command -v jq >/dev/null 2>&1 || {
  echo "docs-check-citations: 'jq' not found on PATH (needed to enumerate item kinds)" >&2
  exit 2
}

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
  # Placeholder / glob notation is a pattern class, not a defect (REQ-01).
  case "$path" in
    *'<'* | *'{'* | *'}'* | *'*'*) continue ;;
  esac
  # Must look like a file path: contains a '/' (the feed already requires this)
  # and the final segment carries an extension. No leading-segment / tracked-root
  # filter — a citation under a misspelled or nonexistent root is a real dangling
  # reference and must be surfaced. An extensionless token is a directory /
  # conceptual reference, mechanically out of reach: deferred to doc-review.
  case "$path" in */*) ;; *) continue ;; esac
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
