# Rework Prompt Template

Prepend this to the original agent dispatch prompt when sending work back for revision.

---

## Rework Required (attempt [N] of [MAX_REWORK_ATTEMPTS])

Previous work on issue **[ISSUE_TITLE]** ([SHORT_ID]) failed the lead's quality review. The specific failures are listed below. Fix **only** the listed issues. Do not refactor unrelated areas, do not change the overall approach unless the feedback specifically requires it.

### Review verdict

[PASTE THE FULL REVIEW VERDICT FROM lead-review-protocol.md HERE]

### What to fix

[FOR EACH REQUIRED CHANGE FROM THE REVIEW:]

**[N]. [Brief description of the problem]**
- Location: [file path, section, or artifact reference]
- What's wrong: [concrete description of the deficiency]
- What's expected: [what the lead wants to see instead]

### Constraints

- Fix only the listed issues. Unrelated changes will be flagged in re-review.
- All gates on this issue must still pass after your changes.
- If you believe a required change is incorrect or impossible, explain why clearly rather than silently ignoring it.
- When done, confirm which items you addressed and how.

### Resolution table (required — do not submit without completing)

Before submitting, produce one row per reviewer finding. Submissions with any empty row are rejected without re-review.

| # | Reviewer finding (paste verbatim) | Resolution (file:line or commit SHA proving fix) |
|---|-----------------------------------|--------------------------------------------------|
| 1 | <paste finding 1>                 | <file:line>                                      |
| 2 | <paste finding 2>                 | <file:line>                                      |
| … |                                   |                                                  |

### Mandatory workspace-wide sweeps

Reviewers commonly flag one instance of a problem that actually exists in several files. Before filling the resolution table, run each of the following searches and paste the raw results. Fix **every** match — not just the one the reviewer cited.

```
For each reviewer finding, run and paste results for BOTH:
  rg -n "<key phrase from the finding>"  <source roots>
  rg -n "<stale claim from the finding>" <docs/bench roots>

Replace <source roots> and <docs/bench roots> with project-appropriate paths
(e.g., crates/ benches/ dev/ docs/ for a Cargo workspace).
```

A finding is not resolved until the workspace-wide sweep returns zero unfixed matches. If the sweep finds additional instances the reviewer did not cite, add them to the resolution table as additional rows and fix them in the same submission.

---

## Usage Notes

When filling this template:
- Be specific. "Improve the tests" is not actionable. "Add a test for the empty-input edge case in [file]" is.
- Reference concrete artifacts (file paths, function names, document sections) so the rework agent can find the problem quickly.
- If the failure was in holistic coherence (naming mismatch with another agent's output), include the other agent's conventions so the rework agent knows what to align with.
- Include the accumulated feedback from all prior attempts if this is attempt 2+, so the agent has the full picture.
- The resolution-table and workspace-sweep sections are mandatory. They exist because "fix the cited instance, miss the identical issue in three other files" is the single most common cause of multi-cycle rework loops. Do not remove them when filling the template.
