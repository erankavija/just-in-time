# Session Handoff Template

Write a session handoff at the end of any session that does not complete the epic, so that the next-session lead (possibly a different agent or model) can resume with full context.

Save to the project's managed docs path (typically `dev/active/<epic-short-id>-handoff.md`, or `dev/active/<epic-short-id>-handoff-<N>.md` for subsequent handoffs — do not overwrite the earlier one).

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
- Open escalations: [list any escalations still awaiting user input]
- Progress file: `dev/active/[SHORT_ID]-progress.json` (reflects the above)

## What just happened

Concrete log of this session's actions. Bullet each significant action with a short result. Examples:

- Dispatched `abc12345` (implementation of X); review PASS on attempt 2 after fixing [specific finding].
- Dispatched `def67890` (docs for Y); review FAIL attempt 3; escalated per `escalation-policy.md` entry 5.
- Created child `ghi01234` (decision task: resolve LDPC variant) in Section 4.3 open-question sweep.

Do NOT write prose narrative; write dense bullets. The next lead reads this to reconstruct state, not to be entertained.

## What to do next

The specific next actions in order of priority. Each bullet must be actionable by the reader without further investigation.

- [ ] Resume wave [N]: dispatch issues [list] per the wave plan in the progress file.
- [ ] Check the escalation on `[SHORT_ID]` — user response may have arrived in [link/chat].
- [ ] Re-review `[SHORT_ID]` after rework (attempt [N]).

## Traps — do not repeat these

**This section is mandatory and must be populated from the session's experience — not left empty.**

List every wrong approach that was tried or considered during this session, and every false lead in the spec/handoff chain that misled a worker. Examples:

- **Do NOT use `NMS` for the LDPC baseline.** The earlier planning doc (`dev/plans/<path>:Step 1 script`) embedded `NMS` in a shell comment as if it were correct; it is not. The paper requires BP (`dev/plans/<path>:314`). A prior session lost 5 review cycles to this. If the next lead sees `NMS` anywhere in a dispatch prompt or handoff script, replace it with `BP` before dispatching.
- **Do NOT attach Kani harnesses to the test-copied `scalar_clmul` helpers.** They must attach to the production `gf2-core::gf2m::barrett::clmul` path (see `crates/gf2-core/src/gf2m/barrett.rs:94`). Eight review cycles were lost to this on `8889e712`.
- **Do NOT implement ZMM (AVX-512) lanes on MSRV 1.80.** `_mm512_clmulepi64_epi128` and `_mm512_extracti64x2_epi64` require unstable features (stabilised in 1.89). The reference host is Zen 3 (no AVX-512 hardware) — the lane cannot even be exercised. Explicit scope reduction to YMM-only was approved for epic `e095a100`.

Each trap should state: (a) the wrong approach, (b) the evidence it's wrong (file/line or prior-incident reference), (c) optionally, what to do instead.

If no traps were identified this session, write: "None identified this session. Re-read prior handoffs' trap sections before dispatching." Traps from earlier handoffs remain in force until explicitly resolved.

## Open questions needing user input

List every question that blocks further progress and requires the user's decision. One bullet per question; include a recommendation if you have one.

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
