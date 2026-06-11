"""Unit tests for the steering eval harness (stdlib unittest, no network,
no jit subprocesses). Run with:

    python3 -m unittest discover -s dev/eval/steering -p 'test_*.py'

These cover the harness's own logic: run-count policy, atomic result writes,
expect-assertion checking, aggregation/stability math, env sanitization, and
schema strictness. The end-to-end loop is covered by `run_eval.py --smoke`.
"""

from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path

import run_eval as re_mod
from run_eval import (
    CmdResult,
    Expect,
    ScenarioStats,
    Step,
    _atomic_write_text,
    check_expect_assertions,
    child_env,
    main,
    oracle_action,
)


def _res(exit_code: int = 0, stdout: str = "", stderr: str = "") -> CmdResult:
    return CmdResult(argv=["jit"], exit=exit_code, stdout=stdout, stderr=stderr)


class TestRunCountPolicy(unittest.TestCase):
    def test_runs_below_three_rejected_without_flag(self) -> None:
        with self.assertRaises(SystemExit):
            main(["--agent-cmd", "builtin:oracle", "--runs", "2"])

    def test_runs_zero_rejected_even_with_flag(self) -> None:
        with self.assertRaises(SystemExit):
            main(
                [
                    "--agent-cmd",
                    "builtin:oracle",
                    "--runs",
                    "0",
                    "--allow-few-runs",
                ]
            )


class TestAtomicWrite(unittest.TestCase):
    def test_write_is_temp_then_rename(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            target = Path(d) / "out.json"
            _atomic_write_text(target, "payload")
            self.assertEqual(target.read_text(), "payload")
            # No leftover temp file.
            self.assertEqual(sorted(p.name for p in Path(d).iterdir()), ["out.json"])


class TestExpectAssertions(unittest.TestCase):
    def test_default_expects_exit_zero(self) -> None:
        self.assertIsNone(check_expect_assertions(_res(0), None))
        self.assertIn("exited 4", check_expect_assertions(_res(4), None) or "")

    def test_contains_and_not_contains(self) -> None:
        exp = Expect(exit=0, contains=["created"], not_contains=["error"])
        self.assertIsNone(check_expect_assertions(_res(0, stdout="created x"), exp))
        self.assertIn(
            "missing expected substring",
            check_expect_assertions(_res(0, stdout="nope"), exp) or "",
        )
        self.assertIn(
            "forbidden substring",
            check_expect_assertions(_res(0, stdout="created error"), exp) or "",
        )


class TestAggregation(unittest.TestCase):
    def _stats(self, greens: list[int | None], cap: int = 5) -> ScenarioStats:
        # greens: per-run iterations-to-green, None = never greened (cap hit).
        green_iters = [g for g in greens if g is not None]
        return ScenarioStats(
            name="s",
            runs=len(greens),
            greens=len(green_iters),
            cap=cap,
            iterations=green_iters,
        )

    def test_stability_classification(self) -> None:
        self.assertEqual(self._stats([1, 1, 2]).stability, "always")
        self.assertEqual(self._stats([1, None, 2]).stability, "sometimes")
        self.assertEqual(self._stats([None, None, None]).stability, "never")

    def test_penalized_mean_charges_cap(self) -> None:
        stats = self._stats([1, None, None], cap=5)
        # Mean over greens is 1; penalized charges the two failures as 5 each.
        self.assertAlmostEqual(stats.penalized_mean, (1 + 5 + 5) / 3)


class TestEnvSanitization(unittest.TestCase):
    def test_child_env_strips_and_pins(self) -> None:
        # child_env reads os.environ; patch a copy in for the call.
        import unittest.mock as mock

        polluted = dict(os.environ)
        polluted["JIT_DATA_DIR"] = "/should/not/survive"
        polluted["JIT_ALLOW_DELETION"] = "1"
        with mock.patch.dict(os.environ, polluted, clear=True):
            env = child_env()
        self.assertNotIn("JIT_DATA_DIR", env)
        self.assertNotIn("JIT_ALLOW_DELETION", env)
        self.assertEqual(env.get("JIT_AGENT_ID"), "agent:steering-eval")


class TestSchemaStrictness(unittest.TestCase):
    def test_unknown_top_level_key_rejected(self) -> None:
        from run_eval import Scenario

        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "scenario.toml").write_text(
                'ruleset = "sdd"\nbogus = 1\nsteps = []\n'
            )
            with self.assertRaises(ValueError):
                Scenario.load("x", Path(d))

    def test_unknown_expect_key_rejected(self) -> None:
        with self.assertRaises(ValueError):
            Expect.from_toml({"exit": 0, "tyop": []})


class TestResultsReportability(unittest.TestCase):
    def test_debug_artifact_marked_and_named(self) -> None:
        stats: list[ScenarioStats] = []
        with tempfile.TemporaryDirectory() as d:
            path = re_mod.write_results_json(
                stats, 1, 5, 120, "m", Path(d), [], reportable=False
            )
            self.assertTrue(path.name.startswith("results-debug-"))
            self.assertFalse(json.loads(path.read_text())["reportable"])

    def test_reportable_artifact_marked(self) -> None:
        stats: list[ScenarioStats] = []
        with tempfile.TemporaryDirectory() as d:
            path = re_mod.write_results_json(
                stats, 3, 5, 120, "m", Path(d), [], reportable=True
            )
            self.assertTrue(path.name.startswith("results-2"))
            self.assertTrue(json.loads(path.read_text())["reportable"])


class TestDrivenDiscovery(unittest.TestCase):
    """target_index / is_driven on transition-style and happy-path scenarios."""

    def _scenario(self, steps: list[Step]) -> re_mod.Scenario:
        return re_mod.Scenario(name="s", ruleset="sdd", steps=steps)

    def _step(self, argv: list[str], exit_code: int | None) -> Step:
        expect = None if exit_code is None else Expect(
            exit=exit_code, contains=[], not_contains=[]
        )
        return Step(argv=argv, capture="none", id_slot=None, expect=expect)

    def test_transition_scenario_is_driven(self) -> None:
        # premature-done shape: a failing `issue update <epic> --state done`.
        sc = self._scenario(
            [
                self._step(["issue", "create", "--title", "E"], None),
                self._step(["issue", "update", "$epic", "--state", "in_progress"], None),
                self._step(["issue", "update", "$epic", "--state", "done"], 4),
            ]
        )
        self.assertEqual(sc.target_index(), 2)
        self.assertTrue(sc.is_driven())

    def test_happy_path_is_skipped(self) -> None:
        # All steps succeed -> no failing target step -> not driven.
        sc = self._scenario(
            [
                self._step(["issue", "create", "--title", "E"], None),
                self._step(["issue", "update", "$epic", "--state", "done"], 0),
            ]
        )
        self.assertIsNone(sc.target_index())
        self.assertFalse(sc.is_driven())


class TestOracleSequencing(unittest.TestCase):
    """The premature-done oracle emits one repair command per iteration, in
    order, sequenced from the history it is given (no subprocesses)."""

    EPIC = "cbb39085-3415-42c2-9d9c-551e9fb4c0be"
    CHILD = "1f67bb58-4164-4845-a579-8740eb7a81d5"
    FAILING = ["issue", "update", EPIC, "--state", "done"]

    def _task(self, history: list[dict]) -> dict:
        return {
            "failing_argv": self.FAILING,
            "failing_stderr": "sdd-hard-criteria-covered ... not satisfied",
            "history": history,
        }

    def _hist(self, argv: list[str], exit_code: int = 0, stdout: str = "") -> dict:
        return {"argv": argv, "exit": exit_code, "stdout": stdout, "output": stdout}

    def test_step1_creates_satisfying_child(self) -> None:
        action = oracle_action(self._task([self._hist(self.FAILING, 4)]), {})
        self.assertIsNone(action.body)
        assert action.argv is not None
        self.assertEqual(action.argv[:2], ["issue", "create"])
        self.assertIn("satisfies:REQ-01", action.argv)

    def _child_create_hist(self) -> dict:
        return self._hist(
            ["issue", "create", "--label", "satisfies:REQ-01", "--description", "x"],
            stdout=f"Created issue: {self.CHILD}\n",
        )

    def test_step2_adds_dependency_to_extracted_child(self) -> None:
        history = [self._hist(self.FAILING, 4), self._child_create_hist()]
        action = oracle_action(self._task(history), {})
        self.assertEqual(action.argv, ["dep", "add", self.EPIC, self.CHILD])

    def test_step3_walks_child_in_progress(self) -> None:
        history = [
            self._hist(self.FAILING, 4),
            self._child_create_hist(),
            self._hist(["dep", "add", self.EPIC, self.CHILD]),
        ]
        action = oracle_action(self._task(history), {})
        self.assertEqual(
            action.argv, ["issue", "update", self.CHILD, "--state", "in_progress"]
        )

    def test_step4_completes_child(self) -> None:
        history = [
            self._hist(self.FAILING, 4),
            self._child_create_hist(),
            self._hist(["dep", "add", self.EPIC, self.CHILD]),
            self._hist(["issue", "update", self.CHILD, "--state", "in_progress"]),
        ]
        action = oracle_action(self._task(history), {})
        self.assertEqual(
            action.argv, ["issue", "update", self.CHILD, "--state", "done"]
        )


if __name__ == "__main__":
    unittest.main()
