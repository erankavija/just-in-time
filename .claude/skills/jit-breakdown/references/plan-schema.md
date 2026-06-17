# Breakdown Plan JSON Schema

The analysis agent must return **only** a JSON object matching this schema.
No prose before or after — the main agent parses the output directly.

```json
{
  "issues": [
    {
      "ref":         "string  — stable short ID within this plan (e.g. 'C1', 'C2', 'C3')",
      "title":       "string  — concise, action-oriented title",
      "description": "string  — what needs to be done and why; self-contained; include acceptance criteria",
      "type":        "string  — exactly one of the configured child type names",
      "priority":    "string  — 'low' | 'normal' | 'high' | 'critical'",
      "depends_on":  ["ref-of-sibling"],
      "source":      "string  — section heading or excerpt from the spec that motivated this issue",
      "satisfies":   ["string  — container [hard] criterion id(s) this child covers, e.g. 'REQ-01'; [] if none"],
      "decompose_further": "boolean — true if this child is itself several distinct deliverables and should become a parent at the next level down (only when a finer child type exists below it)",
      "gate_tier":   "string  — which project gate tier this task needs, chosen from the tiers the skill supplies in [GATE_TIERS] (never invent gate names)"
    }
  ],
  "notes": "string — ambiguities, assumptions, items that could not be classified, or open questions"
}
```

## Field rules

**`ref`**
Unique within this plan. Used only to express `depends_on` relationships; never
written to JIT. Keep short and stable (survives copy-paste editing by the user).
Suggested format: `C1`, `C2`, etc. (C for child) or a short mnemonic.

**`type`**
Must be one of the child type names at level+1 below the parent — these are
passed to the agent in `[CHILD_TYPES_TABLE]`. Never invent new type names.

**`decompose_further`**
`true` when this child is not a single coherent unit but bundles several distinct
deliverables — i.e. it should become a parent at the next level down rather than a
leaf. Set it **only** when a finer child type exists below this one in the hierarchy.
Heuristic: if the child's own Success Criteria would split into three or more groups
addressing separate concerns, it is a story, not a task — set `true`. Default `false`.
The skill recurses on every child flagged `true`.

**`gate_tier`**
Which of the project's gate tiers applies, chosen from the `[GATE_TIERS]` list the skill
supplies in the analysis prompt. The skill derives these tiers from the project's own gate
registry, so the same mechanism fits any domain: software (a full CI+review tier alongside
a review-only tier), research (a peer-review+reproducibility tier alongside a notes tier),
and others. Give core deliverables the **primary/full** tier and clearly supporting work
(documentation, notes) a lighter tier. When unsure, choose the primary tier; pick from the
supplied tier labels.

**`depends_on`**
`["ref-X"]` means this issue **is blocked by** `ref-X` — it cannot start until
`ref-X` is complete. Only reference other issues in this plan's `issues` array.
Do NOT reference the parent issue (that relationship is handled by the skill).
Use an empty array `[]` when there are no sibling prerequisites.

This is the most important field. Take time to reason carefully about sequencing.

**`satisfies`** (bracket breakdown only)
The container's `[hard]` criterion ids that THIS child delivers. The skill supplies
the container's `[hard]` criteria (id + text) in `[CONTAINER_HARD_CRITERIA]`; for
each criterion, name the child(ren) that complete it and list that criterion's id
in their `satisfies` array. The skill turns each entry into a `satisfies:<id>` label
on the created child, which is how the **coverage-preview** gate credits coverage —
a `[hard]` criterion no child lists here is reported as *uncovered* by the gate.

- Use the exact id token from `[CONTAINER_HARD_CRITERIA]` (e.g. `REQ-01`), not the
  prose.
- A criterion may be split across several children (list its id on each); a child
  may satisfy several criteria (list all their ids).
- **Every** container `[hard]` criterion must appear in at least one child's
  `satisfies` — coverage must be total, or the gate fails.
- Plain breakdown (no `[CONTAINER_HARD_CRITERIA]` supplied): leave `satisfies: []`.

**`description`**
Must be self-contained. The reader will not have access to the spec document.
Include:
- What needs to be built or done
- Why (motivation / requirement)
- Acceptance criteria or definition of done
- Relevant constraints, edge cases, or interface requirements

**`priority`**
Default to `"normal"` when uncertain.
- `"high"` for items that block most other siblings.
- `"critical"` only for hard blockers with no workaround.
- `"low"` for optional polish or deferred enhancements.

**`source`**
Quote the section heading, bullet, or short excerpt from the spec document that
this issue came from. Helps the user verify coverage and trace requirements.

## Dependency reasoning guide

### Sequencing (sibling-to-sibling)

Ask for each pair of siblings: "Can work on B begin before A is complete?"
- **No** → `B.depends_on` includes A's ref.
- **Yes** → no edge needed.

Common signals in spec documents:
- "after X is implemented", "once X is available" → explicit dep
- "phase 2", "next step", "then" → likely sequential
- "blocked by", "requires", "depends on" → explicit dep
- Items in a numbered list where order implies sequence → likely deps
- An API, schema, or data structure that other items consume → infra is dep
- Tests, benchmarks, or validation that require an implementation → impl is dep

**Prefer fewer edges over speculative ones.** Only add an edge when the constraint
is real. If two items could reasonably be worked on in parallel by different
people, do not add an edge.

### What NOT to include in depends_on

- The parent issue itself (handled externally by the skill).
- Issues from other parent issues (this plan only covers siblings).
- Duplicate edges (if A→B and B→C, you don't need A→C unless it's a genuine
  hard constraint independent of B).

## Example

Given a spec for "GPU Acceleration Pipeline" epic, the output might be:

```json
{
  "issues": [
    {
      "ref": "C1",
      "title": "Define compute shader interface and data layout",
      "description": "Specify the WGSL interface for the compute shader: buffer layouts, workgroup sizes, and push constants. This interface must be agreed upon before the shader and the CPU dispatch code can be written in parallel. Acceptance: interface document reviewed and committed.",
      "type": "task",
      "priority": "high",
      "depends_on": [],
      "source": "Section 2: Shader Interface",
      "satisfies": ["REQ-01"]
    },
    {
      "ref": "C2",
      "title": "Implement compute shader for matrix multiply",
      "description": "Write the WGSL compute shader that performs batched matrix multiplication on the GPU. Must conform to the interface defined in C1. Acceptance: shader passes all unit tests in the test harness with correct numerical output.",
      "type": "task",
      "priority": "normal",
      "depends_on": ["C1"],
      "source": "Section 3: Shader Implementation"
    },
    {
      "ref": "C3",
      "title": "Implement CPU dispatch and buffer management",
      "description": "Write the Rust code that allocates GPU buffers, encodes compute passes, and reads back results. Must use the interface from C1. Acceptance: integration test dispatches a 1024×1024 multiply and returns correct results.",
      "type": "task",
      "priority": "normal",
      "depends_on": ["C1"],
      "source": "Section 4: CPU-side Dispatch"
    },
    {
      "ref": "C4",
      "title": "Benchmark GPU vs CPU path and document results",
      "description": "Run the benchmarks defined in the perf suite for both the CPU and GPU code paths. Record results in dev/perf-results.md. Acceptance: benchmark report committed, regression threshold set in CI.",
      "type": "task",
      "priority": "normal",
      "depends_on": ["C2", "C3"],
      "source": "Section 5: Performance Validation",
      "satisfies": ["REQ-04"]
    }
  ],
  "notes": "Section 6 (fallback CPU path) was too vague to decompose into a single task; included as C5 with a note in its description."
}
```

In this example: C1 has no deps (can start immediately). C2 and C3 both depend on
C1 but not on each other (can be worked in parallel). C4 depends on both C2 and
C3 (needs both implementations to benchmark). The container's `[hard]` criteria
REQ-01..REQ-04 are each credited to the child that delivers them via `satisfies`
(the interface task covers REQ-01, the two implementation tasks REQ-02/REQ-03, the
benchmark REQ-04), so the coverage-preview gate sees every `[hard]` criterion
covered. `C2` and `C3` (omitted above for brevity) carry `"satisfies": ["REQ-02"]`
and `["REQ-03"]` respectively.
