# Epic Complete: Graph templates: configurable parameterized subgraphs applied to the work DAG (9ac9fdac)

**Started:** 2026-06-21
**Completed:** 2026-06-24
**Assignee:** agent:project-lead

### Summary

Generalized the hardcoded planning bracket into a configurable graph-template layer: named, parameterized subgraphs declared in `.jit/templates.toml` and spliced into the work DAG by a generic `jit apply <template> <container>` command. The planning bracket is now the first template (`plan`); `jit apply plan` creates both `P` and `B` from config, breakdown consumes the pre-created `B`, and the flat `[planning]` config / `PlanningConfig` path is gone.

### Metrics

| Metric | Value |
|---|---|
| Children completed | 19 / 19 (6 stories + 13 leaf tasks; all done) |
| Waves executed | 10 |
| Rework cycles | ~25 (heavily weighted to W5/W6 docs-consistency and the dogfood-surfaced jit gaps) |
| Escalations | 5 (2 session-1, 3 session-2) |
| Sub-agent dispatches | ~12 (3 session-2 workers + their reworks, plus session-1) |
| Issues created during execution | 0 (epic-scoped) |

### Success Criteria

- [x] REQ-01: Graph templates declared in `.jit/templates.toml` (role/type/gates/doc/description, role-based `depends_on`, named anchors; nothing hardcoded in engine logic) — `04559558` (`45e7f203`, `8b15b679`)
- [x] REQ-02: `jit apply <template> <anchors>` instantiates in one operation, result validated acyclic — `643874f2` (`14137e1a`, `73e5e853`), `3143ad87` (`9cccfe0c`, `e75805f6`)
- [x] REQ-03: `description`/`doc` interpolate from anchor context; `plan` produces a non-empty planning description — `14137e1a`; verified by the dogfood (`fc414353`) and reseed (`57f52018`)
- [x] REQ-04: `plan` template reproduces the bracket — `P` (plan-review) + `B` (coverage-preview + breakdown-review), `B→P`, `C→B` — `04559558`, `643874f2`
- [x] REQ-05: multiple named anchors bound at apply; `plan` uses `container` — `04559558`, `643874f2`
- [x] REQ-06: small, fixed, DAG-preserving transform vocabulary with extensible dispatch; ships `move-upstream-to-role` — `73e5e853`
- [x] REQ-07: breakdown engine + jit-breakdown skill consume the pre-created `B`; B-creation removed from skill prose — `3143ad87` (`e75805f6`), story `adc640e4` (`6e38a577`, `c8aa199e`)
- [x] REQ-08: engine exercised by extensive tests against isolated temp repositories, and dogfooded on a real container — story `99a0c161` (`840e155c`, `fc414353`)
- [x] REQ-09: existing empty planning-node descriptions (`1eb0bdfd`, `2937919f`) reseeded from their containers — `57f52018`
- [x] REQ-10: this repo's config rewritten to the templates.toml model; flat `[planning]` / `PlanningConfig` removed — story `96fc8cd8` (`c703a4b3`, `3a7e13cf`)

### Wave Execution Log

- **W1–W2:** template registry + `templates.toml` loader; authored the `plan` template (`04559558`).
- **W3–W4:** `apply_template` core (validate → snapshot → instantiate) + `move-upstream-to-role` transform (`643874f2`).
- **W5–W6:** `jit apply` CLI; atomic scaffold switch — `jit apply plan` becomes the sole scaffold, breakdown consumes the pre-created `B`; removed `jit plan` / `--with-planning` (`3143ad87`).
- **W7–W8:** migrated scoped validation + plan-content to the template registry; removed `PlanningConfig` and every flat `[planning]` block (`96fc8cd8`).
- **W9 (session 2):** skills + memory migration (`6e38a577`), engine tests (`840e155c`), reseed (`57f52018`).
- **W10 (session 2):** user docs + bracket-id reconciliation (`c8aa199e`), dogfood on a real container (`fc414353`).

### Key Decisions

- **Reseed matches the template literal, not the polished example.** The empty planning-node descriptions were reseeded to the exact `plan`-template interpolation (not the richer hand-written variant on `7d88e37d`), since RESEED-01 wants what the template produces.
- **Reseed and dogfood done by the lead, not sub-agents.** Both mutate the production `.jit` store; doing them directly avoided the known sub-agent MCP-contamination risk.
- **`brackets:` convention reconciled to the short id everywhere** (template, tests, fixtures, scripts, docs, gate-preset comments), keeping one deliberate full-id backward-compat test (`test_container_indirection_full_id_label_still_resolves`).

### Escalations

- **Session 1:** repo-wide `jit-validate` blocker (16 epics missing plan docs) → user approved stub-plan-doc backfill. Apply-engine cycle-atomicity gap → user chose the edge-simulation pre-check.
- **Dogfood target (session 2):** `25064508` had a legacy P-only bracket; user chose to clean-migrate it (remove the legacy `1eb0bdfd`, then `jit apply plan`).
- **Dogfood findings (session 2):** the clean-migrate exposed jit gaps (deletion + dep-removal not event-logged; apply not detecting a pre-existing planning node); user chose to fix the gaps in-epic rather than defer.
- **Codex quota (session 2):** the code-review reviewer hit its usage limit mid-completion; user chose to retry until it cleared.

### Issues Discovered During Execution

No epic-scoped issues were created. The dogfood surfaced three pre-existing jit gaps, all fixed within the epic (see `fix(jit:99a0c161)`):
- `jit issue delete` logged no event (no `IssueDeleted` type existed).
- `jit dep add`/`rm` logged no `issue_updated` for the dependency edit; no-op removals also bumped `updated_at` without an event.
- `jit apply plan` detected only an existing breakdown node, not a pre-existing planning node, so it could duplicate the planning node on a legacy P-only container.

### Holistic Quality Notes

- The graph-template engine is the deliverable; the planning bracket is now one configuration of it, leaving the door open to other templates without further engine work.
- Story-level code-review earned its keep: the `99a0c161` review caught the event-logging gaps that none of the per-task reviews saw, because only the aggregate dogfood exercised delete/dep-rm on the production store.
- An unrelated backlog item (`f2532a2d`, "milestone-level vision steward" skill) and two `dev/sessions/` notes appeared during the session from outside this epic; they were preserved (not deleted) and left for the user to triage.
