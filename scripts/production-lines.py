#!/usr/bin/env python3
"""Production code lines by module at a git rev, or for one working-tree file.

Excludes: blank lines; doc comments (///, //!, /** */ blocks); `#[cfg(test)]`
item extents (brace-matched from the attribute); whole files declared by a
`#[cfg(test)]`-attributed `mod x;` (incl. `#[path = ...]`); files named
*_tests.rs / *_proptests.rs / *_property_tests.rs.
Flags: --strip-line-comments also drops plain `//` comment lines;
--test-support-as-test also drops files with an inner
`#![cfg(any(test, feature = "test-support"))]` and items under that cfg.
Usage: scripts/production-lines.py [REV | --file PATH] [--strip-line-comments] [--test-support-as-test]
"""
import argparse, collections, os, re, subprocess, sys
from pathlib import Path
parser=argparse.ArgumentParser(description=__doc__)
source=parser.add_mutually_exclusive_group()
source.add_argument('revision',nargs='?',default='HEAD')
source.add_argument('--file',metavar='PATH',help='print the production-line count of one working-tree file')
parser.add_argument('--strip-line-comments',action='store_true')
parser.add_argument('--test-support-as-test',action='store_true')
args=parser.parse_args()
REV,FILE=args.revision,args.file
STRIP_LC=args.strip_line_comments
TS=args.test_support_as_test
ROOT='crates/jit/src'
def git(*a): return subprocess.run(['git',*a],capture_output=True,text=True,check=True).stdout
files=[FILE] if FILE else [f for f in git('ls-tree','-r','--name-only',REV,ROOT).split() if f.endswith('.rs')]
src={f:(Path(f).read_text() if FILE else git('show',f'{REV}:{f}')).split('\n') for f in files}
def is_test_attr(s):
    s=s.strip()
    if s.startswith('#[cfg(test)]'): return True
    if TS and re.match(r'#\[cfg\(any\(test,\s*feature\s*=\s*"test-support"\)\)\]',s): return True
    return False
test_files=set()
for f,L in src.items():
    for i,l in enumerate(L):
        if not is_test_attr(l): continue
        j=i+1
        path=None
        while j<len(L) and L[j].strip().startswith('#['):
            m=re.search(r'#\[path\s*=\s*"([^"]+)"\]',L[j]); 
            if m: path=m.group(1)
            j+=1
        if j<len(L):
            m=re.match(r'\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;',L[j])
            if m:
                d=os.path.dirname(f); base=os.path.basename(f)
                if path: test_files.add(os.path.normpath(os.path.join(d,path)))
                else:
                    sub=d if base in('mod.rs','lib.rs','main.rs') else os.path.join(d,base[:-3])
                    for c in (os.path.join(sub,m.group(1)+'.rs'),os.path.join(sub,m.group(1),'mod.rs')): test_files.add(c)
# propagate: children of test-only dir modules
def file_is_test(f):
    b=os.path.basename(f)
    if re.search(r'_(tests|proptests|property_tests)\.rs$',b): return True
    if f in test_files: return True
    L=src[f]
    if TS and any(re.match(r'\s*#!\[cfg\(any\(test,\s*feature\s*=\s*"test-support"\)\)\]',l) for l in L[:40]): return True
    if any(l.strip()=='#![cfg(test)]' for l in L[:40]): return True
    d=os.path.dirname(f)
    while d.startswith(ROOT) and d!=ROOT:
        if d+'/mod.rs' in test_files or d+'.rs' in test_files: return True
        d=os.path.dirname(d)
    return False
def count(L):
    skip=set()
    for i,l in enumerate(L):
        if not is_test_attr(l): continue
        j,depth,started=i+1,0,False
        while j<len(L):
            for ch in L[j]:
                if ch=='{': depth+=1; started=True
                elif ch=='}': depth-=1
            if started and depth<=0: break
            if not started and L[j].rstrip().endswith(';'): break
            j+=1
        skip.update(range(i,min(j+1,len(L))))
    n=0; inblock=False
    for i,l in enumerate(L):
        s=l.strip()
        if i in skip: continue
        if inblock:
            if '*/' in s: inblock=False
            continue
        if s.startswith('/**') or s.startswith('/*!'):
            if '*/' not in s[3:]: inblock=True
            continue
        if not s or s.startswith('///') or s.startswith('//!'): continue
        if STRIP_LC and s.startswith('//'): continue
        n+=1
    return n
if FILE:
    print(0 if file_is_test(FILE) else count(src[FILE]))
    sys.exit()
per=collections.Counter(); tot=0; testonly=0
for f in files:
    rel=f[len(ROOT)+1:]
    mod=rel.split('/')[0]
    if mod.endswith('.rs'): mod=mod[:-3]
    if file_is_test(f): testonly+=len(src[f]); continue
    c=count(src[f]); per[mod]+=c; tot+=c
for m,c in sorted(per.items(),key=lambda kv:-kv[1]): print(f"{c:7d}  {m}")
print(f"{tot:7d}  TOTAL production ({REV})")
