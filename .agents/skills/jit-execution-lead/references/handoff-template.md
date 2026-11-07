# Session Handoff Template

Write a handoff at the end of any session that does not complete the epic. Save it to `handoff.md` in the epic's artifact directory (`jit doc dir <epic-id> dev/active`), or `handoff-<N>.md` for later ones; never overwrite an earlier handoff.

Use dense bullets under these sections:

```
# Handoff — [EPIC_TITLE] ([SHORT_ID]) — session [N]

**Date:** [ISO-8601]
**Prior handoffs:** [paths]

## Current state
Epic state, wave N of M, children by state, active claims, open escalations.
`progress.json` reflects the same.

## What just happened
One bullet per significant action and its result.

## What to do next
Ordered, actionable next steps.

## Traps — do not repeat these
Each trap: the wrong approach, the evidence it is wrong (file:line or
incident), and optionally what to do instead.

## Open questions needing invoker input
Each: the question, one-line context, options, recommendation.

## Reference artefacts
Epic, design and planning docs, result artefacts.
```

## Traps

Traps is required. Fill it from the session's actual experience: approaches tried and rejected, misleading references in the handoff chain or spec docs, dispatch lines that led a worker astray (quote them and give the correct alternative). Write "None identified this session." only when nothing wrong was tried.

- Unresolved traps from earlier handoffs stay in force: link them, do not copy them. Remove a trap only when its cause is provably resolved.
- Put a trap above any command it qualifies, not in an inline comment.
- Keep the handoff under about 200 lines.
