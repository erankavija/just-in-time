## Final plan: Zoom-aware, hierarchy-aware DAG exploration with milestone columns (React Flow)

### Guiding invariants
1. **Milestones define the global X-axis** (chronological columns). Layout width is proportional to *visible milestones*, not graph complexity.
2. **Dependencies flow with time**: for every edge `u → v`, enforce `milestoneIndex(u) ≤ milestoneIndex(v)` and visually `x(u) < x(v)` (or flagged as invalid).
3. **Hierarchy-first**: Milestone → Epic → Story → Task is the primary structure. Leaf-level edges are not rendered by default unless relevant.
4. **Height is budgeted**: never “stack everything”; collapse progressively with deterministic rules.
5. **Stability**: ordering is derived from (a) local dependency topological order and (b) stable issue IDs as tie-breakers, so small changes don’t reshuffle the whole view.

---

## 1) Data & view-model foundations

### A. Normalize time ownership
Assign every node a `milestoneIndex`:
- Milestone: own index
- Epic/Story/Task: inherit the milestone they belong to
- If any node can be “unassigned”, give it a special “pre-milestone” bucket immediately left of its earliest dependent milestone (only shown on demand).

### B. Build three graphs (explicit in code)
1. **Base DAG**: all nodes/edges.
2. **Exploration DAG (what React Flow renders)**: subset + *virtual nodes*:
   - `CollapsedMilestonesBucket(start,end)`
   - `CollapsedEpic(epicId, countStories, countTasks)`
   - `CollapsedStory(storyId, countTasks)`
   - `MoreNode(scope, count)` e.g. “+17 stories”, “+12 tasks”
3. **Layout DAG**: Exploration DAG + computed geometry + routing hints.

### C. Validate time monotonicity
On ingest/build:
- if any edge violates `milestoneIndex(u) ≤ milestoneIndex(v)`, mark it and optionally exclude from layout constraints; render as “back-in-time dependency” warning.

---

## 2) Default depth control: primary tier windowing

### A. Visible primary tier window
Default view shows:
- the last **N primary tier items** expanded (start with N=8–12; tune with telemetry)
- older primary tier items collapsed into 1–K buckets (e.g. by quarter or fixed-size ranges)

Each collapsed bucket is a single node with:
- counts (primary tier items, secondary tier items, tactical items by type)
- bundled inbound/outbound edge counts to adjacent visible columns

### B. Expansion rules
- Focusing/searching an item auto-expands the bucket containing it.
- Expanding history is incremental: expand one bucket at a time, keep others collapsed.

This bounds width even with arbitrarily long project history.

### C. Edge case: minimal hierarchy
If the configured hierarchy has only 1 strategic type (e.g., `minimal` template: milestone → task), there is no secondary tier. In this case, tactical items collapse directly under primary tier items without an intermediate grouping layer.

---

## 3) Layout engine (replacing Dagre): anchored columns + packed lanes

### A. X placement (deterministic)
- `x = milestoneIndex * MILESTONE_GAP + localBandOffset`
- Within a milestone column, use small **micro-bands** (fixed offsets) rather than new global ranks:
  - Band 0: (optional) shared prerequisites row
  - Band 1: structure nodes (epic headers, story nodes)
  - Band 2: expanded leaf tasks (when a story is expanded)
This preserves L→R semantics and avoids width explosion.

### B. Y placement: hierarchical packing with budgets
Pack top-to-bottom in each milestone column:

1. Milestone header node
2. Epic rows (each epic is a “row container”)
3. Optional shared/unassigned rows
4. “+N more” summary nodes where needed

Within an epic row:
- show story nodes (collapsed or expanded)
- expanded stories show some tasks (budgeted)

All packing is deterministic; no global crossing-minimization pass required.

### C. Derived ordering (deps + stable IDs)
You said ordering should be derived from dependencies and issue IDs:
- For **epics within a milestone**: compute a local topological order using epic→epic edges (if any). Tie-break by stable epic issue ID.
- For **stories within an epic**: local topological order using story→story edges if present; else stable ID.
- For **tasks within a story**: local topological order using task→task edges *within that story* (common case); tie-break by stable ID.

If cross-container edges exist (task in Story A → task in Story B), don’t fully reorder everything; handle them primarily at aggregated levels (below), and only expose leaf edges when relevant.

---

## 4) Height control: progressive disclosure by hierarchy (critical)

### A. Budgets (suggested starting values)
These are per tier and per scope; tune later.

- **Tier 1 (default)**
  - per milestone: show all epic headers, but allow only **E_expanded = 1–2** expanded epics at once
  - per expanded epic: show **S_visible = up to 8–12** story nodes; rest collapsed into “+N stories”
  - per story: tasks collapsed (show 0–2 key tasks as badges only, not nodes)
- **Tier 2 (detail)**
  - per expanded story: show **T_visible = 8–15** task nodes; rest “+N tasks”
  - global cap on visible task nodes across canvas (e.g. 80–150)

### B. Relevance scoring (what gets shown first)
When a container is partially expanded, choose children in this order:
1. on selected/highlighted path (blocker chain / explain view)
2. blocked / blocking items
3. high-degree items (shared deps across stories/epics)
4. then stable ID order

This keeps the graph short and informative.

### C. Where “everything needed” shows up
For a focused milestone, you do not expand all upstream tasks. You:
- expand only the *containers on the relevant upstream paths* (epics/stories), up to budgets
- represent the rest as aggregated nodes/edges with counts (“+23 upstream tasks not shown”)

So the invariant (“needed is left”) stays true without forcing vertical explosion.

---

## 5) Shared dependencies across stories/epics: de-duplicate nodes

Tasks can be prerequisites for multiple stories/epics. To prevent duplication and height multiplication:

### Rule: Single visual home for each leaf
Within a milestone column, each Task node is rendered in exactly one place:
1. under its parent Story if it has one
2. else under an “Unstructured tasks” story/container inside its Epic
3. else under milestone-level “Shared prerequisites” row (if truly cross-epic)

Other references are represented by:
- aggregated edges to the owning Story/Epic
- optionally small “linked prereq” badges (count + click to reveal)

This is the main height safeguard for shared dependencies.

---

## 6) Edge strategy: aggregate by default, explode on demand

### A. Render at the “right level” for the current zoom
- Tier 0: milestone→milestone bundled edges with counts
- Tier 1: epic→epic and story→story edges (bundled); task edges only for selection/highlight
- Tier 2: task edges within expanded stories; cross-story task edges shown only if both endpoints are visible or the user expands the edge bundle

### B. Bundle keys
Use deterministic bundle IDs so edges are stable:
- `milestoneBundle(fromMilestone, toMilestone, edgeType)`
- `epicBundle(fromEpic, toEpic, edgeType)`
- `storyBundle(fromStory, toStory, edgeType)`

Edge bundles display:
- count
- optionally “top blockers” (derived list) on hover/side panel

### C. “Explain why blocked” action (path-based reveal)
When user asks “why”:
- compute 1–k paths (shortest / most relevant) from blocker sources to target
- auto-expand only the milestones/epics/stories on those paths
- show leaf task edges only along the highlighted path(s)

This is the core “exploration” loop that replaces full rendering.

---

## 7) Zoom tiers (React Flow) and what recomputes

Define tiers based on `viewport.zoom`:

- **Tier 0** (overview): milestones only (+ optional epic badges), bundled milestone edges
- **Tier 1** (default): milestones + epic headers + limited story nodes; aggregated edges
- **Tier 2** (detail): expanded stories show task nodes and task edges locally

Recompute rules:
- **Never move milestone columns** across tier changes.
- Repack Y within affected columns only.
- Keep deterministic ordering so expansions feel like “unfolding”, not “teleporting”.

---

## 8) Implementation roadmap (concrete steps)

### Phase 1 — Exploration DAG + milestone windowing (immediate relief)
- Add/derive `milestoneIndex` everywhere.
- Build exploration graph with:
  - last N milestones visible, older collapsed
  - epic headers visible in visible milestones
  - stories collapsed into “+N stories” per epic
- Add focus/search auto-expansion.

### Phase 2 — Replace Dagre: anchored X + hierarchical Y packing
- Implement layout function:
  - x from milestoneIndex + micro-bands
  - y packing: milestone header → epic rows → story nodes → optional expanded tasks
- Add invariant tests: all rendered edges go left→right.

### Phase 3 — Edge bundling + tiers
- Implement custom edge renderer for bundles with counts.
- Implement Tier switching and “expand bundle” interaction.
- Default to story-level edges; show task edges only when story expanded/selected.

### Phase 4 — Incremental + performance
- Layout in a Web Worker.
- Cache layout per `(visibleMilestones, expandedEpics, expandedStories, tier, focusSelection)`.
- Incremental repack: expanding a story only repacks its epic row and its milestone column.

### Phase 5 — UX polish for high-density cases
- Side panel for “show all tasks in story/epic” instead of putting 50 nodes on canvas.
- Mini-map and “jump to milestone”.
- Telemetry to tune N milestones window and budgets.

---

## 9) Acceptance criteria (what “done” means)
1. Default view stays readable with long histories: width bounded by visible milestone window; old milestones collapsed.
2. Height remains bounded: epics always visible, but stories/tasks fold by default with budgets.
3. L→R semantics always hold: prerequisites appear left of dependents; no Dagre-induced inversions.
4. Stability: expanding/collapsing unfolds deterministically; issue-ID tie-breakers prevent jitter.
5. “Why/what’s needed” flows are fast: path-based reveal shows the minimal subgraph required to answer the question.

---

If you want, I can also provide a recommended set of exact default thresholds (N milestones, per-epic story cap, per-story task cap, zoom tier boundaries) based on a “20 deps max” assumption and typical React Flow performance envelopes.
