# Escalation Policy

## Constants

| Setting | Value | Rationale |
|---|---|---|
| MAX_REWORK_ATTEMPTS | 2 | Two retries (3 total attempts) gives enough signal. Beyond that, the problem is likely unclear requirements or a genuinely hard design issue. |

## Decision Tree

Before every non-trivial decision, run through this tree:

### ESCALATE — these require the invoking user's input

1. **Scope expansion: creating a story or higher-level type**
   The lead can autonomously create tasks and bugs (leaf work items). Creating stories or above implies cross-cutting scope that was not part of the original epic definition.

2. **Cross-epic dependency discovered**
   If completing this epic requires work tracked under a different epic, the lead cannot unilaterally modify another epic's scope. Surface the dependency and let the user decide how to handle it.

3. **Epic success criteria need modification**
   The success criteria define the contract with whoever assigned the epic. Changing them changes the deliverable. The user must approve.

4. **Any issue scope change (gates, criteria, description)**
   Modifying an issue's quality gates, success criteria, description, or other scope-defining attributes is a scope change. This includes removing gates, changing gate modes, or weakening criteria. Always escalate — even when the change appears to be a false positive or out-of-scope judgment by an automated reviewer. Present the gate failure, your analysis of why it may be incorrect, and let the user decide.

5. **Rework exceeded MAX_REWORK_ATTEMPTS**
   Repeated failure after specific feedback suggests the requirements are ambiguous, the task is harder than scoped, or there's a systemic issue. Present the full history and let the user decide: provide guidance, take over, or reject.

6. **Architectural decision with significant trade-offs**
   When multiple valid approaches exist and the choice has lasting consequences (data model shape, public API surface, integration patterns), the user should make the call. Routine implementation choices (internal data structures, local algorithms) are fine to make autonomously.

7. **Blocker outside this epic's scope**
   Infrastructure issues, permissions, access to external systems, or dependencies on work owned by others. The lead cannot resolve these alone.

8. **Changes to shared infrastructure**
   CI/CD configuration, test frameworks, project-wide configuration, build tooling — anything that affects work beyond this epic. Even if the change is small, the blast radius is large.

### HANDLE AUTONOMOUSLY — no escalation needed

- Creating tasks and bugs within the epic
- Choosing implementation approach for a task (when there's a clearly better option)
- Deciding file/document structure within established project patterns
- Reworking failed outputs (up to MAX_REWORK_ATTEMPTS)
- Reordering waves when blockers resolve or new issues are discovered
- Adding gates to child issues
- Creating design docs and linking them to issues
- Prioritizing within the epic (which wave items to tackle first)

## Escalation Prompt Template

When escalating, present to the user:

```
## Escalation: [CATEGORY from decision tree]

**Epic:** [EPIC_TITLE] ([SHORT_ID])
**Issue:** [ISSUE_TITLE] ([SHORT_ID]) (if applicable)

### Situation
[What happened — concrete facts, not interpretation]

### Options
1. [Option A] — [trade-offs]
2. [Option B] — [trade-offs]

### Lead's recommendation
[If the lead has a preference, state it and why. Otherwise: "No strong recommendation — this depends on factors outside the epic's scope."]

### What I need from you
[Specific decision or action needed to unblock]
```

Keep escalations concise. The user's time is the scarcest resource.
