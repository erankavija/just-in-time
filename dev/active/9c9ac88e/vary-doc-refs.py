import json, os, shutil, sys, subprocess
src, dst, keep = sys.argv[1], sys.argv[2], int(sys.argv[3])
if os.path.exists(dst): shutil.rmtree(dst)
shutil.copytree(src, dst, ignore=shutil.ignore_patterns('.git'))
kept = 0
for f in sorted(os.listdir(f"{dst}/.jit/issues")):
    p = f"{dst}/.jit/issues/{f}"; d = json.load(open(p))
    docs = d.get("documents") or []
    if not docs: continue
    take = docs[:max(0, keep-kept)]
    kept += len(take)
    d["documents"] = take
    json.dump(d, open(p,"w"), indent=2)
subprocess.run(["git","init","-q","."], cwd=dst)
subprocess.run(["git","add","-A"], cwd=dst, capture_output=True)
subprocess.run(["git","-c","user.email=a@b","-c","user.name=a","commit","-qm","i"], cwd=dst, capture_output=True)
print(f"{dst}: {kept} doc refs")
