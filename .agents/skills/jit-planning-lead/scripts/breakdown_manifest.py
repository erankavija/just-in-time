#!/usr/bin/env python3
"""Validate and render JIT's authoritative breakdown manifest."""

import argparse
import json
import re
import sys
import tempfile
from pathlib import Path

try:
    import tomllib
except ImportError:  # pragma: no cover - Python <3.11
    tomllib = None

BEGIN = "<!-- jit:breakdown-overview:begin -->"
END = "<!-- jit:breakdown-overview:end -->"
KEY = re.compile(r"^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$")
ORDINAL = re.compile(r"^(?:[a-z]+-?)?\d+$")
REQUIRED = {"key", "title", "description", "type", "priority", "labels", "gates", "depends_on", "planning"}
PLANNING = {"outcome", "contract_refs", "produces_contracts", "source_refs", "landing_group", "terminal"}
TERMINAL_TEXT = {"consumer_family", "test_boundary", "worker_sized_reason"}
TERMINAL_REQUIRED = TERMINAL_TEXT | {"footprint"}
TERMINAL_ALLOWED = TERMINAL_REQUIRED | {"warning_overrides"}
WARNING_CODES = {
    "acceptance-clusters",
    "broad-quantifier",
    "implementation-release",
    "independent-verbs",
    "mixed-deliverables",
    "multiple-consumer-families",
    "multiple-test-boundaries",
    "weak-worker-sized-reason",
}


def load(path):
    return json.loads(Path(path).read_text())


def type_levels(config_path, explicit):
    if explicit:
        return set(explicit), None
    if tomllib is None:
        raise ValueError("cannot resolve terminal types: Python tomllib is unavailable; pass --terminal-type")
    path = Path(config_path)
    if not path.is_file():
        raise ValueError(f"cannot resolve terminal types: config not found at {path}; pass --terminal-type")
    types = tomllib.loads(path.read_text()).get("type_hierarchy", {}).get("types", {})
    if not types or not all(isinstance(name, str) and isinstance(level, int) for name, level in types.items()):
        raise ValueError("cannot resolve terminal types: config has no valid type hierarchy; pass --terminal-type")
    finest = max(types.values())
    return {name for name, level in types.items() if level == finest}, types


def strings(value):
    return isinstance(value, list) and all(isinstance(v, str) and v.strip() for v in value) and len(value) == len(set(value))


def cycle(keys, entries):
    graph = {
        entry["key"]: [dep for dep in entry.get("depends_on", []) if isinstance(dep, str)]
        for entry in entries
        if isinstance(entry, dict)
        and isinstance(entry.get("key"), str)
        and entry["key"] in keys
        and isinstance(entry.get("depends_on"), list)
    }
    active, done = set(), set()

    def visit(node):
        if node in active:
            return True
        if node in done:
            return False
        active.add(node)
        found = any(dep in graph and visit(dep) for dep in graph.get(node, []))
        active.remove(node)
        done.add(node)
        return found

    return any(visit(node) for node in graph)


def redundant_edges(keys, entries):
    """Edges the graph already implies through a longer path.

    Repository integrity requires a transitively reduced dependency graph, so a
    manifest carrying an implied edge creates a graph that fails validation the
    moment it is published. Returns the offending (key, dependency) pairs.
    """
    graph = {
        entry["key"]: [dep for dep in entry.get("depends_on", []) if isinstance(dep, str) and dep in keys]
        for entry in entries
        if isinstance(entry, dict)
        and isinstance(entry.get("key"), str)
        and entry["key"] in keys
        and isinstance(entry.get("depends_on"), list)
    }

    def reachable_without(node, skipped):
        seen, stack = set(), [dep for dep in graph.get(node, []) if dep != skipped]
        while stack:
            current = stack.pop()
            if current in seen:
                continue
            seen.add(current)
            stack.extend(graph.get(current, []))
        return seen

    return [
        (node, dep)
        for node, deps in graph.items()
        for dep in deps
        if dep in reachable_without(node, dep)
    ]


def structural_codes(entries):
    if not isinstance(entries, list):
        return ["invalid-root"]
    keys = [entry.get("key") for entry in entries if isinstance(entry, dict) and isinstance(entry.get("key"), str)]
    codes = []
    if len(keys) != len(set(keys)):
        codes.append("duplicate-key")
    known = set(keys)
    if any(
        dependency not in known
        for entry in entries if isinstance(entry, dict)
        for dependency in entry.get("depends_on", []) if isinstance(dependency, str)
    ):
        codes.append("unknown-dependency")
    if cycle(known, entries):
        codes.append("cycle")
    return codes


def check_conformance(path):
    suite = load(path)
    failures = []
    for case in suite.get("cases", []):
        actual = structural_codes(case.get("issues"))
        if actual != case.get("expected"):
            failures.append({"name": case.get("name"), "expected": case.get("expected"), "actual": actual})
    return failures


def sizing_warnings(entry):
    planning = entry["planning"]
    terminal = planning.get("terminal")
    if not terminal:
        return []
    title = str(entry.get("title", "")).lower()
    outcome = str(planning.get("outcome", "")).lower()
    text = f"{title} {outcome}"
    description = entry.get("description") if isinstance(entry.get("description"), str) else ""
    warnings = []
    if re.search(r"\b(all|every|entire|repo-wide|across the (?:repo|codebase|project))\b", text):
        warnings.append(("broad-quantifier", "uses a broad quantifier"))
    verb = r"(?:add|build|create|define|delete|deploy|document|implement|migrate|publish|release|remove|render|update|validate|wire)"
    independent_verbs = any(
        re.search(rf"\b{verb}\w*\b.*(?:[;,.]|\band\b).*\b{verb}\w*\b", field)
        for field in (title, outcome)
    )
    if independent_verbs:
        warnings.append(("independent-verbs", "states multiple independent verbs"))
    if re.search(r"[,/]|\band\b", terminal["consumer_family"].lower()):
        warnings.append(("multiple-consumer-families", "names more than one consumer family"))
    if re.search(r"[,/]|\band\b", terminal["test_boundary"].lower()):
        warnings.append(("multiple-test-boundaries", "names more than one test boundary"))
    categories = sum(bool(re.search(rf"\b{word}\w*\b", text)) for word in ("foundation", "migrat", "delet", "document", "releas"))
    if categories > 1:
        warnings.append(("mixed-deliverables", "mixes independently testable deliverable categories"))
    if len(re.findall(r"^###\s+", description, re.MULTILINE)) >= 3:
        warnings.append(("acceptance-clusters", "contains three or more acceptance clusters"))
    if re.search(r"\b(implement|migrate|update)\w*\b", text) and re.search(r"\b(release|publish|deploy)\w*\b", text):
        warnings.append(("implementation-release", "combines implementation with release work"))
    if len(terminal["worker_sized_reason"].split()) < 4:
        warnings.append(("weak-worker-sized-reason", "does not explain why the work fits one focused cycle"))
    return warnings


def contract_modes(plan):
    section = list(re.finditer(r"^## Shared architectural contracts\s*$", plan, re.MULTILINE))
    if len(section) != 1:
        return set(), ["plan must contain exactly one '## Shared architectural contracts' section"]
    start = section[0].end()
    following = re.search(r"^##\s+", plan[start:], re.MULTILINE)
    end = start + following.start() if following else len(plan)
    body = plan[start:end]
    heading = re.compile(
        r"^###\s+`([a-z][a-z0-9]*(?:-[a-z0-9]+)*)`\s+"
        r"\[(plan-fixed|implementation-produced)\]\s+—\s+\S.*$",
        re.MULTILINE,
    )
    declarations = heading.findall(body)
    ids = [contract for contract, _ in declarations]
    errors = []
    if len(ids) != len(set(ids)):
        errors.append("shared architectural contract ids must be unique")
    for line in body.splitlines():
        if line.startswith("### ") and not heading.fullmatch(line):
            errors.append(f"malformed shared contract heading: {line}")
    outside = plan[:start] + plan[end:]
    if re.search(r"^###\s+`[^`]+`", outside, re.MULTILINE):
        errors.append("contract-like heading appears outside the shared architectural contracts section")
    return dict(declarations), errors


def footprint_is_valid(footprint):
    if not isinstance(footprint, dict) or not set(footprint) <= {"creates", "touches", "uncertainty"}:
        return False
    creates, touches, uncertainty = footprint.get("creates", []), footprint.get("touches", []), footprint.get("uncertainty")
    return (
        strings(creates)
        and strings(touches)
        and (uncertainty is None or isinstance(uncertainty, str) and uncertainty.strip())
        and bool(creates or touches or uncertainty)
    )


def dependency_reachable(key, target, by_key):
    dependencies = by_key.get(key, {}).get("depends_on", [])
    pending = list(dependencies) if strings(dependencies) else []
    seen = set()
    while pending:
        dependency = pending.pop()
        if dependency == target:
            return True
        if dependency in seen or dependency not in by_key:
            continue
        seen.add(dependency)
        dependencies = by_key[dependency].get("depends_on", [])
        if strings(dependencies):
            pending.extend(dependencies)
    return False


def reaches_finer(key, level, by_key, type_levels):
    dependencies = by_key[key].get("depends_on", [])
    pending = list(dependencies) if strings(dependencies) else []
    seen = set()
    while pending:
        dependency = pending.pop()
        if dependency in seen or dependency not in by_key:
            continue
        seen.add(dependency)
        candidate = by_key[dependency]
        candidate_level = type_levels.get(candidate.get("type"))
        if candidate_level is not None and candidate_level > level:
            return True
        dependencies = candidate.get("depends_on", [])
        if strings(dependencies):
            pending.extend(dependencies)
    return False


def validate(entries, terminal_types, type_levels, known_sources, required_sources, required_criteria, contracts, contract_errors=()):
    errors, warnings, overridden, advisories = list(contract_errors), [], [], []
    if not isinstance(entries, list):
        return ["manifest root must be a bare JSON array"], [], [], []
    if not entries:
        return ["manifest must contain at least one issue"], [], [], []
    keys = [entry.get("key") for entry in entries if isinstance(entry, dict) and isinstance(entry.get("key"), str)]
    if len(keys) != len(set(keys)):
        errors.append("keys must be unique")
    key_set = set(keys)
    covered, satisfied = set(), set()
    for index, entry in enumerate(entries):
        at = f"entry[{index}]"
        if not isinstance(entry, dict):
            errors.append(f"{at} must be an object")
            continue
        missing, extra = REQUIRED - set(entry), set(entry) - REQUIRED
        if missing:
            errors.append(f"{at} missing: {', '.join(sorted(missing))}")
        if extra:
            errors.append(f"{at} has unsupported fields: {', '.join(sorted(extra))}")
        key = entry.get("key", "")
        if not isinstance(key, str) or not KEY.fullmatch(key) or ORDINAL.fullmatch(key):
            errors.append(f"{at}.key must be semantic kebab-case, not an ordinal")
        for field in ("title", "description", "type"):
            if not isinstance(entry.get(field), str) or not entry.get(field, "").strip():
                errors.append(f"{at}.{field} must be non-empty")
        if not isinstance(entry.get("priority"), str) or entry["priority"] not in {"low", "normal", "high", "critical"}:
            errors.append(f"{at}.priority is invalid")
        for field in ("labels", "gates", "depends_on"):
            if not strings(entry.get(field)):
                errors.append(f"{at}.{field} must be a unique string array")
        labels = entry.get("labels")
        if strings(labels):
            satisfied.update(label.removeprefix("satisfies:") for label in labels if label.startswith("satisfies:"))
        dependencies = entry.get("depends_on")
        if strings(dependencies):
            for dep in dependencies:
                if dep not in key_set:
                    errors.append(f"{at}.depends_on references unknown key '{dep}'")
        if type_levels and isinstance(entry.get("type"), str) and entry.get("type") not in type_levels:
            errors.append(f"{at}.type '{entry.get('type')}' is not configured")
        if type_levels is None and isinstance(entry.get("type"), str) and entry.get("type") not in terminal_types:
            errors.append(f"{at}.type cannot be checked for non-terminal aggregation without a hierarchy config")
        if isinstance(entry.get("description"), str) and "## Success Criteria" not in entry["description"]:
            errors.append(f"{at}.description lacks ## Success Criteria")
        planning = entry.get("planning")
        if not isinstance(planning, dict):
            errors.append(f"{at}.planning must be an object")
            continue
        extra = set(planning) - PLANNING
        if extra:
            errors.append(f"{at}.planning has unsupported fields: {', '.join(sorted(extra))}")
        if not isinstance(planning.get("outcome"), str) or not planning.get("outcome", "").strip():
            errors.append(f"{at}.planning.outcome must be non-empty")
        elif "\n" in planning["outcome"] or len(planning["outcome"]) > 160:
            errors.append(f"{at}.planning.outcome must be one concise line")
        if not strings(planning.get("contract_refs")):
            errors.append(f"{at}.planning.contract_refs must be a unique string array")
        else:
            for contract in planning["contract_refs"]:
                if not KEY.fullmatch(contract):
                    errors.append(f"{at}.planning.contract_refs contains non-semantic id '{contract}'")
                elif contracts is None:
                    errors.append(f"{at}.planning.contract_refs cannot be checked without --plan")
                elif contracts is not None and contract not in contracts:
                    errors.append(f"{at}.planning.contract_refs names undeclared contract '{contract}'")
        produces = planning.get("produces_contracts", [])
        if not strings(produces):
            errors.append(f"{at}.planning.produces_contracts must be a unique string array")
        elif any(not KEY.fullmatch(contract) for contract in produces):
            errors.append(f"{at}.planning.produces_contracts must contain semantic ids")
        if not strings(planning.get("source_refs")) or not planning.get("source_refs"):
            errors.append(f"{at}.planning.source_refs must be a non-empty unique string array")
        if strings(planning.get("source_refs")):
            covered.update(planning["source_refs"])
        landing = planning.get("landing_group")
        if landing is not None and (not isinstance(landing, str) or not KEY.fullmatch(landing)):
            errors.append(f"{at}.planning.landing_group must be semantic kebab-case")
        terminal = planning.get("terminal")
        is_terminal = isinstance(entry.get("type"), str) and entry["type"] in terminal_types
        if is_terminal and not isinstance(terminal, dict):
            errors.append(f"{at}.planning.terminal is required for finest-tier issues")
        if terminal is not None:
            valid_terminal = (
                isinstance(terminal, dict)
                and TERMINAL_REQUIRED <= set(terminal) <= TERMINAL_ALLOWED
                and all(isinstance(terminal.get(field), str) and terminal[field].strip() for field in TERMINAL_TEXT)
                and footprint_is_valid(terminal.get("footprint"))
            )
            if not valid_terminal:
                errors.append(f"{at}.planning.terminal must contain consumer_family, test_boundary, worker_sized_reason, a non-empty footprint, and optional warning_overrides")
            else:
                overrides = terminal.get("warning_overrides", {})
                if not isinstance(overrides, dict) or any(code not in WARNING_CODES or not isinstance(reason, str) or not reason.strip() for code, reason in overrides.items()):
                    errors.append(f"{at}.planning.terminal.warning_overrides must map stable warning codes to non-empty reasons")
                    overrides = {}
                findings = sizing_warnings(entry) if is_terminal else []
                found_codes = {code for code, _ in findings}
                for code, message in findings:
                    if code in overrides:
                        overridden.append(f"{at}: {code} overridden: {overrides[code]}")
                    else:
                        warnings.append(f"{at}: {code}: {message}")
                unused = set(overrides) - found_codes
                if unused:
                    errors.append(f"{at}.planning.terminal.warning_overrides has unused codes: {', '.join(sorted(unused))}")
    if cycle(key_set, entries):
        # Reachability in a cyclic graph makes almost every edge look implied, so the
        # reduction check would bury the cycle under noise. Report the cycle alone.
        errors.append("depends_on graph contains a cycle")
    else:
        for node, dep in redundant_edges(key_set, entries):
            errors.append(
                f"{node}.depends_on lists '{dep}', which a longer path already implies; "
                "repository integrity requires a transitively reduced graph"
            )
    by_key = {entry["key"]: entry for entry in entries if isinstance(entry, dict) and isinstance(entry.get("key"), str)}
    if contracts is not None:
        producers = {}
        for entry in by_key.values():
            planning = entry.get("planning")
            if not isinstance(planning, dict) or not strings(planning.get("produces_contracts", [])):
                continue
            for contract in planning.get("produces_contracts", []):
                if contract not in contracts:
                    errors.append(f"entry '{entry['key']}' produces undeclared contract '{contract}'")
                elif contracts[contract] == "plan-fixed":
                    errors.append(f"entry '{entry['key']}' cannot produce plan-fixed contract '{contract}'")
                producers.setdefault(contract, []).append(entry["key"])
        for contract, mode in contracts.items():
            if mode == "implementation-produced" and len(producers.get(contract, [])) != 1:
                errors.append(f"implementation-produced contract '{contract}' must have exactly one producer")
        for entry in by_key.values():
            planning = entry.get("planning")
            if not isinstance(planning, dict) or not strings(planning.get("contract_refs")):
                continue
            for contract in planning["contract_refs"]:
                candidates = producers.get(contract, [])
                if contracts.get(contract) == "implementation-produced" and len(candidates) == 1:
                    producer = candidates[0]
                    if producer != entry["key"] and not dependency_reachable(entry["key"], producer, by_key):
                        errors.append(f"entry '{entry['key']}' references contract '{contract}' without depending on producer '{producer}'")
    missing_sources = set(required_sources) - covered
    if missing_sources:
        errors.append(f"source coverage missing: {', '.join(sorted(missing_sources))}")
    undeclared_required_sources = set(required_sources) - set(known_sources)
    if undeclared_required_sources:
        errors.append(f"required sources are outside the known-source universe: {', '.join(sorted(undeclared_required_sources))}")
    unknown_sources = covered - set(known_sources)
    if unknown_sources:
        errors.append(f"unknown source refs: {', '.join(sorted(unknown_sources))}")
    missing_criteria = set(required_criteria) - satisfied
    if missing_criteria:
        errors.append(f"satisfies coverage missing: {', '.join(sorted(missing_criteria))}")
    unknown_criteria = satisfied - set(required_criteria) if required_criteria else set()
    if unknown_criteria:
        errors.append(f"unknown satisfies labels: {', '.join(sorted(unknown_criteria))}")
    if type_levels:
        finest = max(type_levels.values())
        for index, entry in enumerate(entries):
            issue_type = entry.get("type") if isinstance(entry, dict) else None
            level = type_levels.get(issue_type) if isinstance(issue_type, str) else None
            key = entry.get("key") if isinstance(entry, dict) else None
            if level is not None and level < finest and isinstance(key, str) and key in by_key and not reaches_finer(key, level, by_key, type_levels):
                errors.append(f"entry[{index}] non-finest issue must depend on a strictly finer in-manifest descendant")
    footprints = {}
    for entry in by_key.values():
        terminal = entry.get("planning", {}).get("terminal") if isinstance(entry.get("planning"), dict) else None
        footprint = terminal.get("footprint") if isinstance(terminal, dict) else None
        if not footprint_is_valid(footprint):
            continue
        if footprint.get("uncertainty"):
            advisories.append(f"entry '{entry['key']}' footprint uncertainty: {footprint['uncertainty']}")
        for kind in ("creates", "touches"):
            for path in footprint.get(kind, []):
                footprints.setdefault(path, []).append(f"{entry['key']}:{kind}")
    for path, owners in footprints.items():
        if len(owners) > 1:
            advisories.append(f"footprint overlap '{path}': {', '.join(owners)}")
    return errors, warnings, overridden, advisories


def cell(value):
    return str(value).replace("|", "\\|").replace("\n", " ")


def overview(entries):
    rows = ["| Key | Title | Type | Outcome | Contracts | Sources | Footprint | Landing | Depends on |", "|---|---|---|---|---|---|---|---|---|"]
    for entry in entries:
        p = entry["planning"]
        footprint = p.get("terminal", {}).get("footprint", {})
        # Counts, not paths: the manifest is the authority for exact footprints, and
        # rendering them inline scales the overview with the graph until the plan stops
        # being concise.
        creates, touches = footprint.get("creates", []), footprint.get("touches", [])
        footprint_text = ", ".join([
            *([f"creates {len(creates)}"] if creates else []),
            *([f"touches {len(touches)}"] if touches else []),
            *(["uncertain"] if footprint.get("uncertainty") else []),
        ]) or "—"
        values = (entry["key"], entry["title"], entry["type"], p["outcome"], ", ".join(p["contract_refs"]) or "—", ", ".join(p["source_refs"]), footprint_text, p.get("landing_group", "—"), ", ".join(entry["depends_on"]) or "—")
        rows.append("| " + " | ".join(map(cell, values)) + " |")
    nodes = [f'    N{i}["{entry["key"]}: {entry["title"].replace(chr(34), chr(39))}"]' for i, entry in enumerate(entries)]
    indexes = {entry["key"]: i for i, entry in enumerate(entries)}
    edges = [f"    N{indexes[dep]} --> N{i}" for i, entry in enumerate(entries) for dep in entry["depends_on"]]
    return "\n".join([BEGIN, *rows, "", "```mermaid", "flowchart LR", *nodes, *edges, "```", END])


def render(args):
    entries, path = load(args.manifest), Path(args.plan)
    text = path.read_text()
    if text.count(BEGIN) != 1 or text.count(END) != 1 or text.index(BEGIN) > text.index(END):
        raise ValueError("plan must contain one ordered generated overview region")
    expected = overview(entries)
    actual = text[text.index(BEGIN): text.index(END) + len(END)]
    if args.check:
        if actual != expected:
            raise ValueError("plan breakdown overview is stale")
    else:
        replacement = text.replace(actual, expected)
        with tempfile.NamedTemporaryFile("w", dir=path.parent, delete=False) as handle:
            handle.write(replacement)
            staged = Path(handle.name)
        staged.chmod(path.stat().st_mode & 0o777)
        staged.replace(path)


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    check = sub.add_parser("validate")
    check.add_argument("manifest")
    check.add_argument("--config", default=".jit/config.toml")
    check.add_argument("--terminal-type", action="append", default=[])
    check.add_argument("--known-source", action="append", required=True)
    check.add_argument("--required-source", action="append", default=[])
    check.add_argument("--required-criterion", action="append", default=[])
    check.add_argument("--plan")
    check.add_argument("--deny-warnings", action="store_true")
    check.add_argument("--json", action="store_true")
    draw = sub.add_parser("render")
    draw.add_argument("manifest")
    draw.add_argument("plan")
    mode = draw.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    suite = sub.add_parser("conformance")
    suite.add_argument("fixture")
    suite.add_argument("--json", action="store_true")
    args = parser.parse_args()
    try:
        if args.command == "render":
            render(args)
            return
        if args.command == "conformance":
            failures = check_conformance(args.fixture)
            result = {"valid": not failures, "failures": failures}
            print(json.dumps(result, indent=2) if args.json else "valid" if not failures else json.dumps(failures, indent=2))
            if failures:
                sys.exit(1)
            return
        terminal, levels = type_levels(args.config, args.terminal_type)
        contracts = None
        contract_errors = []
        if args.plan:
            plan = Path(args.plan).read_text()
            contracts, contract_errors = contract_modes(plan)
        errors, warnings, overridden, advisories = validate(
            load(args.manifest), terminal, levels, args.known_source, args.required_source,
            args.required_criterion, contracts, contract_errors
        )
        result = {
            "valid": not errors and not (args.deny_warnings and warnings),
            "errors": errors,
            "warnings": warnings,
            "overridden_warnings": overridden,
            "advisories": advisories,
        }
        lines = [
            *(f"error: {error}" for error in errors),
            *(f"warning: {warning}" for warning in warnings),
            *(f"overridden: {override}" for override in overridden),
            *(f"advisory: {advisory}" for advisory in advisories),
        ]
        print(json.dumps(result, indent=2) if args.json else "\n".join(lines) or "valid")
        if not result["valid"]:
            sys.exit(1)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
