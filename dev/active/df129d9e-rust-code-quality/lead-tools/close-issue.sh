#!/usr/bin/env zsh
# usage: close-one.sh <id> <worker-summary>; close a gated issue and commit its tracker state
id=$1; cd /home/vkaskivuo/Projects/just-in-time; export JIT_AGENT_ID=agent:df129d9e-lead
jit issue status $id --json | python3 -c "import json,sys;d=json.load(sys.stdin);print(d['short_id'],d['gates']);sys.exit(any(g['status']!='passed' for g in d['gates']))" || { echo "$id: gates not all passed"; exit 1; }
jit issue update $id --state done 2>&1 | tail -1
jit claim release $id 2>&1 | tail -1
F=(.jit/issues/$id-*.json)
R=$(python3 - $id <<'P'
import glob,json,sys
print(' '.join(f.rsplit('/',1)[0] for f in glob.glob('.jit/gate-runs/*/result.json') if json.load(open(f)).get('issue_id','').startswith(sys.argv[1])))
P
)
jit worktree exec -- bash -c "git add $F .jit/events.jsonl; for d in $R; do git ls-files --error-unmatch \$d/result.json >/dev/null 2>&1 || git add \$d; done; git commit -q -m 'chore(jit:$id): complete wave 7 issue

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>' -- $F .jit/events.jsonl .jit/gate-runs" 2>&1 | tail -2
git log --oneline -1 | cut -c1-80
