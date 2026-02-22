# Migration Plan JSON Schema

The analysis agent must return **only** a JSON object matching this schema.
No prose before or after — the main agent parses the output directly.

```json
{
  "issues": [
    {
      "ref":         "string  — stable short ID within this plan (e.g. 'A', 'B1', 'T3')",
      "title":       "string  — concise, action-oriented title",
      "description": "string  — what needs to be done and why; include context from the source doc",
      "type":        "string  — exactly one of the configured type names (see prompt)",
      "priority":    "string  — 'low' | 'normal' | 'high' | 'critical'",
      "depends_on":  ["ref-X", "ref-Y"],
      "source":      "string  — filename and section/heading where this was found"
    }
  ],
  "notes": "string — ambiguities, items that could not be classified, or assumptions made"
}
```

## Field rules

**`ref`**
Unique within the plan. Used only to express `depends_on` relationships; never
written to JIT. Keep short and stable (survives copy-paste editing by the user).

**`type`**
Must be one of the type names from the project's configured hierarchy — these
are passed to you in the prompt. Never invent new type names.

**`depends_on`**
`["ref-X"]` means this issue **is blocked by** `ref-X` — it cannot be
considered complete until `ref-X` is done.
Use an empty array `[]` when there are no prerequisites.
This field is the most important output — take time to reason about it.

**Containment vs sequencing:**
- **Containment** (parent–child): A parent issue lists its own children in
  `depends_on` because the parent cannot close until its children are done.
  Example: epic "GPU acceleration" depends on task "Write compute shader."
- **Sequencing** (peer-to-peer): A task that must wait for an unrelated
  predecessor lists that predecessor in `depends_on`.
  Example: task "Run benchmark" depends on task "Write compute shader."
- **Cross-branch sequencing**: A task may depend on an epic or story from a
  different branch of the hierarchy when that entire body of work is a
  genuine prerequisite.
  Example: task "Integrate QAM into FEC chain" depends on epic "Implement QAM."

A child issue should NOT list its own parent in `depends_on` — that inverts
the containment relationship.

**`priority`**
Default to `"normal"` when uncertain. Use `"high"` for things that block many
other items. Use `"critical"` only for hard blockers with no workaround.

## Dependency reasoning guide

### Sequencing dependencies (peer-to-peer)

Ask for each pair of peer issues: "Can work on B begin while A is still in progress?"
- If **no** → `B.depends_on` includes `A`'s ref
- If **yes** → no dependency edge needed

Common dependency signals in planning documents:
- "after X is done", "once X lands", "requires X" → explicit dep
- "phase 2", "next step", "then" → likely sequential dep
- "blocked by", "needs", "depends on" → explicit dep
- Items in a numbered list where order implies sequence → likely deps
- Feature requiring infrastructure that doesn't exist yet → infrastructure is a dep

A task may also depend on an epic or story from a **different** branch of the
hierarchy when that body of work is a genuine prerequisite (cross-branch dep).

**Prefer fewer dependencies over speculative ones.** Only add an edge when
the sequencing constraint is real, not just a suggested order.

### Containment dependencies (parent–child)

A parent issue (broader scope) must list its own children in `depends_on`
because the parent cannot close until all its children are complete.
A child issue must NOT list its own parent in `depends_on` — that inverts
the relationship.

The resulting DAG flows upward: leaf tasks → stories → epics → milestones.
