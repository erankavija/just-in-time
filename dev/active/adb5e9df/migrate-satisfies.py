#!/usr/bin/env python3
"""Plan the migration of unqualified `satisfies:<id>` labels (jit:adb5e9df).

Reads the tracker read-only: issue records from `.jit/issues/*.json` and the
declared requirement items from `jit item list --kind <kind> --json`. For each
unqualified `satisfies:<id>` label it finds the containers the label credited
under the removed unqualified semantics: every issue whose `label-coverage` walk
reaches the labelled issue and that declares `<id>`. The walk mirrors
`children_of` in `crates/jit/src/validation/graph.rs` for this repository's
coverage rule (`child-link = "dependencies"`, `child-type-exclude`): a
container reaches its transitive dependencies, never descending through an
excluded-type node, and an excluded-type node is never credited.

Mapping: one declaring ancestor -> `satisfies:<short>/<id>`; none -> orphan,
removed; several -> listed as ambiguous and resolved by narrowing the declaring
ancestors to bracketed containers (named by a `brackets:` label; the only
containers this repository's coverage rule evaluates), then to containers the
labelled issue is a member of (a shared `[type_hierarchy.label_associations]`
label such as `epic:<name>`), then taking the nearest (fewest dependency edges,
lowest short id on a tie). A narrowing step applies only when it keeps at least
one candidate.

Outputs a dry-run report and a shell script of `jit issue update` commands.
Use `--verify` against a scratch copy to compare coverage before and after.
"""

import argparse
import collections
import glob
import json
import os
import re
import shlex
import subprocess
import sys
import tomllib

SHORT = 8


def load_issues(root):
    issues = {}
    for path in glob.glob(os.path.join(root, ".jit", "issues", "*.json")):
        with open(path) as handle:
            issue = json.load(handle)
        issues[issue["id"]] = issue
    return issues


def issue_type(issue):
    return next((l[5:] for l in issue.get("labels", []) if l.startswith("type:")), None)


def membership_namespaces(root):
    with open(os.path.join(root, ".jit", "config.toml"), "rb") as handle:
        config = tomllib.load(handle)
    return config.get("type_hierarchy", {}).get("label_associations", {})


def is_member(issue, container, associations):
    """Whether `issue` carries the membership label naming `container`."""
    namespace = associations.get(issue_type(container))
    if not namespace:
        return False
    own = {l for l in container.get("labels", []) if l.startswith(f"{namespace}:")}
    return bool(own & set(issue.get("labels", [])))


def requirement_items(root, jit, kind):
    """Map short id -> {self_id: item text} for every declared item of `kind`."""
    out = subprocess.run(
        [jit, "item", "list", "--kind", kind, "--json"],
        cwd=root, check=True, capture_output=True, text=True,
    ).stdout
    declared = collections.defaultdict(dict)
    for item in json.loads(out)["items"]:
        declared[item["scope"]][item["self_id"]] = item["text"]
    return declared


def ancestors(issue_id, issues, dependents, exclude):
    """Issues whose coverage walk reaches `issue_id`, with their edge distance."""
    found, frontier = {}, [(dep, 1) for dep in dependents[issue_id]]
    while frontier:
        node, distance = frontier.pop(0)
        if node in found:
            continue
        found[node] = distance
        # A container may itself be of an excluded type, but no walk passes
        # through one to reach deeper issues.
        if issue_type(issues[node]) not in exclude:
            frontier.extend((dep, distance + 1) for dep in dependents[node])
    return found


def plan(issues, declared, namespace, exclude, associations):
    dependents = collections.defaultdict(list)
    for issue in issues.values():
        for dep in issue.get("dependencies", []):
            if dep in issues:
                dependents[dep].append(issue["id"])
    bracketed = {l.split(":", 1)[1] for i in issues.values() for l in i.get("labels", [])
                 if l.startswith("brackets:")}
    rows = []
    prefix = f"{namespace}:"
    for issue in sorted(issues.values(), key=lambda i: i["id"]):
        bare = [l[len(prefix):] for l in issue.get("labels", [])
                if l.startswith(prefix) and "/" not in l[len(prefix):]]
        if not bare:
            continue
        excluded = issue_type(issue) in exclude
        reach = {} if excluded else ancestors(issue["id"], issues, dependents, exclude)
        for value in bare:
            declaring = sorted(
                ({"short": anc[:SHORT], "distance": distance,
                  "member": is_member(issue, issues[anc], associations),
                  "bracketed": anc[:SHORT] in bracketed}
                 for anc, distance in reach.items()
                 if value in declared.get(anc[:SHORT], {})),
                key=lambda d: (d["distance"], d["short"]),
            )
            row = {
                "issue": issue["id"],
                "state": issue.get("state"),
                "label": f"{namespace}:{value}",
                "declaring": declaring,
            }
            if not declaring:
                row["kind"] = "orphan"
                row["reason"] = "labelled issue has an excluded type" if excluded else (
                    "no ancestor declares the id" if reach else "no ancestor")
                row["replacement"] = None
            else:
                row["kind"] = "ambiguous" if len(declaring) > 1 else "unambiguous"
                pool, steps = declaring, []
                for name in ("bracketed", "member"):
                    narrowed = [d for d in pool if d[name]]
                    if narrowed and len(narrowed) < len(pool):
                        pool = narrowed
                        steps.append(name)
                if len(pool) > 1:
                    steps.append("nearest-tie" if pool[0]["distance"] == pool[1]["distance"]
                                 else "nearest")
                row["resolved_by"] = "+".join(steps) or "only"
                row["replacement"] = f"{namespace}:{pool[0]['short']}/{value}"
            rows.append(row)
    return rows


def script(rows, jit):
    by_issue = collections.OrderedDict()
    for row in rows:
        by_issue.setdefault(row["issue"], []).append(row)
    lines = [
        "#!/usr/bin/env bash",
        "# Generated by dev/active/adb5e9df/migrate-satisfies.py; review the report first.",
        "# Label updates need a lease under enforce_leases = strict; run with leases",
        "# disabled for the migration (worktree.enforce_leases) or acquire them first.",
        "set -euo pipefail",
        f"JIT=${{JIT:-{jit}}}",
    ]
    for issue, issue_rows in by_issue.items():
        args = []
        for row in issue_rows:
            args += ["--remove-label", row["label"]]
            if row["replacement"]:
                args += ["--label", row["replacement"]]
        kinds = ",".join(sorted({row["kind"] for row in issue_rows}))
        lines.append(f"# {kinds}")
        lines.append('"$JIT" issue update ' + shlex.quote(issue) + " " + " ".join(shlex.quote(a) for a in args))
    return "\n".join(lines) + "\n"


def coverage(issues, declared, namespace, exclude, marker, qualified):
    """Credited (container short, criterion) pairs for every container with
    `marker` criteria, under the old (unqualified) or new (qualified) matcher."""
    dependents = collections.defaultdict(list)
    for issue in issues.values():
        for dep in issue.get("dependencies", []):
            if dep in issues:
                dependents[dep].append(issue["id"])
    credited, required = set(), set()
    prefix = f"{namespace}:"
    for container in issues.values():
        short = container["id"][:SHORT]
        hard = {sid for sid, text in declared.get(short, {}).items()
                if re.sub(r"^\[[ xX]\]\s*", "", text).startswith(marker)}
        if not hard:
            continue
        required |= {(short, sid) for sid in hard}
        # Descendants via the same walk, from the container downward.
        seen, frontier = set(), list(container.get("dependencies", []))
        while frontier:
            node = frontier.pop()
            if node in seen or node not in issues:
                continue
            seen.add(node)
            if issue_type(issues[node]) in exclude:
                continue
            for label in issues[node].get("labels", []):
                if not label.startswith(prefix):
                    continue
                value = label[len(prefix):]
                if qualified:
                    scope, _, sid = value.partition("/")
                    if sid and scope == short and sid in hard:
                        credited.add((short, sid))
                elif "/" not in value and value in hard:
                    credited.add((short, value))
            frontier.extend(issues[node].get("dependencies", []))
    return required, credited


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=".", help="repository root (read-only)")
    parser.add_argument("--jit", default="jit", help="jit binary used for item listing")
    parser.add_argument("--namespace", default="satisfies")
    parser.add_argument("--kind", default="requirement", help="item kind linked through the namespace")
    parser.add_argument("--exclude", action="append", default=None,
                        help="child-type-exclude of the coverage rule (default: breakdown, planning)")
    parser.add_argument("--marker", default="[hard]")
    parser.add_argument("--script", help="write the jit issue update script here")
    parser.add_argument("--json", action="store_true", help="print the report as JSON")
    parser.add_argument("--verify-against", metavar="MIGRATED_ROOT",
                        help="compare old-semantics coverage of --root with new-semantics coverage of this migrated copy")
    args = parser.parse_args()
    exclude = set(args.exclude or ["breakdown", "planning"])

    issues = load_issues(args.root)
    declared = requirement_items(args.root, args.jit, args.kind)

    if args.verify_against:
        required, before = coverage(issues, declared, args.namespace, exclude, args.marker, False)
        migrated = load_issues(args.verify_against)
        _, after = coverage(migrated, declared, args.namespace, exclude, args.marker, True)
        lost, gained = sorted(before - after), sorted(after - before)
        bracketed = {l.split(":", 1)[1] for i in issues.values() for l in i.get("labels", [])
                     if l.startswith("brackets:")}
        print(json.dumps({
            "required": len(required), "credited_before": len(before),
            "credited_after": len(after), "lost": len(lost), "gained": gained,
            "lost_on_bracketed_containers": [p for p in lost if p[0] in bracketed],
        }, indent=2))
        return

    shorts = collections.Counter(i[:SHORT] for i in issues)
    collisions = sorted(s for s, n in shorts.items() if n > 1)
    rows = plan(issues, declared, args.namespace, exclude, membership_namespaces(args.root))
    counts = collections.Counter(row["kind"] for row in rows)
    report = {
        "labels": len(rows),
        "issues": len({row["issue"] for row in rows}),
        "unambiguous": counts["unambiguous"],
        "ambiguous": counts["ambiguous"],
        "orphan": counts["orphan"],
        "ambiguous_resolved_by": dict(collections.Counter(
            r["resolved_by"] for r in rows if r["kind"] == "ambiguous")),
        "short_id_collisions": collisions,
        "ambiguous_rows": [r for r in rows if r["kind"] == "ambiguous"],
        "orphan_rows": [r for r in rows if r["kind"] == "orphan"],
    }
    if args.script:
        with open(args.script, "w") as handle:
            handle.write(script(rows, args.jit))
        os.chmod(args.script, 0o755)
    if args.json:
        print(json.dumps(report, indent=2))
        return
    print(f"unqualified labels: {report['labels']} on {report['issues']} issues")
    print(f"unambiguous: {report['unambiguous']}  ambiguous: {report['ambiguous']}  orphan: {report['orphan']}")
    print(f"ambiguous resolved by: {report['ambiguous_resolved_by']}")
    if collisions:
        print(f"short-id collisions (qualified labels would be ambiguous): {', '.join(collisions)}")
    for row in report["ambiguous_rows"]:
        others = ", ".join(
            f"{d['short']}@{d['distance']}{'m' if d['member'] else ''}{'b' if d['bracketed'] else ''}"
            for d in row["declaring"])
        print(f"ambiguous [{row['resolved_by']}]: {row['issue'][:SHORT]} {row['label']} -> "
              f"{row['replacement']} (declaring, @distance m=member b=bracketed: {others})")
    for row in report["orphan_rows"]:
        print(f"orphan: {row['issue'][:SHORT]} [{row['state']}] {row['label']} ({row['reason']})")


if __name__ == "__main__":
    sys.exit(main())
