# jit-planning-lead — Eval Results

Adjudicated baseline for the scenarios in [`evals.json`](evals.json), graded under the
method in [`docs/reference/skill-eval-adjudication.md`](../../../../docs/reference/skill-eval-adjudication.md):
each scenario's `expected_output` is decomposed into an itemized checklist and every item
is scored against observable repo-state evidence from a recorded run.

The three scenarios map one-to-one onto jit-planning-lead's three entry paths:

| Scenario | Entry path | Seed state |
|---|---|---|
| `research-and-plan` | **research-and-plan** | no container exists; only a vague idea (in the prompt) |
| `plan-from-existing` | **plan-from-existing** | an epic with `[hard]` REQ criteria already exists |
| `plan-from-import` | **plan-from-import** | an external planning doc exists, not yet in jit |

**Runner:** harness-governed fresh-context sub-agent given the scenario `prompt` (the
equivalent-runner path documented in the adjudication method), one per scenario, each
against an isolated repo built by [`setup-test-repo.sh`](setup-test-repo.sh) under
`/tmp/jit-evals/<scenario>`. MCP disabled; the `jit` CLI binary used directly; the run read
the production skill from `~/.claude/skills/jit-planning-lead/SKILL.md` (stable symlink to
the main repo). Verdicts below were scored by inspecting the final repo state
(`jit issue show --json`, `jit item list --json`, `jit gate check`, `jit validate`, the
`.jit/` event log and gate-run records, and the working tree), not the run's self-report.

Because each scenario carries a negative command-attempt item ("no implementation code
written"), the equivalent-runner path requires a **command / gate-invocation log** in each
run's completion report; all three graded runs include one, and each claim in it is
corroborated below against the run repo's own `.jit/gates.json`, `.jit/events.jsonl`, and
`.jit/gate-runs/` (repo-state), so the negative item is checked against the action log, not
just the final tree.

| Scenario | Date | Verdict | Backing |
|---|---|---|---|
| `research-and-plan` | 2026-07-03 | **PASS** | itemized checklist below + [run report](transcripts/research-and-plan.completion-report.md) |
| `plan-from-existing` | 2026-07-03 | **PASS** | itemized checklist below + [run report](transcripts/plan-from-existing.completion-report.md) |
| `plan-from-import` | 2026-07-03 | **PASS** (8/8, with finding) | itemized checklist below + [run report](transcripts/plan-from-import.completion-report.md) |

> **Finding on `plan-from-import`.** The run passed every `expected_output` item, but it
> wrote the container's success criteria in GitHub-checkbox form (`- [ ] [hard] REQ-NN …`)
> instead of the skill-prescribed `- [hard] REQ-NN: …` (see
> `jit-planning-lead/references/plan-doc-template.md`: *"Never bare, never mixed markers"*).
> Because the `[hard]` marker is matched only at the **start** of a projected list item
> (`crates/jit/src/validation/graph.rs`), the leading `[ ]` checkbox hides every criterion
> from both the item projector (`jit item list` → **0 items**) and the `label-coverage`
> rule. The **coverage-preview gate therefore passed vacuously**: it verified nothing. Real
> coverage is nonetheless complete — the four stories carry `satisfies:` labels covering all
> six REQs (checklist item 5, verified by direct label inspection, independent of the
> parser). Proven non-destructively on a throwaway copy of the run repo: reformatting the
> criteria to `- [hard] REQ-NN` makes `jit item list` report all 6 and gives the gate teeth
> (stripping one `satisfies:` label then fails scope validation naming the uncovered REQ).
> This is a genuine weakness surfaced by the eval; the eval itself (this issue) only adds the
> scenario, so fixing the skill's criteria-format discipline is a separate decision.

---

## `research-and-plan` — PASS (2026-07-03)

- **Run:** container `cc89c947` (`type:epic`, `epic:config-loader`, 8 `[hard]` criteria
  REQ-01..REQ-08); plan node `758c433c`; breakdown node `6da405a1`; impl children
  `edbf4c76` (REQ-01), `d9ca0f9b` (REQ-03,04), `2acec3bc` (REQ-02,05,06,07), `3e06ee15`
  (REQ-08).
- **Run report:** [`transcripts/research-and-plan.completion-report.md`](transcripts/research-and-plan.completion-report.md).

`expected_output`: *"A container issue (type:epic) is created with a `## Success Criteria`
section holding at least one `[hard] REQ-NN` criterion that captures the typed
configuration-loading idea. A decision log records the autonomous intake decisions with
their rejected options and reasons. The plan bracket is scaffolded on the container via
`jit apply plan` (a planning node P and a breakdown node B on the C -> B -> P spine). A plan
document exists at P's plan-doc path (dev/active/<container-short-id>-plan.md), is linked to
P, and P is done with its plan-review gate passed. The breakdown created impl children that
are non-breakable leaves and together carry a `satisfies:REQ-NN` label for every `[hard]`
criterion of the container; B's coverage-preview and breakdown-review gates passed and B is
done. No implementation code for the planned helper was written (planning and breakdown
only). `jit validate` passes on the final repo."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Container issue (`type:epic`) created with `## Success Criteria` holding ≥1 `[hard] REQ-NN` | `cc89c947` is `type:epic`; `jit item list --json` parses 8 `requirement` items REQ-01..REQ-08 (all `[hard]`) | PASS |
| 2 | Decision log records autonomous intake decisions with rejected options + reasons | plan doc `dev/active/cc89c947-plan.md` Decisions section records D-1..D-9 (+A-1..A-3), each with chosen and rejected options/reasons | PASS |
| 3 | Plan bracket scaffolded via `jit apply plan` (P + B on the `C → B → P` spine) | `758c433c` `type:planning`, `6da405a1` `type:breakdown` (`brackets:cc89c947`); dependency spine `C → impl → B → P` present; `jit validate` acyclic | PASS |
| 4 | Plan doc at P's plan-doc path, linked to P | file `dev/active/cc89c947-plan.md` exists; `jit doc list 758c433c` → `dev/active/cc89c947-plan.md [HEAD] <design>` | PASS |
| 5 | P is `done` with `plan-review` passed | `jit issue show 758c433c` → state `done`; required gate `plan-review` status `passed`; `events.jsonl` has one `plan-review` `gate_passed`; by INV-GATE-SEMANTICS P could not be `done` otherwise | PASS |
| 6 | Impl children are non-breakable leaves (frontier empty) | 4 children all `type:task`; `task` is in no template's `applies_to` (only `epic` is breakable), so none is breakable → frontier empty | PASS |
| 7 | Children together carry `satisfies:REQ-NN` for every `[hard]` criterion | union of children's `satisfies:` labels = REQ-01..REQ-08 = full item set; `coverage-preview` (real `jit validate --scope`) passed exit 0 | PASS |
| 8 | B's `coverage-preview` and `breakdown-review` passed and B is `done` | `jit issue show 6da405a1` → state `done`; `coverage-preview` passed (exit 0, `.jit/gate-runs/`), `breakdown-review` status `passed` (attested; `gate_passed` event) | PASS |
| 9 | No implementation code written (negative — both halves) | **Repo-state:** working tree holds only seed `src/__init__.py` and `tests/__init__.py`; no feature module. **Run-record command log:** the only auto gate-runs in `.jit/gate-runs/` are 2× `coverage-preview` (`jit validate --scope`); no run contains `pytest`/`cargo`/`npm`; the `tests` gate never executed (children left `backlog`/`ready`); report's Command/Gate-Invocation Log affirms no build/test runner invoked | PASS |
| 10 | `jit validate` passes on the final repo | `jit validate` → `✓ Repository validation passed` | PASS |

Also observed: exactly one epic exists; container `cc89c947` remains `backlog` with its
`repo-validate` gate pending — correct, since planning does not execute the work.

**Verdict: PASS** (10/10).

---

## `plan-from-existing` — PASS (2026-07-03)

- **Run:** pre-existing epic `7af9ff75` "Retry Utility" (4 `[hard]` criteria REQ-01..REQ-04);
  plan node `4dac8c81`; breakdown node `06de7133`; impl children `863bea83` (REQ-01),
  `71619b54` (REQ-02), `f52146d4` (REQ-03), `3693f426` (REQ-04).
- **Run report:** [`transcripts/plan-from-existing.completion-report.md`](transcripts/plan-from-existing.completion-report.md).

`expected_output`: *"The pre-existing epic (the one named in the prompt) is planned in
place, not re-created: no second epic is introduced, and its four `[hard]` REQ criteria are
reconciled/verified. The plan bracket is scaffolded on that epic via `jit apply plan`
(planning node P and breakdown node B on the C -> B -> P spine). A plan document exists at
P's plan-doc path (dev/active/<container-short-id>-plan.md), is linked to P, and P is done
with its plan-review gate passed. The breakdown created impl children that are non-breakable
leaves and together carry a `satisfies:REQ-NN` label for every one of the container's four
`[hard]` criteria (REQ-01..REQ-04); B's coverage-preview and breakdown-review gates passed
and B is done. No implementation code for the retry helper was written (planning and
breakdown only). `jit validate` passes on the final repo."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Pre-existing epic planned in place, not re-created (no second epic); its four `[hard]` criteria reconciled/verified | exactly 1 epic (`7af9ff75`), same id as the prompt; `jit item list --json` = REQ-01..REQ-04; report records the reconcile pass (greenfield → all four valid-and-open, none stale) | PASS |
| 2 | Plan bracket scaffolded via `jit apply plan` (P + B on `C → B → P`) | `4dac8c81` `type:planning`, `06de7133` `type:breakdown` (`brackets:7af9ff75`); spine present; `jit validate` acyclic | PASS |
| 3 | Plan doc at P's plan-doc path, linked to P | `dev/active/7af9ff75-plan.md` exists; `jit doc list 4dac8c81` → `dev/active/7af9ff75-plan.md [HEAD] <design>` | PASS |
| 4 | P is `done` with `plan-review` passed | `jit issue show 4dac8c81` → state `done`; required gate `plan-review` status `passed`; `plan-review` `gate_passed` event; INV-GATE-SEMANTICS | PASS |
| 5 | Impl children are non-breakable leaves covering all four `[hard]` via `satisfies` | 4 children all `type:task` (non-breakable); `satisfies:` union = REQ-01,02,03,04 = full item set; `coverage-preview` passed exit 0 | PASS |
| 6 | B's `coverage-preview` and `breakdown-review` passed and B is `done` | `jit issue show 06de7133` → state `done`; `coverage-preview` passed (exit 0, `.jit/gate-runs/`), `breakdown-review` status `passed` (attested; `gate_passed` event) | PASS |
| 7 | No implementation code written (negative — both halves) | **Repo-state:** working tree holds only seed `src/__init__.py` (28 bytes) and `tests/__init__.py`; no `src/retry.py` or other feature module. **Run-record command log:** only auto gate-runs are 2× `coverage-preview`; no `pytest`/`cargo`/`npm` in any run; `tests` gate never executed (children `backlog`/`ready`); report's Command/Gate-Invocation Log affirms no build/test runner invoked | PASS |
| 8 | `jit validate` passes on the final repo | `jit validate` → `✓ Repository validation passed` | PASS |

Also observed: container `7af9ff75` remains `backlog` with `repo-validate` pending — correct
for planning-only.

**Verdict: PASS** (8/8).

---

## `plan-from-import` — PASS (2026-07-03)

- **Run:** external note `notes/ttl-cache-design.md` imported into container `b3d778c0`
  (`type:epic`, `epic:ttl-cache`, 6 `[hard]` criteria REQ-01..REQ-06); plan node `f48d19ff`;
  breakdown node `a6785c53`; impl children `ad923f75` (satisfies REQ-01,02,06), `9a40fcdb`
  (REQ-03), `7226b03e` (REQ-04), `ce8f4de6` (REQ-05).
- **Run report:** [`transcripts/plan-from-import.completion-report.md`](transcripts/plan-from-import.completion-report.md).

`expected_output`: *"The external design note notes/ttl-cache-design.md is reconciled into a
container issue (type:epic) whose `## Success Criteria` section holds `[hard] REQ-NN` criteria
derived from the note's stated requirements (TTL expiry-as-miss, max-size LRU eviction,
get_or_compute, hit/miss statistics). The plan bracket is scaffolded on the container via
`jit apply plan` (planning node P and breakdown node B on the C -> B -> P spine). A plan
document exists at P's plan-doc path (dev/active/<container-short-id>-plan.md), is linked to P,
and P is done with its plan-review gate passed; the plan (or a linked planning artifact)
references the imported note as its external knowledge source. The breakdown created impl
children that are non-breakable leaves and together carry a `satisfies:REQ-NN` label for every
`[hard]` criterion of the container; B's coverage-preview and breakdown-review gates passed and
B is done. No implementation code for the cache was written (planning and breakdown only). `jit
validate` passes on the final repo."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Note reconciled into a container (`type:epic`) whose `## Success Criteria` holds `[hard] REQ-NN` derived from the note (expiry-as-miss, LRU eviction, get_or_compute, hit/miss stats) | `b3d778c0` is `type:epic`; its `## Success Criteria` holds 6 `[hard] REQ-01..06` lines covering all four named note requirements (REQ-02 expiry-as-miss, REQ-03 max-size LRU, REQ-04 `get_or_compute`, REQ-05 hit/miss counts) plus REQ-01 store/retrieve and REQ-06 per-entry TTL; note linked as a document. Format caveat: written as `- [ ] [hard] REQ-NN …` (checkbox), non-canonical — see finding | PASS |
| 2 | Plan bracket scaffolded via `jit apply plan` (P + B on `C → B → P`) | `f48d19ff` `type:planning`, `a6785c53` `type:breakdown` (`brackets:b3d778c0`); spine present; `jit validate` acyclic | PASS |
| 3 | Plan doc at P's plan-doc path, linked to P; P `done` with `plan-review` passed | `dev/active/b3d778c0-plan.md` exists; `jit doc list f48d19ff` → `dev/active/b3d778c0-plan.md [HEAD] <design>`; `jit issue show f48d19ff` → state `done`; `plan-review` status `passed` + `gate_passed` event; INV-GATE-SEMANTICS | PASS |
| 4 | The plan (or a linked artifact) references the imported note as external knowledge source | `dev/active/b3d778c0-plan.md` cites `notes/ttl-cache-design.md` in its header and §1 (whole-second granularity, per-entry-overrides-default, single-threaded scope), plus a linked grounding study `dev/studies/ttl-cache-investigation.md` | PASS |
| 5 | Impl children are non-breakable leaves covering every `[hard]` via `satisfies` | 4 children all `type:story`; `story` is in no `.jit/templates.toml` template's `applies_to` (only `epic` is breakable) → frontier empty; `satisfies:` union = REQ-01..06 = full criterion set (verified by direct label inspection, independent of the parser) | PASS |
| 6 | B's `coverage-preview` and `breakdown-review` passed and B is `done` | `jit issue show a6785c53` → state `done`; `coverage-preview` status `passed` (`.jit/gate-runs/…/result.json` exit 0, `auto:executor`), `breakdown-review` status `passed` (attested; `gate_passed` event). **Caveat:** the `coverage-preview` pass is **vacuous** — the checkbox criteria format hides all 6 REQs from the `label-coverage` rule (proven on a throwaway copy: stripping a `satisfies:` label still validates clean; reformatting to `- [hard] REQ-NN` then fails naming the uncovered REQ). Real coverage holds via item 5. See finding | PASS |
| 7 | No implementation code written (negative — both halves) | **Repo-state:** working tree holds only seed `src/__init__.py` (28 bytes) and empty `tests/__init__.py`; no cache module. **Run-record command log:** the only auto gate-run is 1× `coverage-preview` (`jit validate --scope`); no `pytest`/`cargo`/`npm` in any run; the `tests` gate never executed (children `backlog`/`ready`); report's Command/Gate-Invocation Log affirms no build/test runner invoked | PASS |
| 8 | `jit validate` passes on the final repo | `jit validate` → `✓ Repository validation passed` | PASS |

Also observed: exactly one epic exists (`b3d778c0`), still `backlog` with `repo-validate`
pending — correct for planning-only. The run report's claim that coverage-preview credited
"all 6 REQ" is a self-report inaccuracy (it credited none); graded against repo-state per the
method, the gate passed vacuously, which is why item 6 carries the caveat and the finding is
recorded above.

**Verdict: PASS** (8/8), with the recorded finding on the vacuous `coverage-preview` gate.

---

## Reproducing / completing a verdict

1. Build the scenario's repo: `bash setup-test-repo.sh /tmp/jit-evals/<scenario> <setup_scenario>`.
   (The setup script reads the default ruleset from a source JIT repo via `$JIT_SRC`, default
   `/home/vkaskivuo/Projects/just-in-time`; override `$JIT_SRC`/`$JIT` for another checkout.)
2. Run the scenario `prompt` (from `evals.json`) with an equivalent fresh-context runner,
   substituting the scratch repo path for `{REPO_PATH}` and, for `plan-from-existing`, the
   epic ID the setup script printed for `{EPIC_ID}`. The runner must read the production skill
   from `~/.claude/skills/jit-planning-lead/SKILL.md` and leave a completion report (with a
   command / gate-invocation log, since every scenario has a negative item).
3. Decompose the scenario's `expected_output` into a checklist (as above) and re-score each
   item against the final repo state: `jit issue list`/`show --json`, `jit item list --json`,
   `jit gate check`, `jit validate`, `.jit/events.jsonl`, `.jit/gate-runs/`, and the working
   tree. Record the verdict here with its run date, run-record location, and the inlined
   checklist.

See [`docs/reference/skill-eval-adjudication.md`](../../../../docs/reference/skill-eval-adjudication.md)
for the full method.
