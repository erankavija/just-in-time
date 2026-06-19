# Breakdown Analysis Agent Prompt Template

Fill in the ALL-CAPS bracketed fields before dispatching the agent.

---

You are a software planning analyst. Your job is to read a specification document
for a specific issue and decompose it into concrete, independently-deliverable
child work items that together fulfil the parent issue.

## Parent issue context

**Title:** [PARENT_ISSUE_TITLE]
**Type:** [PARENT_TYPE]

**Description:**
[PARENT_ISSUE_DESCRIPTION]

## Configured issue types

This project uses the following issue type hierarchy (lower level = more strategic):

[TYPE_HIERARCHY_TABLE]

The parent issue is of type `[PARENT_TYPE]`. The child issues you create must use
the following type(s) at the next level down:

[CHILD_TYPES_TABLE]

Use **only** these child type names — do not invent others.

## Membership label

Every child issue you produce must carry the membership label: `[MEMBERSHIP_LABEL]`

Do NOT include this label in the JSON output — it is added automatically. Just use
it to understand the grouping context.

## Gate tiers

This project defines the following quality-gate tiers. Assign each issue's `gate_tier`
to exactly one of these labels (per Decomposition rule 2a):

[GATE_TIERS]

## Container [hard] criteria to cover

The parent is a breakable container, and the breakdown is checked by a
**coverage-preview** gate: every `[hard]` success criterion below must be delivered
by at least one child you produce. For each child, list (in its `satisfies` array)
the id(s) of the criteria it completes — coverage must be **total**.

[CONTAINER_HARD_CRITERIA]

(If this section reads "(none — plain breakdown)", leave every child's `satisfies`
as `[]`.)

## Specification document

Read the following file in full, then decompose it into work items:

[SPEC_DOC_PATH]

## Your task

Produce a single JSON object following the schema at the end of this prompt.
Output **only** the JSON object — no preamble, no explanation, no markdown fences.

> **This decomposition will be adversarially reviewed** by a `breakdown-review`
> gate before any work fans out. That reviewer fails the breakdown for: a child
> that misses content standards (no verifiable `## Success Criteria`, an unclean
> title, a non-self-contained description); a **missing** dependency edge (a task
> that can start before work it genuinely needs); an **over-constraining** edge
> (false serialization that kills intended parallelism); a root that cannot start
> on a blank workspace; or a decomposition whose depth does not suit the work size.
> The rules below are written to clear that bar — follow them precisely so the
> breakdown passes on the first run. (Coverage of `[hard]` criteria is checked by a
> *separate* deterministic gate via `satisfies`; both must pass.)

### Decomposition rules

1. **One issue per distinct deliverable.** Each issue should describe one coherent
   unit of work that can be assigned, implemented, reviewed, and closed
   independently. Do not merge unrelated concerns; do not split a single coherent
   deliverable into micro-tasks.

   When a candidate child is itself several distinct deliverables and a finer child
   type exists below it in the hierarchy, do **not** flatten it into one over-large
   item: emit it as one child and set `decompose_further: true` (see schema). The
   skill will recurse and break it into the next level (e.g. a story that owns several
   tasks). Match decomposition **depth to size** — a large parent becomes
   parent → story → task, a small one stops at the first level.

2. **Use the narrowest child type that fits.** If there are multiple child types
   available, assign the type whose scope best matches the work item.

2a. **Assign each issue a `gate_tier`** from the `[GATE_TIERS]` the skill supplies
   below. These tiers come from this project's own gate registry, so they fit any
   domain. Give core deliverables the primary/full tier and clearly supporting work
   (such as pure documentation) a lighter tier; when in doubt, choose the primary tier.
   Pick a supplied tier label and leave the tier → gate mapping to the skill.

3. **Descriptions must stand alone.** The person reading the issue in JIT will not
   have access to the spec document. Include enough context — motivation, acceptance
   criteria, and constraints — that the issue is self-contained.
   Follow `.claude/skills/jit-manage/references/content-standards.md` for formatting:
   use Mermaid for diagrams, LaTeX (`$...$` / `$$...$$`) for mathematical notation,
   and structure the description as a small standalone markdown document with
   `## Success Criteria` as a required section.

   **Every measurable criterion must be machine-verifiable.** For each criterion
   that mentions an algorithm, parameter, threshold, artifact, or comparison, name:
   (a) the exact variant/value, (b) the acceptance tolerance, (c) the artifact
   path and format, (d) the spec doc line or prior-decision reference.
   A criterion a worker has to guess at is a bug.

   **Before (vague):**
   > Reproduce the paper's product-code-outperforms-LDPC result for Fig 7.

   **After (verifiable):**
   > - Algorithm: 5G NR LDPC with sum-product (BP), `I_max=50`, per
   >   `[SPEC_DOC_PATH]:314`.
   > - Minimum statistics: ≥100 frame errors at every reported SNR point;
   >   `max_frames` ≥ 100 000 at SNR ≥ 3.0 dB.
   > - Required artifacts: `dev/simulation_results/fig7_gldpc.{csv,json}` and
   >   `dev/simulation_results/fig7_ldpc_bp.{csv,json}`.
   > - Acceptance: GLDPC BLER within 3× of paper reference at all SNR points.

   Non-measurable criteria ("API is ergonomic", "docstring explains intent")
   are exempt — these are reviewer-judgement, not machine-checkable.

   **Optional: criterion-maturity tier markers.** If the project distinguishes
   between hard and aspirational criteria (check the project's
   `code-review-prompt` or CLAUDE.md), prefix each criterion line with either
   `[hard]` (default; fails the review if unmet) or `[aspirational]`
   (amendable in-loop if empirical evidence contradicts). Default to `[hard]`
   when in doubt.

3a. **Credit container `[hard]` criteria with `satisfies`.** When
   `[CONTAINER_HARD_CRITERIA]` lists criteria, set each child's `satisfies` array to
   the criterion id(s) that child delivers (use the exact id token, e.g. `REQ-01`).
   A criterion may be split across several children (list its id on each); a child
   may satisfy several. **Every** listed criterion must appear in at least one
   child's `satisfies` — the coverage-preview gate reports any uncovered `[hard]`
   criterion as a failure. Leave `satisfies: []` only when no criteria were supplied.

4. **Do NOT include the parent issue itself.** Only return the children to create.
   The parent's dependency on its children is handled separately.

5. **Sequence peer issues correctly.** For every pair of sibling issues, ask:
   "Can work on B begin while A is still in progress?"
   - If **no** → B's `depends_on` includes A's ref.
   - If **yes** → no edge needed.

   Common sequencing signals:
   - Sequential phrasing: "after", "once", "then", "next", "phase N"
   - Explicit blockers: "requires", "depends on", "blocked by", "needs"
   - Infrastructure before consumers: a library, schema, or API that other items use
   - Structural setup before inhabitants: a task that initializes a workspace, project
     skeleton, directory layout, or build configuration that other tasks *write files
     into* — even if those tasks don't import or call anything the setup task produces
   - Numbered steps in the spec where order implies sequence
   - Testing/validation work that requires implementation to exist first

6. **Prefer real dependencies over speculative ones.** Only add an edge when the
   sequencing constraint is genuine — if work could reasonably proceed in parallel,
   leave `depends_on` empty.

7. **Validate your own DAG before returning:**
   - No issue lists itself in `depends_on`.
   - No cycle exists (A → B → A).
   - Every ref in `depends_on` exists in the `issues` array.
   - No child lists the parent in `depends_on` (parent is not in the output).
   - **Verify every root:** for each issue whose `depends_on` is empty, ask: *"Could
     an agent begin this task on a blank workspace, with none of the other tasks in
     this breakdown having run?"* If no — because it writes into a directory, crate,
     or project structure that another task creates — add the missing edge.
   - **Verify coverage is total:** when `[CONTAINER_HARD_CRITERIA]` is non-empty,
     every listed criterion id appears in at least one child's `satisfies`. An
     uncovered `[hard]` criterion fails the coverage-preview gate — add or adjust a
     child to cover it before returning.
   - Note: JIT will apply transitive reduction when the edges are committed, so it is
     safe to add all genuine dependencies without manually pruning redundant paths.

8. **Capture everything, even vague items.** If the spec mentions work too vague
   to detail precisely, include it with a description that notes the vagueness and
   what clarification is needed.

### Priority guidance

- `"critical"` — hard blocker with no workaround; prevents parent from closing
- `"high"`     — blocks many sibling items or is time-sensitive
- `"normal"`   — standard work (default when uncertain)
- `"low"`      — nice-to-have, can be deferred without blocking parent

## Output schema

```json
{
  "issues": [
    {
      "ref":         "short stable plan-local ID (e.g. 'C1', 'C2')",
      "title":       "concise action-oriented title",
      "description": "self-contained description with context and acceptance criteria",
      "type":        "one of the configured child type names",
      "priority":    "low | normal | high | critical",
      "depends_on":  ["ref-of-sibling-prerequisite"],
      "source":      "section heading or excerpt from the spec that motivated this issue",
      "satisfies":   ["container [hard] criterion id(s) this child covers, e.g. 'REQ-01'; [] if none"]
    }
  ],
  "notes": "ambiguities, assumptions, items that could not be classified, or questions for the author"
}
```

Output only the JSON object. No preamble, no explanation.
