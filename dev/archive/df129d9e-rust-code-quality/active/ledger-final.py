#!/usr/bin/env python3
"""Completion-report figures for df129d9e, from scripts/production-lines.py at each first-parent step.

Usage (repository root): python3 dev/active/df129d9e-rust-code-quality/ledger-final.py [BASE] [FINAL]
Sections: `module` rows (default and flagged counts at BASE and FINAL), `step` rows (REQ-04 ledger:
default-mode TOTAL delta of each first-parent step against its first parent), class sums, and
`attr` rows (per-step default delta of the five modules with the largest absolute delta).
Membership is the `epic:rust-code-quality` label, df129d9e's own epic label. A merge's issue ids come
from its branch name and `jit:<id>` subject or body tokens, else from the merged side's commit
messages. All ids members: `epic`; none a member or an unlabelled issue in df129d9e's transitive
dependency tree: `other`; otherwise (mixed, unlabelled in-tree, or no id): `ambiguous`. A non-merge
first-parent commit is `direct`. The class sums reconcile to TOTAL(FINAL) - TOTAL(BASE).
"""
import collections, json, re, subprocess, sys

BASE = sys.argv[1] if len(sys.argv) > 1 else 'dc2ea8706edd285b09746ec7f111ce405845e961'
FINAL = sys.argv[2] if len(sys.argv) > 2 else 'ce7da387c72e5b36c2f5e1dc2fc982213e6d1e52'
SRC = 'crates/jit/src'
FLAGS = ['--strip-line-comments', '--test-support-as-test']
ID = re.compile(r'\bjit:([0-9a-f]{8})\b')
BRANCH = re.compile(r"^Merge branch '(?:worktree-agent-)?([0-9a-f]{8})[^']*'")


def run(*a):
    return subprocess.run(a, capture_output=True, text=True, check=True).stdout


_counts = {}


def counts(rev, *flags):
    """Module -> production lines at rev, plus 'TOTAL'."""
    key = (rev, flags)
    if key not in _counts:
        out = run('python3', 'scripts/production-lines.py', rev, *flags)
        _counts[key] = {('TOTAL' if 'TOTAL' in l else l.split()[1]): int(l.split()[0])
                        for l in out.splitlines() if l.strip()}
    return _counts[key]


def touches_src(rev):
    return subprocess.run(['git', 'diff', '--quiet', f'{rev}^1', rev, '--', SRC]).returncode != 0


def step_delta(rev, module='TOTAL'):
    return counts(rev).get(module, 0) - counts(f'{rev}^1').get(module, 0)


def issue_ids(rev, subject, body):
    ids = set(ID.findall(subject + '\n' + body))
    m = BRANCH.match(subject)
    if m:
        ids.add(m.group(1))
    return ids or set(ID.findall(run('git', 'log', '--format=%B', f'{rev}^1..{rev}')))


issues = json.loads(run('jit', 'issue', 'list', '--full', '--json'))['issues']
epic_of = {i['id'][:8]: next((l for l in i.get('labels', []) if l.startswith('epic:')), 'epic:-')
           for i in issues}
members = {k for k, v in epic_of.items() if v == 'epic:rust-code-quality' and k != 'df129d9e'}


def tree(nodes):
    return {i for n in nodes for i in {n['short_id']} | tree(n['children'])}


in_tree = tree(json.loads(run('jit', 'graph', 'deps', 'df129d9e', '--depth', '0', '--json'))['nodes'])
unlabelled = {k for k in in_tree if epic_of.get(k) == 'epic:-'}

base, final = counts(BASE), counts(FINAL)
fbase, ffinal = counts(BASE, *FLAGS), counts(FINAL, *FLAGS)
fmt = lambda d, m: str(d[m]) if m in d else '-'
diff = lambda a, b, m: f'{b.get(m, 0) - a.get(m, 0):+d}' if m in a or m in b else '-'
modules = sorted(set(base) | set(final), key=lambda m: (m == 'TOTAL', -max(base.get(m, 0), final.get(m, 0))))
print('# module\tname\tdefault-base\tdefault-final\tdefault-delta\tflagged-base\tflagged-final\tflagged-delta')
for m in modules:
    print('\t'.join(['module', m, fmt(base, m), fmt(final, m), diff(base, final, m),
                     fmt(fbase, m), fmt(ffinal, m), diff(fbase, ffinal, m)]))
top = sorted((m for m in modules if m != 'TOTAL'),
             key=lambda m: -abs(final.get(m, 0) - base.get(m, 0)))[:5]

steps = []
for line in run('git', 'log', '--first-parent', '--reverse', '--format=%H%x00%P%x00%s%x00%b%x1e',
                f'{BASE}..{FINAL}').split('\x1e'):
    if not line.strip():
        continue
    sha, parents, subject, body = line.strip('\n').split('\x00')
    merge = len(parents.split()) > 1
    ids = issue_ids(sha, subject, body) if merge else set()
    cls = ('direct' if not merge else 'epic' if ids and ids <= members
           else 'other' if ids and not ids & (members | unlabelled) else 'ambiguous')
    src = touches_src(sha)
    steps.append(dict(cls=cls, sha=sha[:9], ids=','.join(sorted(ids)) or '-',
                      epics=','.join(sorted({epic_of.get(i, 'epic:?') for i in ids})) or '-',
                      d=step_delta(sha) if src else 0, src=src, subject=subject))

print('# step\tclass\tmerge\tissues\tepics\tdelta\tsubject  (epic and ambiguous merges; others when delta != 0)')
for s in steps:
    if s['cls'] in ('epic', 'ambiguous') or s['d']:
        print(f"step\t{s['cls']}\t{s['sha']}\t{s['ids']}\t{s['epics']}\t{s['d']:+d}\t{s['subject']}")
sums, n = collections.Counter(), collections.Counter()
for s in steps:
    sums[s['cls']] += s['d']
    n[s['cls']] += 1
for cls in ('epic', 'ambiguous', 'other', 'direct'):
    print(f'sum\t{cls}\tsteps={n[cls]}\tdelta={sums[cls]:+d}')
print(f"sum\ttotal\tbase={base['TOTAL']}\tfinal={final['TOTAL']}\t"
      f"delta={final['TOTAL'] - base['TOTAL']:+d}\tclass-sum={sum(sums.values()):+d}")

print('# attr\tmodule\tclass\tmerge\tissues\tepics\tmodule-delta\tsubject')
for m in top:
    by = collections.Counter()
    for s in steps:
        d = step_delta(s['sha'], m) if s['src'] else 0
        by[s['cls']] += d
        if d:
            print(f"attr\t{m}\t{s['cls']}\t{s['sha']}\t{s['ids']}\t{s['epics']}\t{d:+d}\t{s['subject']}")
    print(f"attr-sum\t{m}\t" + '\t'.join(f'{c}={by[c]:+d}' for c in ('epic', 'ambiguous', 'other', 'direct')))
