---
name: jit-breakdown
description: Break down a JIT issue into child work items based on its specification document. Reads the spec doc linked to the issue (or user-specified), uses an analysis sub-agent to decompose it into child issues with proper dependency edges, presents a plan for approval, then creates the issues and wires up the dependency graph. Use when asked to "break down", "decompose", "detail out", or "create subtasks for" a JIT issue.
compatibility: Requires JIT CLI on PATH. JIT MCP tools used where available.
---

# JIT Issue Breakdown

Read the specification document attached to a parent issue, decompose it into
child issues with a correct dependency DAG, and populate JIT.

**Two breakdown shapes, selected by the ruleset (Step 1.5):**

- **Bracket breakdown** — when the parent is a *breakable container*, i.e. its
  type appears in some `.jit/templates.toml` template's `applies_to`. The plan is
  a first-class, gated node `P` sequenced *before* the fan-out, and breakdown
  splices a **source/sink spine** `C → impl → B → P` (not parent-centric
  containment). This is the plan-before-fan-out flow. See [The bracket](#the-bracket) below.
- **Plain breakdown** — when the parent is NOT a breakable container (no
  `.jit/templates.toml` template applies to the parent's type). The classic
  parent-centric flow: create children, then make the parent depend on all of
  them (Step 6c-plain).

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

- **`B`** (`type:<breakdown_type>`) was created by `jit apply plan <C>`, not by
  breakdown — it already carries the `brackets:<C-short-id>` label, **both** of the
  breakdown node's preset gates (the **coverage-preview** gate and the
  **breakdown-review** gate), and a dependency on `P`. Breakdown CONSUMES this
  pre-created `B`. Both gates the fan-out: jit will not release the impl children
  until each passes. Run the coverage-preview inline for immediate feedback (Step
  6c-bracket). **Breakdown-review is left PENDING** for the standard gate runner.
  Jit's own gate enforcement holds the fan-out until both pass.
- **Impl children** are created in backlog.
- **Sources** (impl issues with no intra-subgraph predecessor) depend on `B`;
  internal chains carry the rest. This transitively gates ALL impl behind `B`.
- **Sinks** (impl issues with no intra-subgraph successor) are depended-on by `C`
  (`C depends on each sink`). Transitive reduction drops the scaffold's direct
  `C → B` edge and any redundant `C → non-sink` edge automatically.

The node types (`breakdown_type`, `planning_type`) and the gate preset names come
from the `.jit/templates.toml` template's node declarations.

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
   project configured. Present the proposed tiers and
   their gate sets and confirm with the user once. These become `[GATE_TIERS]` for the
   analysis prompt and the tier → gate mapping applied at creation (Step 6).

---

## Step 1.5: Bracket detection and plan-approval gate

Decide which breakdown shape applies, and — if bracket — confirm the plan is
approved before drafting any children.

1. **Read the templates.** Inspect `.jit/templates.toml` for a template whose
   `applies_to` lists the parent's `type:*` value. If none applies, the repo does
   not bracket this type → this is a **plain breakdown**; skip the rest of this
   step and use the plain wiring (Step 6c-plain).

2. **Is the parent breakable?** From the templates' `applies_to` sets, check
   whether the parent's `type:*` value appears. If NOT → **plain breakdown** (skip
   the rest of this step). If it does → **bracket breakdown**; continue. From the
   matching template's nodes, read and record `planning_type` and `breakdown_type`
   (the planning- and breakdown-role node `type`s) and the gate presets they
   declare — the planning node's gate (plan-review), and the breakdown node's two
   gates (`coverage-preview`, then `breakdown-review`). The concrete gate names
   shown throughout this doc (`plan-review`, `coverage-preview`, `breakdown-review`)
   are the DEFAULT ruleset's; read the actual preset names from the template and
   substitute them in the commands below whenever a ruleset differs.

3. **Require a scaffolded bracket (`P` and `B`).** The container must already be
   bracketed (`C → B → P`) by the scaffold step `jit apply plan <id>`, which
   creates BOTH the planning node `P` and the breakdown node `B` and wires
   `B → P`. Breakdown CONSUMES the pre-created `B`; it does not create it. Locate
   `B`, then find `P` through it:
   ```bash
   jit issue show <C> --json | jq -r '.depends_on[]'
   # B is the dependency typed <breakdown_type> carrying brackets:<C-short-id>
   jit issue show <B> --json | jq -r '.depends_on[]'
   # P is B's dependency typed <planning_type>
   ```
   If no breakdown node exists, STOP and tell the user:
   > "This breakable container has no bracket. Scaffold it first with
   > `jit apply plan <id>`, produce and review the plan, then re-run breakdown."

4. **Require an APPROVED plan.** Bracket breakdown consumes an *approved* plan, so
   the plan-quality gate on `P` must have passed. Check:
   ```bash
   jit issue show <P> --json | jq '{state, gates_required, gates_status}'
   ```
   The plan is approved when `P`'s plan-quality gate status is `passed` (and `P` is
   Done or Gated-passing). If the plan gate is pending or failed, STOP:
   > "The plan node <P> has not passed its plan-quality gate. Review and pass the
   > plan before fanning out (the breakdown must consume an approved plan)."
   Do not proceed to draft children until the plan is approved.

5. **Extract the container's `[hard]` criteria (for coverage).** The
   coverage-preview gate on `B` checks that every `[hard]` success criterion of the
   container `C` is credited by some child via a `satisfies:<id>` label. Pull the
   container's `## Success Criteria` and collect each line tagged `[hard]`, with its
   id token (the ruleset's `id-pattern`, e.g. `REQ-01`):
   ```bash
   jit issue show <C> --json | jq -r '.description'   # read the Success Criteria section
   ```
   Record these as `[CONTAINER_HARD_CRITERIA]` (id + text, one per line) for the
   analysis prompt (Step 4). If the container has no `[hard]` criteria, note
   "(none — plain breakdown)" so the analysis agent leaves `satisfies` empty.
   The `satisfies-namespace` (default `satisfies`) and `id-pattern` come from the
   coverage rule in `.jit/rules.toml` — read them there; never hardcode `satisfies`
   or `REQ-` if the rule declares otherwise.

---

## Step 2: Read the configured type hierarchy

Read `.jit/config.toml` and extract the `[type_hierarchy]` section. Build a table
of type names sorted by level (ascending), for example:

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

**Depth is size-driven, not fixed at one level.** Break one level at a
time (parent → level+1), but a large parent should end up multi-level (e.g.
epic → story → task), not a flat layer of leaves. The analysis agent flags any child
that is itself several deliverables with `decompose_further: true`; Step 6e recurses
on those into the next level. A small parent simply produces leaf children directly.

---

## Step 3: Determine the membership label

Children need a label that groups them with this parent.

1. Inspect the parent issue's existing labels for one matching the membership
   namespace (e.g., look for `epic:*` if parent type is named `epic`).

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
| `[CONTAINER_HARD_CRITERIA]` | Bracket only: the container's `[hard]` criteria (id + text) from Step 1.5 step 5, one per line; "(none — plain breakdown)" otherwise |

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
    - Bracket breakdown: the pre-created breakdown node B (created by
      `jit apply plan`, already depending on the approved plan P) is consumed;
      children are drafted in Backlog; source children depend on B; the container
      depends on the sink children. The scaffold's C → B edge is dropped by
      reduction. (See "The bracket".)

Notes from analysis agent:
  <notes field from JSON>
```

Show all sequencing edges. Cross-dependencies (where a child depends on another
child that is not an immediate predecessor) should be explicitly called out.

**Before asking for approval**, verify that every proposed child meets the minimum output quality bar: a descriptive title with no ordinals, `feat(...)` prefixes, or embedded IDs (content-standards Issue Titles); a type derived from the configured hierarchy (Step 2); and a non-empty `gate_tier` mapped to at least one gate (Step 1.6). An item missing any of these will fail the content lint in Step 7; catch it now and use **edit** to correct the plan before creation.

Ask: **"Create these N child issues and wire up dependencies? [yes / edit / abort]"**

- **yes** — proceed to Step 6
- **edit** — print the raw JSON and ask the user to paste a corrected version;
  re-render the list and ask again
- **abort** — stop; nothing has been written to JIT

---

## Step 6: Execution

### 6a. Create child issues

For each child issue:

```bash
jit issue create \
  --title "<title>" \
  --description "<description>" \
  --label "type:<child-type>" \
  --label "<membership-label>" \
  --label "satisfies:<id>" ...        # one per id in this child's `satisfies` (bracket only; Step 1.5 step 5)
  --priority "<priority>" \
  --gate "<g1>" --gate "<g2>"          # the gates this issue's gate_tier maps to (Step 1.6)
```

Apply the quality gates from the issue's `gate_tier`. Every implementation issue gets
its tier's gates — skipping them leaves work that can be closed with no quality check.
Use `--gate` at creation (above), or `jit gate add <uuid> <gates>` / `jit gate preset
apply <preset> <uuid>` afterward. **This per-task gate assignment applies to BOTH
breakdown shapes**.

**Bracket only — attach the coverage credits.** For each id in this child's
`satisfies` array (the plan JSON field), add a `<satisfies-namespace>:<id>` label
(default namespace `satisfies`, e.g. `satisfies:REQ-01`) — via `--label` above or
`jit issue update <uuid> --label satisfies:<id>` afterward. These labels are what the
coverage-preview gate on `B` reads to credit each container `[hard]` criterion; a
criterion no child carries is reported uncovered (references/bracket-spine.md step 5). When the in-process
engine path is used, pass them as the child's `labels` so they are attached at creation.
Plain breakdown produces no `satisfies` labels.

Capture the returned UUID and store in an in-memory map: `ref → UUID`.

### 6b. Add sequencing dependencies between children

After **all** children are created, add peer-to-peer sequencing edges (shared by
both shapes):

```bash
# For each child that has depends_on entries:
jit dep add <child-UUID> <dep-UUID-1> [<dep-UUID-2> ...]
```

### 6c-plain. Wire containment: parent depends on all children (PLAIN only)

For a **non-breakable** parent (Step 1.5 selected plain breakdown):

```bash
jit dep add <parent-UUID> <child-UUID-1> <child-UUID-2> ... <child-UUID-N>
```

This expresses the containment invariant: the parent cannot be marked done until
every child is complete. **Do NOT use this wiring for a bracket breakdown** — use
6c-bracket instead.

### 6c-bracket. Wire the bracket spine (BRACKET only)

For a **breakable** parent with an approved plan (Step 1.5 selected bracket
breakdown), splice the source/sink spine `C → impl → B → P` around the
pre-created `B`, then run the coverage-preview gate inline and block the fan-out
on its result.

**Follow [references/bracket-spine.md](references/bracket-spine.md)** for the full
procedure: locate the pre-created `B` (and `P` through it), wire sources → `B` and
`C` → sinks (transitive reduction drops the scaffold's `C → B` edge), then run
coverage-preview via the standard runner and gate the fan-out on its recorded
status. The agent breakdown-review gate is left PENDING for the runner, like
`plan-review` on `P`.

Do **not** also run 6c-plain — the bracket spine REPLACES parent-centric
containment.

### 6d. Error handling

If any `jit issue create` or `jit dep add` fails:
- Report which step failed and why.
- Do **not** roll back already-created issues (partial state is recoverable).
- Show the ref-to-UUID map so the user can complete the wiring manually.

### 6e. Recurse into oversized children (multi-level breakdown)

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
> only if its own type is in a `.jit/templates.toml` template's `applies_to` AND it
> has been scaffolded with its own plan node; otherwise it recurses as a plain
> breakdown. In practice the impl
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
   - **Coverage credits present (bracket only)** — every container `[hard]` criterion
     (Step 1.5 step 5) is carried by at least one child as a `satisfies:<id>` label, so
     the coverage-preview gate on `B` can credit it. The standard gate runner records
     that gate's verdict on `B` when you run `jit gate pass <B> <coverage-gate>` (the
     breakdown node's coverage gate from the template; `coverage-preview` in the
     default ruleset) (references/bracket-spine.md step 5); a `[hard]` criterion no child satisfies is reported uncovered
     and the gate fails — add the missing `satisfies:<id>` label
     (`jit issue update <child> --label satisfies:<id>`) and re-run the gate.

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
     Breakdown node (B): <short-id>  (coverage-preview: <pass/fail>, breakdown-review: attached, pending gate runner)
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
