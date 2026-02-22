---
name: jit-migrate
description: Migrate an existing project to JIT issue tracking by scanning planning artifacts (ROADMAP, TODO, etc.), extracting work items, inferring a dependency DAG, and populating JIT. Use when asked to "migrate to jit", "import tasks into jit", or "set up jit for this project".
compatibility: Requires JIT CLI on PATH. JIT MCP tools used where available.
---

# JIT Migration

Scan planning artifacts, build a dependency DAG, populate JIT, and confirm
the result with `jit validate`.

---

## Step 1: Pre-flight

1. Identify the project root — the directory containing `.git`, or a manifest
   file (`Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`, `Makefile`,
   etc.). Use the current working directory if unambiguous.

2. Check for an existing `.jit/` directory:

   | Situation | Action |
   |---|---|
   | No `.jit/` | Run `jit init` (ask which `--hierarchy-template` to use, or leave blank for the default). |
   | `.jit/` exists, index empty | Proceed. |
   | `.jit/` exists and already has issues | Warn: "Migration will **add** issues; existing issues are untouched." Ask the user to confirm before continuing. |

3. Confirm `jit validate` passes on the current state before adding anything.

---

## Step 2: Read the configured type hierarchy

Read `.jit/config.toml` and extract the `[type_hierarchy]` section. Build a
table of type names sorted by level (ascending). This table will be passed
verbatim to the analysis agent.

Example of what to construct:

```
Level 1 (broadest): initiative
Level 2:            feature
Level 3 (finest):   task, bug
```

If `config.toml` has no `[type_hierarchy]` section, use the JIT defaults:
level 1 = `milestone`, level 2 = `epic`, level 3 = `story`, level 4 = `task`.

**Never pass hardcoded type names to the analysis agent — always use the
actual configured names.**

### Dependency direction invariant

The `depends_on` field means "this issue is blocked by its dependencies."
Two kinds of edge exist:

| Kind | Direction | Example |
|------|-----------|---------|
| **Containment** | Parent depends on its children | Epic "GPU accel" depends on task "Write shader" |
| **Sequencing** | Later work depends on earlier work | Task "Run benchmark" depends on task "Write shader" |
| **Cross-branch** | Work depends on a different epic/story | Task "Integrate QAM" depends on epic "Implement QAM" |

The resulting DAG flows **upward**: leaf tasks → stories → epics → milestones.
A child never lists its own parent in `depends_on`; a task *may* depend on an
unrelated epic when that entire body of work is a genuine prerequisite.

Verify this invariant holds during plan review (Step 5) and after execution
(Step 7).

---

## Step 3: Discover planning artifacts

Search the project tree for files that likely contain planning information.
Collect their paths (do not read content yet — the analysis agent will read
them directly).

**Glob patterns to try (case-insensitive):**

```
ROADMAP*          TODO*             PLAN*
MILESTONE*        BACKLOG*          NOTES*
CHANGELOG*        CHANGES*          HISTORY*
docs/**/*.md      doc/**/*.md       .github/**/*.md
*.org             *.rst             (if no .md found)
```

**Exclude:**
- `node_modules/`, `vendor/`, `target/`, `.git/`, `dist/`, `build/`
- API reference docs, auto-generated files, licence files
- Files larger than ~200 KB (likely generated)

**Inline TODO/FIXME comments** (off by default): if the project has no
standalone planning files at all, ask the user whether to search source files
for `TODO:` / `FIXME:` comments. If yes, collect up to the 50 most recent
(by file modification time) to avoid noise.

Present the file list to the user: "Found N planning files — [list]. Shall I
analyse these?" Add or remove files based on user feedback before continuing.

---

## Step 4: Analysis (sub-agent)

Dispatch a `general-purpose` sub-agent using the prompt template at
[references/analysis-prompt.md](references/analysis-prompt.md).

Fill in the template fields:

| Field | Value |
|---|---|
| `[PROJECT_ROOT]` | Absolute path to the project root |
| `[TYPE_HIERARCHY_TABLE]` | The table built in Step 2 |
| `[FILE_LIST]` | Newline-separated absolute paths from Step 3 |

The agent must return **only** a JSON object — see
[references/plan-schema.md](references/plan-schema.md) for the schema.

Parse the returned JSON. If parsing fails, show the raw output to the user
and ask whether to retry or abort.

---

## Step 5: Plan review

Present the plan to the user as a tree, then ask for approval.

**Render format:**

```
Migration plan  (N issues, M dependency edges)

[level-1-type] Title of broad issue (ref: A, priority: high)  ← A depends on B, D
  └─ [level-2-type] Title of mid-scope issue (ref: B)         ← B depends on C
       └─ [leaf-type] Fine-grained task (ref: C)               (no deps — leaf)
  └─ [level-2-type] Parallel item (ref: D)                     (no deps — leaf)

Notes from analysis agent:
  <notes field from JSON>
```

Show dependency edges explicitly when they cross branches of the tree (e.g.
"C also depends on D"). Show total edge count separately.

Ask: **"Create these N issues with M dependency edges? [yes / edit / abort]"**

- **yes** — proceed to Step 6
- **edit** — print the raw JSON and ask the user to paste a corrected version;
  re-render the tree and ask again
- **abort** — stop; nothing has been written to JIT

---

## Step 6: Execution

### 6a. Topological sort

Sort the issues so that every issue's dependencies appear before it in the
creation order. (Standard Kahn's algorithm on the `depends_on` graph.) This
ensures no forward-reference problems during creation.

### 6b. Create issues

For each issue in topological order:

```bash
jit issue create \
  --title "<title>" \
  --description "<description>" \
  --label "type:<type>" \
  --priority "<priority>"
```

Capture the returned UUID and store it in an in-memory map:
`ref → UUID` (e.g. `"T3" → "a3e2ac99-..."`).

### 6c. Add dependency edges

After **all** issues are created, add the dependency edges:

```bash
# For each issue that has depends_on entries:
jit dep add <issue-UUID> <dep-UUID-1> [<dep-UUID-2> ...]
```

Translate `depends_on` refs to UUIDs using the map from 6b.

### 6d. Error handling

If any `jit issue create` or `jit dep add` fails:
- Report which step failed and why
- Do **not** roll back already-created issues (partial state is recoverable)
- Tell the user the ref-to-UUID map so they can complete the migration manually

---

## Step 7: Validation and summary

1. Run `jit validate`. If it reports errors:
   - Show the errors
   - Identify which dependency edges are problematic
   - Ask whether to remove the offending edges (`jit dep rm`) and re-validate

2. Show a summary:
   ```
   Migration complete
     Issues created : N
     Dependency edges: M
     Longest chain  : K issues
     Warnings       : <any from jit validate>
   ```

3. Optionally export a visual of the DAG (requires graphviz or mermaid viewer):
   ```bash
   jit graph export --format mermaid
   ```
   Print the first 40 lines of the mermaid output so the user can paste it
   into a renderer if they wish.
