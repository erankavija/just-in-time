# Charter: {{STRATEGIC_CONTAINER_TITLE}}

> Strategic container: {{STRATEGIC_CONTAINER_SHORT_ID}} · {{MEMBERSHIP_LABEL}}

<!--
Instantiation (see references/vision-charter.md):
  - Write this to <permanent-root>/{{STRATEGIC_CONTAINER_SHORT_ID}}-charter.md,
    where <permanent-root> is the first entry of `permanent_paths` in
    `.jit/config.toml` [documentation]. Never hardcode the directory.
  - Fill the Vision, then add one ### D-N per consequential decision.
  - Link it: jit doc add {{STRATEGIC_CONTAINER_SHORT_ID}} <charter-path>
             --doc-type design --label "Project charter"
  - On resume, read this back in full before deciding anything new. Decisions
    below are binding; append new D-N entries, never edit landed ones.
  - Remove this comment block once instantiated.
-->

## Vision

{{VISION_STATEMENT}}

<!--
Two to five sentences: what this project is for, the outcome it must deliver,
and the standard by which a sub-strategic container's work is judged coherent
with it. This is the yardstick for accepting containers and resolving
escalations. Keep it short and load-bearing.
-->

## Decision Log

<!--
One ### D-N per consequential decision, ascending id order, append-only.
Every entry names what was chosen, what was rejected, and why. A decision with
no considered alternative is not consequential — leave it out. To overturn a
landed decision, add a NEW D-N that cites and supersedes it; never edit the old
entry's outcome. Delete this comment and the example below once real entries
exist.
-->

### D-1: {{DECISION_TITLE}}

- **Chosen:** {{CHOSEN_OPTION}}
- **Rejected:** {{REJECTED_OPTION_A}} — {{WHY_A_LOST}}; {{REJECTED_OPTION_B}} — {{WHY_B_LOST}}
- **Reasoning:** {{WHY_CHOSEN_WINS}}
- **Date:** {{ISO_8601_DATE}}
