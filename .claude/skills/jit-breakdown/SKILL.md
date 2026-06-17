---
name: jit-breakdown
description: Break down a JIT issue into child work items based on its specification document. Reads the spec doc linked to the issue (or user-specified), uses an analysis sub-agent to decompose it into child issues with proper dependency edges, presents a plan for approval, then creates the issues and wires up the dependency graph. Use when asked to "break down", "decompose", "detail out", or "create subtasks for" a JIT issue.
compatibility: Requires JIT CLI on PATH. JIT MCP tools used where available.
---

# JIT Issue Breakdown

Read the specification document attached to a parent issue, decompose it into
child issues with a correct dependency DAG, and populate JIT.

**Two breakdown shapes, selected by the ruleset (Step 1.5):**

- **Bracket breakdown** — when the parent is a *breakable container* declared in
  the repo's `[planning]` config. The plan is a first-class, gated node `P`
  sequenced *before* the fan-out, and breakdown splices a **source/sink spine**
  `C → impl → B → P` (not parent-centric containment). This is the
  plan-before-fan-out flow. See [The bracket](#the-bracket) below.
- **Plain breakdown** — when the parent is NOT a breakable container (or the repo
  declares no `[planning]` config). The classic parent-centric flow: create
  children, then make the parent depend on all of them (Step 6d-plain).

Everything in Steps 2-5 (read hierarchy, membership label, analysis, plan review)
is shared. The two shapes differ only in **Step 6 execution wiring**.

---

## The bracket

For a breakable container `C` whose plan has been approved, breakdown produces:

```
C ──dep→ {impl subgraph} ──dep→ B ──dep→ P
```

precedence `P > B > impl > C` (plan first, then breakdown, then work, then the
container closes). Concretely:

- **`B`** (`type:<breakdown_type>` from config) carries a `brackets:<C-id>` label
  and the **coverage-preview** gate (`coverage_gate_preset`), and depends on `P`.
- **Impl children** are drafted in **Backlog** (they depend on `B`, which is not
  done, so `jit ready` never surfaces them before the breakdown is approved).
- **Sources** (impl issues with no intra-subgraph predecessor) depend on `B`;
  internal chains carry the rest. This transitively gates ALL impl behind `B`.
- **Sinks** (impl issues with no intra-subgraph successor) are depended-on by `C`
  (`C depends on each sink`). Transitive reduction drops the scaffold's direct
  `C → P` edge and any redundant `C → non-sink` edge automatically.

The type names (`breakdown_type`, `planning_type`) and the gate preset name come
from `[planning]` config — never hardcode `breakdown`/`planning`/`epic` literals.

---

## Step 1: Pre-flight

1. Identify the parent issue:
   - If the user provided an issue ID, fetch it with `jit issue show <id>`.
   - If the user described an issue by title, search with `jit issue search <query>`
     and confirm the match before continuing.

2. Display the parent issue's title, state, type, labels, and description.

3. Check whether the parent already has children by inspecting its `depends_on`
   list (via `jit graph downstream <id>` or by noting deps in `jit issue show`).
   If children already exist, warn:
   > "This issue already has N dependency edge(s). Continuing will add more child
   > issues and update the parent's depends_on. Existing issues are untouched."
   Ask the user to confirm before continuing.

4. Locate the specification document:
   - Run `jit doc list <id>`. If one or more docs are listed, present them and
     ask which to use (default: the most recently modified).
   - If no docs are linked, ask: "No spec doc is linked to this issue. Please
     provide the path to the specification document." Accept a path, verify it
     exists. Offer to link it: `jit doc add <id> --path <path> --doc-type design`

5. Run `jit validate` on the current repository state. Surface any errors to the
   user before adding anything new.

6. **Determine the project's gate tiers.** Every implementation issue needs quality
   gates — a breakdown that ships gateless work is incomplete. Run `jit gate list` and
   `jit gate preset list`, and sample a few existing issues
   (`jit issue show <id> --json | jq .gates_required`) to learn the convention. From
   these, define a small set of named tiers for this project — at minimum a
   **primary/full** tier (the standard gates a core deliverable must pass) and a lighter
   tier for clearly supporting work (e.g. documentation). The gates are whatever the
   project configured, so this works for any domain. Present the proposed tiers and
   their gate sets and confirm with the user once. These become `[GATE_TIERS]` for the
   analysis prompt and the tier → gate mapping applied at creation (Step 6).

---

## Step 1.5: Bracket detection and plan-approval gate

Decide which breakdown shape applies, and — if bracket — confirm the plan is
approved before drafting any children.

1. **Read the `[planning]` config.** Inspect `.jit/config.toml` for a `[planning]`
   section. If absent, the repo does not use brackets → this is a **plain
   breakdown**; skip the rest of this step and use the plain wiring (Step 6d-plain).

2. **Is the parent breakable?** From `[planning].breakable_types`, check whether the
   parent's `type:*` value is listed. If NOT listed → **plain breakdown** (skip the
   rest of this step). If listed → **bracket breakdown**; continue. Read and record
   `planning_type`, `breakdown_type`, and `coverage_gate_preset` from `[planning]` —
   never hardcode these names.

3. **Require a scaffolded planning node `P`.** The container must already be
   bracketed (`C → P`) by the scaffold step (`jit plan <id>` or
   `jit issue create --with-planning`). Find `P` among the container's dependencies:
   ```bash
   jit issue show <C> --json | jq -r '.depends_on[]'
   # inspect each: the one whose type: label == <planning_type> is P
   ```
   If no planning node exists, STOP and tell the user:
   > "This breakable container has no planning node. Scaffold it first with
   > `jit plan <id>`, produce and review the plan, then re-run breakdown."

4. **Require an APPROVED plan.** Bracket breakdown consumes an *approved* plan, so
   the plan-quality gate on `P` must have passed. Check:
   ```bash
   jit issue show <P> --json | jq '{state, gates_required, gate_status}'
   ```
   The plan is approved when `P`'s plan-quality gate status is `passed` (and `P` is
   Done or Gated-passing). If the plan gate is pending or failed, STOP:
   > "The plan node <P> has not passed its plan-quality gate. Review and pass the
   > plan before fanning out (the breakdown must consume an approved plan)."
   Do not proceed to draft children until the plan is approved.

---

## Step 2: Read the configured type hierarchy

Read `.jit/config.toml` and extract the `[type_hierarchy]` section. Build a table
of type names sorted by level (ascending):

```
Level 1 (broadest): milestone
Level 2:            epic
Level 3:            story
Level 4 (finest):   task, bug
```

Determine the **parent issue's level** from its `type:*` label and the hierarchy
table.

Identify the **child types** — the type(s) at level+1. If the parent is already at
the finest level, warn: "This issue type has no child level in the configured
hierarchy. Cannot break it down further."

Also read `[type_hierarchy.label_associations]` to find the membership namespace
for the parent's type. For example, if `epic = "epic"` in label_associations, then
children of an epic carry an `epic:<name>` label.

**Never hardcode type names — always use the configured hierarchy.**

**Depth is size-driven, not fixed at one level.** This skill breaks one level at a
time (parent → level+1), but a large parent should end up multi-level (e.g.
epic → story → task), not a flat layer of leaves. The analysis agent flags any child
that is itself several deliverables with `decompose_further: true`; Step 6f recurses
on those into the next level. A small parent simply produces leaf children directly.

---

## Step 3: Determine the membership label

Children need a label that groups them with this parent.

1. Inspect the parent issue's existing labels for one matching the membership
   namespace (e.g., look for `epic:*` if parent is type `epic`).

2. If such a label already exists on the parent (e.g., `epic:gpu-acceleration`),
   use that value for all children.

3. If no such label exists:
   - Suggest a slug derived from the parent's title (lowercase, hyphens, ≤ 30 chars).
     Example: "GPU Acceleration Pipeline" → `epic:gpu-acceleration-pipeline`
   - Confirm with the user or accept their alternative.
   - Add the label to the parent: `jit issue update <id> --label <namespace>:<slug>`

Record the full label (e.g., `epic:gpu-acceleration-pipeline`) — it will be added
to every created child issue.

---

## Step 4: Analysis (sub-agent)

Dispatch a `general-purpose` sub-agent using the prompt template at
[references/analysis-prompt.md](references/analysis-prompt.md).

Fill in the template fields:

| Field | Value |
|---|---|
| `[PARENT_ISSUE_TITLE]` | Parent issue title |
| `[PARENT_ISSUE_DESCRIPTION]` | Parent issue description (or "(none provided)") |
| `[PARENT_TYPE]` | Parent's type name (e.g., `epic`) |
| `[CHILD_TYPES_TABLE]` | Child type name(s) at level+1, one per line with level |
| `[MEMBERSHIP_LABEL]` | The membership label determined in Step 3 |
| `[SPEC_DOC_PATH]` | Absolute path to the spec document |
| `[TYPE_HIERARCHY_TABLE]` | Full hierarchy table from Step 2 |
| `[GATE_TIERS]` | The gate tiers and their gate sets from Step 1.6 (one per line) |

The agent must return **only** a JSON object — see
[references/plan-schema.md](references/plan-schema.md) for the schema.

Parse the returned JSON. If parsing fails, show the raw output to the user and
ask whether to retry or abort.

---

## Step 5: Plan review

Present the proposed child issues as a list, then ask for approval.

**Render format:**

```
Breakdown plan for: "<Parent Issue Title>"  (N child issues, M sequencing edges)

  [child-type] Title of first work item (ref: A, priority: high)   (no deps — can start immediately)
  [child-type] Title of second work item (ref: B, priority: normal) ← depends on A
  [child-type] Title of third work item (ref: C, priority: normal)  ← depends on A, B

  Wiring after creation:
    - Plain breakdown:   the parent will depend on all N children (containment).
    - Bracket breakdown: a breakdown node B (coverage-preview gate) is created
      depending on the approved plan P; children are drafted in Backlog; source
      children depend on B; the container depends on the sink children. The
      direct C → P edge is dropped by reduction. (See "The bracket".)

Notes from analysis agent:
  <notes field from JSON>
```

Show all sequencing edges. Cross-dependencies (where a child depends on another
child that is not an immediate predecessor) should be explicitly called out.

Ask: **"Create these N child issues and wire up dependencies? [yes / edit / abort]"**

- **yes** — proceed to Step 6
- **edit** — print the raw JSON and ask the user to paste a corrected version;
  re-render the list and ask again
- **abort** — stop; nothing has been written to JIT

---

## Step 6: Execution

### 6a. Topological sort

Sort child issues so that every issue's dependencies appear before it in the
creation order. Use Kahn's algorithm on the `depends_on` graph within the child
issue set. This ensures all dep-target UUIDs exist before they are referenced.

### 6b. Create child issues

For each child issue in topological order:

```bash
jit issue create \
  --title "<title>" \
  --description "<description>" \
  --label "type:<child-type>" \
  --label "<membership-label>" \
  --priority "<priority>" \
  --gate "<g1>" --gate "<g2>"          # the gates this issue's gate_tier maps to (Step 1.6)
```

Apply the quality gates from the issue's `gate_tier`. Every implementation issue gets
its tier's gates — skipping them leaves work that can be closed with no quality check.
Use `--gate` at creation (above), or `jit gate add <uuid> <gates>` / `jit gate preset
apply <preset> <uuid>` afterward. **This per-task gate assignment applies to BOTH
breakdown shapes** — bracket children still carry their tier's quality gates; the
coverage-preview gate added in 6d-bracket lives on `B`, not on the children.

Capture the returned UUID and store in an in-memory map: `ref → UUID`.

> Bracket note: in a bracket breakdown the children are **drafts** — they will be
> pulled into Backlog automatically once they gain a dependency on `B` or on a
> predecessor sibling (6d-bracket). No special create flag is needed; `jit` demotes
> a dependency-less Ready issue to Backlog the moment a not-done dependency is added.

### 6c. Add sequencing dependencies between children

After **all** children are created, add peer-to-peer sequencing edges (shared by
both shapes):

```bash
# For each child that has depends_on entries:
jit dep add <child-UUID> <dep-UUID-1> [<dep-UUID-2> ...]
```

### 6d-plain. Wire containment: parent depends on all children (PLAIN only)

For a **non-breakable** parent (Step 1.5 selected plain breakdown):

```bash
jit dep add <parent-UUID> <child-UUID-1> <child-UUID-2> ... <child-UUID-N>
```

This expresses the containment invariant: the parent cannot be marked done until
every child is complete. **Do NOT use this wiring for a bracket breakdown** — use
6d-bracket instead.

### 6d-bracket. Wire the bracket spine (BRACKET only)

For a **breakable** parent with an approved plan (Step 1.5 selected bracket
breakdown), create `B` and splice the source/sink spine `C → impl → B → P`. Use
the in-process engine path when available, or the CLI primitives below.

**1. Create the breakdown node `B`.** It carries the configured breakdown type, a
`brackets:<C-id>` label, and the container's membership label (NOT its type):

```bash
jit issue create \
  --title "Breakdown: <container title>" \
  --label "type:<breakdown_type>" \
  --label "brackets:<C-UUID>" \
  --label "<membership-label>"
# capture B-UUID
```

**2. Attach the coverage-preview gate to `B` and wire `B → P`:**

```bash
jit gate preset apply <coverage_gate_preset> <B-UUID>   # coverage-preview gate
jit dep add <B-UUID> <P-UUID>                            # B depends on approved plan
```

**3. Wire sources → `B`.** A *source* child has an empty `depends_on` (no
intra-subgraph predecessor). Every source depends on `B`, gating all impl behind
the approved breakdown:

```bash
# For each source child:
jit dep add <source-child-UUID> <B-UUID>
```

**4. Wire `C` → sinks.** A *sink* child has no sibling listing it in `depends_on`
(no intra-subgraph successor). The container depends on each sink:

```bash
# For each sink child:
jit dep add <C-UUID> <sink-child-UUID>
```

`jit` keeps the DAG transitively reduced, so the scaffold's direct `C → P` edge
and any redundant `C → non-sink` edge are dropped automatically. Verify with
`jit issue show <C> --json | jq .depends_on` (should list sinks only, not `P`).

**5. Run the coverage-preview gate on `B`** (the plan-time coverage check):

```bash
jit gate run <coverage_gate_preset-gate-key> <B-UUID>   # or `jit gate pass` per project flow
```

If it FAILS (a `[hard]` criterion is left uncovered by the drafted children),
surface the findings and revise the plan/children before declaring the breakdown
done — do not force the gate. (Checking the recorded gate *status*, not just the
exit code, is the project convention.)

> Do NOT also run 6d-plain. The bracket spine REPLACES parent-centric containment:
> `C` depends on sinks only, children never copy `C`'s deps, and the container does
> not depend on `B` or every child.

### 6e. Error handling

If any `jit issue create` or `jit dep add` fails:
- Report which step failed and why.
- Do **not** roll back already-created issues (partial state is recoverable).
- Show the ref-to-UUID map so the user can complete the wiring manually.

### 6f. Recurse into oversized children (multi-level breakdown)

For each created child whose plan entry had `decompose_further: true` **and** a finer
child type exists below it in the hierarchy, break that child down another level by
re-running this skill with the child as the new parent:

- **Spec source:** the child has no linked spec doc. Materialize one — write the
  child's description plus the parent-spec section named in its `source` field to a
  temp markdown file under `dev/active/`, link it with `jit doc add`, and pass it as
  `[SPEC_DOC_PATH]`.
- Re-run Steps 2–7 with the child as parent: child type is the next level down, and
  the membership label is the child's own identifying label (a `type:story` child gets
  a `story:<slug>` label that its tasks then carry).
- Present each sub-breakdown for its own approval (Step 5) before creating anything.
- Recursion ends when no child is flagged or the finest type is reached.

This is what turns a large epic into epic → story → task rather than a flat layer of
leaves. Keep depth proportional to size — do not force a story level onto small work.

> Bracket note: recursion shape is decided per-child in Step 1.5 by the *child's*
> type, not the root's. A `decompose_further` child is itself a bracket breakdown
> only if its own type is in `breakable_types` AND it has been scaffolded with its
> own plan node; otherwise it recurses as a plain breakdown. In practice the impl
> children of a bracket are leaves or plain sub-containers — the bracket lives at
> the breakable-container level.

---

## Step 7: Validation and summary

1. Run `jit validate`. If it reports errors:
   - Show the errors.
   - Identify which edges are problematic.
   - Offer to remove offending edges with `jit dep rm` and re-validate.

2. **Content lint — verify every created issue (all levels) meets the standards**
   (`jit-manage/references/content-standards.md`). Via `jit issue show <id> --json`:
   - **Success Criteria present** — the description has a `## Success Criteria` section
     (or an accepted equivalent). Missing → fix the description before finishing.
   - **Clean title** — no embedded metadata: reject ordinals (`T1`, `S0:`),
     `feat(...)`/`type:` prefixes, or parent IDs. Position lives in the DAG and labels.
   - **Correct type + membership** — `type:*` matches the level created; the membership
     label (`epic:<slug>`, `story:<slug>`, …) is present and is a kebab slug, never a
     JIT short ID; every `type:story`/`type:epic` carries its own identifying label.
   - **Quality gates present** — `gates_required` is non-empty and matches the issue's
     `gate_tier` set (Step 1.6). A leaf issue with no gates can be closed with no quality
     check — that is a defect; add the tier's gates before finishing.
   Report violations and offer to fix them (`jit issue update --label … --remove-label …`
   for labels, `jit issue update -d` for descriptions, `jit gate add` for gates) before
   declaring done.

3. Show a summary (shape-aware):

   Plain breakdown:
   ```
   Breakdown complete
     Parent issue      : <title> (<short-id>)
     Children created  : N
     Sequencing edges  : M (between siblings)
     Containment edges : N (parent → each child)
     Longest chain     : K issues
     Warnings          : <any from jit validate>
   ```

   Bracket breakdown:
   ```
   Bracket breakdown complete
     Container (C)     : <title> (<short-id>)
     Plan node (P)     : <short-id>  (plan-quality gate: passed)
     Breakdown node (B): <short-id>  (coverage-preview gate: <pass/fail>)
     Children drafted  : N (in Backlog)
     Spine             : C → {S sinks} … {sources} → B → P
     Sequencing edges  : M (between siblings)
     Warnings          : <any from jit validate>
   ```

4. Optionally export a Mermaid sub-graph for the parent and its new children:
   ```bash
   jit graph export --format mermaid
   ```
   Print the first 40 lines so the user can paste it into a renderer.
