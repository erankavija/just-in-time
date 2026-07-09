# Lead-skills eval baseline

Single checkable place confirming both renamed lead skills — `jit-execution-lead`
(formerly `project-lead`) and `jit-planning-lead` (formerly `jit-plan`) — pass
their evals. Each row cites the on-disk artifact that backs it; re-run the
procedure in [`skill-eval-adjudication.md`](skill-eval-adjudication.md) to refresh a
verdict. This baseline is a record of what was run, not a reusable framework.

## Scenario evals (adjudicated against `expected_output`)

Method: [`dev/eval/skill-eval-adjudication.md`](skill-eval-adjudication.md) —
each scenario's `expected_output` is decomposed into an itemized checklist and
scored against observable final repo state from a fresh-context runner (not the
run's self-report).

| Skill | Evals | Runner + verdicts | Result |
|---|---|---|---|
| `jit-execution-lead` | [`evals/evals.json`](../../.claude/skills/jit-execution-lead/evals/evals.json) — `sw-epic-with-children`, `sw-epic-needs-breakdown`, `content-project-epic`, `parent-invoked-escalation` | [`evals/results.md`](../../.claude/skills/jit-execution-lead/evals/results.md) + transcripts | 4/4 PASS |
| `jit-planning-lead` | [`evals/evals.json`](../../.claude/skills/jit-planning-lead/evals/evals.json) — `research-and-plan`, `plan-from-existing`, `plan-from-import` (one per entry path) | [`evals/results.md`](../../.claude/skills/jit-planning-lead/evals/results.md) + transcripts | 3/3 PASS |

`jit-planning-lead`'s `plan-from-import` verdict is PASS with a recorded finding:
the run's coverage-preview gate passed vacuously on checkbox-prefixed criteria.
That is a discovered defect in the coverage rule, not an eval failure (real
coverage was verified independently); it is tracked as bug `16402e14` and does
not affect this baseline.

## Trigger (activation) evals

Method: the skill-creator plugin's trigger-eval runner over each skill's
`trigger_eval.json`; `runs_per_query = 5`, results in `trigger_eval_results.json`.

| Skill | `trigger_eval.json` | Results | Should-trigger | Should-not-trigger | Total |
|---|---|---|---|---|---|
| `jit-execution-lead` | [file](../../.claude/skills/jit-execution-lead/trigger_eval.json) | [results](../../.claude/skills/jit-execution-lead/trigger_eval_results.json) | 8/8 | 9/9 | 17/17, failed 0 |
| `jit-planning-lead` | [file](../../.claude/skills/jit-planning-lead/trigger_eval.json) | [results](../../.claude/skills/jit-planning-lead/trigger_eval_results.json) | 8/8 | 9/9 | 17/17, failed 0 |

## Success-criteria map (issue `c23dfe71`)

- REQ-01 (adjudication method written down) → [`skill-eval-adjudication.md`](skill-eval-adjudication.md).
- REQ-02 (`jit-execution-lead` scenario evals run + recorded) → its `evals/results.md`, 4/4 PASS across `sw-epic-with-children`, `sw-epic-needs-breakdown`, `content-project-epic`, and `parent-invoked-escalation` (context-aware escalation target).
- REQ-03 (`jit-planning-lead` gains `evals/`, run + recorded) → its `evals/results.md`, 3/3 PASS across the three entry paths.
- REQ-04 (both skills gain a trigger check + recorded passing results) → both `trigger_eval_results.json`, 17/17 each.
