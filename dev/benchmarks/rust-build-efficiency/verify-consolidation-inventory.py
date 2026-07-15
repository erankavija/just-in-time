#!/usr/bin/env python3
"""Machine-check REQ-04 for jit:8d4f7084.

Runs `cargo test --workspace -- --list`, extracts the integration-test case
names Cargo now reports, and asserts they equal the `case_name_map` post_case
set recorded in consolidation-inventory-diff.json. That map was in turn derived
from the recorded pre-change inventory under the documented qualification rule
(see the artifact's `naming_qualification_rule`), so a green run proves the
post-consolidation inventory still matches the baseline modulo that rule plus
the documented, pre-existing jit:5d862134 provenance drift.

Exit 0 on match, 1 on mismatch. Set CARGO_TARGET_DIR to reuse an existing build.
"""
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ARTIFACT = os.path.join(HERE, "consolidation-inventory-diff.json")


def list_integration_cases():
    """Return the set of integration-test case names from a fresh --list."""
    # Merge stderr into stdout so the `Running <binary>` markers (stderr) stay
    # interleaved with the case names (stdout) they introduce.
    out = subprocess.run(
        ["cargo", "test", "--workspace", "--", "--list"],
        cwd=os.path.join(HERE, "..", "..", ".."),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        check=True,
    )
    cases = set()
    in_integration = False
    for line in out.stdout.splitlines():
        run = re.match(r"\s*Running (unittests \S+|tests/(\S+))", line)
        if run:
            in_integration = run.group(2) is not None
            continue
        if line.startswith("   Doc-tests "):
            in_integration = False
            continue
        case = re.match(r"(.+): test$", line)
        if case and in_integration:
            name = case.group(1)
            # Doctest --list lines carry a `src/... - name (line N)` shape; skip.
            if not (" - " in name and "(line " in name):
                cases.add(name)
    return cases


def main():
    artifact = json.load(open(ARTIFACT))
    expected = {row["post_case"] for row in artifact["case_name_map"]}
    actual = list_integration_cases()
    missing = sorted(expected - actual)
    extra = sorted(actual - expected)
    if missing or extra:
        print("REQ-04 inventory mismatch")
        for m in missing:
            print("  missing (expected, not listed):", m)
        for e in extra:
            print("  extra   (listed, not expected):", e)
        return 1
    print(f"REQ-04 OK: {len(expected)} integration cases match the recorded map.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
