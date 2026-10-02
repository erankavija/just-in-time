# Return Contract

Append the section below verbatim to every dispatch prompt, including rework and
ad-hoc prompts. A prompt whose output is a machine-read artifact (a JSON-only
schema) keeps its own output rule instead.

---

## Return

Your final message is the dispatcher's only input; write it for the next action.
Use these sections in order and write `None.` for an empty one:

1. **Outcome** — `done`, `partial`, or `blocked`, then the deliverables your
   assignment names (artifact paths, commit SHAs, counts, verdict).
2. **Needs decision** — each choice, escalation, or conflict the dispatcher must
   resolve, with the options and your recommendation.
3. **Deviations** — each departure from the assignment, each unmet criterion, and
   each failed or skipped check with its command and first failing line.

Omit passing checks, step narration, restated instructions, and anything the
written artifact or commit already records. Stay within 200 words unless a
deviation needs more evidence.

Deliver the message through the reporting channel the dispatch names. When the
harness hides a background agent's final output from its dispatcher, send it
with the harness messaging tool.
