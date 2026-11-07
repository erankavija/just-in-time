# Rework Prompt Template

Prepend this to the original dispatch prompt when sending work back for revision.

---

## Rework Required (attempt [N] of [MAX_REWORK_ATTEMPTS])

Previous work on issue **[ISSUE_TITLE]** ([SHORT_ID]) failed the lead's review. Fix the findings below and every finding the audit surfaces. Keep the approach unless a finding requires changing it, and leave unrelated areas alone.

### Audit before editing

The lead reruns these and checks your resolution table against them.

```bash
# 1. Every finding from every failed review run
jit gate status [SHORT_ID] [REVIEW_GATE] --all --status failed --json \
  | jq -r '.results[].findings.findings[] | "[\(.severity)] \(.id // "-"): \(.summary) (\(.file // "-"):\(.line // "-"))"'

# 2. Deferred-item markers in each linked doc (paths from `jit doc list [SHORT_ID]`)
grep -inE '\b(deferred|todo|future work|open question|not (yet )?implemented|follow-?up|out of scope|won'\''t (do|fix)|stays (available|open).*generic|punt(ed)?)\b' [LINKED_PATH]
```

For each finding, also search the workspace for the same defect (`rg -n "<key phrase>" <source and doc roots>`) and fix every instance, not just the cited one.

### Review verdict

[PASTE THE FULL REVIEW VERDICT FROM lead-review-protocol.md HERE]

Qualified addresses (`@/…`) in the verdict and issue resolve with `jit item show`.

### What to fix

[FOR EACH REQUIRED CHANGE FROM THE REVIEW:]

**[N]. [Brief description of the problem]**
- Location: [file path, section, or artifact reference]
- What's wrong: [concrete deficiency]
- What's expected: [what the lead wants instead]

### Constraints

- All gates on this issue must still pass after your changes.
- If a required change is incorrect or impossible, state why under Needs decision.

### Resolution table (required)

The resolution table and the commit SHA are the Outcome deliverables of your return. Give one row per finding from every round, every audit match, and every extra instance the workspace search found. A submission with an empty row is rejected without re-review.

| # | Round | Source (reviewer / audit-1 / audit-2 / search) | Finding (verbatim) | Resolution (file:line or commit SHA at HEAD) |
|---|-------|------------------------------------------------|--------------------|----------------------------------------------|
| 1 | R1    | reviewer                                       | <finding>          | <file:line>                                  |
| 2 | —     | audit-2 (design doc §N)                        | <deferred item>    | <file:line, or "OUT-OF-SCOPE per issue Non-goals §…"> |

A prior-round finding without a closure cite at HEAD is a regression. A deferred item without a closure cite or a Non-goals citation is a scope gap. Both are rejected.

---

## Usage notes

- Make each required change actionable: name the file, function, or section, and the expected result.
- For a coherence failure, include the conventions of the work it must align with.
- From attempt 2 on, include the feedback from every prior attempt.
- Keep the audit, workspace search, and resolution table when filling the template.
