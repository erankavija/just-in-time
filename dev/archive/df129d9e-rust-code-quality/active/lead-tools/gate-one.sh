#!/usr/bin/env zsh
# usage: gate-one.sh <id>; evaluate an already-landed issue's gates (cargo-ci first, code-review last)
id=$1; S=${0:A:h}; cd /home/vkaskivuo/Projects/just-in-time || exit 2
export JIT_AGENT_ID=agent:df129d9e-lead CARGO_INCREMENTAL=0 TMPDIR=/dev/shm/jit-ci/df129d9e
mkdir -p $TMPDIR
ps -C sccache -o pid= >/dev/null 2>&1 && unset CARGO_CI_NO_SCCACHE || export CARGO_CI_NO_SCCACHE=1
gates=(${(f)"$(jit issue status $id --json | python3 -c "import json,sys;g=[x['key'] for x in json.load(sys.stdin)['gates'] if x['status']!='passed'];print('\n'.join(['cargo-ci']*('cargo-ci' in g)+[k for k in g if k not in('cargo-ci','code-review')]+['code-review']*('code-review' in g)))")"})
for g in $gates; do
  while [[ -e $S/hold ]]; do sleep 30; done
  if [[ $g == cargo-ci ]]; then
    flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock jit gate evaluate $id $g > $S/gate-$id-$g.log 2>&1
  else jit gate evaluate $id $g > $S/gate-$id-$g.log 2>&1; fi
  rc=$?; echo "$id $g rc=$rc"
  [[ $rc == 0 ]] || exit 7
done
echo "$id: GATED"
