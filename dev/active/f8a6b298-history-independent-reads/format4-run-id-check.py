#!/usr/bin/env python3
"""Check that every gate-run id cited in dev/, docs/ or an issue record resolves.

Usage: format4-run-id-check.py <pre-window-commit>

Run ids are the run directories under .jit/gate-runs/ at <pre-window-commit>
(format 3) together with the run records under .jit/gate-runs/<issue-id>/ in
the working tree (format 4). Every UUID in a cited file that names such a run
must resolve to a working-tree run record. Prints the counts and exits 1 if
any cited run id is unresolved.
"""
import pathlib
import re
import subprocess
import sys

UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")


def main(pre_window: str) -> int:
    listing = subprocess.check_output(
        ["git", "ls-tree", "-r", "--name-only", pre_window, ".jit/gate-runs"], text=True
    )
    old = {parts[2] for parts in (line.split("/") for line in listing.splitlines())
           if len(parts) >= 3 and UUID.fullmatch(parts[2])}
    root = pathlib.Path(".")
    new = {p.stem for p in root.glob(".jit/gate-runs/*/*.json") if p.parent.name != "verdicts"}
    cited_files = [p for d in ("dev", "docs") for p in (root / d).rglob("*") if p.is_file()]
    cited_files += list(root.glob(".jit/issues/*.json"))
    cited = {u for f in cited_files for u in UUID.findall(f.read_text(errors="ignore"))
             if u in old or u in new}
    unresolved = sorted(cited - new)
    print(f"pre-window runs {len(old)}, runs now {len(new)}, "
          f"cited run ids {len(cited)}, unresolved {len(unresolved)}")
    for run_id in unresolved:
        print(f"unresolved: {run_id}")
    return 1 if unresolved else 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: format4-run-id-check.py <pre-window-commit>")
    sys.exit(main(sys.argv[1]))
