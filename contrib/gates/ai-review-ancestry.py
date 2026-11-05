#!/usr/bin/env python3
"""Resolve gate ancestry without changing the captured review context."""

import json
import os
import subprocess
import sys


def command(*arguments, **options):
    return subprocess.run(arguments, capture_output=True, text=True,
                          timeout=30, check=False, **options)


def unavailable(reason, **fields):
    return {"status": "unavailable", "reason": reason, **fields}


def run_commit(issue, gate):
    if gate.get("commit"):
        return gate["commit"]
    # An unrun projection cannot identify a run from mutable tracker state.
    if not gate.get("last_run_at") or "exit_code" not in gate:
        return None
    environment = dict(os.environ)
    environment.pop("JIT_DATA_DIR", None)
    result = command("jit", "gate", "status", issue["id"], gate["key"],
                     "--json", env=environment)
    if result.returncode:
        return None
    run = json.loads(result.stdout)
    matches = (run.get("key") == gate["key"]
               and run.get("status") == gate.get("status")
               and run.get("started_at") == gate["last_run_at"]
               and run.get("exit_code") == gate["exit_code"])
    return run.get("commit") if matches else None


def gate_fact(issue, gate, commits):
    try:
        commit = run_commit(issue, gate)
    except (OSError, ValueError, KeyError, subprocess.TimeoutExpired):
        commit = None
    if not commit:
        return unavailable("No run commit matches the captured gate projection.",
                           gate=gate["key"])
    resolved = command("git", "rev-parse", "--verify", "--end-of-options",
                       commit + "^{commit}")
    relations = []
    for tagged in commits:
        result = command("git", "merge-base", "--is-ancestor", tagged,
                         resolved.stdout.strip()) if not resolved.returncode else None
        contained = result.returncode == 0 if result and result.returncode in (0, 1) else None
        relations.append({"commit": tagged, "contained": contained})
    fields = {"gate": gate["key"], "run_commit": commit, "commits": relations}
    if any(relation["contained"] is None for relation in relations):
        return unavailable("Git could not resolve the run ancestry.", **fields)
    return {"status": "available", **fields}


def ancestry(context):
    issue = context.get("issue", {})
    short_id = issue.get("short_id") or issue.get("id", "").split("-")[0]
    if not short_id:
        return unavailable("The context has no issue ID.")
    tag = "jit:" + short_id
    history = command("git", "log", "--fixed-strings", "--grep=" + tag,
                      "--format=%H", "HEAD", "--")
    if history.returncode:
        return unavailable("Git history is unavailable.", issue_tag=tag)
    commits = history.stdout.splitlines()
    if not commits:
        return unavailable("No tagged commit is reachable from HEAD.", issue_tag=tag)
    gates = [gate_fact(issue, gate, commits) for gate in issue.get("gates", [])]
    if not gates:
        return unavailable("The context names no required gate run.", issue_tag=tag)
    return {"status": "available" if all(g["status"] == "available" for g in gates)
            else "unavailable", "issue_tag": tag, "gates": gates}


if __name__ == "__main__":
    try:
        with open(sys.argv[1], encoding="utf-8") as source:
            facts = ancestry(json.load(source))
    except (OSError, ValueError, KeyError, subprocess.TimeoutExpired):
        facts = unavailable("Git or context data is unavailable.")
    print(json.dumps(facts))
