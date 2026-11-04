#!/usr/bin/env bash
# Dry run of the format 4 rewrite over copies of repositories. Writes only
# under this checkout's target/format4-dry/ and only reads each repository.
#
# Usage: format4-dry-run.sh <repository>...
#
# Each repository's tracked tree is cloned at its HEAD, and its .jit and
# <common-dir>/jit are copied over the clone with plain reads, taking no lock.
# On each copy it runs format4-rewrite.py twice, then, with the jit built from
# this checkout, jit validate, gate status (latest and --all) for the gate
# with the most runs on the issue with the most runs, claim list, and an
# archive container preview of a target the rewrite seeded. It prints one row
# of counts per repository and exits nonzero if any step failed.
# shellcheck disable=SC2329 # the checks run through step()
set -uo pipefail

[ $# -ge 1 ] || { echo "usage: $0 <repository>..."; exit 2; }
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(git -C "$HERE" rev-parse --show-toplevel)
REWRITE=$HERE/format4-rewrite.py
DRY=$ROOT/target/format4-dry
JIT=$ROOT/target/debug/jit
FAILED=0

(cd "$ROOT" && CARGO_INCREMENTAL=0 cargo build --quiet -p jit) || { echo "cargo build failed"; exit 1; }
rm -rf "$DRY"
mkdir -p "$DRY/logs"

tree_digest() {
  local jit_dir
  jit_dir=$(git -C "$1" rev-parse --path-format=absolute --git-common-dir)/jit
  (cd "$1" && find .jit "$jit_dir" -type f -print0 | sort -z | xargs -0 sha256sum; find .jit "$jit_dir" -type d | sort) | sha256sum
}
json_count() { python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))["count"])' "$1" 2>/dev/null || echo "?"; }
# The issue with the most runs, and its gate with the most runs.
busiest_gate() {
  python3 -I - "$1" <<'EOF'
import collections, json, sys
from pathlib import Path
root = Path(sys.argv[1]) / ".jit" / "gate-runs"
issue = max((d for d in root.iterdir() if d.name != "verdicts"), key=lambda d: len(list(d.iterdir())), default=None)
if issue is not None:
    gates = collections.Counter(json.loads(p.read_bytes())["gate_key"] for p in issue.iterdir())
    print(issue.name, *gates.most_common(1)[0])
EOF
}
step() { # <label> <command...>: run, record pass or failure
  local label=$1; shift
  if "$@"; then RESULT+=" $label=ok"; else RESULT+=" $label=FAIL"; FAILED=1; fi
}
# Checks over the current copy; each reads the globals the loop sets.
second_run_unchanged() {
  python3 -I "$REWRITE" "$copy" >"$log-rewrite-2.out" 2>&1 && [ "$before" = "$(tree_digest "$copy")" ]
}
validate() { timeout 600 "$JIT" validate >"$log-validate.out" 2>&1; }
gate_status() { "$JIT" gate status "$issue" "$gate" --json >"$log-gate.json" 2>&1; }
gate_status_all() {
  "$JIT" gate status "$issue" --gate "$gate" --all --json >"$log-gate-all.json" 2>&1 &&
    [ "$(json_count "$log-gate-all.json")" = "$gate_runs" ]
}
claim_list() { "$JIT" claim list --json >"$log-claims.json" 2>&1; }
archive_preview() { "$JIT" archive container "$(basename "$target" .json)" --json >"$log-archive.json" 2>&1; }

printf '%s\n' "| repo | HEAD | runs | source_issue | run_id set | verdicts | evidence | markers removed | leases | claims.seq | checks |" "|---|---|---|---|---|---|---|---|---|---|---|"
for repo in "$@"; do
  name=$(basename "$repo")
  copy=$DRY/$name
  log=$DRY/logs/$name
  if ! { git clone --quiet --no-hardlinks --no-checkout "$repo" "$copy" &&
    git -C "$copy" checkout --quiet --detach "$(git -C "$repo" rev-parse HEAD)" &&
    rm -rf "$copy/.jit" && cp -a "$repo/.jit" "$copy/.jit" &&
    src_jit=$(git -C "$repo" rev-parse --path-format=absolute --git-common-dir)/jit &&
    dst_jit=$(git -C "$copy" rev-parse --path-format=absolute --git-common-dir)/jit &&
    { [ ! -d "$src_jit" ] || cp -a "$src_jit" "$dst_jit"; }; }; then
    echo "| $name | copy failed |"; FAILED=1; continue
  fi
  RESULT=""
  summary=$(python3 -I "$REWRITE" "$copy" 2>&1)
  rc=$?
  echo "$summary" >"$log-rewrite.out"
  [ "$rc" -eq 0 ] || { echo "| $name | rewrite refused: ${summary:0:200} |"; FAILED=1; continue; }
  before=$(tree_digest "$copy")
  step second-run-unchanged second_run_unchanged
  cd "$copy" || exit 1
  step validate validate
  issue=""
  read -r issue gate gate_runs < <(busiest_gate "$copy")
  if [ -n "${issue:-}" ]; then
    step gate-status gate_status
    step gate-status-all gate_status_all
  else
    RESULT+=" gate-status=no-runs"
  fi
  step claim-list claim_list
  target=$(find .jit/archive-evidence/container -name '*.json' 2>/dev/null | sort | head -1)
  if [ -n "$target" ]; then
    step archive-preview archive_preview
  else
    RESULT+=" archive-preview=no-seeded-target"
  fi
  cd "$DRY" || exit 1
  python3 -I - "$name" "$(git -C "$repo" rev-parse --short HEAD)" "$summary" "$RESULT" "$(json_count "$log-claims.json")" <<'EOF'
import re, sys
name, head, summary, result, leases = sys.argv[1:]
def number(pattern):
    match = re.search(pattern, summary)
    return match.group(1) if match else "?"
print(" | ".join([
    f"| {name}", head,
    number(r"of (\d+) run"), number(r"source_issue on (\d+)"), number(r"run_id on (\d+) gate"),
    number(r"(\d+) verdict"), number(r"(\d+) evidence"), number(r"removed (\d+) torn"),
    leases, number(r"claims.seq (\d+)"), result.strip() + " |",
]))
EOF
done
echo "logs in $DRY/logs"
exit "$FAILED"
