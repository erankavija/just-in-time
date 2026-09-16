#!/bin/zsh
# bench.sh <jit-binary> <label> ; runs each scenario on a fresh copy, prints scenario, git spawns, wall ms
JIT=$1; LABEL=$2; B=${BENCH_DIR:?set BENCH_DIR to the directory holding jit1071-445, gf2-copy, and wrap/git}
run() { # name repo cmd...
  local name=$1 repo=$2; shift 2
  rm -rf $B/work && cp -a $B/$repo $B/work
  cd $B/work
  [[ -n $SETUP ]] && eval "$JIT $SETUP" >/dev/null 2>&1
  export GIT_COUNT_LOG=$B/work.gitlog; : > $GIT_COUNT_LOG
  local t0=$(date +%s%N)
  PATH=$B/wrap:$PATH $JIT "$@" >/dev/null 2>$B/work.err || { echo "$name FAILED: $(head -2 $B/work.err)"; }
  local t1=$(date +%s%N)
  printf '%s | %s | %d | %d ms\n' $LABEL $name $(wc -l < $GIT_COUNT_LOG) $(( (t1-t0)/1000000 ))
  cd $B
}
A=$(jq -r '.all_ids[0]' $B/jit1071-445/.jit/index.json)
C=$(jq -r '.all_ids[1070]' $B/jit1071-445/.jit/index.json)
run "jit1071-445 dep add (published, --reduce)" jit1071-445 dep add --reduce 3342dc7c e748afbb
run "jit1071-445 dep add (refused as redundant)" jit1071-445 dep add 3342dc7c e748afbb
run "gf2 idempotent label update" gf2-copy issue update -q 0064b0a2 -l epic:zen3-cpu-performance
