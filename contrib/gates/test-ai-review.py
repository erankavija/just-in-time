#!/usr/bin/env python3
"""Exercise review ancestry facts through the dry-run reviewer."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


WRAPPER = Path(__file__).resolve().with_name("ai-review.sh")


class ReviewAncestryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.repo = Path(self.temporary.name)
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.name", "Review test")
        self.git("config", "user.email", "review@example.invalid")
        self.base = self.commit("seed")
        self.git("checkout", "-q", "-b", "feature")
        self.first = self.commit("implement jit:1234abcd")
        self.second = self.commit("extend feature\n\njit:1234abcd")
        self.git("checkout", "-q", "main")
        self.commit("unrelated jit:ffffffff")
        self.git("merge", "-q", "--no-ff", "feature", "-m", "merge feature")
        self.merged = self.git("rev-parse", "HEAD")
        self.git("checkout", "-q", "-b", "unmerged")
        self.commit("unmerged jit:1234abcd")
        self.git("checkout", "-q", "main")
        self.context = {
            "prompt": "Inspect the work.",
            "issue": {"id": "1234abcd-0000-0000-0000-000000000000",
                      "short_id": "1234abcd", "gates": []},
        }

    def git(self, *arguments):
        return subprocess.check_output(
            ["git", *arguments], cwd=self.repo, text=True, stderr=subprocess.PIPE
        ).strip()

    def commit(self, message):
        self.git("commit", "-q", "--allow-empty", "-m", message)
        return self.git("rev-parse", "HEAD")

    def review(self, expected_exit=1, **environment):
        context = self.repo / "context.json"
        context.write_text(json.dumps(self.context))
        result = subprocess.run(
            ["bash", str(WRAPPER)], cwd=self.repo,
            env={**os.environ, "JIT_CONTEXT_FILE": str(context),
                 "REVIEWER_AGENT": "cat", **environment},
            text=True, capture_output=True, check=False,
        )
        # cat echoes the wrapper's final example VERDICT: FAIL line.
        self.assertEqual(result.returncode, expected_exit, result.stderr)
        marker = "## Gate evidence ancestry\n\n```json\n"
        self.assertIn(marker, result.stdout)
        return json.loads(result.stdout.split(marker, 1)[1].split("\n```", 1)[0])

    def gate(self, key, commit):
        return {"key": key, "status": "passed", "exit_code": 0,
                "last_run_at": "2026-01-01T00:00:00+00:00", "commit": commit}

    def test_ancestry_distinguishes_merge_from_predating_run(self):
        self.context["issue"]["gates"] = [
            self.gate("merged", self.merged), self.gate("old", self.base),
            self.gate("partial", self.first),
        ]
        facts = self.review()
        for fact, commit, first, second in zip(
            facts["gates"], (self.merged, self.base, self.first),
            (True, False, True), (True, False, False),
        ):
            self.assertEqual(fact["run_commit"], commit)
            self.assertEqual(
                {entry["commit"]: entry["contained"] for entry in fact["commits"]},
                {self.first: first, self.second: second},
            )

    def test_ancestry_without_tagged_commit_is_unavailable(self):
        self.context["issue"]["short_id"] = "00000000"
        self.context["issue"]["gates"] = [self.gate("ci", self.merged)]
        self.assertEqual(self.review()["status"], "unavailable")

    def test_ancestry_without_run_commit_is_unavailable(self):
        self.context["issue"]["gates"] = [self.gate("ci", None)]
        self.assertEqual(self.review()["gates"][0]["status"], "unavailable")

    def test_ancestry_with_unknown_run_commit_is_unavailable(self):
        self.context["issue"]["gates"] = [self.gate("ci", "f" * 40)]
        fact = self.review()["gates"][0]
        self.assertEqual(fact["status"], "unavailable")
        self.assertTrue(all(entry["contained"] is None for entry in fact["commits"]))

    def test_ancestry_without_git_checkout_still_runs_review(self):
        self.context["issue"]["gates"] = [self.gate("ci", self.merged)]
        (self.repo / ".git").rename(self.repo / "hidden-git")
        self.assertEqual(self.review()["status"], "unavailable")

    def test_ancestry_without_optional_tool_still_accepts_reviewer_verdict(self):
        self.context["issue"]["gates"] = [self.gate("ci", self.merged)]
        for missing in ("git", "python3"):
            with self.subTest(missing=missing):
                directory = self.repo / ("without-" + missing)
                directory.mkdir()
                for tool in ("bash", "cat", "grep", "mktemp", "rm", "dirname",
                             "tail", "sed", "python3"):
                    if tool != missing:
                        (directory / tool).symlink_to(shutil.which(tool))
                facts = self.review(
                    expected_exit=0, PATH=str(directory),
                    REVIEWER_AGENT="cat; printf 'VERDICT: PASS\\n'",
                )
                self.assertEqual(facts["status"], "unavailable")

    def fake_jit(self, run):
        directory = self.repo / "bin"
        directory.mkdir(exist_ok=True)
        executable = directory / "jit"
        executable.write_text("#!/bin/sh\ncat <<'RUN'\n" + json.dumps(run) + "\nRUN\n")
        executable.chmod(0o755)
        return {"PATH": str(directory) + os.pathsep + os.environ["PATH"]}

    def test_ancestry_resolves_matching_latest_run_from_jit(self):
        gate = self.gate("ci", self.merged)
        del gate["commit"]
        self.context["issue"]["gates"] = [gate]
        run = {"key": "ci", "commit": self.merged, "status": "passed",
               "exit_code": 0, "started_at": gate["last_run_at"]}
        fact = self.review(**self.fake_jit(run))["gates"][0]
        self.assertEqual(fact["run_commit"], self.merged)
        self.assertTrue(all(entry["contained"] for entry in fact["commits"]))

    def test_ancestry_rejects_latest_run_that_disagrees_with_capture(self):
        gate = self.gate("ci", self.merged)
        del gate["commit"]
        self.context["issue"]["gates"] = [gate]
        for field, value in (("status", "failed"), ("exit_code", 1),
                             ("started_at", "2026-01-02T00:00:00+00:00")):
            with self.subTest(field=field):
                run = {"key": "ci", "commit": self.merged, "status": "passed",
                       "exit_code": 0, "started_at": gate["last_run_at"], field: value}
                fact = self.review(**self.fake_jit(run))["gates"][0]
                self.assertEqual(fact["status"], "unavailable")


if __name__ == "__main__":
    unittest.main()
