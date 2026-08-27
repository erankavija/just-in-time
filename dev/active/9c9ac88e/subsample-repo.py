import json, os, shutil, sys, subprocess
src, dst, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
if os.path.exists(dst): shutil.rmtree(dst)
os.makedirs(dst)
idx = json.load(open(f"{src}/.jit/index.json"))
keep = idx["all_ids"][:n]
keepset = set(keep)
os.makedirs(f"{dst}/.jit/issues")
for f in os.listdir(f"{src}/.jit"):
    p = f"{src}/.jit/{f}"
    if f in ("issues", "gate-runs", "events.jsonl", "index.json", "server.log", "server.pid.json"): continue
    (shutil.copytree if os.path.isdir(p) else shutil.copy)(p, f"{dst}/.jit/{f}")
for i in keep:
    d = json.load(open(f"{src}/.jit/issues/{i}.json"))
    d["dependencies"] = [x for x in d.get("dependencies", []) if x in keepset]
    json.dump(d, open(f"{dst}/.jit/issues/{i}.json", "w"), indent=2)
json.dump({"all_ids": keep, "deleted_ids": [], "schema_version": idx["schema_version"]},
          open(f"{dst}/.jit/index.json", "w"), indent=2)
open(f"{dst}/.jit/events.jsonl", "w").close()
os.makedirs(f"{dst}/.jit/gate-runs", exist_ok=True)
subprocess.run(["git","init","-q","."], cwd=dst)
subprocess.run(["git","add","-A"], cwd=dst, capture_output=True)
subprocess.run(["git","-c","user.email=a@b","-c","user.name=a","commit","-qm","init"], cwd=dst, capture_output=True)
print(f"built {dst} with {len(keep)} issues")
