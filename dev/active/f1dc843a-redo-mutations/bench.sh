#!/usr/bin/env zsh
# Usage: bench.sh <jit-binary> <label>
#
# Times issue mutations on fresh copies of gf2 and just-in-time, and on a third
# fixture carrying just-in-time's issues and index under an event log inflated
# to ten times the bytes. Wall clock only, median of three runs.
#
# The inflated log is built during the sync step, by replaying the source log's
# own lines under fresh event ids, so the fixture is a real data root, every
# line parses, and no id repeats (@/inv/event-log). The harness verifies all
# three before measuring.
#
# Fixtures are built beside this script, so run it from a copy on tmpfs. Every
# absolute path it needs is listed under "path dependencies" in its output and
# checked before the first fixture is built.
zmodload zsh/datetime
zmodload -F zsh/stat b:zstat
set -u

(( $# == 2 )) || { print -u2 "bench.sh: usage: bench.sh <jit-binary> <label>"; exit 2 }
BIN=${commands[$1]:-${1:A}} LABEL=$2
S=${0:A:h}
WORK=$S/bench-$LABEL

GF2_REPO=$HOME/Projects/gf2
JIT_REPO=$HOME/Projects/just-in-time
LOG=.jit/events.jsonl
GIT=${commands[git]:-} RSYNC=${commands[rsync]:-} PYTHON=${commands[python3]:-}

fail() { print -u2 "bench.sh: $*"; exit 1 }

require() { # <what> <path>; the path must exist
  [[ -n $2 && -e $2 ]] || fail "missing $1: ${2:-not on PATH}"
}

FIXTURES=(gf2 jit jit-10x)
typeset -A FIXTURE_SOURCE=(gf2 $GF2_REPO jit $JIT_REPO jit-10x $JIT_REPO)
typeset -A FIXTURE_ISSUE=(gf2 79873126 jit a68eb999 jit-10x a68eb999)
typeset -A FIXTURE_LOG_MULTIPLE=(gf2 1 jit 1 jit-10x 10)
typeset -A SOURCE_LOG_BYTES FIXTURE_LOG_BYTES MEDIAN

print "path dependencies"
printf "  %-14s %s\n" jit-binary "$BIN" git "$GIT" rsync "$RSYNC" python3 "$PYTHON" \
  gf2-repo "$GF2_REPO" gf2-log "$GF2_REPO/$LOG" jit-repo "$JIT_REPO" jit-log "$JIT_REPO/$LOG" \
  work-root "$WORK"
require "jit binary" "$BIN"
[[ -x $BIN ]] || fail "jit binary is not executable: $BIN"
require git "$GIT"
require rsync "$RSYNC"
require python3 "$PYTHON"
for name repo in gf2 $GF2_REPO jit $JIT_REPO; do
  require "$name repository" $repo
  require "$name event log" $repo/$LOG
done
[[ -w $S ]] || fail "work root is not writable: $S"

rm -rf $WORK && mkdir -p $WORK

inflate_event_log() { # <log> <multiple>; replays the log's lines under fresh ids
  $PYTHON - $1 $2 <<'PY'
import json, sys, uuid

path, multiple = sys.argv[1], int(sys.argv[2])
with open(path, "rb") as log:
    source = log.read()
lines = [line for line in source.split(b"\n") if line.strip()]
for number, line in enumerate(lines, 1):
    try:
        json.loads(line)
    except ValueError as error:
        sys.exit(f"source event line {number} does not parse: {error}")
target = len(source) * multiple
with open(path, "ab") as log:
    written = len(source)
    while written < target:
        for line in lines:
            record = json.loads(line)
            if "id" in record:
                record["id"] = str(uuid.uuid4())
            written += log.write(json.dumps(record, separators=(",", ":")).encode() + b"\n")
            if written >= target:
                break
PY
}

verify_fixture() { # <fixture> <source-repo> <multiple>
  local fixture=$1 repo=$2 multiple=$3 root=$WORK/$1/.jit
  diff -rq $repo/.jit/issues $root/issues >/dev/null \
    || fail "$fixture: issue set differs from $repo"
  cmp -s $repo/.jit/index.json $root/index.json \
    || fail "$fixture: index differs from $repo"
  (( multiple > 1 )) || return 0
  (( FIXTURE_LOG_BYTES[$fixture] >= multiple * SOURCE_LOG_BYTES[$fixture] )) \
    || fail "$fixture: event log is under ${multiple}x the source log"
  # Uniqueness is the harness's to check; `jit validate` is the parser's verdict
  # on every line, and names the first one it rejects.
  $PYTHON - $root/events.jsonl <<'PY' || fail "$fixture: event log verification failed"
import collections, json, sys

ids = collections.Counter()
for number, line in enumerate(open(sys.argv[1], "rb"), 1):
    line = line.strip()
    if not line:
        continue
    try:
        record = json.loads(line)
    except ValueError as error:
        sys.exit(f"event line {number} does not parse: {error}")
    if "id" in record:
        ids[record["id"]] += 1
repeated = [event_id for event_id, count in ids.items() if count > 1]
if repeated:
    sys.exit(f"{len(repeated)} repeated event ids, first {repeated[0]}")
print(f"  {sum(ids.values())} event ids, all distinct")
PY
  (cd $WORK/$fixture && $BIN validate >/dev/null 2>>$WORK/stderr.log) \
    || fail "$fixture: jit validate rejects the fixture"
  print "  jit validate accepts $fixture"
}

prepare() { # <fixture>
  local fixture=$1 repo=$FIXTURE_SOURCE[$1] multiple=$FIXTURE_LOG_MULTIPLE[$1]
  $GIT clone -q --shared $repo $WORK/$fixture || fail "git clone of $repo failed"
  $RSYNC -a --delete $repo/.jit/ $WORK/$fixture/.jit/ || fail "rsync of $repo/.jit failed"
  SOURCE_LOG_BYTES[$fixture]=$(zstat -L +size $repo/$LOG)
  if (( multiple > 1 )); then
    inflate_event_log $WORK/$fixture/$LOG $multiple || fail "$fixture: inflating the event log failed"
  fi
  FIXTURE_LOG_BYTES[$fixture]=$(zstat -L +size $WORK/$fixture/$LOG)
  verify_fixture $fixture $repo $multiple
}

elapsed_ms() { # runs "$@", prints wall ms and exit code
  typeset -F s=$EPOCHREALTIME
  "$@" >/dev/null 2>>$WORK/stderr.log
  local rc=$?
  typeset -F d=$(( (EPOCHREALTIME - s) * 1000 ))
  printf "%.0f %d\n" $d $rc
}

median() { print -l "$@" | sort -n | sed -n 2p }

HELD_LEASE='import json, sys
leases = json.load(sys.stdin).get("leases", [])
held = any(l["issue_id"].startswith(sys.argv[1]) and not l["stale"] for l in leases)
sys.exit(0 if held else 1)'

claim_lease() { # <fixture> <issue>; the changing and idempotent runs need the lease
  local fixture=$1 id=$2 note rc
  note=$(cd $WORK/$fixture && $BIN claim acquire $id --agent-id agent:bench 2>&1 >/dev/null)
  rc=$?
  # The lease lands before the claim's repository half, so a nonzero exit still
  # admits the timed runs. Report it: on a large log it is the claim's own
  # whole-log capture failing, and it leaves the issue record unclaimed.
  printf "%-8s %-16s exit %d %s\n" $fixture claim-acquire $rc "${note%%$'\n'*}"
  (cd $WORK/$fixture && JIT_AGENT_ID=agent:bench $BIN claim status --json 2>/dev/null) |
    $PYTHON -c $HELD_LEASE $id || fail "$fixture: no active lease for $id after claim acquire"
}

scenario() { # <fixture> <scenario> <reset-cmd|-> <cmd...>; three runs, records the median
  local fixture=$1 name=$2 reset=$3; shift 3
  local -a times codes
  for i in 1 2 3; do
    local out=($(cd $WORK/$fixture && elapsed_ms "$@"))
    times+=($out[1]); codes+=($out[2])
    [[ $reset == - ]] || (cd $WORK/$fixture && ${=reset} >/dev/null 2>&1)
  done
  MEDIAN[$fixture/$name]=$(median $times)
  printf "%-8s %-16s %7s ms  (runs: %s; exits %s)\n" \
    $fixture $name $MEDIAN[$fixture/$name] "${(j:, :)times}" "${(j:,:)codes}"
}

print "\nfixtures"
for fixture in $FIXTURES; do
  print "  $fixture <- $FIXTURE_SOURCE[$fixture] (log x$FIXTURE_LOG_MULTIPLE[$fixture])"
  prepare $fixture
done

print "\nevent log bytes"
printf "  %-8s %12s %12s %9s\n" fixture source-log fixture-log multiple
for fixture in $FIXTURES; do
  printf "  %-8s %12d %12d %8.2fx\n" $fixture \
    $SOURCE_LOG_BYTES[$fixture] $FIXTURE_LOG_BYTES[$fixture] \
    $(( 1.0 * FIXTURE_LOG_BYTES[$fixture] / SOURCE_LOG_BYTES[$fixture] ))
done

print "\nruns"
for fixture in $FIXTURES; do
  id=$FIXTURE_ISSUE[$fixture]
  if [[ $FIXTURE_SOURCE[$fixture] == $JIT_REPO ]]; then
    # This repository enforces leases; the refused case runs before acquiring one.
    scenario $fixture lease-refused - $BIN issue update $id --label team:perf-bench
    claim_lease $fixture $id
    export JIT_AGENT_ID=agent:bench
  fi
  scenario $fixture changing-label "$BIN issue update $id --remove-label team:perf-bench" \
    $BIN issue update $id --label team:perf-bench
  (cd $WORK/$fixture && $BIN issue update $id --label team:perf-bench >/dev/null 2>&1)
  scenario $fixture idempotent-label - $BIN issue update $id --label team:perf-bench
  unset JIT_AGENT_ID
done

print "\nmedian ms, unmodified beside the ten-times log"
printf "  %-16s %8s %8s %8s %8s\n" scenario $FIXTURES ratio
for name in lease-refused changing-label idempotent-label; do
  base=${MEDIAN[jit/$name]:-0} scaled=${MEDIAN[jit-10x/$name]:-0}
  (( base > 0 )) && ratio=$(printf "%.2fx" $(( 1.0 * scaled / base ))) || ratio=-
  printf "  %-16s %8s %8s %8s %8s\n" $name \
    ${MEDIAN[gf2/$name]:--} ${MEDIAN[jit/$name]:--} ${MEDIAN[jit-10x/$name]:--} $ratio
done
