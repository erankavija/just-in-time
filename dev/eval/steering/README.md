# LLM-agent steering eval

Measures whether JIT's validation steering actually helps a real agent converge
on valid plans. It drives an agent through the deterministic steering scenarios
in fully isolated temp repos and records **iterations-to-green** and
**rule-compliance** per scenario.

This eval is the LLM tier of the two-tier steering measurement (design CC-9,
issue `6a6af4e3`). It **reuses the same fixtures** as the deterministic harness
at `crates/jit/tests/fixtures/steering/<name>/scenario.toml`, so the two layers
stay in lockstep: the deterministic harness (`crates/jit/tests/steering_scenarios.rs`)
guarantees the message *content* the agent relies on; this eval measures whether
an agent can act on that content.

> **Schema note:** The TOML parser in `run_eval.py` is a deliberate duplicate
> subset of the schema parsed by `steering_scenarios.rs`. It covers only the
> keys the Python driver needs to drive the loop. Any schema evolution in the
> fixture TOML (new top-level, step, or expect keys) **must be mirrored** in
> both parsers. The Python parser fails fast on unknown keys so schema drift
> surfaces immediately rather than silently producing wrong behavior.

It is **not** a `cargo test` — it is a runnable harness that needs an agent
(network/model access for real runs) and is not deterministic across models. CI
stays deterministic; this lives outside the test gate.

## What it drives

The eval runs the **failing-input** steering scenarios — those whose fix is a
content or label correction surfaced at a write or validate enforcement point:

| Scenario | Failure | Correct fix |
|---|---|---|
| `sloppy-spec-body` | Prose (no bullets) under Success Criteria; create blocked (exit 4) | Re-author the body with Markdown bullet criteria |
| `typo-heading` | `## Sucess Criteria` typo; create blocked with a did-you-mean hint (exit 4) | Fix the heading spelling in the body |
| `stray-req` | `req:REQ-77` names no criterion; `jit validate` exits 1 | Remove the stray label (or add the criterion) |

Transition/structure scenarios (e.g. `premature-done`) require building out the
dependency graph rather than correcting an input, so they are out of scope for
this eval and are covered by the deterministic harness instead. The runner
auto-skips any scenario whose failing step is an `issue update --state`
transition.

## Isolation

Every run happens in a fresh `tempfile.mkdtemp()` repo that is removed after the
run (try/finally). Two mechanisms guarantee the production `.jit/` is never
touched, mirroring `jit_cmd()` in `steering_scenarios.rs`:

1. **cwd isolation** — every `jit` invocation runs with `cwd=<tempdir>`, so the
   default `.jit/` resolution anchors inside the temp directory.
2. **env sanitization** — `JIT_DATA_DIR`, `JIT_ALLOW_DELETION`,
   `JIT_WORKTREE_MODE`, `JIT_ENFORCE_LEASES`, and `JIT_LOCK_TIMEOUT` are stripped
   from every child environment (an absolute `JIT_DATA_DIR` would otherwise
   discard the temp cwd and write to an arbitrary path), and `JIT_AGENT_ID` is
   pinned to `agent:steering-eval`.

The runner prints a note if any of those vars were set in its own environment
(it strips them regardless).

## Requirements

- `python3` 3.11+ (stdlib only — `tomllib` is in the stdlib; no `pip` deps).
- The `jit` binary. The runner builds it (`cargo build -p jit`) if
  `target/debug/jit` is missing.

## Quick start

```bash
# Self-test the eval loop (oracle agent, deterministic, no network):
python3 dev/eval/steering/run_eval.py --smoke

# Full run with the built-in oracle (sanity check, 3 runs/scenario):
python3 dev/eval/steering/run_eval.py --agent-cmd builtin:oracle --runs 3
```

## The agent interface

The agent is pluggable behind `--agent-cmd`. An external agent is a command that
reads a JSON **task** on stdin and writes a JSON **action** on stdout.

Task (stdin):

```json
{
  "scenario": "sloppy-spec-body",
  "ruleset": "sdd",
  "failing_argv": ["issue", "create", "--title", "...", "--description", "..."],
  "failing_exit": 4,
  "failing_stderr": "<the exact error text the agent may act on>",
  "history": [{"argv": ["..."], "exit": 4, "stderr": "..."}]
}
```

Action (stdout — a single JSON object; if the agent prints logs too, the **last
non-empty line** must be the JSON action):

```json
{"body": "## Requirements\n\n- ...\n\n## Success Criteria\n\n- [hard] REQ-01: ..."}
```

or

```json
{"argv": ["issue", "update", "<id>", "--remove-label", "req:REQ-77"]}
```

Semantics of the loop, per iteration:

- The agent sees **only** the command, its exit code, and its error text — never
  the rules or the correct answer.
- A `body` action replaces the failing command's `--description` value; the
  failing command is then re-run with the new body.
- An `argv` action is run as a side-effect command (e.g. remove a label); the
  **original failing command is then re-run unchanged** to test for green.
- The loop repeats until the failing command exits 0 (green) or the `--cap`
  (default 5) is reached.

### Built-in agents

- `builtin:oracle` — applies the known-correct fix per scenario (well-formed
  body for the write failures; remove-stray-label for the validate failure).
  Deterministic; used by `--smoke` and as a loop sanity check. Expected to green
  every scenario in exactly 1 iteration.

### Hooking a real model (`claude -p`)

Write a thin adapter that turns the task JSON into a prompt and prints the action
JSON. Example using the Claude CLI in print mode:

```bash
#!/usr/bin/env bash
# agent-claude.sh — reads task JSON on stdin, prints action JSON on stdout.
# Pin the model and report it with results (see below).
task="$(cat)"
prompt="You are fixing a failing 'jit' command. You are given the command, its
exit code, and its error text as JSON. Respond with ONE JSON object and nothing
else: {\"body\": \"<corrected --description markdown>\"} to replace the issue
body, OR {\"argv\": [...]} to run one corrective jit subcommand (argv excludes
the leading 'jit'). Task:
$task"
claude -p "$prompt" --model claude-opus-4-8
```

Then run:

```bash
python3 dev/eval/steering/run_eval.py \
  --agent-cmd 'bash dev/eval/steering/agent-claude.sh' \
  --model-id claude-opus-4-8 \
  --runs 3
```

`--model-id` is **required** for any non-builtin agent so results are
comparable across rule changes. Pin the same model id every time you compare.
The pinned model for the project baseline is **`claude-opus-4-8`** (Claude Opus
4.8); record the exact model id you used alongside any results you report.

> The runner itself makes no network calls and invokes no model — that happens
> only inside your `--agent-cmd`. Building and smoke-testing the harness is
> entirely offline.

## Output

Two outputs:

1. A human table on stdout: per scenario, the stability classification, mean /
   min / max iterations-to-green, and the green/runs ratio.
2. A results JSON at `dev/eval/steering/results/results-<timestamp>.json`
   (gitignored) carrying `jit_commit`, `model_id`, `runs`, `timestamp`, and
   `per_scenario` stats. This is the artifact you diff across rule/message
   changes.

Stability classification per scenario:

- **always** — every run reached green within the cap.
- **sometimes** — mixed (some runs green, some not).
- **never** — no run reached green within the cap.

Two iteration-count metrics are reported (both in the table and in the results
JSON):

- **mean** (`mean_iterations`) — mean iterations-to-green computed **over green
  runs only**. This is the primary signal for "how long does it take when it
  works", but it is an optimistic metric: non-green runs are excluded, so a
  model that rarely converges can still show a low mean (survivorship bias).
- **penalized** (`penalized_mean`) — mean iterations where every non-green run
  (including timeouts, malformed responses, and cap-exhausted runs) is charged
  the full `--cap` value. This is the bias-corrected companion; prefer it when
  comparing models with different convergence rates.

Report both metrics over ≥3 runs, not the best/peak run.

## Re-run procedure (comparing after rule or message changes)

1. **Rebuild** the binary so the eval exercises the new rules/messages:
   `cargo build -p jit` (the runner does this automatically if the binary is
   missing, but rebuild explicitly after changing Rust or example rulesets).
2. **Baseline before the change** (if you do not already have one): run with the
   pinned model id and `--runs 3` (or more); keep the
   `results/results-<timestamp>.json`.
3. **Make the rule/message change** (edit `docs/examples/<ruleset>/rules.toml`,
   schemas, or the engine messages), then rebuild.
4. **Re-run with the same `--model-id` and the same `--runs`.** The fixtures are
   shared with the deterministic harness, so the scenario set is identical
   across runs by construction.
5. **Diff the JSONs.** Compare `per_scenario` mean iterations-to-green and the
   stability classification between the two `results-*.json` files. A rule/
   message change "helps" if it lowers mean iterations and/or moves a scenario
   from `sometimes`/`never` toward `always`. Each JSON records its `jit_commit`
   and `model_id` so comparisons stay apples-to-apples.

Always use n ≥ 3 runs and report the mean, not the peak — a single lucky run is
not evidence.

## Testing the harness itself

Unit tests (stdlib `unittest`, no jit subprocesses, no network):

```bash
python3 -m unittest discover -s dev/eval/steering -p 'test_*.py'
```

They cover the run-count policy (`--runs` < 3 rejected without
`--allow-few-runs`), atomic result writes, expect-assertion checking (setup
steps validate exit codes AND `contains`/`not_contains`), aggregation and
stability math (including the penalized mean), env sanitization, schema
strictness, and results reportability. The end-to-end loop self-test is
`run_eval.py --smoke`.

Runs below 3 (debug only, via `--allow-few-runs`) write a
`results-debug-*.json` artifact carrying `"reportable": false` — these are
never comparable output and must not be cited as eval results.
