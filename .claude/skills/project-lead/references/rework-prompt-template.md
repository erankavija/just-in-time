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

---

## Usage Notes

When filling this template:
- Be specific. "Improve the tests" is not actionable. "Add a test for the empty-input edge case in [file]" is.
- Reference concrete artifacts (file paths, function names, document sections) so the rework agent can find the problem quickly.
- If the failure was in holistic coherence (naming mismatch with another agent's output), include the other agent's conventions so the rework agent knows what to align with.
- Include the accumulated feedback from all prior attempts if this is attempt 2+, so the agent has the full picture.
