#!/usr/bin/env zsh
# land-pipeline.zsh <short-id> [gate...]
# Merge main into the issue's worktree branch, run cargo-ci there, land on main,
# then evaluate the issue's gates on main. Waits at most 30 minutes for load1 < 4
# before CI (user ruling: bounded; exit 75 budget faults are the known
# environment fault, fixed by f487aeef).
set -u
id=$1; shift
gates=("$@"); (( ${#gates} )) || gates=(cargo-ci code-review)
repo=/home/vkaskivuo/Projects/just-in-time
wt=$repo/.agents/worktrees/agent-$id
log=$repo/.agents/worktrees/ci-$id.log
export JIT_AGENT_ID=agent:9876a23a-lead
export CARGO_INCREMENTAL=0
STOP=/tmp/claude-1000/-home-vkaskivuo-Projects-just-in-time/04c36bd3-9126-41e7-a804-3c2880c8e2ec/scratchpad/ENOSPC-STOP
[ -e $STOP ] && { echo "$id skipped: ENOSPC stop file present"; exit 3; }

# Authoritative runs use a lead-owned directory on /dev/shm (steward rule: /tmp
# is out of inodes); it is created before and removed after each run.
CI_TMP=/dev/shm/jit-ci/9876a23a
ci_tmp_begin() {
  mkdir -p /dev/shm/jit-ci/9876a23a
  # Use sccache only through an already running daemon, so no run starts it
  # under this run's TMPDIR (steward rule).
  if ps -C sccache >/dev/null; then unset CARGO_CI_NO_SCCACHE; else export CARGO_CI_NO_SCCACHE=1; echo "no sccache daemon: running without sccache"; fi
}
ci_tmp_end() { rm -rf /dev/shm/jit-ci/9876a23a; }

wait_load() {
  local deadline=$(( $(date +%s) + 1800 ))
  until awk '{exit !($1<4)}' /proc/loadavg || [ $(date +%s) -ge $deadline ]; do sleep 60; done
}

if [ -d $wt ]; then
  cd $wt || exit 1
  git merge -q --no-edit main || { echo "$id CONFLICT: $(git diff --name-only --diff-filter=U | tr '\n' ' ')"; exit 1; }
  wait_load
  ci_tmp_begin; TMPDIR=$CI_TMP flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock ./scripts/cargo-ci.sh > $log 2>&1; r=$?; ci_tmp_end
  echo "$id worktree cargo-ci exit $r"; tail -3 $log
  grep -E "^\s+FAIL|^error" $log | sort -u | head -3
  grep -q "No space left on device" $log && { echo "$id ENOSPC in authoritative branch CI"; touch $STOP; exit 2; }
  [ $r = 0 ] || exit 1
  cd $repo && JIT_LOCK_TIMEOUT=1800 .agents/skills/jit-execution-lead/scripts/land.sh worktree-agent-$id 2>&1 | tail -2
  echo "$id landed $(git log --oneline -1)"
fi
cd $repo
jit claim acquire $id --ttl 7200 --agent-id agent:9876a23a-lead >/dev/null 2>&1
for g in $gates; do
  [ $g = cargo-ci ] && wait_load
  if [ $g = cargo-ci ]; then
    ci_tmp_begin
    out=$(TMPDIR=$CI_TMP flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock jit gate evaluate $id $g ${=FORCE:-} 2>&1 | tail -1)
    ci_tmp_end
    echo "$id gate $g: $out"
    r=$(ls -t $repo/.jit/gate-runs/*/result.json | head -20 | xargs grep -l "\"issue_id\": \"$id" | head -1)
    [ -n "$r" ] && jq -r .stdout $r | grep -q "No space left on device" && { echo "$id ENOSPC in cargo-ci gate run $r"; touch $STOP; exit 2; }
  else
    echo "$id gate $g: $(jit gate evaluate $id $g 2>&1 | tail -1)"
  fi
done
