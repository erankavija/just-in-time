#!/usr/bin/env python3
"""Measure production-line footprint per jit task over a commit range.

Evidence generator for `21e976d1` REQ-06 and, at wave 21, for `722878d4`/ER-01.

Production lines exclude test code in three forms, each of which a simpler
method gets wrong:

  1. `*_tests.rs` files and anything under a `tests/` directory.
  2. Modules declared `#[cfg(test)] mod <name>;` in a `mod.rs`/`lib.rs` — the
     attribute sits on the declaration, not inside the file, so the whole file
     is test code. `crates/jit/src/storage/redo_conformance.rs` is one.
  3. Every line inside a `#[cfg(test)]` item, whose extent is found by matching
     braces from the attribute. Cutting a file at its first `#[cfg(test)]`
     instead truncates `memory.rs` at the two `#[cfg(test)]` function
     attributes that sit above 200-odd production lines.

Attribution is by the `jit:<short-id>` tag in each non-merge commit subject.
Figures are net (added minus removed) per task: a rework commit that rewrites a
block counts every rewritten line as an addition, so cumulative additions
measure churn rather than footprint.

Usage:
    python3 dev/active/f1dc843a-redo-mutations/footprint.py [BASE] [--tasks a,b,c]

BASE defaults to 34d37f8be, the baseline epic REQ-06 names.
"""

import argparse
import collections
import os
import re
import subprocess
import sys

DEFAULT_BASE = "34d37f8be"
CFG_TEST = "#[cfg(test)]"


def git(*args: str) -> str:
    return subprocess.run(["git", *args], capture_output=True, text=True, check=False).stdout


def cfg_test_only_modules() -> set[str]:
    """Files whose whole content is test code, declared `#[cfg(test)] mod x;`."""
    out: set[str] = set()
    roots = git("ls-files", "*/mod.rs", "*/lib.rs").split()
    for decl in roots:
        try:
            lines = open(decl, encoding="utf-8").read().splitlines()
        except OSError:
            continue
        for i, line in enumerate(lines[:-1]):
            if not line.strip().startswith(CFG_TEST):
                continue
            m = re.match(r"\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z0-9_]+)\s*;", lines[i + 1])
            if m:
                out.add(os.path.join(os.path.dirname(decl), m.group(1) + ".rs"))
    return out


def is_test_path(path: str, test_only: set[str]) -> bool:
    return path.endswith("_tests.rs") or "/tests/" in path or path in test_only


def test_line_ranges(rev: str, path: str) -> list[tuple[int, int]]:
    """1-based inclusive line ranges covered by `#[cfg(test)]` items at `rev`."""
    lines = git("show", f"{rev}:{path}").splitlines()
    ranges: list[tuple[int, int]] = []
    for i, line in enumerate(lines):
        if not line.strip().startswith(CFG_TEST):
            continue
        j, depth, started = i + 1, 0, False
        while j < len(lines):
            for ch in lines[j]:
                if ch == "{":
                    depth += 1
                    started = True
                elif ch == "}":
                    depth -= 1
            if started and depth <= 0:
                break
            if not started and lines[j].rstrip().endswith(";"):
                break  # an attributed `use`, not a block
            j += 1
        ranges.append((i + 1, min(j + 1, len(lines))))
    return ranges


def measure(base: str) -> collections.Counter:
    """Net production lines per task tag over base..HEAD."""
    test_only = cfg_test_only_modules()
    net: collections.Counter = collections.Counter()
    log = git("log", "--no-merges", "--format=%H\t%s", f"{base}..HEAD").splitlines()
    for entry in log:
        sha, subject = entry.split("\t", 1)
        m = re.search(r"jit:([0-9a-f]{8})", subject)
        task = m.group(1) if m else "untagged"
        files = git("diff-tree", "--no-commit-id", "--name-only", "-r", sha, "--", "crates/*.rs").split()
        for path in files:
            if is_test_path(path, test_only):
                continue
            new_ranges = test_line_ranges(sha, path)
            old_ranges = test_line_ranges(sha + "^", path)
            diff = git("diff", "-U0", f"{sha}^", sha, "--", path)
            new_no = old_no = 0
            for line in diff.splitlines():
                hunk = re.match(r"^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@", line)
                if hunk:
                    old_no, new_no = int(hunk.group(1)), int(hunk.group(2))
                    continue
                if line.startswith(("+++", "---")):
                    continue
                if line.startswith("+"):
                    if not any(a <= new_no <= b for a, b in new_ranges):
                        net[task] += 1
                    new_no += 1
                elif line.startswith("-"):
                    if not any(a <= old_no <= b for a, b in old_ranges):
                        net[task] -= 1
                    old_no += 1
    return net


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", nargs="?", default=DEFAULT_BASE)
    parser.add_argument("--tasks", help="comma-separated short-ids to subtotal")
    args = parser.parse_args()

    net = measure(args.base)
    print(f"Net production lines per task, {args.base}..HEAD\n")
    for task, lines in sorted(net.items(), key=lambda kv: -kv[1]):
        print(f"  {task}  {lines:+6d}")

    if args.tasks:
        wanted = [t.strip() for t in args.tasks.split(",") if t.strip()]
        subtotal = sum(net[t] for t in wanted)
        print(f"\n  subtotal over {len(wanted)} tasks: {subtotal:+d}")
        missing = [t for t in wanted if t not in net]
        if missing:
            print(f"  no commits found for: {', '.join(missing)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
