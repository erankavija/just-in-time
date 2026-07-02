# Trigger eval runner for jit lead skills

Runs skill-activation trigger evals (jit:6662f738) for `jit-execution-lead` and
`jit-planning-lead` against their **live** SKILL.md frontmatter descriptions.

## Why not the skill-creator plugin's stock runner unmodified

The skill-creator plugin ships its own trigger-eval runner
(`scripts/run_eval.py`): for each query it drops a temp command file into
`.claude/commands/` carrying the skill's description under a synthetic,
unique name, runs `claude -p <query>` headlessly, and checks whether the
model invokes that synthetic name via the `Skill` (or `Read`) tool.

That technique assumes the skill under test is *not* otherwise present in
`available_skills` — otherwise the model has no reason to reach for the
synthetic proxy when the real skill is right there with the same
description. That assumption breaks for `jit-execution-lead` and
`jit-planning-lead`: both are installed at the **user level**
(`~/.claude/skills/jit-execution-lead`, `~/.claude/skills/jit-planning-lead`),
symlinked into this repo, so they're in `available_skills` for every
`claude -p` invocation regardless of cwd. A smoke test confirmed this: for
the query "Take charge of epic 6662f738 and drive it to completion with a
team of agents", the model correctly called
`Skill(skill="jit-execution-lead", ...)` immediately — but the stock
runner's unique-name match reported it as a non-trigger, because it was
looking for `jit-execution-lead-skill-<uuid>`, not the real skill's name.

`run_trigger_eval.py` in this directory keeps the plugin's methodology
(temp command file, headless `claude -p`, stream-json tool-use detection,
N runs per query for a reliable rate) and changes exactly one thing: the
match is on the skill's **base name** (e.g. `jit-execution-lead`) rather
than the synthetic unique suffix. That accepts either the real,
already-installed skill or the synthetic proxy as a valid trigger — both
carry the identical live description, so either one firing is equally
strong evidence the description works.

It also runs `claude -p` from a fresh temp directory with no project-level
`.claude/skills` of its own, so the only skill matching the target name is
the real user-level installation — the same set of skills any other project
would see.

## Usage

```bash
python3 dev/eval/skill-triggers/run_trigger_eval.py \
  --eval-set .claude/skills/jit-execution-lead/trigger_eval.json \
  --skill-path <path-to-jit-execution-lead-skill-dir> \
  --out .claude/skills/jit-execution-lead/trigger_eval_results.json \
  --model <model-id-powering-this-session> \
  --runs-per-query 3 \
  --verbose
```

`--skill-path` must point at a directory containing `SKILL.md` — the
description under test is read live from there, not from the eval-set file.
Use the model id powering the current session so the eval matches what the
user actually experiences (per the skill-creator plugin's own guidance).

## Output

`--out` receives a JSON file with per-query results (`trigger_rate`,
`pass`) and a `summary` broken out by category:

```json
{
  "summary": {
    "total": 17,
    "passed": 17,
    "failed": 0,
    "should_trigger": {"total": 8, "passed": 8, "pass_rate": 1.0},
    "should_not_trigger": {"total": 9, "passed": 9, "pass_rate": 1.0}
  }
}
```

A query "passes" if its measured trigger rate over `--runs-per-query` runs
is `>= --trigger-threshold` (default 0.5) for should-trigger queries, or
`< --trigger-threshold` for should-not-trigger queries.
