# Session Handoff Template

Write a session handoff at the end of any session that does not complete the epic, so that the next-session lead (possibly a different agent or model) can resume with full context.

Save to `handoff.md` in the epic's artifact directory (`jit doc dir <epic-id> dev/active`), or `handoff-<N>.md` for subsequent handoffs — do not overwrite the earlier one.

Use exactly the sections below, in this order. Do not skip sections — if a section has nothing to report, write "None." rather than removing it. Every section serves a specific purpose for the next lead.

```
# Handoff — [EPIC_TITLE] ([SHORT_ID]) — session [N]

**Date:** [ISO-8601]
**Session number:** [N]
**Prior handoffs:** [list of prior handoff file paths, if any]

## Current state

- Epic: `[SHORT_ID]` — state: [backlog|ready|in_progress]
- Wave in progress: [wave N of M]
- Children summary: [X done, Y in_progress, Z backlog/ready, W rejected]
- Active claims: [list any issues currently claimed, with agent IDs and claim age]
- Open escalations: [list any escalations still awaiting invoker input]
- Progress file: `progress.json` in the epic's artifact directory, `jit doc dir <epic-id> dev/active` (reflects the above)

## What just happened

Concrete log of this session's actions. Bullet each significant action with a short result. Examples:

- Dispatched `<issue-a>` (implementation of X); review PASS on attempt 2 after fixing the named boundary case.
- Dispatched `<issue-b>` (docs for Y); review FAIL attempt 3; escalated per `escalation-policy.md`.
- Created child `<issue-c>` to resolve the open design decision discovered during review.

Do NOT write prose narrative; write dense bullets. The next lead reads this to reconstruct state, not to be entertained.

## What to do next

The specific next actions in order of priority. Each bullet must be actionable by the reader without further investigation.

- [ ] Resume wave [N]: dispatch issues [list] per the wave plan in the progress file.
- [ ] Check the escalation on `[SHORT_ID]` — the invoker's response may have arrived in [link/chat/parent-lead reply].
- [ ] Re-review `[SHORT_ID]` after rework (attempt [N]).

## Traps — do not repeat these

**This section is mandatory and must be populated from the session's experience — not left empty.**

List every wrong approach that was tried or considered during this session, and every false lead in the spec/handoff chain that misled a worker. Examples:

- **Do NOT update a generated file without its source.** A prior attempt edited
  the projection directly; the freshness gate reverted it. Change the registry
  or template, then render.
- **Do NOT dispatch two workers into the same central module.** The prior wave
  produced a needless merge conflict. Serialize those issues or split the
  shared prerequisite first.
- **Do NOT rely on a source-checkout helper from an installed workflow.** Use
  repository-relative installed assets or native JIT operations.

Each trap should state: (a) the wrong approach, (b) the evidence it's wrong (file/line or prior-incident reference), (c) optionally, what to do instead.

If no traps were identified this session, write: "None identified this session. Re-read prior handoffs' trap sections before dispatching." Traps from earlier handoffs remain in force until explicitly resolved.

## Open questions needing invoker input

List every question that blocks further progress and requires the invoker's decision (the human standalone, the parent lead when dispatched — see `escalation-policy.md`). One bullet per question; include a recommendation if you have one.

- Question: [exact question]
  - Context: [one-sentence summary]
  - Options: [A, B, C]
  - Recommendation: [preferred option + reason], or "No strong preference."

If no open questions, write "None."

## Reference artefacts

Links to the key docs, PRs, and JIT issues the next lead will need to load:

- Epic: `jit issue show [SHORT_ID]`
- Design docs: [paths]
- Planning docs: [paths]
- Benchmark/result artefacts: [paths]
- External references: [links, if the user has authorised them]
```

## Usage notes

- The **Traps** section is the most important part of the handoff. It is the first section a new-session lead must read after Current state. Traps prevent the next session from relitigating decisions that were already made the hard way.
- Traps accumulate across sessions — always carry forward unresolved traps from prior handoffs (link them; do not copy-paste), and add new ones from this session. Do not remove a trap unless the underlying issue has been provably resolved (e.g., the wrong-approach document was amended, the confused worker was replaced with an updated prompt).
- Do NOT embed commands or scripts in inline comments if they contain an approach that is wrong. If you must include a command that uses a specific algorithm or parameter, put the trap above the command, not as an inline "# note: this is wrong" comment — inline comments are routinely missed by the next worker.
- Keep each section tight. Handoffs that exceed ~200 lines of prose tend not to be read in full; the next lead skims. Dense bullets > prose.
