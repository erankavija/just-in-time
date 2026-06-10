#!/usr/bin/env python3
"""LLM-agent steering eval (JIT issue 6a6af4e3, design CC-9).

Measures whether JIT's validation steering helps a *real* agent converge on
valid plans. It reuses the deterministic steering fixtures at
``crates/jit/tests/fixtures/steering/<name>/scenario.toml`` so this eval and the
deterministic harness (``crates/jit/tests/steering_scenarios.rs``) stay in
lockstep on the scenario schema and ruleset.

For each *steering* scenario (one whose target step expects a nonzero exit), the
eval runs an agent loop in a fully isolated temp repo:

  1. Build repo state by running the scenario's setup steps.
  2. Run the failing step; show the agent ONLY the command, its exit code, and
     its error text.
  3. The agent proposes a correction (a replacement body, or a side-effect
     command). Apply it, re-run the failing command, repeat until green (exit 0)
     or a cap is reached.
  4. Record iterations-to-green and rule-compliance.

The agent is pluggable behind ``--agent-cmd``. See the module ``AGENT
INTERFACE`` section and ``README.md``.

stdlib only (python3.11+ for ``tomllib``); no pip dependencies; no network calls
are made by this script itself (a real ``--agent-cmd`` may call out — that is the
user's choice and responsibility).
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional

# ---------------------------------------------------------------------------
# Paths and constants
# ---------------------------------------------------------------------------

# This file lives at <repo>/dev/eval/steering/run_eval.py.
THIS_FILE = Path(__file__).resolve()
REPO_ROOT = THIS_FILE.parents[3]
FIXTURES_DIR = REPO_ROOT / "crates" / "jit" / "tests" / "fixtures" / "steering"
EXAMPLES_DIR = REPO_ROOT / "docs" / "examples"
JIT_BIN = REPO_ROOT / "target" / "debug" / "jit"

# Pinned harness identity, mirroring jit_cmd() in steering_scenarios.rs.
JIT_AGENT_ID = "agent:steering-eval"

# JIT_* variables that can redirect or alter storage. These are stripped from
# every child environment so a stray JIT_DATA_DIR can never point a temp repo at
# production .jit state (see project memory on eval repo isolation).
JIT_ENV_STRIP = (
    "JIT_DATA_DIR",
    "JIT_ALLOW_DELETION",
    "JIT_WORKTREE_MODE",
    "JIT_ENFORCE_LEASES",
    "JIT_LOCK_TIMEOUT",
)

UUID_RE = re.compile(
    r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"
)

DEFAULT_CAP = 5
DEFAULT_RUNS = 3
DEFAULT_AGENT_TIMEOUT = 120  # seconds
JIT_TIMEOUT = 600  # seconds; generous to cover slow CI but not an infinite hang

# Known top-level, step-level, and expect-level keys in scenario.toml.
# Fail fast if the schema drifts — see NOTE in Scenario.load().
_KNOWN_TOP_KEYS = {"ruleset", "steps"}
_KNOWN_STEP_KEYS = {"argv", "capture", "id_slot", "expect"}
_KNOWN_EXPECT_KEYS = {"exit", "contains", "not_contains", "enforcement_point"}


# ---------------------------------------------------------------------------
# Scenario schema (subset of fixtures/steering/README.md, the parts we drive)
#
# NOTE: This parser is a deliberate duplicate subset of the schema parsed by
# steering_scenarios.rs. Any schema evolution in the TOML fixtures MUST be
# mirrored here. To surface schema drift early, the parser fails fast on
# unknown top-level, step, and expect keys (see _KNOWN_* sets above).
# ---------------------------------------------------------------------------


@dataclass
class Expect:
    exit: Optional[int]
    contains: list[str]
    not_contains: list[str]

    @staticmethod
    def from_toml(raw: Optional[dict[str, Any]]) -> Optional["Expect"]:
        if raw is None:
            return None
        unknown = set(raw) - _KNOWN_EXPECT_KEYS
        if unknown:
            raise ValueError(f"unknown expect key(s) in scenario.toml: {sorted(unknown)}")
        return Expect(
            exit=raw.get("exit"),
            contains=list(raw.get("contains", [])),
            not_contains=list(raw.get("not_contains", [])),
        )


@dataclass
class Step:
    argv: list[str]
    capture: str  # "id" | "none"
    id_slot: Optional[str]
    expect: Optional[Expect]

    @staticmethod
    def from_toml(raw: dict[str, Any]) -> "Step":
        unknown = set(raw) - _KNOWN_STEP_KEYS
        if unknown:
            raise ValueError(f"unknown step key(s) in scenario.toml: {sorted(unknown)}")
        return Step(
            argv=list(raw["argv"]),
            capture=raw.get("capture", "id"),
            id_slot=raw.get("id_slot"),
            expect=Expect.from_toml(raw.get("expect")),
        )


@dataclass
class Scenario:
    name: str
    ruleset: str
    steps: list[Step]

    @staticmethod
    def load(name: str, fixture_dir: Path) -> "Scenario":
        with (fixture_dir / "scenario.toml").open("rb") as fh:
            raw = tomllib.load(fh)
        unknown = set(raw) - _KNOWN_TOP_KEYS
        if unknown:
            raise ValueError(
                f"unknown top-level key(s) in {fixture_dir / 'scenario.toml'}: "
                f"{sorted(unknown)}"
            )
        return Scenario(
            name=name,
            ruleset=raw["ruleset"],
            steps=[Step.from_toml(s) for s in raw["steps"]],
        )

    def target_index(self) -> Optional[int]:
        """Index of the failing step: the last step whose expect.exit is nonzero.

        A steering scenario is one with such a step. Scenarios whose steps all
        succeed (happy-path-done, pending-req-quiet) have no target and are
        skipped by the eval.
        """
        target = None
        for i, step in enumerate(self.steps):
            if step.expect is not None and step.expect.exit not in (None, 0):
                target = i
        return target

    def is_failing_input_scenario(self) -> bool:
        """True for the *failing-input* steering scenarios this eval drives.

        Per the design (CC-9 / issue 6a6af4e3), the LLM eval drives the
        failing-input scenarios — those whose fix is a content or label
        correction surfaced at a write or validate enforcement point
        (sloppy-spec-body, typo-heading, stray-req). Transition-target
        scenarios (e.g. premature-done) require building out the dependency
        graph rather than correcting an input and are out of scope here; the
        deterministic harness covers them.
        """
        idx = self.target_index()
        if idx is None:
            return False
        target_argv = self.steps[idx].argv
        is_transition = (
            target_argv[:2] == ["issue", "update"]
            and any(a in ("--state", "-s") for a in target_argv)
        )
        return not is_transition


# ---------------------------------------------------------------------------
# jit invocation (isolated, env-sanitized)
# ---------------------------------------------------------------------------


def child_env() -> dict[str, str]:
    """Build a sanitized environment for `jit` child processes.

    Strips every storage-redirecting JIT_* variable and pins JIT_AGENT_ID,
    mirroring jit_cmd() in steering_scenarios.rs.
    """
    env = dict(os.environ)
    for key in JIT_ENV_STRIP:
        env.pop(key, None)
    env["JIT_AGENT_ID"] = JIT_AGENT_ID
    return env


@dataclass
class CmdResult:
    argv: list[str]
    exit: int
    stdout: str
    stderr: str

    @property
    def combined(self) -> str:
        return self.stdout + self.stderr


def run_jit(argv: list[str], cwd: Path) -> CmdResult:
    """Run `jit <argv>` in an isolated repo and capture its output.

    A generous fixed timeout (JIT_TIMEOUT seconds) prevents a hung jit process
    from stalling the eval indefinitely. TimeoutExpired propagates to the caller
    which counts it as a failed attempt.
    """
    proc = subprocess.run(  # noqa: S603 - argv is constructed, not shell.
        [str(JIT_BIN), *argv],
        cwd=str(cwd),
        env=child_env(),
        capture_output=True,
        text=True,
        timeout=JIT_TIMEOUT,
    )
    return CmdResult(argv, proc.returncode, proc.stdout, proc.stderr)


# ---------------------------------------------------------------------------
# Repo setup (mirrors setup_scenario_repo in steering_scenarios.rs)
# ---------------------------------------------------------------------------


def setup_scenario_repo(ruleset: str) -> Path:
    """Create a fresh isolated temp repo and install the named ruleset.

    Returns the temp repo path. The caller is responsible for removing it (see
    ``run_scenario`` which always cleans up via try/finally).
    """
    repo = Path(tempfile.mkdtemp(prefix="jit-steering-eval-"))

    init = run_jit(["init"], repo)
    if init.exit != 0:
        shutil.rmtree(repo, ignore_errors=True)
        raise RuntimeError(f"jit init failed in {repo}: {init.stderr}")

    example_dir = EXAMPLES_DIR / ruleset
    if not example_dir.is_dir():
        shutil.rmtree(repo, ignore_errors=True)
        raise RuntimeError(f"ruleset directory missing: {example_dir}")

    shutil.copy(example_dir / "rules.toml", repo / ".jit" / "rules.toml")

    schemas_src = example_dir / "schemas"
    if schemas_src.is_dir():
        schemas_dst = repo / ".jit" / "schemas"
        if schemas_dst.exists():
            shutil.rmtree(schemas_dst)
        shutil.copytree(schemas_src, schemas_dst)

    return repo


# ---------------------------------------------------------------------------
# argv helpers
# ---------------------------------------------------------------------------


def substitute_argv(argv: list[str], ids: dict[str, str]) -> list[str]:
    """Substitute `$<slot>` and `$<slot>_short` with captured ids.

    Longest slot name first so `$epic2` is not partially consumed by `epic`.
    Mirrors substitute_argv in steering_scenarios.rs.
    """
    slots = sorted(ids.items(), key=lambda kv: len(kv[0]), reverse=True)
    out = []
    for arg in argv:
        for slot, uuid in slots:
            arg = arg.replace(f"${slot}_short", uuid[:8])
            arg = arg.replace(f"${slot}", uuid)
        out.append(arg)
    return out


def first_uuid(text: str) -> Optional[str]:
    m = UUID_RE.search(text)
    return m.group(0) if m else None


def description_index(argv: list[str]) -> Optional[int]:
    """Index of the description *value* in argv, or None if absent.

    Handles `-d`/`--description` followed by the value.
    """
    for i, arg in enumerate(argv):
        if arg in ("-d", "--description") and i + 1 < len(argv):
            return i + 1
    return None


# ---------------------------------------------------------------------------
# AGENT INTERFACE
# ---------------------------------------------------------------------------
#
# An agent is a command that reads a JSON task on stdin and writes a JSON action
# on stdout.
#
# Task (stdin):
#   {
#     "scenario":     "<name>",
#     "ruleset":      "<ruleset>",
#     "failing_argv": ["issue", "create", ...],   # the command that failed
#     "failing_exit": 4,
#     "failing_stderr": "<error text the agent may act on>",
#     "history": [ {"argv": [...], "exit": 4, "stderr": "..."}, ... ]
#   }
#
# Action (stdout, a single JSON object):
#   {"body": "<replacement --description body>"}    # re-runs failing cmd w/ new body
#   {"argv": ["issue", "update", "<id>", "--remove-label", "req:REQ-77"]}
#                                                    # side-effect, then re-run failing cmd
#
# The loop applies the action, then re-runs the *failing* command and checks for
# exit 0. A `body` action rebuilds the failing command's --description; an `argv`
# action is run as a side-effect command first (its own exit is recorded but
# greenness is judged by re-running the failing command).
#
# Built-in agents (no network, deterministic) are selected with
# `--agent-cmd builtin:oracle`. Any other value is treated as an external
# command run via the shell-free argv split below; see README for hooking a real
# model (e.g. `claude -p`).


@dataclass
class AgentAction:
    body: Optional[str] = None
    argv: Optional[list[str]] = None

    @staticmethod
    def from_json(text: str) -> "AgentAction":
        data = json.loads(text)
        return AgentAction(body=data.get("body"), argv=data.get("argv"))


def build_task(
    scenario: Scenario,
    failing: CmdResult,
    history: list[CmdResult],
) -> dict[str, Any]:
    return {
        "scenario": scenario.name,
        "ruleset": scenario.ruleset,
        "failing_argv": failing.argv,
        "failing_exit": failing.exit,
        "failing_stderr": failing.combined,
        "history": [
            {"argv": h.argv, "exit": h.exit, "stderr": h.combined} for h in history
        ],
    }


# --- built-in oracle ------------------------------------------------------

# The canonical valid Success-Criteria body the oracle uses for body fixes.
# It mirrors the corrected shape the deterministic fixtures imply: real Markdown
# bullets and the correctly spelled heading.
ORACLE_VALID_BODY = """## Requirements

- REQ-01: the system validates specs

## Scenarios

- Given a spec When parsed Then valid

## Success Criteria

- [hard] REQ-01: the system validates specs"""


def oracle_action(task: dict[str, Any], ids: dict[str, str]) -> AgentAction:
    """Deterministic known-correct fix per scenario; expected to green in 1 step.

    Used to smoke-test the loop. The oracle keys off the failing command shape,
    not scenario name strings, so it stays generic:
      - a failing write (`issue create/update` with a --description) -> replace
        the body with a well-formed one.
      - a failing `validate` caused by a stray req label -> remove that label
        from the offending issue, then re-run validate.
    """
    failing_argv = task["failing_argv"]
    stderr = task["failing_stderr"]

    # Stray-req style: validate fails naming a stray "req:<id>" label.
    if failing_argv[:1] == ["validate"]:
        m = re.search(r"label '(req:[A-Za-z0-9-]+)'", stderr)
        m_issue = re.search(r"on issue ([0-9a-fA-F]{8})", stderr)
        if m and m_issue:
            label = m.group(1)
            short = m_issue.group(1)
            issue_id = next(
                (full for full in ids.values() if full.startswith(short)), short
            )
            return AgentAction(
                argv=["issue", "update", issue_id, "--remove-label", label]
            )

    # Write-path body failures: hand back a well-formed body.
    if description_index(failing_argv) is not None:
        return AgentAction(body=ORACLE_VALID_BODY)

    raise RuntimeError(
        f"oracle has no fix for failing command {failing_argv!r}; "
        f"stderr was:\n{stderr}"
    )


def run_external_agent(
    agent_cmd: list[str], task: dict[str, Any], agent_timeout: int
) -> AgentAction:
    """Run an external agent command, piping the JSON task to stdin.

    ``agent_timeout`` caps how long the agent process may run. On
    TimeoutExpired the exception propagates to the caller which counts the
    iteration as a failed attempt.
    """
    proc = subprocess.run(  # noqa: S603
        agent_cmd,
        input=json.dumps(task),
        capture_output=True,
        text=True,
        env=dict(os.environ),  # agent env is the operator's concern, not jit's.
        timeout=agent_timeout,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"agent command exited {proc.returncode}\n"
            f"stdout: {proc.stdout}\nstderr: {proc.stderr}"
        )
    # The agent may print logs; take the last non-empty line as the JSON action.
    lines = [ln for ln in proc.stdout.splitlines() if ln.strip()]
    if not lines:
        raise RuntimeError("agent produced no output (expected a JSON action)")
    return AgentAction.from_json(lines[-1])


# ---------------------------------------------------------------------------
# Single-run agent loop
# ---------------------------------------------------------------------------


@dataclass
class RunResult:
    """Outcome of one scenario run."""

    green: bool
    iterations: int  # iterations-to-green (1 = fixed on first attempt)
    failure_reason: Optional[str] = None  # set when green=False due to an error
    history: list[dict[str, Any]] = field(default_factory=list)


def apply_action(
    failing_step_argv: list[str],
    action: AgentAction,
    repo: Path,
    ids: dict[str, str],
) -> CmdResult:
    """Apply the agent action and return the result of the *failing* command.

    - body action: rebuild the failing command with the new --description and run it.
    - argv action: run the side-effect command, then re-run the failing command.
    """
    if action.body is not None:
        idx = description_index(failing_step_argv)
        if idx is None:
            raise RuntimeError(
                "agent returned a body but the failing command has no "
                f"--description: {failing_step_argv!r}"
            )
        patched = list(failing_step_argv)
        patched[idx] = action.body
        return run_jit(substitute_argv(patched, ids), repo)

    if action.argv is not None:
        side = run_jit(substitute_argv(action.argv, ids), repo)
        # Re-run the original failing command to test for green regardless of the
        # side-effect command's own exit code.
        retry = run_jit(substitute_argv(failing_step_argv, ids), repo)
        # Record the side-effect in the retry's combined output for transparency.
        retry.stdout = f"[side-effect {side.argv} exit={side.exit}]\n" + retry.stdout
        return retry

    raise RuntimeError("agent action has neither 'body' nor 'argv'")


def run_one(
    scenario: Scenario,
    target_idx: int,
    agent_cmd: str,
    cap: int,
    agent_timeout: int,
) -> RunResult:
    """Drive a single scenario run through the agent loop in an isolated repo."""
    repo = setup_scenario_repo(scenario.ruleset)
    ids: dict[str, str] = {}
    try:
        # 1. Run setup steps (everything before the target). They must succeed
        #    (or match their own non-failing expectation). Capture ids.
        for step in scenario.steps[:target_idx]:
            res = run_jit(substitute_argv(step.argv, ids), repo)
            expected_exit = step.expect.exit if step.expect else 0
            if res.exit != expected_exit:
                raise RuntimeError(
                    f"setup step {step.argv} exited {res.exit} "
                    f"(expected {expected_exit}); stderr: {res.stderr[-500:]}"
                )
            uuid = first_uuid(res.combined)
            if uuid is not None:
                if step.id_slot:
                    ids[step.id_slot] = uuid
                ids.setdefault("_last", uuid)

        # 2. Run the failing step.
        failing_step = scenario.steps[target_idx]
        # Also capture any id produced by setup-side of the failing step's repo
        # state: the failing create itself may have created the issue under test.
        failing = run_jit(substitute_argv(failing_step.argv, ids), repo)
        first = first_uuid(failing.combined)
        if first is not None:
            ids.setdefault("_last", first)

        history: list[CmdResult] = [failing]
        run_history: list[dict[str, Any]] = [
            {"argv": failing.argv, "exit": failing.exit}
        ]

        if failing.exit == 0:
            # The scenario's "failing" step already passed (shouldn't happen for
            # a steering scenario, but treat as green in 0 corrective iterations).
            return RunResult(green=True, iterations=0, history=run_history)

        # 3. Agent loop.
        for attempt in range(1, cap + 1):
            try:
                task = build_task(scenario, failing, history)
                if agent_cmd == "builtin:oracle":
                    action = oracle_action(task, ids)
                else:
                    action = run_external_agent(
                        _split_agent_cmd(agent_cmd), task, agent_timeout
                    )

                result = apply_action(failing_step.argv, action, repo, ids)
            except subprocess.TimeoutExpired as exc:
                reason = f"timeout on attempt {attempt}: {exc}"
                run_history.append({"attempt": attempt, "error": reason})
                return RunResult(
                    green=False,
                    iterations=cap,
                    failure_reason=reason,
                    history=run_history,
                )
            except (json.JSONDecodeError, RuntimeError, ValueError) as exc:
                reason = f"error on attempt {attempt}: {exc}"
                run_history.append({"attempt": attempt, "error": reason})
                return RunResult(
                    green=False,
                    iterations=cap,
                    failure_reason=reason,
                    history=run_history,
                )

            history.append(result)
            run_history.append(
                {
                    "argv": result.argv,
                    "exit": result.exit,
                    "action": "body" if action.body is not None else "argv",
                }
            )
            failing = result

            if result.exit == 0:
                return RunResult(green=True, iterations=attempt, history=run_history)

        return RunResult(green=False, iterations=cap, history=run_history)
    finally:
        shutil.rmtree(repo, ignore_errors=True)


def _split_agent_cmd(agent_cmd: str) -> list[str]:
    import shlex

    return shlex.split(agent_cmd)


# ---------------------------------------------------------------------------
# Aggregation across runs
# ---------------------------------------------------------------------------


@dataclass
class ScenarioStats:
    name: str
    runs: int
    greens: int
    cap: int
    iterations: list[int]  # iterations-to-green for green runs only
    failure_reasons: list[str] = field(default_factory=list)

    @property
    def stability(self) -> str:
        if self.greens == self.runs:
            return "always"
        if self.greens == 0:
            return "never"
        return "sometimes"

    @property
    def mean_iterations(self) -> Optional[float]:
        """Mean iterations over green runs only.

        This metric is optimistic: non-green runs are excluded. See
        ``penalized_mean`` for the bias-corrected metric.
        """
        return statistics.fmean(self.iterations) if self.iterations else None

    @property
    def penalized_mean(self) -> float:
        """Mean iterations with non-green runs charged as ``cap``.

        This is the bias-corrected companion to ``mean_iterations``. Non-green
        runs (whether stopped by the cap, a timeout, or a malformed agent
        response) are charged the full cap value, so a model that never
        converges cannot appear to have a low mean through survivorship bias.
        """
        all_iters = list(self.iterations) + [self.cap] * (self.runs - self.greens)
        return statistics.fmean(all_iters)

    def to_json(self) -> dict[str, Any]:
        return {
            "runs": self.runs,
            "greens": self.greens,
            "stability": self.stability,
            "mean_iterations": self.mean_iterations,
            "penalized_mean": self.penalized_mean,
            "min_iterations": min(self.iterations) if self.iterations else None,
            "max_iterations": max(self.iterations) if self.iterations else None,
            "iterations": self.iterations,
            "failure_reasons": self.failure_reasons,
        }


# ---------------------------------------------------------------------------
# Discovery
# ---------------------------------------------------------------------------


def discover_scenarios() -> tuple[list[Scenario], list[str]]:
    """Load all fixtures; return (failing_input_scenarios, skipped_names).

    ``skipped_names`` are scenario directory names that exist but are not
    failing-input scenarios (e.g. happy-path or transition scenarios).
    """
    driven: list[Scenario] = []
    skipped: list[str] = []
    for entry in sorted(FIXTURES_DIR.iterdir()):
        if not entry.is_dir():
            continue
        if not (entry / "scenario.toml").exists():
            continue
        sc = Scenario.load(entry.name, entry)
        if sc.is_failing_input_scenario():
            driven.append(sc)
        else:
            skipped.append(entry.name)
    return driven, skipped


# ---------------------------------------------------------------------------
# jit commit + binary
# ---------------------------------------------------------------------------


def ensure_jit_binary() -> None:
    """Build the jit binary if it is missing."""
    if JIT_BIN.exists():
        return
    print(f"building jit binary (cargo build -p jit) -> {JIT_BIN}", file=sys.stderr)
    proc = subprocess.run(  # noqa: S603
        ["cargo", "build", "-p", "jit"], cwd=str(REPO_ROOT)
    )
    if proc.returncode != 0 or not JIT_BIN.exists():
        raise SystemExit("failed to build jit binary")


def jit_commit() -> str:
    try:
        out = subprocess.run(  # noqa: S603
            ["git", "rev-parse", "HEAD"],
            cwd=str(REPO_ROOT),
            capture_output=True,
            text=True,
        )
        return out.stdout.strip() if out.returncode == 0 else "unknown"
    except OSError:
        return "unknown"


# ---------------------------------------------------------------------------
# Isolation self-check (warns + strips; does not refuse)
# ---------------------------------------------------------------------------


def warn_inherited_jit_env() -> None:
    """Warn if the eval's own environment contains JIT_* vars that will be stripped.

    JIT_DATA_DIR in *our* environment would otherwise be inherited; child_env()
    strips it from every jit child process, but we surface a clear warning so
    the operator knows it was set.
    """
    leaked = [k for k in JIT_ENV_STRIP if k in os.environ]
    if leaked:
        print(
            f"note: stripping inherited JIT_* var(s) from jit children: {leaked}",
            file=sys.stderr,
        )


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def print_table(stats: list[ScenarioStats]) -> None:
    header = (
        f"{'scenario':<22} {'stability':<10} {'mean':>6} {'penalized':>10} "
        f"{'min':>4} {'max':>4} {'green/runs':>11}"
    )
    print(header)
    print("-" * len(header))
    for s in stats:
        mean = f"{s.mean_iterations:.2f}" if s.mean_iterations is not None else "-"
        pen = f"{s.penalized_mean:.2f}"
        mn = str(min(s.iterations)) if s.iterations else "-"
        mx = str(max(s.iterations)) if s.iterations else "-"
        print(
            f"{s.name:<22} {s.stability:<10} {mean:>6} {pen:>10} {mn:>4} {mx:>4} "
            f"{f'{s.greens}/{s.runs}':>11}"
        )


def write_results_json(
    stats: list[ScenarioStats],
    runs: int,
    cap: int,
    agent_timeout: int,
    model_id: str,
    out_dir: Path,
    skipped_scenarios: list[str],
    timestamp: Optional[str] = None,
) -> Path:
    ts = timestamp or _dt.datetime.now(_dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    payload = {
        "jit_commit": jit_commit(),
        "model_id": model_id,
        "runs": runs,
        "cap": cap,
        "agent_timeout": agent_timeout,
        "timestamp": ts,
        "scenarios": [s.name for s in stats],
        "skipped_scenarios": skipped_scenarios,
        "per_scenario": {s.name: s.to_json() for s in stats},
    }
    out_path = out_dir / f"results-{ts}.json"
    _atomic_write_text(out_path, json.dumps(payload, indent=2) + "\n")
    return out_path


# ---------------------------------------------------------------------------
# Main eval driver
# ---------------------------------------------------------------------------


def run_eval(
    agent_cmd: str,
    runs: int,
    cap: int,
    agent_timeout: int,
    model_id: str,
    out_dir: Optional[Path],
    skipped_scenarios: list[str],
) -> list[ScenarioStats]:
    scenarios, _ = discover_scenarios()
    if not scenarios:
        raise SystemExit(f"no steering scenarios found under {FIXTURES_DIR}")

    all_stats: list[ScenarioStats] = []
    for sc in scenarios:
        target = sc.target_index()
        assert target is not None  # discover_scenarios guarantees this.
        iterations: list[int] = []
        greens = 0
        failure_reasons: list[str] = []
        for run_idx in range(runs):
            try:
                result = run_one(sc, target, agent_cmd, cap, agent_timeout)
            except Exception as exc:  # noqa: BLE001
                reason = f"run {run_idx + 1} setup error: {exc}"
                print(f"  [{sc.name}] {reason}", file=sys.stderr)
                failure_reasons.append(reason)
                # Count as a non-green attempt (charged cap in penalized_mean).
                continue
            if result.green:
                greens += 1
                iterations.append(result.iterations)
            elif result.failure_reason:
                failure_reasons.append(result.failure_reason)
                print(
                    f"  [{sc.name}] attempt failed: {result.failure_reason}",
                    file=sys.stderr,
                )

        scenario_stats = ScenarioStats(
            name=sc.name,
            runs=runs,
            greens=greens,
            cap=cap,
            iterations=iterations,
            failure_reasons=failure_reasons,
        )
        all_stats.append(scenario_stats)

        # Write results incrementally after each scenario completes so a late
        # crash preserves earlier scenarios' data.
        if out_dir is not None:
            out_dir.mkdir(parents=True, exist_ok=True)
            _write_partial_results(
                all_stats, runs, cap, agent_timeout, model_id, out_dir,
                skipped_scenarios,
            )

    return all_stats



def _atomic_write_text(path: Path, text: str) -> None:
    """Write via temp file + rename so an interrupted run cannot leave a
    truncated results artifact (project atomic-write convention)."""
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(text)
    tmp.replace(path)

def _write_partial_results(
    stats: list[ScenarioStats],
    runs: int,
    cap: int,
    agent_timeout: int,
    model_id: str,
    out_dir: Path,
    skipped_scenarios: list[str],
) -> None:
    """Overwrite a fixed-name partial results file after each scenario completes."""
    payload = {
        "jit_commit": jit_commit(),
        "model_id": model_id,
        "runs": runs,
        "cap": cap,
        "agent_timeout": agent_timeout,
        "timestamp": _dt.datetime.now(_dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ"),
        "partial": True,
        "scenarios": [s.name for s in stats],
        "skipped_scenarios": skipped_scenarios,
        "per_scenario": {s.name: s.to_json() for s in stats},
    }
    partial_path = out_dir / "results-partial.json"
    _atomic_write_text(partial_path, json.dumps(payload, indent=2) + "\n")


def run_smoke(agent_timeout: int) -> int:
    """Oracle agent, 1 run; assert every steering scenario greens in exactly 1
    corrective iteration. CI-runnable self-test of the eval loop (not wired into
    cargo test; run manually per README).
    """
    scenarios, skipped = discover_scenarios()
    if not scenarios:
        print(f"SMOKE FAIL: no steering scenarios under {FIXTURES_DIR}")
        return 1

    if skipped:
        print(f"skipped (not failing-input scenarios): {', '.join(skipped)}", file=sys.stderr)

    failures: list[str] = []
    for sc in scenarios:
        target = sc.target_index()
        assert target is not None
        result = run_one(sc, target, "builtin:oracle", cap=DEFAULT_CAP,
                         agent_timeout=agent_timeout)
        status = "green" if result.green else "STUCK"
        print(
            f"smoke: {sc.name:<22} {status:<6} iterations={result.iterations}"
        )
        if not result.green:
            reason = result.failure_reason or "oracle did not reach green"
            failures.append(f"{sc.name}: {reason}")
        elif result.iterations != 1:
            failures.append(
                f"{sc.name}: oracle expected 1 iteration, got {result.iterations}"
            )

    if failures:
        print("\nSMOKE FAIL:")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("\nSMOKE PASS: all steering scenarios green in 1 oracle iteration")
    return 0


def parse_args(argv: Optional[list[str]] = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        description="LLM-agent steering eval over deterministic JIT fixtures.",
    )
    p.add_argument(
        "--agent-cmd",
        default="builtin:oracle",
        help="agent command. 'builtin:oracle' applies known-correct fixes; "
        "any other value is an external command receiving a JSON task on "
        "stdin and printing a JSON action on stdout.",
    )
    p.add_argument(
        "--runs",
        type=int,
        default=DEFAULT_RUNS,
        help=f"runs per scenario (default {DEFAULT_RUNS}; minimum 3 for "
        "reportable results — means over fewer runs are not comparable).",
    )
    p.add_argument(
        "--allow-few-runs",
        action="store_true",
        help="debugging escape hatch: permit --runs below 3. Results produced "
        "this way are NOT reportable (the methodology requires n >= 3).",
    )
    p.add_argument(
        "--cap",
        type=int,
        default=DEFAULT_CAP,
        help=f"max corrective iterations before giving up (default {DEFAULT_CAP}).",
    )
    p.add_argument(
        "--agent-timeout",
        type=int,
        default=DEFAULT_AGENT_TIMEOUT,
        help=f"seconds before the agent process is killed (default {DEFAULT_AGENT_TIMEOUT}). "
        "Applies to external agents only; builtin:oracle is not time-limited.",
    )
    p.add_argument(
        "--model-id",
        default=None,
        help="model identifier recorded in the results JSON. REQUIRED for "
        "non-builtin agents so re-runs are comparable.",
    )
    p.add_argument(
        "--out-dir",
        default=str(THIS_FILE.parent / "results"),
        help="directory for results-<timestamp>.json (default ./results).",
    )
    p.add_argument(
        "--smoke",
        action="store_true",
        help="oracle agent, 1 run; assert every scenario greens in exactly 1 "
        "iteration. Self-test of the eval loop.",
    )
    return p.parse_args(argv)


def main(argv: Optional[list[str]] = None) -> int:
    args = parse_args(argv)
    if args.runs < 1:
        raise SystemExit("--runs must be at least 1")
    if not args.smoke and args.runs < 3 and not args.allow_few_runs:
        raise SystemExit(
            "--runs must be at least 3 for reportable results "
            "(use --allow-few-runs for non-reportable debugging runs)"
        )
    warn_inherited_jit_env()
    ensure_jit_binary()

    if args.smoke:
        return run_smoke(agent_timeout=args.agent_timeout)

    is_builtin = args.agent_cmd.startswith("builtin:")
    if not is_builtin and not args.model_id:
        raise SystemExit(
            "--model-id is required for non-builtin agents (so results are "
            "comparable across rule changes). See README.md."
        )
    model_id = args.model_id or args.agent_cmd

    _, skipped = discover_scenarios()
    if skipped:
        print(
            f"skipped (not failing-input scenarios): {', '.join(skipped)}",
            file=sys.stderr,
        )

    out_dir = Path(args.out_dir)

    stats = run_eval(
        agent_cmd=args.agent_cmd,
        runs=args.runs,
        cap=args.cap,
        agent_timeout=args.agent_timeout,
        model_id=model_id,
        out_dir=out_dir,
        skipped_scenarios=skipped,
    )

    print(f"\njit_commit: {jit_commit()}")
    print(f"model_id:   {model_id}")
    print(f"runs:       {args.runs}")
    print(f"cap:        {args.cap}\n")
    print_table(stats)

    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = write_results_json(
        stats, args.runs, args.cap, args.agent_timeout, model_id, out_dir, skipped
    )
    print(f"\nresults written to {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
