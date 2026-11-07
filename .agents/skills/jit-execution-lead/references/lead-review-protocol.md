# Lead Review Protocol

Review every worker's output before accepting it. Run the five checks in order; any failure is a **FAIL**. Run all five on every round, including after rework, and before dispatching any rework.

## 1. Gates

`jit gate status-all <issue-id>` reports readiness from recorded runs and runs no checker. Every gate on the issue must be `passed`. Trust a recorded pass; do not re-run the underlying checks. Evaluating a gate (`jit gate evaluate <issue-id> <gate-key>`) is the lead's job.

## 2. Prior findings

From the second round on, list every finding from every failed run of the review gate (`code-review` in this repository's configuration):

```bash
jit gate status <issue-id> <review-gate> --all --status failed --json \
  | jq -r '.results[].findings.findings[] | "[\(.severity)] \(.id // "-"): \(.summary) (\(.file // "-"):\(.line // "-"))"'
```

Record each finding's status at HEAD with a file:line cite. A finding closed in an earlier round and open again is a **regression**: it outranks every new finding, and the rework prompt must require a full audit.

## 3. Success criteria

Check each criterion against the artifacts (diffs, files, test output), not the worker's report. Mark each MET or UNMET.

## 4. Stale narrative and deferred scope

- **Stale narrative.** Grep the workspace for forward-looking references to the landed work, using the issue's short ID and the names it introduced:

  ```bash
  git grep -nE "future work.*<term>|<term> lands|pending <term>|upcoming <term>|is planned.*<term>"
  ```

  Each match is a FAIL; the rework must resolve every match in one submission.

- **Deferred scope.** Grep every linked doc (`jit doc list <issue-id>`) for punted work:

  ```bash
  grep -inE '\b(deferred|todo|future work|open question|not (yet )?implemented|follow-?up|out of scope|won'\''t (do|fix)|stays (available|open).*generic|punt(ed)?)\b' <linked-path>
  ```

  Categorize each match:
  - **In scope but deferred:** the issue's description or criteria promise it. FAIL; cite the match and quote the promise.
  - **Out of scope:** the issue's Non-goals defer it. Record the mapping in the verdict.
  - **Cross-doc inconsistency:** a doc promises behavior the code does not deliver, or the reverse. FAIL, even under Non-goals.

## 5. Coherence across the epic

- The same concepts carry the same names and conventions as the epic's other completed work.
- Interfaces this issue produces match what their consumers expect, and no assumption about formats, locations, ordering, or configuration conflicts with completed pieces.
- Docs stay consistent with the epic's other docs. Every `@/<kind>/<self-id>` citation the work introduces resolves via `jit item show`; a dangling citation is a FAIL.
- Changes outside the issue's scope are reverted or split into a separate issue.

## Before a container's final gate: reconcile pitfalls against criteria

A container review (a story checkpoint, or the container's own completion gate) reconciles the findings the lead deferred during execution, recorded in the progress file's `surfaced_pitfalls` (`progress-file.md`), against the container's criteria. Recording a finding there does not make it out of scope.

1. Re-read every `[hard]` criterion on the container.
2. Read every entry in `surfaced_pitfalls`.
3. A pitfall whose subject a criterion names is a criterion violation, not a follow-up. Close it before the gate runs.

A plan that scoped a criterion's subject out of the fan-out binds the fan-out shape only; it cannot make an unmet criterion met. When a container gate fails on a criterion the plan deferred, create a remediation task inside the container and close the criterion.

## Rework dispatch

The rework prompt directs the worker to read the **complete** verdict and account for **every** numbered finding before re-requesting the gate.

## No-argue discipline

When a reviewer cites a contract document (acceptance protocol, design doc, governance file) and says the artifact violates it, the lead either changes the artifact to satisfy the contract literally or escalates. Softening the contract's reading by argument is forbidden; wording such as "non-blocking marker", "redundant witness", or "the contract doesn't really require this for our case" signals argue mode. If the contract is wrong for the project, amend the contract. This applies even when the reviewer's reading looks overly literal.

## Recording the verdict

```
## Review: [ISSUE_TITLE] ([SHORT_ID]) — PASS | FAIL

- Gates: [all passed / unpassed gates]
- Prior findings (from round 2):
  | Round | Finding (1-line) | Status at HEAD | file:line |
- Criteria: one line each, MET/UNMET with evidence
- Stale narrative: [none / matches]
- Deferred scope: [each match and its category]
- Coherence: [findings / none]
- Required changes (FAIL): numbered, each with a file or location
```

On FAIL, pass the verdict to the rework agent (`rework-prompt-template.md`).
