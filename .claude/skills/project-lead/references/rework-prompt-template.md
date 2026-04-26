# Rework Prompt Template

Prepend this to the original agent dispatch prompt when sending work back for revision.

---

## Rework Required (attempt [N] of [MAX_REWORK_ATTEMPTS])

Previous work on issue **[ISSUE_TITLE]** ([SHORT_ID]) failed the lead's quality review. The specific failures are listed below. Fix **only** the listed issues (plus anything surfaced by the mandatory pre-commit audit below). Do not refactor unrelated areas, do not change the overall approach unless the feedback specifically requires it.

### Required pre-commit audit (attempt [N] must include this)

**Before editing any source**, run the following and include the raw output at the top of your final report. The lead cross-references this output against your resolution table; any omission is a rejection.

```bash
# 1. Enumerate every prior code-review failure for this issue
jit gate runs [SHORT_ID] --gate code-review --json \
  | jq -r '.runs[] | select(.status=="failed") | .run_id' \
  | while read run; do
      echo "=== prior run $run ==="
      cat .jit/gate-runs/$run/result.json \
        | jq -r '.stdout' \
        | grep -E '^(\*\*(Problem|Fail):|##\s+Issue:|\- \*\*Fail)'
    done

# 2. List every design doc the worker produced for this issue
jit doc list [SHORT_ID]

# 3. Grep each linked doc for deferred-item markers
for doc in $(jit doc list [SHORT_ID] --json | jq -r '.documents[].path'); do
  echo "=== $doc ==="
  grep -inE '\b(deferred|todo|future work|open question|not (yet )?implemented|follow-?up|out of scope|won'\''t (do|fix)|stays (available|open).*generic|punt(ed)?)\b' "$doc"
done
```

**For every match** from steps 1 and 3, a row MUST appear in the resolution table below. Step 1 gives you every finding that has ever been raised, not just the latest round — you must show closure at HEAD for all of them. Step 3 gives you the worker's own deferred-item notes, which are silent scope gaps the reviewer will catch sooner or later.

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

Before submitting, produce one row per finding **across all rounds plus every match from the pre-commit audit**. Submissions with any empty row, or any finding from a prior round without a closure cite, are rejected without re-review.

| # | Round | Source (reviewer / audit-step-1 / audit-step-3) | Finding (paste verbatim) | Resolution (file:line or commit SHA proving closure at HEAD) |
|---|-------|--------------------------------------------------|--------------------------|---------------------------------------------------------------|
| 1 | R1    | reviewer                                         | <paste finding>          | <file:line>                                                   |
| 2 | R2    | reviewer                                         | <paste finding>          | <file:line>                                                   |
| 3 | —     | audit-step-3 (design doc §N)                     | <paste deferred item>    | <either closure file:line OR "OUT-OF-SCOPE per issue Non-goals §…"> |
| … |       |                                                  |                          |                                                               |

A prior-round finding without a closure cite at HEAD = regression. A design-doc deferred-item without either a closure cite or an explicit "OUT-OF-SCOPE per issue Non-goals" citation = silent scope gap. Both are rejected.

### Mandatory workspace-wide sweeps

Reviewers commonly flag one instance of a problem that actually exists in several files. Before filling the resolution table, run each of the following searches and paste the raw results. Fix **every** match — not just the one the reviewer cited.

**Sweep 1 — Prior-findings cumulative.** Run the pre-commit audit block above. Every finding from every prior round must have a closure cite.

**Sweep 2 — Design-doc deferred-items.** Run the grep from audit step 3. Every match must be categorized.

**Sweep 3 — Per-finding workspace grep.** For each reviewer finding, run:
```
rg -n "<key phrase from the finding>"  <source roots>
rg -n "<stale claim from the finding>" <docs/bench roots>

Replace <source roots> and <docs/bench roots> with project-appropriate paths
(e.g., crates/ benches/ dev/ docs/ for a Cargo workspace).
```

A finding is not resolved until sweeps 1, 2, and 3 all return zero unfixed matches. If sweeps surface additional instances the reviewer did not cite, add them to the resolution table as additional rows and fix them in the same submission.

---

## Usage Notes

When filling this template:
- Be specific. "Improve the tests" is not actionable. "Add a test for the empty-input edge case in [file]" is.
- Reference concrete artifacts (file paths, function names, document sections) so the rework agent can find the problem quickly.
- If the failure was in holistic coherence (naming mismatch with another agent's output), include the other agent's conventions so the rework agent knows what to align with.
- Include the accumulated feedback from all prior attempts if this is attempt 2+, so the agent has the full picture.
- The resolution-table and workspace-sweep sections are mandatory. They exist because "fix the cited instance, miss the identical issue in three other files" is the single most common cause of multi-cycle rework loops. Do not remove them when filling the template.
