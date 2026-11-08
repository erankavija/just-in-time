#!/usr/bin/env zsh
# usage: land-one.sh <id>; serialized merge, CI and gates for one wave issue
id=$1; S=${0:A:h}; R=/home/vkaskivuo/Projects/just-in-time; W=/mnt/nvme/jit/agents/worktrees/agent-$id
export JIT_AGENT_ID=agent:df129d9e-lead CARGO_INCREMENTAL=0
# Authoritative runs use their own tmpfs; cargo-ci makes and removes a per-run
# directory inside it (d952d057).
export TMPDIR=/dev/shm/jit-ci/df129d9e
# Use the shared sccache daemon only when one is already running (started with a
# stable TMPDIR by the steward); never let this run start one under its per-run TMPDIR.
sccache_guard() { ps -C sccache -o pid= >/dev/null 2>&1 && unset CARGO_CI_NO_SCCACHE || { export CARGO_CI_NO_SCCACHE=1; echo "$id: no sccache daemon; building without it"; }; }
mkdir -p $TMPDIR
# flock -o holds the lock in the flock process and closes it for children, so a
# daemon a child spawns (sccache) cannot inherit and keep it.
[[ -n $LAND_LOCKED ]] || LAND_LOCKED=1 exec flock -o $S/land2.lock $0 $@
# Host-quiet hold: while $S/hold exists, start no new merge, CI or gate run.
while [[ -e $S/hold ]]; do sleep 30; done
cd $W || exit 2
git merge -q --no-edit main || { git merge --abort; echo "$id: CONFLICT merging main"; exit 3; }
[[ -z "$(git status --porcelain)" ]] || { echo "$id: worktree dirty"; git status --short | head; exit 4; }
# A host-fault run is retried at most once per landing (steward, D-46); a second
# failure is a verdict.
for try in 1 2; do
  sccache_guard
  flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock ./scripts/cargo-ci.sh > $S/ci-$id.log 2>&1
  rc=$?
  # Retry exit 75 (load), ENOSPC on the shared /tmp tmpfs, and failures made only of
  # TIMEOUTs (jobserver token waits inside nextest's timer).
  env_only=0
  if [[ $rc == 1 ]]; then
    fails=$(grep -aE '^\s+(FAIL|TIMEOUT) \[' $S/ci-$id.log | sort -u)
    others=$(print -r -- "$fails" | grep -avE 'TIMEOUT' | grep -c .)
    [[ -n $fails && $others == 0 ]] && env_only=1
    grep -aq 'No space left on device' $S/ci-$id.log && env_only=1
  fi
  [[ ($rc == 75 || $env_only == 1) && $try == 1 ]] || break
  echo "$id: branch CI environmental failure rc=$rc (try $try), retrying"; sleep 120
done
 tail -12 $S/ci-$id.log | cut -c1-220
[[ $rc == 0 ]] || { echo "$id: branch CI failed rc=$rc"; exit 5; }
cd $R
# land.sh refuses foreign uncommitted files on main; another lead's edit is usually brief.
for i in {1..45}; do
  [[ -z "$(git status --short -uall -- . ':(exclude).jit' ':(exclude,glob)**/.jit-bootstrap.lock')" ]] && break
  sleep 20
done
.agents/skills/jit-execution-lead/scripts/land.sh worktree-agent-$id 2>&1 | tail -4 || exit 6
git merge-base --is-ancestor worktree-agent-$id main || { echo "$id: land did not merge"; exit 6; }
git log --oneline -1
gates=(${(f)"$(jit issue status $id --json | python3 -c "import json,sys;g=[x['key'] for x in json.load(sys.stdin)['gates']];print('\n'.join(['cargo-ci']*('cargo-ci' in g)+[k for k in g if k not in('cargo-ci','code-review')]+['code-review']*('code-review' in g)))")"})
for g in $gates; do
  while [[ -e $S/hold ]]; do sleep 30; done
  # SKIP_GATES: gates a steward ruling defers (each skip is recorded in the plan).
  [[ " $SKIP_GATES " == *" $g "* ]] && { echo "$id $g deferred"; continue; }
  if [[ $g == cargo-ci ]]; then
    for try in 1 2; do
      sccache_guard
      flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock jit gate evaluate $id $g > $S/gate-$id-$g.log 2>&1
      rc=$?; [[ $rc == 10 && $try == 1 ]] && grep -q 'exit code: Some(75)' $S/gate-$id-$g.log || break
      echo "$id cargo-ci gate exit 75 (try $try), retrying"; sleep 120
    done
  else jit gate evaluate $id $g > $S/gate-$id-$g.log 2>&1; rc=$?; fi
   echo "$id $g rc=$rc"; grep -aE 'VERDICT|Gate result|"verdict"' $S/gate-$id-$g.log | cut -c1-900 | tail -3
  [[ $rc == 0 ]] || exit 7
done
[[ -n $SKIP_GATES ]] && echo "$id: LANDED, GATED EXCEPT $SKIP_GATES" || echo "$id: LANDED AND GATED"
