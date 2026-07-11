---
name: jit-breakdown
description: >
  Decompose one specified jit issue into child issues: an analysis sub-agent
  drafts schema-validated children from the linked spec or approved plan, then
  they are created and wired into the dependency graph. To produce the plan
  first, use jit-planning-lead.
compatibility: Requires JIT CLI on PATH. JIT MCP tools used where available.
---

# JIT Issue Breakdown

Read the specification document attached to a parent issue, decompose it into
child issues with a correct dependency DAG, and populate JIT.

Concrete gate, namespace, and id names in this document (`plan-review`,
`coverage-preview`, `breakdown-review`, `satisfies`, `REQ-`) are the default
ruleset's. Read the live names from `.jit/templates.toml` and `.jit/rules.toml`
and substitute them.

**Two breakdown shapes, selected by the ruleset (Step 1.5):**

- **Bracket breakdown** — the parent is a *breakable container* (its type appears
  in some `.jit/templates.toml` template's `applies_to`). See
  [The bracket](#the-bracket).
- **Plain breakdown** — no template applies to the parent's type: create
  children, then make the parent depend on all of them (Step 6c-plain).

Steps 2–5 are shared apart from child-type selection (Step 2); the shapes differ
in Step 6 wiring.

---

## The bracket

For a breakable container `C` whose plan has been approved, breakdown produces:

```
C ──dep→ {impl subgraph} ──dep→ B ──dep→ P
```

precedence `P > B > impl > C` (plan first, then breakdown, then work, then the
container closes). Concretely:

- **`B`** (`type:<breakdown_type>`) was created by `jit apply plan <C>`, not by
  breakdown — it already carries the `brackets:<C-short-id>` label, both preset
  gates (coverage-preview, breakdown-review), and a dependency on `P`. Breakdown
  CONSUMES this pre-created `B`. Run coverage-preview inline (Step 6c-bracket);
  leave breakdown-review PENDING for the standard gate runner.
- **Impl children** are created in backlog.
- **Sources** (impl issues with no intra-subgraph predecessor) depend on `B`.
- **Sinks** (impl issues with no intra-subgraph successor) are depended-on by
  `C`. Transitive reduction drops the scaffold's `C → B` edge and any redundant
  `C → non-sink` edge.

The node types and gate preset names come from the template's node declarations.

---

## Step 1: Pre-flight

1. Identify the parent issue:
   - Issue ID given → `jit issue show <id>`.
   - Title described → `jit issue search <query>`; confirm the match.

2. Display the parent's title, state, type, labels, and description.

3. Check for existing children via the parent's `depends_on` list. If any exist,
   warn that breakdown will add more child issues alongside them and confirm
   before continuing.

4. Locate the specification document:
   - `jit doc list <id>` — if docs are listed, ask which to use (default: most
     recently modified).
   - None linked → ask for the path, verify it exists, offer to link it:
     `jit doc add <id> --path <path> --doc-type design`

5. Run `jit validate`; surface any errors before adding anything new.

6. **Determine the project's gate tiers.** Run `jit gate list` and
   `jit gate preset list`, and sample existing issues
   (`jit issue show <id> --json | jq .gates_required`) to learn the convention.
   Define a small set of named tiers — at minimum a **primary/full** tier and a
   lighter tier for clearly supporting work. Present the tiers with their gate
   sets and confirm with the user once. These become `[GATE_TIERS]` for the
   analysis prompt and the tier → gate mapping applied at creation (Step 6).

---

## Step 1.5: Bracket detection and plan-approval gate

Decide the breakdown shape; if bracket, require an approved plan before drafting
any children.

1. **Is the parent breakable?** Inspect `.jit/templates.toml` for a template
   whose `applies_to` lists the parent's `type:*` value. None → **plain
   breakdown**; skip the rest of this step and use Step 6c-plain. Found →
   **bracket breakdown**; record the template's `planning_type`,
   `breakdown_type`, and the gate presets they declare.

2. **Require a scaffolded bracket (`P` and `B`).** `jit apply plan <id>` must
   already have created both nodes and wired `B → P`. Locate them:
   ```bash
   jit issue show <C> --json | jq -r '.depends_on[]'
   # B: the dependency typed <breakdown_type> carrying brackets:<C-short-id>
   jit issue show <B> --json | jq -r '.depends_on[]'
   # P: B's dependency typed <planning_type>
   ```
   No breakdown node → STOP: direct the user to scaffold with
   `jit apply plan <id>`, produce and review the plan, then re-run breakdown.

3. **Require an APPROVED plan.** Check
   `jit issue show <P> --json | jq '{state, gates_required, gates_status}'`.
   Proceed only when `P`'s plan-quality gate status is `passed` (and `P` is Done
   or Gated-passing); otherwise STOP and direct the user to pass the plan gate
   first.

4. **Extract the container's `[hard]` criteria (for coverage).** Pull `C`'s
   `## Success Criteria` and collect each `[hard]` line with its id token.
   Record them as `[CONTAINER_HARD_CRITERIA]` (id + text, one per line) for the
   analysis prompt; with no `[hard]` criteria, record "(none — plain breakdown)"
   so the agent leaves `satisfies` empty. Read the `satisfies-namespace` and
   `id-pattern` from the coverage rule in `.jit/rules.toml`.

---

## Step 2: Read the configured type hierarchy

Read `[type_hierarchy]` from `.jit/config.toml` and build a type table sorted by
level. Determine the parent's level from its `type:*` label.

Identify the **child types**:

- **Plain breakdown** — the type(s) at level+1. If the parent is already at the
  finest level, warn that it cannot be broken down further.
- **Bracket breakdown** — the tiers the approved plan's decomposition sketch
  (§3) declares per item; that assignment is the reviewed contract and may span
  more than one level below the parent. Mixed story- and task-typed children
  under one parent are a valid outcome.

Read `[type_hierarchy.label_associations]` for the parent type's membership
namespace (e.g. `epic = "epic"` → children carry `epic:<name>`).

**Never hardcode type names — always use the configured hierarchy.**

**Depth is size-driven.** Break one level at a time, but let a large parent end
up multi-level: the analysis agent flags oversized children with
`decompose_further: true` and Step 6e recurses on them.

---

## Step 3: Determine the membership label

1. Look for a label on the parent matching the membership namespace.
2. Found → use its value for all children.
3. Missing → derive a kebab slug from the parent's title (lowercase, hyphens,
   ≤ 30 chars), confirm with the user, and add it:
   `jit issue update <id> --label <namespace>:<slug>`

Record the full label — every created child carries it.

---

## Step 4: Analysis (sub-agent)

Dispatch a `general-purpose` sub-agent using the prompt template at
[references/analysis-prompt.md](references/analysis-prompt.md), filling:

| Field | Value |
|---|---|
| `[PARENT_ISSUE_TITLE]` | Parent issue title |
| `[PARENT_ISSUE_DESCRIPTION]` | Parent issue description (or "(none provided)") |
| `[PARENT_TYPE]` | Parent's type name |
| `[CHILD_TYPES_TABLE]` | Plain: type(s) at level+1. Bracket: the sketch's declared tiers (Step 2). One per line with level |
| `[MEMBERSHIP_LABEL]` | The label from Step 3 |
| `[SPEC_DOC_PATH]` | Absolute path to the spec document |
| `[TYPE_HIERARCHY_TABLE]` | Full hierarchy table from Step 2 |
| `[GATE_TIERS]` | The gate tiers and their gate sets from Step 1.6 (one per line) |
| `[CONTAINER_HARD_CRITERIA]` | Bracket only: the `[hard]` criteria from Step 1.5 step 4; "(none — plain breakdown)" otherwise |

Require a bare JSON object in return — schema at
[references/plan-schema.md](references/plan-schema.md). If parsing fails, show
the raw output and ask whether to retry or abort.

**Validate every child against the schema before continuing.** The canonical
required-field set is the schema's Requiredness section: every field is required
except `decompose_further` (defaults to `false`). Reject the batch if any child
misses a required field or carries an empty `gates` array where its `gate_tier`
maps to a non-empty gate set; report the offending `ref` and field, then retry
or abort. Nothing is created until the whole batch passes.

---

## Step 5: Plan review

Present the proposed children, then ask for approval.

**Render format:**

```
Breakdown plan for: "<Parent Issue Title>"  (N child issues, M sequencing edges)

  [child-type] Title of first work item (ref: A, priority: high)   (no deps — can start immediately)
  [child-type] Title of second work item (ref: B, priority: normal) ← depends on A
  [child-type] Title of third work item (ref: C, priority: normal)  ← depends on A, B

  Wiring after creation: plain — parent depends on all N children;
  bracket — source/sink spine around the pre-created B (see "The bracket").

Notes from analysis agent:
  <notes field from JSON>
```

Show all sequencing edges; call out cross-dependencies explicitly.

**Before asking for approval**, verify every proposed child meets the quality
bar — an item missing any of these fails the Step 7 lint, so correct it now via
**edit**:

- Descriptive title with no ordinals, `feat(...)` prefixes, or embedded IDs
  (content-standards Issue Titles).
- Type derived from the configured hierarchy (Step 2).
- Identifying `<namespace>:<slug>` label on every child whose type has a
  membership namespace (Step 6a).
- Non-empty `gate_tier` with its `gates` array (validated in Step 4).

Ask: **"Create these N child issues and wire up dependencies? [yes / edit / abort]"**

- **yes** — proceed to Step 6
- **edit** — print the raw JSON, accept a corrected version, re-render, ask again
- **abort** — stop; nothing has been written to JIT

---

## Step 6: Execution

### 6a. Create child issues

For each child:

```bash
jit issue create \
  --title "<title>" \
  --description "<description>" \
  --label "type:<child-type>" \
  --label "<membership-label>" \
  --label "<identifying-label>"        # own <namespace>:<slug>; when the child type has a membership namespace
  --label "satisfies:<id>" ...        # one per id in this child's `satisfies` (bracket only)
  --priority "<priority>" \
  --gate "<g1>" --gate "<g2>"          # this child's validated `gates` array
```

**Identifying label.** When the child's `type` is a key in
`[type_hierarchy.label_associations]`, derive a kebab slug from the child's
title (the Step 3 slug shape) and attach `<namespace>:<slug>` — the child's own
label, alongside the membership label that groups it under the parent (a `story`
child under an epic carries both `epic:<parent-slug>` and `story:<child-slug>`).
The rule keys off each child's own type, so a mixed-tier fan-out labels every
strategic-typed child and leaves association-free leaves with the membership
label alone.

**Gates.** Apply the child's validated `gates` field (its `gate_tier`'s set) —
at creation via `--gate`, or afterwards via `jit gate add` /
`jit gate preset apply`. This applies to BOTH breakdown shapes.

**Coverage credits (bracket only).** Attach a `satisfies:<id>` label per entry
in the child's `satisfies` array — these are what the coverage-preview gate on
`B` reads (references/bracket-spine.md step 5). Plain breakdown produces none.

Capture each returned UUID in a `ref → UUID` map.

### 6b. Add sequencing dependencies between children

After **all** children are created (both shapes):

```bash
jit dep add <child-UUID> <dep-UUID-1> [<dep-UUID-2> ...]
```

### 6c-plain. Wire containment (PLAIN only)

```bash
jit dep add <parent-UUID> <child-UUID-1> <child-UUID-2> ... <child-UUID-N>
```

The parent cannot close until every child completes. Do NOT use this wiring for
a bracket breakdown.

### 6c-bracket. Wire the bracket spine (BRACKET only)

Follow [references/bracket-spine.md](references/bracket-spine.md): wire
sources → `B` and `C` → sinks around the pre-created `B`, run coverage-preview
via the standard runner, and block the fan-out on its recorded status. Leave
breakdown-review PENDING for the runner. The spine REPLACES parent-centric
containment — do not also run 6c-plain.

### 6d. Error handling

If any `jit issue create` or `jit dep add` fails: report which step and why, do
NOT roll back created issues (partial state is recoverable), and show the
ref-to-UUID map so the user can finish the wiring manually.

### 6e. Recurse into oversized children (multi-level breakdown)

For each created child with `decompose_further: true` **and** a finer child type
below it, break it down another level with this skill, the child as parent:

- **Spec source:** write the child's description plus the parent-spec section
  named in its `source` field to a markdown file under `dev/active/`, link it
  with `jit doc add`, pass it as `[SPEC_DOC_PATH]`.
- Re-run Steps 2–7: child type is the next level down; the membership label is
  the child's own identifying label.
- Present each sub-breakdown for its own Step 5 approval.
- Recursion ends when no child is flagged or the finest type is reached.

Keep depth proportional to size — do not force a story level onto small work.

> Bracket note: the recursion shape is decided per-child in Step 1.5 by the
> *child's* type — a `decompose_further` child is a bracket breakdown only if
> its own type is breakable AND it has been scaffolded with its own plan node;
> otherwise it recurses plain.

---

## Step 7: Validation and summary

1. Run `jit validate`. On errors: show them, identify the offending edges, offer
   `jit dep rm` and re-validate.

2. **Content lint — verify every created issue (all levels) against
   `../../../docs/reference/jit-content-standards.md`** via
   `jit issue show <id> --json`:
   - `## Success Criteria` section present (or an accepted equivalent).
   - Clean title: no ordinals, `feat(...)`/`type:` prefixes, or parent IDs.
   - `type:*` matches the level created; membership label is a kebab slug (never
     a JIT short ID); every strategic-typed child carries its own identifying
     label.
   - `gates_required` non-empty and matching the issue's `gate_tier` set — add
     missing gates before finishing.
   - Bracket only: every container `[hard]` criterion is carried by some child
     as a `satisfies:<id>` label; the coverage gate reports any uncovered
     criterion — add the missing label and re-run it
     (references/bracket-spine.md step 5).

3. Show a summary:

   ```
   Breakdown complete (<plain | bracket>)
     Parent/container  : <title> (<short-id>)
     Children created  : N (bracket: drafted in Backlog)
     Sequencing edges  : M (between siblings)
     Wiring            : plain — N containment edges | bracket — spine C → {sinks} … {sources} → B → P;
                         P plan gate passed; coverage-preview <pass/fail>; breakdown-review pending runner
     Longest chain     : K issues
     Warnings          : <any from jit validate>
   ```

4. Optionally export a Mermaid sub-graph (`jit graph export --format mermaid`)
   and print the first 40 lines.
