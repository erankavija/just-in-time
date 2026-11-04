#!/usr/bin/env python3
"""Per-layer duration distribution from nextest JUnit reports.

usage: distribution.py raw/junit-*.xml
Layer: suites `jit` and `jit-server` (lib unit tests) and `jit::bin/jit`,
`jit-server::bin/jit-server` (bin unit tests) are "unit"; every other suite
(crates/*/tests/* targets) is "integration".
A test's duration is its median over the runs; the per-run maximum is also kept.
"""
import sys, statistics, xml.etree.ElementTree as ET

UNIT = {"jit", "jit-server", "jit::bin/jit", "jit-server::bin/jit-server"}

def pct(xs, p):
    xs = sorted(xs)
    k = (len(xs) - 1) * p / 100
    lo, hi = int(k), min(int(k) + 1, len(xs) - 1)
    return xs[lo] + (xs[hi] - xs[lo]) * (k - lo)

runs = []
for path in sys.argv[1:]:
    d = {}
    for suite in ET.parse(path).getroot():
        for tc in suite:
            if tc.tag == "testcase":
                d[(suite.get("name"), tc.get("name"))] = float(tc.get("time"))
    runs.append(d)

keys = sorted(set.intersection(*(set(r) for r in runs)))
rows = [(k, statistics.median(r[k] for r in runs), max(r[k] for r in runs)) for k in keys]
layer = lambda k: "unit" if k[0] in UNIT else "integration"

print(f"runs={len(runs)} tests={len(rows)}")
for name in ("unit", "integration", "all"):
    sel = [r for r in rows if name == "all" or layer(r[0]) == name]
    for label, idx in (("median-of-runs", 1), ("worst-run", 2)):
        xs = [r[idx] for r in sel]
        print(f"{name:12} {label:15} n={len(xs):5} median={pct(xs,50):.3f} p95={pct(xs,95):.3f} p99={pct(xs,99):.3f} max={max(xs):.3f}")
print("\nper-run suite times:", [round(sum(r.values()), 1) for r in runs])
print("\ntop 40 by median-of-runs:")
for k, m, mx in sorted(rows, key=lambda r: -r[1])[:40]:
    print(f"{m:7.3f} {mx:7.3f} {layer(k):11} {k[0]} {k[1]}")
