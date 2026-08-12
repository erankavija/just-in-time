#!/usr/bin/env python3
"""Prove the evidence validator rejects same-count identity substitution."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys


EVIDENCE = Path(__file__).resolve().parent
sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "nextest_concurrency_validator", EVIDENCE / "validate.py"
)
assert SPEC is not None and SPEC.loader is not None
validator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validator)

inventory = validator.load(EVIDENCE / "raw/inventory/stdout.log")
runnable, ignored = validator.inventory_identity_sets(inventory)
events = validator.test_events("w36-1")
validator.assert_run_identity_sets(runnable, ignored, events, successful=True)

mutated = [dict(event) for event in events]
terminal = next(
    event for event in mutated if event.get("event") in ("ok", "failed", "ignored")
)
terminal["name"] = "unexpected::same_count_identity_substitution"
assert len(mutated) == len(events)

try:
    validator.assert_run_identity_sets(runnable, ignored, mutated, successful=True)
except AssertionError:
    print("nextest-concurrency-934985ca: same-count substitution rejected")
else:
    raise AssertionError("same-count terminal identity substitution unexpectedly passed")
