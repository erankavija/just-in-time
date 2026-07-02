# jit-execution-lead — Eval Results

Adjudicated baseline for the scenarios in [`evals.json`](evals.json), graded under the
method in [`docs/reference/skill-eval-adjudication.md`](../../../../docs/reference/skill-eval-adjudication.md):
each scenario's `expected_output` is decomposed into an itemized checklist and every item
is scored against observable repo-state evidence from a recorded run.

**Runner:** harness-governed fresh-context sub-agent given the scenario `prompt` verbatim
(the equivalent-runner path documented in the adjudication method), one per scenario, each
against an isolated repo built by [`setup-test-repo.sh`](setup-test-repo.sh) under
`/tmp/jit-evals/<scenario>`. MCP disabled; the `jit` CLI binary used directly. Verdicts
below were scored by inspecting the final repo state (`jit issue show --json`, `jit validate`,
file contents, `pytest`), not the run's self-report.

| Scenario | Date | Verdict | Backing |
|---|---|---|---|
| `sw-epic-with-children` | 2026-07-02 | **PASS** | itemized checklist below + [run report](transcripts/sw-epic-with-children.completion-report.md) |
| `sw-epic-needs-breakdown` | 2026-07-02 | **PASS** | itemized checklist below + [run report](transcripts/sw-epic-needs-breakdown.completion-report.md) |
| `content-project-epic` | 2026-07-02 | **PASS** | itemized checklist below + [run report](transcripts/content-project-epic.completion-report.md) |

---

## `sw-epic-with-children` — PASS (2026-07-02)

- **Run:** epic `ff50a876`, children `909b7d80` (slugify), `bb167cf6` (truncate).
- **Run report:** [`transcripts/sw-epic-with-children.completion-report.md`](transcripts/sw-epic-with-children.completion-report.md).

`expected_output`: *"Epic and all child issues transitioned to done. src/strings.py
contains slugify and truncate functions. tests/test_strings.py contains passing tests. All
gates passed. Completion report produced."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Epic transitioned to `done` | `jit issue show ff50a876` → state `done` | PASS |
| 2 | Every child transitioned to `done` | `909b7d80` and `bb167cf6` both state `done` | PASS |
| 3 | `src/strings.py` contains `slugify` | `def slugify(text: str) -> str` at src/strings.py:10 | PASS |
| 4 | `src/strings.py` contains `truncate` | `def truncate(text, max_length, suffix='...')` at src/strings.py:40 | PASS |
| 5 | `tests/test_strings.py` contains passing tests | `python -m pytest tests/` → 25 passed | PASS |
| 6 | All gates passed | `tests` + `code-review` = `passed` on both children (epic carries none by design) | PASS |
| 7 | Completion report produced | `COMPLETION_REPORT.md` present, linked to epic via `jit doc list ff50a876` | PASS |

Also observed: `jit validate` clean, working tree committed and clean.

**Verdict: PASS** (7/7).

---

## `sw-epic-needs-breakdown` — PASS (2026-07-02)

- **Run:** epic `c708ae5f` broken into `f3f0d325` (shared models) and `02697be0`
  (add/subtract/multiply/divide endpoints).
- **Run report:** [`transcripts/sw-epic-needs-breakdown.completion-report.md`](transcripts/sw-epic-needs-breakdown.completion-report.md).

`expected_output`: *"Epic broken down into child tasks covering all 4 arithmetic
endpoints. Children created with proper DAG wiring. Implementation completed. All gates
passed. Epic transitioned to done. Completion report produced."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Epic broken into child tasks | epic started child-less; 2 children (`f3f0d325`, `02697be0`) now exist | PASS |
| 2 | Children cover all 4 arithmetic endpoints | `src/routes.py` defines `POST /add` (:15), `/subtract` (:22), `/multiply` (:32), `/divide` (:42) | PASS |
| 3 | Proper DAG wiring | `jit validate` → acyclic, passes; endpoints task depends on models task | PASS |
| 4 | Implementation completed | `src/models.py`, `src/routes.py`, `src/main.py` present; divide-by-zero → HTTP 400 at routes.py:53 | PASS |
| 5 | All gates passed | `tests` + `code-review` = `passed` on epic and both children; `pytest` → 26 passed | PASS |
| 6 | Epic transitioned to `done` | `jit issue show c708ae5f` → state `done` | PASS |
| 7 | Completion report produced | `COMPLETION_REPORT.md` present, linked to epic via `jit doc list c708ae5f` | PASS |

Note: the four endpoints are covered by one endpoints task rather than one task per
endpoint. `expected_output` requires "child tasks covering all 4 arithmetic endpoints",
not one task per endpoint; the four endpoints share a single `src/routes.py` and one
suite-wide `tests` gate, so per-endpoint splitting would collide in one file. Coverage of
all four is the checked assertion (item 2) and is satisfied.

**Verdict: PASS** (7/7).

---

## `content-project-epic` — PASS (2026-07-02)

- **Run:** epic `e17ab7e6` broken into `4731023b` (reference), `eb0d65cd` (how-to),
  `85df0968` (tutorial), `de9bab98` (index update).
- **Run report:** [`transcripts/content-project-epic.completion-report.md`](transcripts/content-project-epic.completion-report.md).

`expected_output`: *"Epic broken down into doc tasks. Tutorial, how-to guide, and
reference page created in correct directories. All follow Diataxis structure and project
conventions. docs/index.md updated. Content-review gate passed. Epic done. No
software-specific commands attempted."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Epic broken into doc tasks | 4 children created (`4731023b`, `eb0d65cd`, `85df0968`, `de9bab98`) | PASS |
| 2 | Tutorial in correct directory | `docs/tutorials/your-first-nexus-project.md` | PASS |
| 3 | How-to guide in correct directory | `docs/how-to/configure-nexus.md` | PASS |
| 4 | Reference page in correct directory | `docs/reference/configuration-options.md` | PASS |
| 5 | Docs follow Diataxis + project conventions | files placed by Diataxis type; lowercase-hyphen names per project CLAUDE.md conventions | PASS |
| 6 | `docs/index.md` updated with links | index links to all three new pages (index.md:7,11,15) | PASS |
| 7 | Content-review gate passed | `content-review` = `passed` on epic and all 4 children | PASS |
| 8 | Epic transitioned to `done` | `jit issue show e17ab7e6` → state `done` | PASS |
| 9 | No software-specific commands attempted | working tree holds no code/test scaffolding (`find` for `*.py`/`package.json`/`Cargo.toml`/`pytest.ini` → none); output is Markdown only | PASS |

Also observed: `jit validate` clean, completion report linked to epic via `jit doc list e17ab7e6`.

**Verdict: PASS** (9/9).

---

## Reproducing a verdict

1. Rebuild the scenario's repo: `bash setup-test-repo.sh /tmp/jit-evals/<scenario> <setup_scenario>`.
2. Run the scenario `prompt` (from `evals.json`) with an equivalent runner, substituting the
   scratch repo path for `{REPO_PATH}` and the epic ID the setup script printed.
3. Re-score the checklist above against the final repo state.

See [`docs/reference/skill-eval-adjudication.md`](../../../../docs/reference/skill-eval-adjudication.md)
for the full method.
