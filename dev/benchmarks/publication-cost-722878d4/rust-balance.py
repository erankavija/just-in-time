#!/usr/bin/env python3
"""Rust line balance over BASE..HEAD, with named work excluded (jit:722878d4).

Walks HEAD's first-parent chain back to BASE and attributes each step's
`git diff --numstat <first-parent> <step> -- '*.rs'` to the `jit:<short-id>`
tag in the step's subject. A merge step therefore carries its whole branch,
rework and conflict resolution included, so excluding a task excludes its
merge. Net lines are additive along the chain: the steps' nets sum exactly to
`git diff --shortstat BASE -- '*.rs'`, which the script checks. Per-step
insertions and deletions are not additive (a line rewritten in two steps counts
twice), so the subtotal is stated as a net.
"""

import argparse
import collections
import re
import subprocess
import sys

PATHSPEC = "*.rs"


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def numstat(old: str, new: str) -> tuple[int, int]:
    added = removed = 0
    for line in git("diff", "--numstat", old, new, "--", PATHSPEC).splitlines():
        a, r, _ = line.split("\t", 2)
        if a != "-":
            added, removed = added + int(a), removed + int(r)
    return added, removed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base")
    parser.add_argument("--exclude-tasks", default="", help="comma-separated short ids")
    parser.add_argument("--exclude-commits", default="", help="comma-separated commit prefixes")
    args = parser.parse_args()
    tasks = {t for t in args.exclude_tasks.split(",") if t}
    commits = [c for c in args.exclude_commits.split(",") if c]

    steps = git("log", "--first-parent", "--format=%H %P\t%s", f"{args.base}..HEAD").splitlines()
    kept = [0, 0, 0]
    dropped: dict[str, list[int]] = collections.defaultdict(lambda: [0, 0, 0])
    for step in steps:
        head, subject = step.split("\t", 1)
        sha, parent = head.split()[:2]
        added, removed = numstat(parent, sha)
        if added == removed == 0:
            continue
        tag = re.search(r"jit:([0-9a-f]{8})", subject)
        key = next((c for c in commits if sha.startswith(c)), None) or (
            tag.group(1) if tag and tag.group(1) in tasks else None
        )
        bucket = dropped[key] if key else kept
        bucket[0], bucket[1], bucket[2] = bucket[0] + added, bucket[1] + removed, bucket[2] + 1

    total = git("diff", "--shortstat", args.base, "--", PATHSPEC).strip()
    ins = int(re.search(r"(\d+) insertion", total).group(1)) if "insertion" in total else 0
    dels = int(re.search(r"(\d+) deletion", total).group(1)) if "deletion" in total else 0
    head = git("rev-parse", "--short=9", "HEAD").strip()
    print(f"Rust line balance, {args.base}..{head}, pathspec '{PATHSPEC}'\n")
    print(f"  shortstat: {total}")
    print(f"  shortstat net: {ins - dels:+d}\n")
    print("  excluded first-parent steps (added / removed / net, steps)")
    for key in sorted(dropped):
        a, r, n = dropped[key]
        print(f"    {key:<10} +{a:<6} -{r:<6} {a - r:+7d}  ({n})")
    excluded_net = sum(a - r for a, r, _ in dropped.values())
    print(f"    {'total':<10} {'':16}{excluded_net:+7d}\n")
    print(f"  remaining steps: +{kept[0]} -{kept[1]} over {kept[2]} steps (per-step sums)")
    print(f"  remaining net: {kept[0] - kept[1]:+d}")
    missing = sorted(tasks - set(dropped)) + [c for c in commits if c not in dropped]
    if missing:
        print(f"  excluded but no Rust-touching step found: {', '.join(missing)}")
    if (kept[0] - kept[1]) + excluded_net != ins - dels:
        sys.exit("step nets do not sum to the shortstat net")
    return 0


if __name__ == "__main__":
    sys.exit(main())
