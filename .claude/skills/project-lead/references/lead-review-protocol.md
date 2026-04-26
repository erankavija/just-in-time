# Lead Review Protocol

The lead reviews every sub-agent's output before accepting it. This review has six tiers (1, 1.5, 2, 2.5, 2.75, 3), applied in order. A failure at any tier is an automatic FAIL — do not continue to later tiers.

## Tier 1: Gate Verification

Run `jit gate check-all <issue-id>` (or `jit issue show <issue-id> --json` and inspect the `gates` field).

- Every gate defined on the issue must show status `passed`.
- If any gate is `pending` or `failed`, the verdict is **FAIL**.
- The worker was responsible for passing all gates. An unpassed gate means the worker did not complete its job.

Note: The lead does NOT re-run the underlying checks (tests, lint, etc.). The gates are the source of truth. If a gate shows `passed`, trust it.

## Tier 1.5: Prior-findings regression check

If this issue has previously failed code-review (any prior `code-review` gate run with `status: failed`), extract every prior finding and independently verify each is still closed at HEAD **before** evaluating new findings. Skip this tier only on the first review of an issue.

Run:

```bash
# Enumerate prior failing runs for this issue's code-review gate
jit gate runs <issue-id> --gate code-review --json \
  | jq -r '.runs[] | select(.status=="failed") | .run_id' \
  | while read run; do
      echo "=== run $run ==="
      cat .jit/gate-runs/$run/result.json \
        | jq -r '.stdout' \
        | grep -E '^(\*\*(Problem|Fail):|##\s+Issue:)'
    done
```

Build a **cumulative resolution table** with one row per finding across all rounds, each with a file:line cite at HEAD. The resolution table must be part of your review verdict; see "Recording the Verdict" below.

A **regression** — a finding that was closed in a prior round and is open again at HEAD — is a higher-priority FAIL than any new finding. Regressions indicate the worker's latest commit reverted or re-broke prior fixes, and the rework prompt must force complete audit on the next attempt.

This tier exists because reviewers (AI or human) do not remember prior rounds. Without explicit regression-checking, later rounds surface a different subset of findings each time, and the overall issue contract drifts round by round.

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

## Tier 2.75: Deferred-items audit

Open every design doc linked to this issue (`jit doc list <issue-id>`). In each doc, grep for patterns that signal the worker punted on a surface rather than completing it:

```bash
for doc in $(jit doc list <issue-id> --json | jq -r '.documents[].path'); do
  echo "=== $doc ==="
  grep -inE '\b(deferred|todo|future work|open question|not (yet )?implemented|follow-?up|out of scope|won'\''t (do|fix)|stays (available|open).*generic|punt(ed)?)\b' "$doc"
done
```

For each match, categorize:

- **In-scope-but-deferred** — worker punted on something the issue's description (`## API surface`, `## Success Criteria`, or equivalent) promises. Automatic **FAIL**; cite the match file:line and quote the promise from the issue description.
- **Legitimately out-of-scope** — the issue's Non-goals section explicitly defers this item to another issue (by short-id or by topic). Document the match → Non-goals mapping in your verdict. OK to pass this tier.
- **Cross-doc inconsistency** — the design doc promises behavior the code does not deliver (or vice versa). This is also an automatic **FAIL** even if the item is technically covered by Non-goals, because inconsistent docs will mislead downstream workers.

This tier exists because workers who cannot complete a surface often document the deferral in their design doc instead of flagging it to the lead. That deferral must not survive the review.

Previous incident: In ab791e27 (FieldMatrix foundation), the worker's design doc §8 explicitly listed "Scalar Mul<F> for non-Fp fields" as deferred across three rework attempts. The code-review gate sampled a different subset of findings each round and only surfaced this gap in round 5 of 6. Had the lead run this deferred-items grep after each rework, the gap would have been caught in round 2 and closed in round 3 at the latest.

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

### Prior-findings regression table (Tier 1.5)
| Round | Finding (1-line) | Status at HEAD | file:line |
|-------|------------------|----------------|-----------|
| R1    | …                | closed / REGRESSED | … |
| R2    | …                | closed / REGRESSED | … |
[Omit this table only on the first review of an issue.]

### Success criteria
- [x] Criterion 1 — [brief evidence]
- [ ] Criterion 2 — [what's missing]

### Stale-narrative sweep (Tier 2.5)
[Zero matches / list of stale references]

### Deferred-items audit (Tier 2.75)
[For each design doc, list any "deferred" / "TODO" / "open question" matches and categorize each as in-scope-FAIL, out-of-scope-OK, or cross-doc-inconsistency-FAIL.]

### Holistic findings
[Any coherence issues, or "No issues found"]

### Required changes (if FAIL)
1. [Specific change needed with file/location reference]
2. [Next change]
```

This verdict is passed to the rework agent if the review fails (see `rework-prompt-template.md`).
