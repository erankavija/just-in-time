#!/usr/bin/env python3
"""Net `*.rs` lines (production and tests) over the delivered children of
story `superseded-design-retirement`, for checkpoint e590fa91 REQ-06.

A child's commits are the non-merge commits reachable from HEAD whose subject
contains `jit:<short-id>`; their `git show --numstat` rows for `*.rs` paths are
summed. Commits touching no `*.rs` path (the `.jit/` chore commits) contribute
nothing, so they need no separate exclusion.

Usage: python3 dev/active/e590fa91/phase-net.py [REV]   (REV defaults to HEAD)
"""

import subprocess
import sys

CHILDREN = [
    "3d9150d7", "51eb1aa6", "b4483ad6", "c88822a5", "ec72359b", "33f97287",
    "8454236c", "e346d387", "816df12a", "91168745", "be13da79",
]


def git(*args: str) -> str:
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout


def child_commits(rev: str, child: str) -> list[str]:
    log = git("log", "--no-merges", "--format=%H%x09%s", rev)
    return [line.split("\t", 1)[0] for line in log.splitlines()
            if f"jit:{child}" in line.split("\t", 1)[1]]


def rs_numstat(commit: str) -> tuple[int, int]:
    added = removed = 0
    for row in git("show", "--numstat", "--format=", commit).splitlines():
        parts = row.split("\t")
        if len(parts) == 3 and parts[2].endswith(".rs") and parts[0] != "-":
            added += int(parts[0])
            removed += int(parts[1])
    return added, removed


def main() -> None:
    rev = sys.argv[1] if len(sys.argv) > 1 else "HEAD"
    print(f"{'child':<10} {'commits':>7} {'added':>8} {'removed':>8} {'net':>8}")
    total_added = total_removed = 0
    for child in CHILDREN:
        commits = child_commits(rev, child)
        added, removed = map(sum, zip((0, 0), *map(rs_numstat, commits)))
        total_added += added
        total_removed += removed
        print(f"{child:<10} {len(commits):>7} {added:>8} {removed:>8} {added - removed:>+8}")
    net = total_added - total_removed
    print(f"{'total':<10} {'':>7} {total_added:>8} {total_removed:>8} {net:>+8}")
    print("REQ-06 " + ("MET: removes more than it adds" if net < 0 else "UNMET"))


if __name__ == "__main__":
    main()
