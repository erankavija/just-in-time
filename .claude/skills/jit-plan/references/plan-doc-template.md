# Plan Document Template

The structure the synthesizer fills and `plan-review` judges. The four top-level sections
**are** the rubric's four areas. Replace every bracket; delete guidance comments. The plan
doc lives at P's plan-doc location and is linked to P (Phase 7).

The criterion markers (`[hard]`), id-pattern (`REQ-NN`), and satisfies-namespace
(`satisfies:`) shown here are the **default ruleset's** — use whatever the live config
declares.

---

```markdown
# Plan: <Container Title> (<C-short-id>)

> Planning node: <P-short-id>. Container criteria source: <C-short-id> `## Success Criteria`.

## 1. Completeness vs criteria

How the plan addresses **every** `[hard]` criterion of <C-short-id> — the *approach* per
criterion, in prose. One row per criterion; no silent narrowing. Flag (do not invent) any
gap discovered. **Do not restate the criterion→item mapping here** — that lives once, in
the §3 coverage map (the single source). This table is the narrative; that table is the
contract. (They drift if both list items; keep them disjoint.)

| Criterion | Approach (how it is met) | Notes / open gap |
|---|---|---|
| REQ-01 … | <prose: the design that satisfies it> | |
| REQ-02 … | | |

## 2. Technical soundness and architectural fit

The approach, grounded in the current system. Cite real `file:path:line`. State which
existing primitives are reused and which layer boundaries are respected. For every asserted
property (atomic / validated-first / reuses X), cite the verification that it holds.

- **Approach:** …
- **Reuses / integrates with:** `path:line` …
- **Grounding (from investigation):** claim → already-done / valid-and-open /
  invalid-as-stated, each with `path:line`.

## 3. Decomposition sketch (near-ready; jit-breakdown instantiates — no issues created here)

Intermediate groupings sized to the work; each group independently landable (green at every
boundary). Express ordering through the `depends-on` field — never as prose ordinals.

### Group A: <name>  — covers REQ-01, REQ-04
- **<item title>**  `type: <story|task>`  `satisfies: REQ-01, REQ-10`  `depends-on: —`
  Outcome: <one observable outcome>.
  Own criteria: `[hard] <local-req>: …`
  Blast radius: <consumers touched, updated in the same change> (or "self-contained").
- **<item title>**  `type: task`  `satisfies: REQ-04`  `depends-on: <prior item>`
  Outcome: …

### Group B: <name>  — covers REQ-05, REQ-06, REQ-07
- …

**Coverage map** (the single source for criterion→item; every `[hard]` criterion → ≥1 item).
§1 must not duplicate this:

| Criterion | Satisfied by (item) |
|---|---|
| REQ-01 | <item> |
| … | |

> For any "remove/rename X" criterion, name the repo-wide acceptance check here (e.g. a
> tree-wide grep including example/fixture dirs) and confirm no wave removes X before its
> consumers migrate.

## 4. Risks and actionability

| Risk / open question | Severity | Mitigation or decision |
|---|---|---|
| … | | |

Every load-bearing question resolved or owned. Each sketch item is executable without
re-deriving the design.

## Decisions

First-class log (consumed by review and breakdown). Decisions are **provisional** — mark
any whose premise later evidence undermines as **REOPEN**.

- **D1 — <decision>:** chosen **<option>**. Rejected: <option> (<reason>), <option> (<reason>).
- **D2 — <decision>:** … 
- **Assumptions** (where intent stayed underspecified): <assumption> — rationale; risk if wrong.
```

---

## Authoring notes

- **Criteria shape** (container `## Success Criteria`, set during the interview, not here):
  one observable outcome per line, stable id, explicit marker —
  `- [hard] REQ-01: <single verifiable outcome>`. Never bare, never mixed markers.
- **Titles** carry no ordinals (`T1`, `S0:`), no `type:`/`feat(...)` prefixes, no parent
  IDs. Position lives in the DAG and labels (`jit-manage/references/content-standards.md`).
- **Descriptions state purpose only** — never how the tracker advances a node, the gate
  command, or the reviewer's identity.
- **Relationships live in the graph**, not in prose. `depends-on` in the sketch becomes a
  real edge at breakdown; do not also narrate prerequisites in sentences.
