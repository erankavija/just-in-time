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
    _atomic_write_text,
    check_expect_assertions,
    child_env,
    main,
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


if __name__ == "__main__":
    unittest.main()
