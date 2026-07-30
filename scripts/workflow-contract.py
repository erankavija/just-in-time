#!/usr/bin/env python3
"""workflow-contract — structural contract verifier for GitHub workflows.

The repository declares, in one contract file, what each committed workflow has
to be: which events trigger it, what its `workflow_call` interface is, which
`needs` edges its job graph guarantees, which permissions its token carries, and
that it contains no construct that turns a red step into a green run. This
program parses the workflows and checks those declarations. `actionlint` (run by
the entry point, `workflow-contract.sh`) validates schema and expression syntax;
everything here is a repository policy no linter can know.

The contract is a declaration, not a verifier copy: a workflow adds assertions by
adding an entry to `.github/workflow-contract.yml`, never by embedding a checking
step in its own YAML.

Assertions
----------
`uses:` pinning (global)
    Every external action and reusable-workflow reference resolves to an exact
    40-character lowercase hexadecimal commit SHA and carries a trailing comment
    naming the upstream version that SHA was resolved from. The scan starts at
    every committed workflow and follows local `./…` references into composite
    action definitions recursively, so a reference cannot hide one level down.
    A local reference that the tree does not contain is a finding, and a
    `docker://` reference must carry an image digest.

Failure escapes (global)
    No `continue-on-error`, no `if:` condition that survives a failed
    predecessor (`always()`, `cancelled()`, or `failure()` in a disjunction),
    and no shell-level suppression (`|| true`, `|| :`, `set +e`) in a `run:`
    script. Matrix `fail-fast: false` is not an escape: it changes how sibling
    matrix legs are cancelled, not whether a failure fails the run.

Publication (global)
    One declared workflow publishes the GitHub release, and no workflow
    publishes to a package or container registry. Both are read from the
    constructs a step runs — the action it uses, or the command its script
    carries — following local composite actions, so a second publication path
    cannot hide one level down.

Triggers, `workflow_call`, `needs`, permissions, calls (per workflow)
    Checked against that workflow's declaration; see the contract file and
    dev/workflow-contract.md for the declaration grammar.

Caller obligations (per reusable workflow)
    A reusable workflow names the jobs a caller inherits by calling it. Each
    has to exist and to run unconditionally, and every job a caller runs of its
    own has to reach the call through `needs`, so nothing of the caller's
    starts before every inherited job has succeeded on the same commit.

Exit codes
----------
0   every declared assertion holds
1   one or more findings
2   environment or contract error — missing tooling, missing or unreadable
    contract, unsupported contract version, missing workflow directory. The
    verifier never treats an unusable contract as a pass.
"""

from __future__ import annotations

import argparse
import re
import sys
from collections import namedtuple
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover - exercised only without the dependency
    sys.stderr.write(
        "workflow-contract: the PyYAML module is required "
        "(python3 -m pip install PyYAML)\n"
    )
    raise SystemExit(2)

EXIT_OK = 0
EXIT_FINDINGS = 1
EXIT_ENV = 2

SUPPORTED_CONTRACT_VERSION = 1

CONTRACT_RELPATH = Path(".github/workflow-contract.yml")
WORKFLOW_RELDIR = Path(".github/workflows")
WORKFLOW_SUFFIXES = (".yml", ".yaml")

Finding = namedtuple("Finding", "path line message")
Workflow = namedtuple("Workflow", "name path relative doc")


def finding(path: Path, message: str, line: int | None = None) -> Finding:
    return Finding(path=path, line=line, message=message)


# --------------------------------------------------------------------------
# YAML loading
# --------------------------------------------------------------------------
#
# GitHub reads workflow YAML under the 1.2 core schema, where `on` is the string
# key "on". PyYAML defaults to the 1.1 resolver, which turns `on`, `off`, `yes`
# and `no` into booleans and would leave every workflow looking as though it
# declared no triggers at all. This loader restricts implicit boolean resolution
# to `true`/`false`, matching what GitHub actually parses.
class WorkflowLoader(yaml.SafeLoader):
    """SafeLoader whose implicit boolean resolution matches GitHub's parser."""


WorkflowLoader.yaml_implicit_resolvers = {
    key: [(tag, regexp) for tag, regexp in resolvers if tag != "tag:yaml.org,2002:bool"]
    for key, resolvers in yaml.SafeLoader.yaml_implicit_resolvers.items()
}
WorkflowLoader.add_implicit_resolver(
    "tag:yaml.org,2002:bool",
    re.compile(r"^(?:true|True|TRUE|false|False|FALSE)$"),
    list("tTfF"),
)


def load_yaml(path: Path):
    """Parse `path`, returning (document, error-message). Exactly one is None."""
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        return None, f"could not be read ({exc.strerror})"
    try:
        return yaml.load(text, Loader=WorkflowLoader), None
    except yaml.YAMLError as exc:
        return None, f"is not parseable YAML ({str(exc).splitlines()[0]})"


def as_mapping(value):
    return value if isinstance(value, dict) else {}


def as_list(value):
    """Normalize a scalar-or-sequence YAML value into a list of strings."""
    if value is None:
        return []
    if isinstance(value, (list, tuple)):
        return [str(item) for item in value]
    return [str(value)]


def is_truthy(value) -> bool:
    """GitHub accepts both the boolean and its string spellings."""
    if isinstance(value, bool):
        return value
    return str(value).strip().lower() in {"true", "yes", "on", "1"}


def is_explicitly_false(value) -> bool:
    """True only for a literal negative.

    An escape key set from an expression (`continue-on-error: ${{ … }}`) is not
    literally false, so it stays a finding: what it evaluates to at run time is
    exactly what a static check cannot know.
    """
    if isinstance(value, bool):
        return not value
    return str(value).strip().lower() in {"false", "no", "off", "0"}


# --------------------------------------------------------------------------
# Shared shape helpers
# --------------------------------------------------------------------------
def workflow_jobs(doc) -> dict:
    return {str(name): as_mapping(job) for name, job in as_mapping(doc).get("jobs", {}).items()}


def job_uses(job) -> str:
    """The workflow or action a job runs in place of steps, or the empty string."""
    return str(as_mapping(job).get("uses") or "")


def step_label(index: int, step) -> str:
    """A stable human-readable identity for a step: its name, else what it runs."""
    step = as_mapping(step)
    if step.get("name"):
        return f"step {index} ({step['name']})"
    if step.get("uses"):
        return f"step {index} (uses {step['uses']})"
    return f"step {index}"


def normalize_permissions(value):
    """Permissions are either a whole-token string or a scope map."""
    if value is None:
        return None
    if isinstance(value, dict):
        return {str(scope): str(level) for scope, level in value.items()}
    return str(value)


def normalize_triggers(doc) -> dict:
    """Normalize the `on:` value into {event: configuration}."""
    events = as_mapping(doc).get("on")
    if isinstance(events, str):
        return {events: {}}
    if isinstance(events, (list, tuple)):
        return {str(event): {} for event in events}
    if isinstance(events, dict):
        return {str(event): ({} if cfg is None else cfg) for event, cfg in events.items()}
    return {}


def trigger_filter_values(event: str, config, key: str) -> list[str]:
    """The values a trigger declares for one filter key.

    `schedule` is the odd one out: its configuration is a list of `{cron: …}`
    entries rather than a mapping of filter keys.
    """
    if event == "schedule":
        return [
            str(entry.get("cron"))
            for entry in (config if isinstance(config, (list, tuple)) else [])
            if isinstance(entry, dict) and entry.get("cron") is not None
        ]
    return as_list(as_mapping(config).get(key))


# --------------------------------------------------------------------------
# `uses:` references — pinning, locality, recursion
# --------------------------------------------------------------------------
SHA_PIN = re.compile(r"^(?P<action>[^@\s]+)@(?P<sha>[0-9a-f]{40})$")
DOCKER_DIGEST = re.compile(r"^docker://[^@\s]+@sha256:[0-9a-f]{64}$")
RAW_USES = re.compile(
    r"^\s*(?:-\s+)?uses:\s*(?P<ref>[^\s#]+?)\s*(?:#\s*(?P<comment>.*?))?\s*$"
)

RawUse = namedtuple("RawUse", "line ref comment")


def raw_uses(path: Path) -> list[RawUse]:
    """Every literal `uses:` line, with the trailing comment the parser discards."""
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError:
        return []
    found = []
    for number, text in enumerate(lines, start=1):
        match = RAW_USES.match(text)
        if match:
            ref = match.group("ref").strip("\"'")
            found.append(RawUse(number, ref, (match.group("comment") or "").strip()))
    return found


def document_uses(doc, is_action: bool) -> list[str]:
    """Every `uses:` value the document actually declares."""
    if is_action:
        steps = as_mapping(as_mapping(doc).get("runs")).get("steps") or []
        return [
            str(as_mapping(step)["uses"])
            for step in steps
            if as_mapping(step).get("uses")
        ]
    refs = []
    for job in workflow_jobs(doc).values():
        if job.get("uses"):  # a reusable-workflow call
            refs.append(str(job["uses"]))
        for step in job.get("steps") or []:
            if as_mapping(step).get("uses"):
                refs.append(str(as_mapping(step)["uses"]))
    return refs


def resolve_local_reference(root: Path, ref: str):
    """Resolve a `./…` reference to the file that defines it, or None."""
    target = (root / ref[len("./") :]).resolve()
    try:
        target.relative_to(root.resolve())
    except ValueError:  # escapes the repository root
        return None
    if target.is_dir():
        for name in ("action.yml", "action.yaml"):
            candidate = target / name
            if candidate.is_file():
                return candidate
        return None
    return target if target.is_file() else None


def check_uses(root: Path, entry: Path, is_action: bool, doc, visited: set) -> list[Finding]:
    """Check every `uses:` in one document and recurse into local definitions."""
    findings = []
    declared = document_uses(doc, is_action)
    literals = raw_uses(entry)
    literal_refs = {use.ref for use in literals}
    relative = entry.relative_to(root)

    for ref in declared:
        if ref not in literal_refs:
            # Fail closed: an unlocatable literal means the pin comment cannot be
            # read, so the reference cannot be certified either way.
            findings.append(
                finding(relative, f"could not locate the literal `uses:` line for {ref!r}")
            )

    for use in literals:
        if use.ref not in declared:
            continue  # incidental text (e.g. inside a `run:` script)
        if use.ref.startswith("./"):
            resolved = resolve_local_reference(root, use.ref)
            if resolved is None:
                findings.append(
                    finding(
                        relative,
                        f"local reference {use.ref!r} does not resolve to a file in the repository",
                        use.line,
                    )
                )
                continue
            if resolved not in visited:
                visited.add(resolved)
                findings.extend(check_document(root, resolved, visited))
            continue
        if use.ref.startswith("docker://"):
            if not DOCKER_DIGEST.match(use.ref):
                findings.append(
                    finding(
                        relative,
                        f"container reference {use.ref!r} is not pinned to an image digest",
                        use.line,
                    )
                )
            continue
        if not SHA_PIN.match(use.ref):
            findings.append(
                finding(
                    relative,
                    f"external reference {use.ref!r} is not pinned to a 40-character commit SHA",
                    use.line,
                )
            )
            continue
        if not use.comment:
            findings.append(
                finding(
                    relative,
                    f"external reference {use.ref!r} carries no upstream-version comment",
                    use.line,
                )
            )
    return findings


def check_document(root: Path, path: Path, visited: set) -> list[Finding]:
    """Check a local composite action or reusable workflow reached by recursion."""
    doc, error = load_yaml(path)
    if error:
        return [finding(path.relative_to(root), error)]
    is_action = path.name in ("action.yml", "action.yaml")
    findings = check_uses(root, path, is_action, doc, visited)
    if is_action:
        steps = as_mapping(as_mapping(doc).get("runs")).get("steps") or []
        findings.extend(check_steps_for_escapes(path.relative_to(root), "composite", steps))
    return findings


# --------------------------------------------------------------------------
# Failure escapes
# --------------------------------------------------------------------------
SHELL_ESCAPES = ("|| true", "|| :", "||true", "set +e", "set +o errexit", "|| exit 0")


def check_condition(relative: Path, subject: str, condition) -> list[Finding]:
    """A condition must not let its subject run after a failed predecessor."""
    if condition is None:
        return []
    text = str(condition).replace("${{", " ").replace("}}", " ").lower()
    if "always()" in text:
        return [
            finding(
                relative,
                f"{subject} condition uses 'always()', which runs it after a failed predecessor",
            )
        ]
    if "cancelled()" in text or ("failure()" in text and "||" in text):
        return [
            finding(
                relative,
                f"{subject} condition tolerates a failed predecessor: {str(condition)!r}",
            )
        ]
    return []


def check_steps_for_escapes(relative: Path, subject: str, steps) -> list[Finding]:
    findings = []
    for index, raw_step in enumerate(steps or [], start=1):
        step = as_mapping(raw_step)
        label = f"{subject} {step_label(index, step)}"
        if "continue-on-error" in step and not is_explicitly_false(step["continue-on-error"]):
            findings.append(
                finding(relative, f"{label} sets 'continue-on-error', which suppresses failure")
            )
        findings.extend(check_condition(relative, label, step.get("if")))
        script = str(step.get("run") or "")
        for escape in SHELL_ESCAPES:
            if escape in script:
                findings.append(
                    finding(relative, f"{label} run script suppresses failure with {escape!r}")
                )
    return findings


def check_failure_escapes(relative: Path, doc) -> list[Finding]:
    findings = []
    for name, job in workflow_jobs(doc).items():
        subject = f"job {name!r}"
        if "continue-on-error" in job and not is_explicitly_false(job["continue-on-error"]):
            findings.append(
                finding(relative, f"{subject} sets 'continue-on-error', which suppresses failure")
            )
        findings.extend(check_condition(relative, subject, job.get("if")))
        findings.extend(check_steps_for_escapes(relative, subject, job.get("steps")))
    return findings


# --------------------------------------------------------------------------
# Publication
# --------------------------------------------------------------------------
#
# What a workflow publishes is read from what its steps run. A release
# construct belongs to the one workflow the contract names; a registry
# construct belongs nowhere, because the release's whole published output is
# one GitHub release (@/charter/D-16, @/charter/D-9). Packaging is not
# publication: `npm pack` writes a tarball into the workspace and reaches no
# registry, which is how the MCP server ships as a release asset.
#
# Each command marker is a tuple of substrings that all have to appear in the
# same script, so the REST form of a release call is caught without every `gh
# api` call being read as publication.
RELEASE_PUBLICATION_ACTIONS = (
    "softprops/action-gh-release",
    "ncipollo/release-action",
    "actions/create-release",
)
RELEASE_PUBLICATION_COMMANDS = (
    ("gh release create",),
    ("gh release upload",),
    ("gh release edit",),
    ("gh release delete",),
    ("gh api", "/releases"),
)
# `docker/build-push-action` is named whatever its `push:` input says: this
# repository builds its image from a Dockerfile in a test of its own, so the
# action reappearing at all is the review question.
REGISTRY_PUBLICATION_ACTIONS = (
    "docker/build-push-action",
    "redhat-actions/push-to-registry",
    "JS-DevTools/npm-publish",
)
REGISTRY_PUBLICATION_COMMANDS = (
    ("npm publish",),
    ("yarn publish",),
    ("pnpm publish",),
    ("cargo publish",),
    ("docker push",),
    ("podman push",),
    ("buildah push",),
    ("skopeo copy",),
    ("docker buildx build", "--push"),
)


def publication_constructs(step, actions, commands) -> list[str]:
    """The publication constructs one step carries, named as they are written."""
    step = as_mapping(step)
    used = str(step.get("uses") or "").split("@")[0]
    script = str(step.get("run") or "")
    return [action for action in actions if used == action] + [
        " ".join(marker)
        for marker in commands
        if all(part in script for part in marker)
    ]


def composite_steps(root: Path, ref: str, seen: set) -> list:
    """Every step a local composite action runs, recursively, with its label."""
    if not ref.startswith("./"):
        return []
    resolved = resolve_local_reference(root, ref)
    if resolved is None or resolved in seen:
        return []
    seen.add(resolved)
    doc, error = load_yaml(resolved)
    if error:
        return []  # reported as a finding by the `uses:` scan
    collected = []
    for index, raw_step in enumerate(as_mapping(as_mapping(doc).get("runs")).get("steps") or [], start=1):
        step = as_mapping(raw_step)
        collected.append((f"{ref} {step_label(index, step)}", step))
        collected.extend(composite_steps(root, str(step.get("uses") or ""), seen))
    return collected


def workflow_steps(root: Path, doc) -> list:
    """Every step a workflow runs, following the local composite actions it uses."""
    collected = []
    seen: set = set()
    for name, job in sorted(workflow_jobs(doc).items()):
        for index, raw_step in enumerate(job.get("steps") or [], start=1):
            step = as_mapping(raw_step)
            collected.append((f"job {name!r} {step_label(index, step)}", step))
            collected.extend(composite_steps(root, str(step.get("uses") or ""), seen))
    return collected


def check_publication(root: Path, workflow: Workflow, publisher: str, forbid_registry: bool) -> list[Finding]:
    """Check that only `publisher` publishes, and that nothing reaches a registry."""
    findings = []
    for label, step in workflow_steps(root, workflow.doc):
        if workflow.name != publisher:
            findings.extend(
                finding(
                    workflow.relative,
                    f"{label} creates a GitHub release with {construct!r}; "
                    f"{WORKFLOW_RELDIR.as_posix()}/{publisher} is the only workflow that publishes",
                )
                for construct in publication_constructs(
                    step, RELEASE_PUBLICATION_ACTIONS, RELEASE_PUBLICATION_COMMANDS
                )
            )
        if forbid_registry:
            findings.extend(
                finding(
                    workflow.relative,
                    f"{label} publishes to a package or container registry with {construct!r}",
                )
                for construct in publication_constructs(
                    step, REGISTRY_PUBLICATION_ACTIONS, REGISTRY_PUBLICATION_COMMANDS
                )
            )
    return findings


# --------------------------------------------------------------------------
# Job graph
# --------------------------------------------------------------------------
def needs_graph(doc) -> dict[str, list[str]]:
    return {name: as_list(job.get("needs")) for name, job in workflow_jobs(doc).items()}


def transitive_predecessors(graph: dict[str, list[str]], job: str) -> set[str]:
    reached, pending = set(), list(graph.get(job, []))
    while pending:
        current = pending.pop()
        if current in reached:
            continue
        reached.add(current)
        pending.extend(graph.get(current, []))
    return reached


def find_cycle(graph: dict[str, list[str]]) -> list[str] | None:
    """Return one cycle as a job path, or None when the graph is acyclic."""
    visiting, done = set(), set()

    def walk(node, trail):
        if node in done:
            return None
        if node in visiting:
            return trail[trail.index(node) :] + [node]
        visiting.add(node)
        for parent in graph.get(node, []):
            if parent in graph:
                cycle = walk(parent, trail + [node])
                if cycle:
                    return cycle
        visiting.discard(node)
        done.add(node)
        return None

    for start in sorted(graph):
        cycle = walk(start, [])
        if cycle:
            return cycle
    return None


def check_job_graph(relative: Path, doc) -> list[Finding]:
    """Structural integrity every workflow owes regardless of its declaration."""
    graph = needs_graph(doc)
    findings = [
        finding(relative, f"job {job!r} needs unknown job {parent!r}")
        for job, parents in sorted(graph.items())
        for parent in parents
        if parent not in graph
    ]
    cycle = find_cycle(graph)
    if cycle:
        findings.append(
            finding(relative, "'needs' graph contains a cycle: " + " -> ".join(cycle))
        )
    return findings


# --------------------------------------------------------------------------
# Declared assertions
# --------------------------------------------------------------------------
def check_triggers(relative: Path, doc, declaration) -> list[Finding]:
    declared = as_mapping(declaration)
    if not declared:
        return []
    actual = normalize_triggers(doc)
    findings = []

    for event, filters in as_mapping(declared.get("required")).items():
        if event not in actual:
            findings.append(finding(relative, f"required trigger {event!r} is not declared"))
            continue
        required_filters = as_mapping(filters)
        for key, values in required_filters.items():
            if key == "forbidden_filters":
                continue
            present = trigger_filter_values(event, actual[event], key)
            findings.extend(
                finding(relative, f"trigger {event!r} declares no {key} value {value!r}")
                for value in as_list(values)
                if value not in present
            )
        actual_filters = as_mapping(actual[event])
        findings.extend(
            finding(relative, f"trigger {event!r} declares forbidden filter {key!r}")
            for key in as_list(required_filters.get("forbidden_filters"))
            if key in actual_filters
        )

    forbidden = [event for event in as_list(declared.get("forbidden")) if event in actual]
    findings.extend(
        finding(relative, f"forbidden trigger {event!r} is declared") for event in forbidden
    )

    allowed = as_list(declared.get("allowed"))
    if allowed:
        # A forbidden event is already reported above; naming it twice would say
        # nothing more about what has to change.
        findings.extend(
            finding(relative, f"trigger {event!r} is outside the declared allowed set")
            for event in sorted(actual)
            if event not in allowed and event not in forbidden
        )
    return findings


def check_workflow_call(relative: Path, doc, declaration) -> list[Finding]:
    declared = as_mapping(declaration)
    if not declared:
        return []
    call = normalize_triggers(doc).get("workflow_call")
    if call is None:
        return [
            finding(
                relative,
                "contract declares a workflow_call interface, but the workflow "
                "declares no 'workflow_call' trigger",
            )
        ]
    call = as_mapping(call)
    findings = []

    inputs = as_mapping(call.get("inputs"))
    declared_inputs = as_mapping(declared.get("inputs"))
    for name, expected in as_mapping(declared_inputs.get("required")).items():
        if name not in inputs:
            findings.append(finding(relative, f"workflow_call input {name!r} is not declared"))
            continue
        actual = as_mapping(inputs[name])
        expected = as_mapping(expected)
        if "type" in expected and str(actual.get("type")) != str(expected["type"]):
            findings.append(
                finding(
                    relative,
                    f"workflow_call input {name!r} declares type {str(actual.get('type'))!r}, "
                    f"the contract requires {str(expected['type'])!r}",
                )
            )
        if "required" in expected and is_truthy(actual.get("required")) != is_truthy(expected["required"]):
            findings.append(
                finding(
                    relative,
                    f"workflow_call input {name!r} declares required "
                    f"{str(is_truthy(actual.get('required'))).lower()!r}, the contract requires "
                    f"{str(is_truthy(expected['required'])).lower()!r}",
                )
            )
    findings.extend(
        _outside_allowed(relative, "input", inputs, declared_inputs.get("allowed"))
    )

    outputs = as_mapping(call.get("outputs"))
    declared_outputs = as_mapping(declared.get("outputs"))
    findings.extend(
        finding(relative, f"workflow_call output {name!r} is not declared")
        for name in as_list(declared_outputs.get("required"))
        if name not in outputs
    )
    findings.extend(
        _outside_allowed(relative, "output", outputs, declared_outputs.get("allowed"))
    )

    declared_secrets = as_mapping(declared.get("secrets"))
    if "allowed" in declared_secrets:
        findings.extend(
            _outside_allowed(
                relative, "secret", as_mapping(call.get("secrets")), declared_secrets["allowed"]
            )
        )
    return findings


def _outside_allowed(relative: Path, kind: str, actual: dict, allowed) -> list[Finding]:
    """A closed set: nothing beyond `allowed` may appear. `None` means unconstrained."""
    if allowed is None:
        return []
    permitted = set(as_list(allowed))
    return [
        finding(relative, f"workflow_call {kind} {name!r} is outside the declared allowed set")
        for name in sorted(actual)
        if str(name) not in permitted
    ]


def check_permissions(relative: Path, doc, declaration, require_workflow_permissions: bool) -> list[Finding]:
    findings = []
    declared = as_mapping(declaration)
    actual = normalize_permissions(as_mapping(doc).get("permissions"))
    has_declaration = "permissions" in declared

    if actual is None and (require_workflow_permissions or has_declaration):
        findings.append(
            finding(
                relative,
                "workflow declares no workflow-level 'permissions' block, so its token "
                "inherits the repository default",
            )
        )
    elif has_declaration:
        expected = normalize_permissions(declared["permissions"])
        if actual != expected:
            findings.append(
                finding(
                    relative,
                    f"workflow-level permissions do not match the contract "
                    f"(contract: {expected!r}, workflow: {actual!r})",
                )
            )

    jobs = workflow_jobs(doc)
    for name, job_declaration in sorted(as_mapping(declared.get("jobs")).items()):
        job_declaration = as_mapping(job_declaration)
        if "permissions" not in job_declaration or name not in jobs:
            continue
        expected = normalize_permissions(job_declaration["permissions"])
        found = normalize_permissions(jobs[name].get("permissions"))
        if found != expected:
            findings.append(
                finding(
                    relative,
                    f"job {name!r} permissions do not match the contract "
                    f"(contract: {expected!r}, workflow: {found!r})",
                )
            )
    return findings


def check_declared_jobs(relative: Path, doc, declaration) -> list[Finding]:
    """Declared jobs must exist, make their declared call, and reach their predecessors.

    A `needs` edge onto a job named `validate` says nothing about what that job
    runs. Declaring the call it makes is what keeps the edge's content: the
    suites a caller inherits are stated by the called workflow, and this is the
    assertion that the call to it is still there to inherit them through.
    """
    jobs = workflow_jobs(doc)
    graph = needs_graph(doc)
    findings = []
    for name, job_declaration in sorted(as_mapping(as_mapping(declaration).get("jobs")).items()):
        if name not in jobs:
            findings.append(
                finding(relative, f"contract declares job {name!r}, which the workflow does not define")
            )
            continue
        required_call = as_mapping(job_declaration).get("uses")
        if required_call is not None and job_uses(jobs[name]) != str(required_call):
            made = job_uses(jobs[name])
            findings.append(
                finding(
                    relative,
                    f"job {name!r} does not call {str(required_call)!r}; "
                    + (f"it calls {made!r}" if made else "it runs steps of its own"),
                )
            )
        reached = transitive_predecessors(graph, name)
        findings.extend(
            finding(
                relative,
                f"job {name!r} does not reach required predecessor {parent!r} through 'needs'",
            )
            for parent in as_list(as_mapping(job_declaration).get("needs"))
            if parent not in reached
        )
    return findings


# --------------------------------------------------------------------------
# Caller obligations
# --------------------------------------------------------------------------
#
# A reusable workflow is called by file name from the same tree
# (`uses: ./.github/workflows/<file>`), which runs it on the caller's own
# commit. Everything below is stated once, by the called workflow, and binds
# every caller of it.
LOCAL_CALL_PREFIX = f"./{WORKFLOW_RELDIR.as_posix()}/"


def check_caller_obligation(workflow: Workflow, declaration, documents: dict) -> list[Finding]:
    """Check what `workflow` promises every caller of it.

    `callers.require_needs` names the jobs a caller inherits by calling this
    workflow. The promise holds only while this workflow defines each of them
    unconditionally — a skipped job leaves the call green, so a condition turns
    an inherited result into an inherited nothing — and only while every caller
    routes the work it runs itself through the call. A caller job that calls
    another workflow of this repository is exempt: it is a verified boundary of
    the same kind, and waiting on a sibling call would only serialize two.
    """
    required = as_list(as_mapping(as_mapping(declaration).get("callers")).get("require_needs"))
    if not required:
        return []

    findings = []
    if "workflow_call" not in normalize_triggers(workflow.doc):
        findings.append(
            finding(
                workflow.relative,
                "contract declares a caller obligation, but the workflow declares "
                "no 'workflow_call' trigger",
            )
        )

    jobs = workflow_jobs(workflow.doc)
    for name in required:
        if name not in jobs:
            findings.append(
                finding(
                    workflow.relative,
                    f"contract requires callers to inherit job {name!r}, "
                    "which the workflow does not define",
                )
            )
        elif jobs[name].get("if") is not None:
            findings.append(
                finding(
                    workflow.relative,
                    f"job {name!r} is promised to every caller but carries an 'if:' "
                    "condition, so a caller can inherit a skip rather than a result",
                )
            )

    reference = f"{LOCAL_CALL_PREFIX}{workflow.name}"
    for caller in documents.values():
        if caller.name == workflow.name:
            continue
        caller_jobs = workflow_jobs(caller.doc)
        calls = {name for name, job in caller_jobs.items() if job_uses(job) == reference}
        if not calls:
            continue
        graph = needs_graph(caller.doc)
        findings.extend(
            finding(
                caller.relative,
                f"job {name!r} does not reach the {reference!r} call through 'needs', "
                "so it starts before every job that call promises has succeeded",
            )
            for name, job in sorted(caller_jobs.items())
            if name not in calls
            and not job_uses(job).startswith(LOCAL_CALL_PREFIX)
            and not transitive_predecessors(graph, name) & calls
        )
    return findings


# --------------------------------------------------------------------------
# Driver
# --------------------------------------------------------------------------
def verify(root: Path) -> tuple[list[Finding], str | None]:
    """Verify one repository root. Returns (findings, environment-error)."""
    contract_path = root / CONTRACT_RELPATH
    workflow_dir = root / WORKFLOW_RELDIR

    if not contract_path.is_file():
        return [], f"no contract file at {CONTRACT_RELPATH}"
    if not workflow_dir.is_dir():
        return [], f"no workflow directory at {WORKFLOW_RELDIR}"

    contract, error = load_yaml(contract_path)
    if error:
        return [], f"{CONTRACT_RELPATH} {error}"
    contract = as_mapping(contract)
    version = contract.get("version")
    if version != SUPPORTED_CONTRACT_VERSION:
        return [], (
            f"unsupported contract version {version!r} in {CONTRACT_RELPATH} "
            f"(this verifier implements version {SUPPORTED_CONTRACT_VERSION})"
        )

    rules = as_mapping(contract.get("global"))
    require_declaration = is_truthy(rules.get("require_declaration", True))
    require_workflow_permissions = is_truthy(rules.get("require_workflow_permissions", True))
    forbid_failure_escapes = is_truthy(rules.get("forbid_failure_escapes", True))
    require_sha_pinned_uses = is_truthy(rules.get("require_sha_pinned_uses", True))
    publication = as_mapping(rules.get("publication"))
    publisher = str(publication.get("github_release") or "")
    forbid_registry = is_truthy(publication.get("forbid_registry", True))

    declarations = as_mapping(contract.get("workflows"))
    workflows = sorted(
        path for path in workflow_dir.iterdir()
        if path.is_file() and path.suffix in WORKFLOW_SUFFIXES
    )
    committed = {path.name for path in workflows}
    # Seeded with every workflow so recursion through a local
    # `./.github/workflows/…` call never re-checks a file the main loop covers,
    # and a composite reached from two workflows is reported once.
    visited = {path.resolve() for path in workflows}

    findings = [
        finding(
            CONTRACT_RELPATH,
            f"contract declares {name!r}, which is not a committed workflow file",
        )
        for name in sorted(declarations)
        if name not in committed
    ]

    # A publication declaration naming nothing in the tree would measure every
    # workflow against a name no file holds, so the declaration itself fails.
    if publication and publisher not in committed:
        findings.append(
            finding(
                CONTRACT_RELPATH,
                f"publication workflow {publisher!r} is not a committed workflow file",
            )
        )

    # Parsed once, up front: a caller obligation is stated by the called
    # workflow and checked against every other workflow in the same tree.
    documents = {}
    for path in workflows:
        relative = path.relative_to(root)
        if require_declaration and path.name not in declarations:
            findings.append(finding(relative, "workflow file is not declared in the contract"))
        doc, error = load_yaml(path)
        if error:
            findings.append(finding(relative, error))
            continue
        documents[path.name] = Workflow(path.name, path, relative, doc)

    for workflow in documents.values():
        relative, doc = workflow.relative, workflow.doc
        declaration = as_mapping(declarations.get(workflow.name))

        findings.extend(check_job_graph(relative, doc))
        if require_sha_pinned_uses:
            findings.extend(check_uses(root, workflow.path, False, doc, visited))
        if forbid_failure_escapes:
            findings.extend(check_failure_escapes(relative, doc))
        if publication:
            findings.extend(check_publication(root, workflow, publisher, forbid_registry))
        findings.extend(check_triggers(relative, doc, declaration.get("triggers")))
        findings.extend(check_workflow_call(relative, doc, declaration.get("workflow_call")))
        findings.extend(
            check_permissions(relative, doc, declaration, require_workflow_permissions)
        )
        findings.extend(check_declared_jobs(relative, doc, declaration))
        findings.extend(check_caller_obligation(workflow, declaration, documents))

    return findings, None


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Verify the committed GitHub workflows against the repository workflow contract."
    )
    parser.add_argument(
        "--root",
        default=".",
        help="repository root holding .github/workflow-contract.yml and .github/workflows/",
    )
    arguments = parser.parse_args(argv)
    root = Path(arguments.root).resolve()

    findings, environment_error = verify(root)
    if environment_error:
        sys.stderr.write(f"workflow-contract: {environment_error}\n")
        return EXIT_ENV

    for item in sorted(findings, key=lambda f: (str(f.path), f.line or 0, f.message)):
        location = f"{item.path}:{item.line}" if item.line else str(item.path)
        print(f"{location}: {item.message}")

    if findings:
        print(f"workflow-contract: {len(findings)} finding(s)")
        return EXIT_FINDINGS
    print("workflow-contract: all declared assertions hold")
    return EXIT_OK


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
