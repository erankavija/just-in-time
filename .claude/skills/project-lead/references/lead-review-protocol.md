# Lead Review Protocol

The lead reviews every sub-agent's output before accepting it. This review has four tiers (1, 2, 2.5, 3), applied in order. A failure at any tier is an automatic FAIL — do not continue to later tiers.

## Tier 1: Gate Verification

Run `jit gate check-all <issue-id>` (or `jit issue show <issue-id> --json` and inspect the `gates` field).

- Every gate defined on the issue must show status `passed`.
- If any gate is `pending` or `failed`, the verdict is **FAIL**.
- The worker was responsible for passing all gates. An unpassed gate means the worker did not complete its job.

Note: The lead does NOT re-run the underlying checks (tests, lint, etc.). The gates are the source of truth. If a gate shows `passed`, trust it.

## Tier 2: Success Criteria Verification

Read each criterion from the issue's `## Success Criteria` section.

For each criterion:
1. Identify what the criterion requires (a concrete deliverable, behavior, or property).
2. Verify the work actually satisfies it by reading the agent's output — diffs, created files, test results, documents.
3. Do not trust the agent's self-assessment alone. Read the actual artifacts.

Mark each criterion as MET or UNMET. If any criterion is UNMET, the verdict is **FAIL**.

Common pitfalls to watch for:
- Agent claims criterion is met but the implementation is partial or superficial.
- Criterion requires a specific behavior but no test covers it.
- Criterion is about documentation but the doc was not created or is placeholder-only.

## Tier 2.5: Pre-close Stale-Narrative Sweep

Before rendering the Tier-3 verdict, sweep the whole workspace for stale references to the work this issue just landed. Long-running projects accumulate forward-looking narrative ("is planned", "future work", "once X lands") in docstrings, benchmark prose, and design docs. When the work lands, those references become lies. The reviewer will find them one or two per cycle unless the lead sweeps proactively.

Run, at minimum:

```
git grep -nE "future work.*<task-key-term>|<task-key-term> lands|pending <task-key-term>|upcoming <task-key-term>|is planned.*<task-key-term>"
```

Also grep for the issue's short ID and any aliases (function names, feature names) that external docs might reference as "future". Examples of terms worth sweeping on past work: a newly-added function name, a just-implemented algorithm name, a benchmark that replaces an old one.

If the sweep finds stale references, the verdict is **FAIL** with those references listed. The rework agent will be required to resolve every match in a single submission (see the Resolution table in `rework-prompt-template.md`) rather than discovering them one cycle at a time.

This tier is cheap to run — a single grep — and eliminates the single largest source of multi-cycle rework loops observed in practice.

## Tier 3: Holistic Coherence Review

This tier catches issues that per-issue gates and criteria cannot: problems that only emerge when looking at the epic as a whole.

### Cross-issue consistency
- **Naming**: Are the same concepts named the same way across different agents' output? If agent A calls it `user_profile` and agent B calls it `account_data` for the same thing, that's a coherence failure.
- **Style**: Does the output follow the same conventions as other completed work in this epic? Read the project's CLAUDE.md for authoritative conventions.
- **Interfaces**: If this issue produces something that other issues consume (an API, a data structure, a document section), does it match what consumers expect?

### Integration fitness
- Does this piece connect correctly with already-completed pieces?
- Are there implicit assumptions (about data formats, file locations, ordering, configuration) that conflict with other pieces?

### Documentation narrative
- If the epic involves documentation, does the overall doc story remain coherent with this addition?
- Are there contradictions between this doc and other docs produced for the same epic?

### Scope discipline
- Did the agent make changes outside the issue's scope? (Unrelated refactors, formatting changes to other files, modifications to other agents' work.)
- If out-of-scope changes exist, they should be reverted or split into a separate issue.

## Recording the Verdict

After review, record a structured verdict:

```
## Review: [ISSUE_TITLE] ([SHORT_ID])

**Verdict:** PASS | FAIL

### Gate status
[All passed / List of unpassed gates]

### Success criteria
- [x] Criterion 1 — [brief evidence]
- [ ] Criterion 2 — [what's missing]

### Holistic findings
[Any coherence issues, or "No issues found"]

### Required changes (if FAIL)
1. [Specific change needed with file/location reference]
2. [Next change]
```

This verdict is passed to the rework agent if the review fails (see `rework-prompt-template.md`).
