#!/usr/bin/env zsh
# Usage: bench.sh <jit-binary> <label>
# Times issue mutations on fresh tmpfs copies of gf2 and just-in-time.
zmodload zsh/datetime
set -u
BIN=$1 LABEL=$2
S=${0:A:h}
WORK=$S/bench-$LABEL
rm -rf $WORK && mkdir -p $WORK

prepare() { # <name> <source-repo>
  git clone -q --shared $2 $WORK/$1
  rsync -a --delete $2/.jit/ $WORK/$1/.jit/
}

elapsed_ms() { # runs "$@", prints wall ms and exit code
  typeset -F s=$EPOCHREALTIME
  "$@" >/dev/null 2>>$WORK/stderr.log
  local rc=$?
  typeset -F d=$(( (EPOCHREALTIME - s) * 1000 ))
  printf "%.0f %d\n" $d $rc
}

median() { print -l "$@" | sort -n | sed -n 2p; }

scenario() { # <repo-name> <scenario> <cmd...> ; three runs, prints median
  local repo=$1 name=$2; shift 2
  local -a times; local rc
  for i in 1 2 3; do
    local out=($(cd $WORK/$repo && elapsed_ms "$@"))
    times+=($out[1]); rc=$out[2]
  done
  printf "%-6s %-22s %6s ms  (runs: %s; last exit %s)\n" $repo $name $(median $times) "${(j:, :)times}" $rc
}

prepare gf2 ~/Projects/gf2
prepare jit ~/Projects/just-in-time

for repo id in gf2 79873126 jit a68eb999; do
  if [[ $repo == jit ]]; then
    # This repository enforces leases; the refused case runs before acquiring one.
    scenario $repo lease-refused $BIN issue update $id --label team:perf-bench
    (cd $WORK/$repo && $BIN claim acquire $id --agent-id agent:bench >/dev/null)
    export JIT_AGENT_ID=agent:bench
  fi
  changing=() codes=()
  for i in 1 2 3; do
    out=($(cd $WORK/$repo && elapsed_ms $BIN issue update $id --label team:perf-bench))
    changing+=($out[1]) codes+=($out[2])
    (cd $WORK/$repo && $BIN issue update $id --remove-label team:perf-bench >/dev/null 2>&1)
  done
  printf "%-6s %-22s %6s ms  (runs: %s; exits %s)\n" $repo changing-label $(median $changing) "${(j:, :)changing}" "${(j:,:)codes}"
  (cd $WORK/$repo && $BIN issue update $id --label team:perf-bench >/dev/null 2>&1)
  scenario $repo idempotent-label $BIN issue update $id --label team:perf-bench
  unset JIT_AGENT_ID
done
