---
name: jit-breakdown
description: Break down a JIT issue into child work items based on its specification document. Reads the spec doc linked to the issue (or user-specified), uses an analysis sub-agent to decompose it into child issues with proper dependency edges, presents a plan for approval, then creates the issues and wires up the dependency graph. Use when asked to "break down", "decompose", "detail out", or "create subtasks for" a JIT issue.
compatibility: Requires JIT CLI on PATH. JIT MCP tools used where available.
---

# JIT Issue Breakdown

Read the specification document attached to a parent issue, decompose it into
child issues with a correct dependency DAG, and populate JIT.

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

  After creation, the parent issue will be updated to depend on all N children.

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
  --priority "<priority>"
```

Capture the returned UUID and store in an in-memory map: `ref → UUID`.

### 6c. Add sequencing dependencies between children

After **all** children are created, add peer-to-peer sequencing edges:

```bash
# For each child that has depends_on entries:
jit dep add <child-UUID> <dep-UUID-1> [<dep-UUID-2> ...]
```

### 6d. Wire containment: parent depends on all children

```bash
jit dep add <parent-UUID> <child-UUID-1> <child-UUID-2> ... <child-UUID-N>
```

This expresses the containment invariant: the parent cannot be marked done until
every child is complete.

### 6e. Error handling

If any `jit issue create` or `jit dep add` fails:
- Report which step failed and why.
- Do **not** roll back already-created issues (partial state is recoverable).
- Show the ref-to-UUID map so the user can complete the wiring manually.

---

## Step 7: Validation and summary

1. Run `jit validate`. If it reports errors:
   - Show the errors.
   - Identify which edges are problematic.
   - Offer to remove offending edges with `jit dep rm` and re-validate.

2. Show a summary:
   ```
   Breakdown complete
     Parent issue      : <title> (<short-id>)
     Children created  : N
     Sequencing edges  : M (between siblings)
     Containment edges : N (parent → each child)
     Longest chain     : K issues
     Warnings          : <any from jit validate>
   ```

3. Optionally export a Mermaid sub-graph for the parent and its new children:
   ```bash
   jit graph export --format mermaid
   ```
   Print the first 40 lines so the user can paste it into a renderer.
