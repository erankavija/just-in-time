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
PLANNING = {"outcome", "contract_refs", "source_refs", "landing_group", "terminal"}
TERMINAL = {"consumer_family", "test_boundary", "worker_sized_reason"}


def load(path):
    return json.loads(Path(path).read_text())


def type_levels(config_path, explicit):
    if explicit:
        return set(explicit), None
    path = Path(config_path)
    if not path.exists() or tomllib is None:
        return set(), None
    types = tomllib.loads(path.read_text()).get("type_hierarchy", {}).get("types", {})
    return ({name for name, level in types.items() if level == max(types.values())}, set(types)) if types else (set(), None)


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


def sizing_warnings(entry):
    planning = entry["planning"]
    terminal = planning.get("terminal")
    if not terminal:
        return []
    title = str(entry.get("title", "")).lower()
    outcome = str(planning.get("outcome", "")).lower()
    text = f"{title} {outcome}"
    description = entry.get("description") if isinstance(entry.get("description"), str) else ""
    evidence = terminal["worker_sized_reason"].lower().startswith("indivisible because")
    warnings = []
    if re.search(r"\b(all|every|entire|repo-wide|across the (?:repo|codebase|project))\b", text) and not evidence:
        warnings.append("uses a broad quantifier")
    verb = r"(?:add|build|create|define|delete|deploy|document|implement|migrate|publish|release|remove|render|update|validate|wire)"
    independent_verbs = any(
        re.search(rf"\b{verb}\w*\b.*(?:[;,.]|\band\b).*\b{verb}\w*\b", field)
        for field in (title, outcome)
    )
    if independent_verbs and not evidence:
        warnings.append("states multiple independent verbs")
    if re.search(r"[,/]|\band\b", terminal["consumer_family"].lower()):
        warnings.append("names more than one consumer family")
    if re.search(r"[,/]|\band\b", terminal["test_boundary"].lower()):
        warnings.append("names more than one test boundary")
    categories = sum(bool(re.search(rf"\b{word}\w*\b", text)) for word in ("foundation", "migrat", "delet", "document", "releas"))
    if categories > 1 and not evidence:
        warnings.append("mixes independently testable deliverable categories")
    if len(re.findall(r"^###\s+", description, re.MULTILINE)) >= 3 and not evidence:
        warnings.append("contains three or more acceptance clusters")
    if re.search(r"\b(implement|migrate|update)\w*\b", text) and re.search(r"\b(release|publish|deploy)\w*\b", text) and not evidence:
        warnings.append("combines implementation with release work")
    if len(terminal["worker_sized_reason"].split()) < 4:
        warnings.append("does not explain why the work fits one focused cycle")
    return warnings


def validate(entries, terminal_types, known_types, required_sources, required_criteria, contracts):
    errors, warnings = [], []
    if not isinstance(entries, list):
        return ["manifest root must be a bare JSON array"], []
    if not entries:
        return ["manifest must contain at least one issue"], []
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
        if known_types and isinstance(entry.get("type"), str) and entry.get("type") not in known_types:
            errors.append(f"{at}.type '{entry.get('type')}' is not configured")
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
                elif contracts is not None and contract not in contracts:
                    errors.append(f"{at}.planning.contract_refs names undeclared contract '{contract}'")
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
            if not isinstance(terminal, dict) or set(terminal) != TERMINAL or not all(isinstance(terminal.get(f), str) and terminal[f].strip() for f in TERMINAL):
                errors.append(f"{at}.planning.terminal must contain exactly consumer_family, test_boundary, worker_sized_reason")
            elif is_terminal:
                warnings.extend(f"{at}: {warning}" for warning in sizing_warnings(entry))
    if cycle(key_set, entries):
        errors.append("depends_on graph contains a cycle")
    missing_sources = set(required_sources) - covered
    if missing_sources:
        errors.append(f"source coverage missing: {', '.join(sorted(missing_sources))}")
    missing_criteria = set(required_criteria) - satisfied
    if missing_criteria:
        errors.append(f"satisfies coverage missing: {', '.join(sorted(missing_criteria))}")
    unknown_criteria = satisfied - set(required_criteria) if required_criteria else set()
    if unknown_criteria:
        errors.append(f"unknown satisfies labels: {', '.join(sorted(unknown_criteria))}")
    return errors, warnings


def cell(value):
    return str(value).replace("|", "\\|").replace("\n", " ")


def overview(entries):
    rows = ["| Key | Title | Type | Outcome | Contracts | Sources | Landing | Depends on |", "|---|---|---|---|---|---|---|---|"]
    for entry in entries:
        p = entry["planning"]
        values = (entry["key"], entry["title"], entry["type"], p["outcome"], ", ".join(p["contract_refs"]) or "—", ", ".join(p["source_refs"]), p.get("landing_group", "—"), ", ".join(entry["depends_on"]) or "—")
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
    args = parser.parse_args()
    try:
        if args.command == "render":
            render(args)
            return
        terminal, known = type_levels(args.config, args.terminal_type)
        contracts = None
        if args.plan:
            plan = Path(args.plan).read_text()
            contracts = set(re.findall(r"^###\s+`([a-z][a-z0-9-]*)`", plan, re.MULTILINE))
        errors, warnings = validate(load(args.manifest), terminal, known, args.required_source, args.required_criterion, contracts)
        result = {"valid": not errors and not (args.deny_warnings and warnings), "errors": errors, "warnings": warnings}
        print(json.dumps(result, indent=2) if args.json else "\n".join([*(f"error: {e}" for e in errors), *(f"warning: {w}" for w in warnings)]) or "valid")
        if not result["valid"]:
            sys.exit(1)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
