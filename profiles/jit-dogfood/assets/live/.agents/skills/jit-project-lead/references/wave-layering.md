# Wave Layering

Group a strategic-tier container's direct sub-strategic children into
dependency-ordered execution waves. The output drives the containers in order:
Wave 1 first, then Wave 2, and so on. This is the one-tier-up analogue of
`jit-execution-lead` Section 4 (Wave Planning), which layers an epic's
issue-level children; here the layered nodes are sub-strategic containers, read
against the strategic-tier parent.

This reference computes the wave list only. It does not dispatch, review, or
transition anything.

## Inputs

- `<container-id>` — the strategic-tier container (the steward anchor).
- `D` — the delegation-boundary types (the sub-strategic container types the
  steward dispatches a lead for), from tier derivation (the union of
  `applies_to` across `.jit/templates.toml` `[[template]]` entries). Never
  hardcode these; take them from the derived boundary.
- The container's **membership label** — the label whose namespace is
  `[type_hierarchy.label_associations][<container-type>]`, read from the
  container's own labels (`jit issue show <container-id> --json`). Every direct
  child of the container carries this same label. This is the discriminator that
  separates genuine children from out-of-container nodes (mirrors
  `jit-execution-lead` Section 1 step 5).
- `jit graph deps <container-id> --depth 0 --json` — the container's full
  transitive dependency subgraph. Shape:

  ```json
  {
    "depth": 1,
    "issue_id": "<short-id>",
    "message": "N dependencies",
    "summary": { "by_state": { "backlog": N }, "total": N },
    "tree": [
      {
        "id": "<full-id>", "short_id": "<short-id>", "title": "...",
        "state": "backlog", "level": 1, "priority": "normal",
        "children": [ /* the issues THIS node depends on, same shape */ ]
      }
    ]
  }
  ```

  A node's `children` are the issues that node depends on (upstream). The
  `tree` nodes carry no `labels` field; confirm type and membership per node
  with `jit issue show <id> --json`.

Use `--depth 0`, not the default depth 1. The container's depth-1 direct
dependencies are transitively reduced to the graph's **sinks**: when a sibling
depends on another sibling, the container-to-that-sibling edge is dropped as
redundant, so depth 1 omits it. Depth 0 returns the whole subgraph, so every
child is reachable. (Observed: a 30-node container surfaces only 4 direct deps
at depth 1.)

## Output

An ordered wave list. Each wave is a set of sub-strategic container ids; the
waves run in order. Every direct sub-strategic child appears in exactly one
wave. Shape (one tier up from `jit-execution-lead`'s `progress.json` `waves`,
containers in place of issues):

```json
{
  "container_id": "<full-id>",
  "container_short_id": "<short-id>",
  "waves": [
    {
      "wave_number": 1,
      "containers": [
        { "id": "<full-id>", "short_id": "<short-id>", "title": "..." }
      ]
    }
  ]
}
```

## Procedure

1. **Roster the sibling set `S`.** From `jit graph deps <container-id> --depth 0
   --json`, collect the distinct nodes in `tree`. Keep a node iff its type is in
   `D` **and** it carries the container's membership label. `S` is the direct
   sub-strategic children. The kept set equals `jit query --label
   <membership-label> --json` filtered to `D`; cross-check against it.

2. **Compute each child's sibling-dependencies.** For each `Ci` in `S`, run
   `jit graph deps <Ci-id> --depth 0 --json` and intersect its `tree` node set
   with `S`:

   `sibling-deps(Ci) = { Cj in S : Cj appears in Ci's dependency subtree }`.

   Dependencies of `Ci` that are not in `S` — out-of-container upstreams,
   `Ci`'s own internal issues, bracket-infrastructure nodes — do not appear in
   the intersection and so never affect layering.

3. **Layer by dependency depth.**
   - **Wave 1:** every `Ci` in `S` with `sibling-deps(Ci)` empty.
   - **Wave N:** every remaining `Ci` whose `sibling-deps(Ci)` all sit in waves
     `1..N-1`.
   - Repeat until `S` is exhausted. Assign each child to exactly one wave.

4. **Emit** the ordered wave list in the Output shape.

## Exclusions

Three node kinds are reachable in the container's subgraph but are **not**
waved. Each fails the step-1 filter and is dropped:

- **Internal issues of a sub-strategic container** — the stories/tasks nested
  inside each child. Their type is below `D`, or they carry a child's membership
  label rather than the strategic container's. They are the dispatched lead's
  concern, not the steward's.
- **Out-of-container nodes** — a child depending on a non-sibling issue owned by
  a different strategic container. It lacks this container's membership label,
  so it is excluded from `S`, and the edge to it is ignored (step 2). Its
  ordering is settled where it lives, one tier up, when the strategic containers
  are themselves scheduled.
- **Bracket-infrastructure nodes** — if the ruleset brackets the strategic
  container, its planning/breakdown children (types read from the applied
  `.jit/templates.toml` template's node roles, never hardcoded). They are
  scaffolding, already gated/done before the containers run, and are excluded
  exactly as `jit-execution-lead` Section 4 step 1 excludes `P` and `B`. In the
  observed rulesets the strategic tier is unbracketed, so these appear one tier
  down, inside each child, and never enter `S` anyway.

## Invariant relied on

**Acyclic dependency graph** — the graph stays acyclic, so the sub-DAG induced on `S`
is acyclic. The layering therefore terminates and
places every child. If a pass ever leaves a non-empty remainder whose
sibling-dependencies never all resolve into earlier waves, that would require a
cycle, which cannot exist. Do not emit a partial wave list — stop and escalate
(the data is corrupt).

## Red flags

- Reading children at the default depth 1. Reduction hides siblings that other
  siblings depend on. Always `--depth 0`.
- Treating an out-of-container upstream as a sibling. It has no membership
  label; it belongs to another container's schedule.
- Waving a child's internal issues or the bracket's planning/breakdown nodes.
  Only delegation-boundary-typed members of this container are waved.
- Hardcoding a type name. Take `D` and the bracket node types from config and
  templates.
- Emitting a wave list that omits a child or lists one twice. Coverage is
  exactly-once; a gap or a duplicate means the roster or the layering is wrong.

## Worked example

Strategic-tier container `M`. Direct sub-strategic children roster
`S = {C1, C2, C3, C4, C5}`. One out-of-container node `X0` (a child of a
different strategic container) is reachable but not in `S`.

Sibling-dependency edges (from step 2, `Ci` depends on `Cj`):

| Child | Raw dependencies                 | sibling-deps (∩ S) |
|-------|----------------------------------|--------------------|
| C1    | —                                | —                  |
| C2    | —                                | —                  |
| C3    | C1                               | {C1}               |
| C4    | C1, C2                           | {C1, C2}           |
| C5    | C3, X0                           | {C3}               |

`M`'s own depth-1 deps are only `{C4, C5}` — the sinks; reduction dropped
`M -> C1`, `M -> C2`, `M -> C3` because `C3`/`C4`/`C5` already reach them.
`--depth 0` recovers `C1, C2, C3` through `C3`/`C4`/`C5`. `C5 -> X0` is dropped
in step 2 because `X0` is not in `S`.

Layering:

- **Wave 1:** `C1`, `C2` — empty sibling-deps.
- **Wave 2:** `C3` (dep `C1` in Wave 1), `C4` (deps `C1, C2` in Wave 1).
- **Wave 3:** `C5` (dep `C3` in Wave 2; `X0` ignored).

Output:

```json
{
  "container_id": "M",
  "container_short_id": "M",
  "waves": [
    { "wave_number": 1, "containers": [ {"short_id": "C1"}, {"short_id": "C2"} ] },
    { "wave_number": 2, "containers": [ {"short_id": "C3"}, {"short_id": "C4"} ] },
    { "wave_number": 3, "containers": [ {"short_id": "C5"} ] }
  ]
}
```

Every child of `M` appears once; `X0` appears nowhere.

## Consumers

The ordered wave list is the input to sub-strategic dispatch: each wave's open
containers are planned by `jit-planning-lead` and, only after their brackets are
fully complete, executed by fresh `jit-execution-lead` agents. A wave completes
before the next begins (wave discipline). Persist the wave list into the
container-level progress file so a resumed session reads back the plan. Dispatch
and the cross-container coherence review are defined separately; this reference
supplies only the wave list they consume.
