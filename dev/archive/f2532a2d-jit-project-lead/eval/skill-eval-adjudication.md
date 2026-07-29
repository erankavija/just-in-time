# Scenario-Eval Adjudication

> **Diátaxis Type:** Reference

How to turn a scenario-eval run into a reproducible pass/fail verdict. A scenario eval
(e.g. `.agents/skills/jit-execution-lead/evals/evals.json`) ships a free-text
`expected_output` description. This document defines the method that converts that
description into an itemized, independently checkable checklist and scores a run against
it, so that anyone — not just the person who ran the eval — can reproduce the verdict.

This is a thin run-and-record procedure for a handful of scenarios, not a general eval
framework. It grades an already-produced run; it does not schedule, batch, or aggregate
runs.

---

## Method: checklist conversion

The `expected_output` of each scenario is a paragraph of prose. Grading it from memory is
not reproducible: two graders read the same paragraph differently, and neither writes down
what "pass" meant. The method removes that ambiguity by **converting the `expected_output`
into an itemized checklist before grading**, then scoring each item against observable
evidence from the run.

Checklist conversion is chosen over an LLM-judge because it is self-contained: it needs no
second model, its items are checkable by inspecting repo state and files with ordinary
commands, and the completed checklist is itself the durable record of the verdict.

---

## Required inputs

To adjudicate one scenario you need all of the following:

1. **The scenario definition** — the object in the skill's `evals/evals.json` with its
   `name`, `prompt`, `setup_scenario`, and `expected_output`.
2. **The setup script** — `evals/setup-test-repo.sh` (or the scenario's documented setup).
   It builds a fresh test repo and prints the epic ID (and any child IDs) the run needs.
3. **A fresh test repo** — produced by the setup script at a scratch path outside the JIT
   repository (e.g. under `/tmp`), so the run cannot contaminate the project's own `.jit/`.
4. **A recorded run** — a transcript of an agent that was given the scenario `prompt`
   (with `{REPO_PATH}` and the epic ID substituted) and ran to completion against the
   production skill files.
5. **The final repo state** — the test repo as the run left it: `jit` issue states, gate
   statuses, the working tree, and any completion report the run produced. This is the
   primary evidence; the transcript is secondary evidence for steps that leave no on-disk
   trace.

---

## Procedure

### 1. Build the test repo

```bash
bash <skill>/evals/setup-test-repo.sh /tmp/jit-evals/<scenario> <setup_scenario>
```

Record the epic ID (and child IDs) the script prints; the run needs them.

### 2. Run the scenario

Launch a fresh-context agent with the scenario's `prompt`, substituting the scratch repo
path for `{REPO_PATH}` and appending the epic ID the setup script printed. The canonical
runner is a headless Claude Code invocation with its output captured to a transcript file:

```bash
cd /tmp/jit-evals/<scenario>
claude -p "<prompt with REPO_PATH and epic ID substituted>" \
  --permission-mode bypassPermissions \
  --strict-mcp-config \
  --verbose --output-format stream-json \
  > transcript.jsonl 2> transcript.err
```

`--strict-mcp-config` with no `--mcp-config` disables all MCP servers, so the run cannot
reach the project's production `.jit/`. Any equivalent runner is acceptable provided it
(a) gives the agent a fresh context, (b) points it at the production skill files named in
the prompt, (c) lets it act without per-step human confirmation, and (d) preserves a
run-record. When the raw `bypassPermissions` subprocess is unavailable, a harness-governed
sub-agent given the same prompt is an equivalent runner; record which runner was used.

The run-record differs by runner, and the negative command-attempt items in step 3
constrain what it must contain:

- **Raw-transcript runner** — the run-record is `transcript.jsonl`. It carries the full
  command/tool-invocation history inherently, so negative items are checkable directly
  against it.
- **Equivalent sub-agent runner** — the run-record is the completion report it produces.
  A completion report summarizes outcomes and need not be a raw transcript, so it does
  **not** carry an invocation history on its own. Therefore, when a scenario has any
  negative command-attempt item, the equivalent runner's completion report MUST include a
  **command / gate-invocation log**: the gates the run defined and executed, and an
  explicit affirmation when no build/test-runner command was invoked, each traceable to
  the run repo's gate registry (`.jit/gates.toml`) and event log (`.jit/events.jsonl`).
  These are the run's own on-disk records, not the report's self-report, so a later reader
  reproduces the check by inspecting (or re-deriving) them. A scenario with no negative
  items needs no such log. If a scenario has a negative item and no invocation log can be
  substantiated for it, the run is ungradeable on that item and must be re-run under the
  raw-transcript path.

### 3. Convert `expected_output` into a checklist

Split the `expected_output` prose into atomic, independently checkable items. Each item
must be a single observable outcome with a stated evidence source. Rules:

- **One assertion per item.** "Epic and all children done" is two items if children must be
  enumerated: the epic is `done`, and every child is `done`.
- **Name the evidence source** for each item: a `jit` query, a file's existence or
  contents, a gate status, or a specific passage of the transcript.
- **Prefer repo-state evidence over transcript evidence.** Check a file or a `jit` state
  before falling back to "the transcript shows the agent did X".
- **Keep negative items checkable, against the run-record's action log.** "No
  software-specific commands attempted" (a content scenario) is checked as: the working
  tree contains no code/test scaffolding **and** the run-record shows no build/test-runner
  invocation. Both halves are required. Repo-state alone never satisfies a negative
  command-attempt item, because absence of code proves nothing about what was attempted.
  The second half is read from whichever run-record the runner preserved (step 2): the raw
  transcript for the raw-transcript path, or the equivalent runner's command / gate-invocation
  log for the equivalent-runner path.
- Do not add requirements the `expected_output` does not state, and do not drop any it
  does. The checklist is a faithful decomposition, not a re-specification.

Write the resulting checklist into the results record (below) so the decomposition is
auditable.

### 4. Score each item

Mark each item against the evidence:

- **PASS** — evidence confirms the outcome. Cite the evidence (command output, file path,
  gate status, or transcript location).
- **FAIL** — evidence contradicts the outcome or the outcome is absent.
- **N/A** — the item does not apply to this run; state why. Use sparingly; an item that is
  merely unmet is FAIL, not N/A.

### 5. Emit the verdict

The scenario verdict is the conjunction of its items:

- **PASS** — every item is PASS (N/A items excluded).
- **FAIL** — one or more items are FAIL. List them.

A verdict is complete only when it records: the scenario name, the run date, the runner
used, the run-record location (the transcript, or the run's completion report when the
equivalent sub-agent runner is used), the itemized checklist with each item's mark and
evidence, and the overall PASS/FAIL.

---

## Recording verdicts

Record every adjudicated run in the skill's `evals/results.md`. For each scenario capture:

- Scenario `name` and the date the run was produced.
- The overall verdict (**PASS** / **FAIL**).
- The run-record location (a transcript or completion report under `evals/transcripts/`)
  and the inlined completed checklist backing the verdict.
- The itemized checklist with per-item marks and evidence pointers.

`results.md` is the durable baseline: a later reader confirms the claim "this skill passes
its evals" by reading the completed checklists, and reproduces any verdict by re-running
the procedure above against the same scenario.

---

## Handling a failing run

A FAIL verdict is recorded honestly; it is never massaged into a pass. When a scenario
fails, distinguish the cause before deciding what happens next:

- **Setup error** — the failure is an artifact of a mis-built test repo (wrong IDs, missing
  gate, stale scratch dir). Rebuild the repo with the setup script and re-run once. Note the
  correction.
- **Real skill regression** — the run followed a correctly built repo and still missed an
  `expected_output` item. Record the FAIL with its failing items and evidence, and surface
  it; fixing the skill is a separate decision, not part of adjudication.

Never edit the skill under test to make a run pass. Grade what the run produced.
