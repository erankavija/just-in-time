#!/usr/bin/env bash
set -euo pipefail

# generate-shipped-policy-regions — render the shipped development-area
# classification into the two adopter configuration files.
#
# Each target holds one region, bounded by markers in that file's own comment
# syntax:
#   docs/reference/configuration.md
#     <!-- jit:shipped-documentation-policy:begin -->  … :end -->
#   docs/reference/example-config.toml
#     # jit:shipped-documentation-policy:begin  …  # jit:…:end
#
# Everything outside the markers is preserved byte for byte; nothing inside
# them is authored. A region carries the `[documentation]` table and nothing
# else — fenced as a code block in the markdown reference, bare in the TOML
# example — so the prose and comments that explain the table stay hand-written
# around it.
#
# WHERE THE VALUES COME FROM. The classification is declared once, in the
# `jit-default` package's `[documentation]` contributions under `profiles/` in
# this checkout. The route to it that respects the dogfooding boundary is to
# initialize a throwaway repository in a temporary directory with that package
# applied, and read the table written there. This repository's own
# `.jit/config.toml` and the effective-configuration report are not sources:
# both describe the policy this repository runs under, which agrees with the
# shipped classification by a hand-maintained claim rather than by binding
# (`@/issue/e204e63d/decision/D-4`).
#
# The package is copied into the throwaway repository before it is applied,
# because an application records the package's worktree-relative location and
# refuses a package read from outside the worktree it is applied to.
#
# Usage:
#   generate-shipped-policy-regions.sh    (takes no arguments — targets are named above)
#
# Exit codes:
#   0 — both regions hold the shipped classification
#   1 — the splice or the publication failed
#   2 — refused to write, or an environment/usage error
#
# The boundary between the two failure codes is whether the classification was
# in hand. Everything up to and including reading it out of the throwaway
# repository decides whether this run is entitled and able to start at all, and
# reports 2; from the splice onward the values exist and the only question is
# whether a target can be brought to hold them, which reports 1. The sibling
# entry points draw it in the same place.

me=${0##*/}

# Refuse, before anything has been produced.
die() {
  echo "$me: $*" >&2
  exit 2
}

# Fail with the classification already in hand: a target could not be spliced
# or the result could not be published.
fail() {
  echo "$me: $*" >&2
  exit 1
}

[ "$#" -eq 0 ] || die "takes no arguments (got: $*)"

command -v git >/dev/null 2>&1 || die "'git' not found on PATH"
command -v jit >/dev/null 2>&1 || die "'jit' not found on PATH"

here=$(cd "$(dirname "$0")" && pwd)
root=$(git -C "$here" rev-parse --show-toplevel 2>/dev/null) ||
  die "not inside a git work tree (needed to locate the targets and the package)"

reference="docs/reference/configuration.md"
example="docs/reference/example-config.toml"
region="shipped-documentation-policy"
package="jit-default"
package_source="profiles/$package"
# Where the package is placed inside the throwaway repository, worktree-relative
# because that is the form a `path:` selector and applied record use.
package_location="packages/$package"

# --- the shipped table, read out of a throwaway repository --------------------

tmp=$(mktemp -d) || die "could not create a temporary directory"
trap 'rm -rf "$tmp" "$root/$reference.jit-region" "$root/$example.jit-region"' EXIT
mkdir -p "$tmp/scaffold/$package_location"

[ -f "$root/$package_source/manifest.toml" ] ||
  die "$package_source carries no manifest.toml in $root — the classification's declaration is not where this script looks for it"
cp -R "$root/$package_source/." "$tmp/scaffold/$package_location/" ||
  die "could not place the $package package inside the throwaway repository"

# `JIT_DATA_DIR` is cleared so discovery cannot reach out of the temporary
# directory.
(cd "$tmp/scaffold" &&
  env -u JIT_DATA_DIR jit init --profile "path:$package_location" --quiet) >/dev/null 2>&1 ||
  die "'jit init --profile path:$package_location' failed in the throwaway repository"

scaffold_config="$tmp/scaffold/.jit/config.toml"
[ -f "$scaffold_config" ] || die "'jit init' wrote no $scaffold_config"

# The table runs from its header to the first blank line, comment, or following
# table, which is the shape the scaffold writes it in.
table=$(awk '
  /^\[documentation\]$/ { inside = 1; print; next }
  inside && (/^$/ || /^#/ || /^\[/) { exit }
  inside { print }
' "$scaffold_config")

for key in development_root archive_root managed_paths permanent_paths issue_scoped_areas; do
  printf '%s\n' "$table" | grep -q "^$key = " ||
    die "the written [documentation] table carries no '$key' — the extraction no longer matches what applying $package writes"
done

# shellcheck disable=SC2016  # The backticks are a literal markdown code fence.
printf '```toml\n%s\n```\n' "$table" >"$tmp/reference-region"
printf '%s\n' "$table" >"$tmp/example-region"

# --- splice ------------------------------------------------------------------

changed=0

# Replace the bytes between the markers, preserving every other byte. A target
# whose markers are absent, duplicated, or out of order is left untouched.
#
# Every failure here reports 1: the classification is already in hand, so what
# fails is producing the target's new bytes or publishing them, never the
# entitlement to run.
splice() {
  local target="$1" begin="$2" end="$3" payload="$4"
  local path="$root/$target" staged="$root/$target.jit-region"

  [ -f "$path" ] || fail "$target does not exist"

  cp -p "$path" "$staged" || fail "could not stage $target"
  if ! awk -v begin="$begin" -v end="$end" -v payload="$payload" '
    $0 == begin {
      begins++
      if (state != 0) { bad = 1 }
      print
      state = 1
      while ((status = (getline line < payload)) > 0) { print line }
      if (status < 0) { bad = 1 }
      close(payload)
      next
    }
    $0 == end {
      ends++
      if (state != 1) { bad = 1 }
      print
      state = 0
      next
    }
    state == 0 { print }
    END { if (begins != 1 || ends != 1 || state != 0 || bad) { exit 3 } }
  ' "$path" >"$staged"; then
    rm -f "$staged"
    fail "$target does not carry exactly one well-formed '$begin' … '$end' region"
  fi

  if cmp -s "$staged" "$path"; then
    rm -f "$staged"
    return 0
  fi
  mv -f "$staged" "$path" ||
    fail "could not publish $target: the spliced bytes were complete and the rename to $target failed"
  echo "updated: $target"
  changed=1
}

splice "$reference" "<!-- jit:$region:begin -->" "<!-- jit:$region:end -->" "$tmp/reference-region"
splice "$example" "# jit:$region:begin" "# jit:$region:end" "$tmp/example-region"

if [ "$changed" -eq 0 ]; then
  echo "OK: shipped-policy regions already carry the shipped classification"
else
  echo "OK: shipped-policy regions rendered from the shipped classification"
fi
