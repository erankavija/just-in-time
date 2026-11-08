#!/usr/bin/env zsh
# usage: close-one.sh <id> <worker-summary>; close a gated issue and commit its tracker state
id=$1; cd /home/vkaskivuo/Projects/just-in-time; export JIT_AGENT_ID=agent:df129d9e-lead
jit issue status $id --json | python3 -c "import json,sys;d=json.load(sys.stdin);print(d['short_id'],d['gates']);sys.exit(any(g['status']!='passed' for g in d['gates']))" || { echo "$id: gates not all passed"; exit 1; }
jit issue update $id --state done 2>&1 | tail -1
jit claim release $id 2>&1 | tail -1
F=(.jit/issues/$id-*.json)
R=(.jit/gate-runs/$id-*)
jit worktree exec -- bash -c "git add $F .jit/events.jsonl; git add -- ${R[@]}; git commit -q -m 'chore(jit:$id): complete issue

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>' -- $F .jit/events.jsonl ${R[@]}" 2>&1 | tail -2
git log --oneline -1 | cut -c1-80
