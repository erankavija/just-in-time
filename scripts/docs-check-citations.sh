#!/usr/bin/env bash
set -euo pipefail

# docs-check-citations — source-path + `@/…` citation-existence check.
# Mechanical check M3 of the `docs-mechanical` gate (epic 2d109173).
#
# Two comparisons, both derived live so the check encodes no product facts:
#   1. Repo-rooted file-path tokens inside backtick spans must exist on disk. A
#      backtick token is reported MISSING iff ALL of the following hold — each a
#      pattern class, never a file allowlist (REQ-01):
#        (a) it is NOT placeholder / glob notation (`<…>`, `{…}`, `*`);
#        (b) it is NOT an external / command token — it does not start with `~`
#            (home-relative), does not start with `/` (absolute), does not
#            contain `://` (URL / scheme), and contains no whitespace or `|`
#            (a shell command line, not a path);
#        (c) it contains a `/` AND its FIRST path segment is a real tracked
#            top-level repo entry (the live set is `git ls-files` first segments,
#            derived below) — this is the repo-root requirement;
#        (d) it does NOT exist on disk — REGARDLESS of whether the final segment
#            carries an extension, so an extensionless repo-rooted citation such
#            as `crates/<crate>/NOTICE` is caught, not skipped.
#   2. `@/<kind>/<self-id>` addressable-item citations must resolve through
#      `jit item show`, but only for a `<kind>` that is a live registered item
#      kind or alias (derived from `item_kinds` config). This skips the
#      reserved `issue` scope segment — `@/issue/REQ-01` and friends are
#      documented parse-error examples, not project-scope citations.
#
# DEFERRED to the semantic doc-review reviewer (NOT silently ignored) — per the
# plan's auditor-adjudicated M3 design (`dev/active/<plan>.md` §2, M3):
#   (i)   bare filenames with no `/` (e.g. `Cargo.toml` in prose);
#   (ii)  tokens rooted at a segment that is NOT a tracked repo top-level entry —
#         relative-to-subdirectory citations (`schemas/spec-body.json`,
#         `lib/cli-executor.js`) and misspelled-root citations
#         (`crate/jit/src/foo.rs`); and
#   (iii) trailing-slash directory references (e.g. `.jit/config/gate-presets/`,
#         a lazily-created config dir) — directory / structural references are
#         conceptual, not file citations, so they are out of mechanical reach.
# The mechanical check cannot distinguish those from ordinary prose or from
# paths relative to some other cwd without producing false positives, so it
# leaves them to the reviewer rather than flagging them. The repo-root
# requirement in (c) is exactly what keeps this check low-false-positive: it
# fired 13 spurious MISSING lines on the adopter surface (external `~/.config`
# and `/etc` paths, a `curl … | jq` command, subdir-relative paths) when an
# earlier revision dropped it.
#
# Usage:
#   docs-check-citations.sh PATH [PATH ...]
# The footprint is a REQUIRED space-separated list of files/dirs — the checker
# encodes no default path list (that would be a product fact, REQ-01). The gate
# entrypoint (docs-mechanical.sh) resolves the whole-surface footprint and
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

# A footprint entry that resolves to nothing — nonexistent OR unreadable — is a
# usage/environment error, not a clean pass: grep over such a path would emit a
# discarded error and the checker would exit 0 without inspecting anything (a
# false-green gate).
for fp in "$@"; do
  [ -e "$fp" ] || {
    echo "docs-check-citations: footprint path does not exist: $fp" >&2
    exit 2
  }
  [ -r "$fp" ] || {
    echo "docs-check-citations: footprint path is not readable: $fp" >&2
    exit 2
  }
  # Directories additionally need traversal (execute) to be walked by grep -r.
  { [ ! -d "$fp" ] || [ -x "$fp" ]; } || {
    echo "docs-check-citations: footprint directory is not traversable: $fp" >&2
    exit 2
  }
done

# Live set of addressable-item leaders (registered kind names + their aliases).
# The reserved `issue` scope segment is absent by construction.
leaders=$(jit config get item_kinds | jq -r 'to_entries[] | ([.key] + (.value.aliases // [])) | .[]' | sort -u)

# Live set of tracked top-level repo entries: the first path segment of every
# tracked file (so `crates`, `docs`, `README.md`, `.jit`, …). Derived, not a
# hardcoded list — it re-derives itself as the repo layout changes (REQ-01).
roots=$(git ls-files | awk -F/ '{print $1}' | sort -u)

status=0

# Scan the footprint once for backtick spans and once for @/ citations, in the
# MAIN shell (not a process substitution) so a grep error can exit the script.
# The `|| rc=$?` idiom captures grep's exit without tripping `set -e` on grep's
# normal exit 1 (no matches). A grep exit >= 2 (e.g. an unreadable file met
# during recursion) is an environment error; surface it as exit 2 rather than
# letting the checker go false-green.
grc=0
# Generated skill evaluation evidence and profile install-time projection
# inputs are not live repository prose. Evaluation fixtures preserve
# disposable paths; install assets contain destination-scoped item ids that
# resolve only after the package is applied. Match their complete structural
# paths rather than suppressing every directory with a common basename.
scan_matches() {
  local regex=$1 raw line path match
  shift
  grc=0
  raw=$(grep -rHoE "$regex" "$@") || grc=$?
  [ "$grc" -ge 2 ] && return 2
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    path=${line%%:*}
    match=${line#*:}
    case "/$path/" in
      */.agents/skills/*/evals/* | */profiles/*/assets/install/*) continue ;;
    esac
    printf '%s\n' "$match"
  done <<<"$raw"
}

# shellcheck disable=SC2016  # the backticks in the grep regex are literal, not expansion
backtick_raw=$(scan_matches '`[^`]+`' "$@") || {
  echo "docs-check-citations: read error scanning footprint for citations" >&2
  exit 2
}
item_raw=$(scan_matches '@/[a-z][a-z-]*/[a-zA-Z0-9-]+' "$@") || {
  echo "docs-check-citations: read error scanning footprint for @/ items" >&2
  exit 2
}

# 1. Repo-rooted file-path citations in backtick spans (see header for the
# four-part rule and the deferral classes).
# shellcheck disable=SC2016  # the backticks in the grep regex are literal, not expansion
while IFS= read -r token; do
  [ -n "$token" ] || continue
  # Strip an optional `:line` / `:line-range` suffix before testing existence.
  path=${token%%:[0-9]*}
  # (a) Placeholder / glob notation is a pattern class, not a defect.
  case "$path" in
    *'<'* | *'{'* | *'}'* | *'*'*) continue ;;
  esac
  # Trailing-slash directory reference — conceptual / structural, deferred.
  case "$path" in */) continue ;; esac
  # (b) External / command tokens by pattern class: home-relative, absolute,
  # a URL/scheme, or a shell command line (whitespace or a pipe). Deferred.
  case "$path" in
    '~'* | /* | *'://'* | *'|'* | *[[:space:]]*) continue ;;
  esac
  # (c) Must contain a '/' (the feed requires this) AND its first segment must
  # be a real tracked top-level repo entry. A bare filename or a token rooted at
  # a non-repo / misspelled segment is deferred to the doc-review reviewer.
  case "$path" in */*) ;; *) continue ;; esac
  first=${path%%/*}
  printf '%s\n' "$roots" | grep -qxF -- "$first" || continue
  # (d) Existence — regardless of whether the final segment has an extension.
  [ -e "$path" ] || {
    echo "MISSING: $token"
    status=1
  }
done < <(printf '%s\n' "$backtick_raw" | tr -d '`' | grep '/' | sort -u)

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
done < <(printf '%s\n' "$item_raw" | sort -u)

if [ "$status" -eq 0 ]; then
  echo "OK: all cited paths and @/ items resolve"
fi
exit "$status"
