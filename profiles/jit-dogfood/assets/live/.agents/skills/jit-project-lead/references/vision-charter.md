# Vision charter

The steward's durable statement of what the project is for and every
consequential decision behind it. It outlives any single container's completion:
a strategic-tier vision has no natural end-of-life, so the charter lives under a
**permanent** documentation path that is never auto-archived when a container
closes. One charter per strategic-tier container (the steward anchor).

This reference defines the charter's location, structure, decision-log format,
its `jit doc` link to the strategic container, and how a resumed session reads it
back. It does not author any concrete project's vision — the skill instantiates
`references/templates/vision-charter.md` at runtime.

## Location (config-derived)

Read the location from `.jit/config.toml` `[documentation]`; never hardcode a
directory.

- **Permanent root** — the first entry of `permanent_paths`. Paths under it are
  excluded from archival on container completion (that is the whole point of
  placing the charter here rather than under a managed/active path).
- **Charter path** — `<permanent-root>/<strategic-container-short-id>-charter.md`.

The short id comes from the steward anchor container resolved in tier derivation
(`jit issue show <container-id> --json` → `short_id`).

> Example: with `permanent_paths = ["notes/"]`, the charter for strategic
> container `<container-id>` lands at `notes/<container-id>-charter.md`. Derive the root from
> the project's own config; a literal directory name from an example is wrong for
> every other project.

## Structure

A standalone markdown document. As a rendered-standalone doc it carries one `#`
title; every other heading is `##`/`###` per the content standards.

Each decision appears in two places: a one-line summary **row** under
`## Decision Log`, and the full **entry** under `## Decision Details`. Refer to a
decision as `D-N` in the linked charter. A repository may additionally declare a
project-scoped item kind whose source is this exact charter path; when it does,
use that repository's configured qualified address. The portable workflow does
not assume such a kind because the charter path is derived per strategic
container. The `### D-N` entries under `## Decision Details` hold the full record.

```markdown
# Charter: <strategic-container title>

> Strategic container: <short-id> · <membership-label>

## Vision

<Two to five sentences: what this project is for, the outcome it must deliver,
and the standard by which any sub-strategic container's work is judged coherent
with it. This is the yardstick the steward resolves escalations against.>

## Decision Log

- D-1: <one-line summary of the decision>
- D-2: <one-line summary of the decision>

## Decision Details

<One `### D-N` subsection per consequential decision, in ascending id order,
matching the rows above. Never delete or renumber a landed row or entry.>
```

Keep the vision short and load-bearing. It is the reference the steward cites
when accepting a container, resolving a subordinate escalation, or refusing work
that drifts from the project's purpose.

## Decision-log entry format

Record a decision here when it is **consequential** — it shapes scope,
architecture, a tier/boundary interpretation, an escalation resolution, or a
rejected alternative someone will otherwise re-propose. Routine, reversible calls
stay out.

Each decision lands as two matched parts under the same `D-N` id: a summary row
under `## Decision Log`, and the full entry under `## Decision Details`. The full
entry states what was **chosen**, what was **rejected**, and **why** (REQ-01).
Each `D-N` id is stable and ascending.

```markdown
## Decision Log

- D-N: <one-line summary of the decision>

## Decision Details

### D-N: <one-line decision title>

- **Chosen:** <the selected option, stated concretely>
- **Rejected:** <option A> — <why it lost>; <option B> — <why it lost>
- **Reasoning:** <why the chosen option wins; the tradeoff that decided it>
- **Date:** <ISO-8601 date, e.g. 2026-07-03>
```

Rules:

- The summary row and its full entry share one `D-N` id and are added together.
  Refer to the row as `D-N` in the linked charter; keep it a single line.
- `Rejected` is never empty for a consequential decision. A decision with no
  considered alternative was not consequential; leave it out. If a real
  alternative existed, name it and say why it lost — that is what stops the
  rejected option from being re-litigated a session later.
- Ids are append-only and never reused. On resume, continue from the highest
  existing `D-N`.
- One decision per entry. Do not bundle unrelated calls under one id.

## Linkage to the strategic container (REQ-03)

After writing (or updating) the charter, link it to the strategic container so
`jit doc list <container-id>` surfaces it:

```bash
jit doc add <strategic-container-short-id> <charter-path> \
    --doc-type design --label "Project charter"
```

The link is a doc reference, not a lifecycle transition — the steward makes it
directly. Re-running `jit doc add` after an update re-points the link at the
current commit; the charter path is stable, so the link target does not move.

## Read-back on resume (REQ-04)

A re-invoked steward must recover the full charter before making any new
decision, so no prior vision or decision is lost:

1. **Resolve the charter path** from config (permanent root + anchor short id),
   as above.
2. **If it exists, read it in full.** Load the vision statement and every
   `### D-N` entry into working memory. These decisions are binding: do not
   re-decide a logged question, and do not re-propose a logged `Rejected`
   option, unless new information explicitly overturns it — in which case add a
   **new** `D-N` entry that cites and supersedes the old one (never edit the old
   entry's outcome in place).
3. **Note the highest `D-N`.** New decisions this session continue the numbering
   from there.
4. **If it does not exist,** this is a first invocation on the container:
   instantiate `references/templates/vision-charter.md`, write the vision, and
   link it (REQ-03) before dispatching any container work.

## Stop and escalate

- Permanent root cannot be resolved (`permanent_paths` empty or `[documentation]`
  unreadable). There is nowhere archival-safe to place the charter. Stop and
  report; do not fall back to a managed/active path — that would let the charter
  be archived on container completion.
- The charter file exists but cannot be parsed into a vision plus zero-or-more
  `D-N` entries (corrupt or hand-mangled). Stop rather than overwrite; a blind
  rewrite loses logged decisions.

## Red flags

- Placing the charter under a managed or active path instead of a permanent one.
  It would be archived when the container completes, and the vision must outlive
  the container.
- Hardcoding `docs/` (or any directory) instead of reading `permanent_paths`.
- Editing or deleting a landed `D-N` entry, or renumbering the log. Superseding
  decisions get a new id that cites the old one.
- Logging a "decision" with an empty `Rejected` field. If nothing was rejected,
  it is not a consequential decision and does not belong in the log.
- Starting a resumed session's dispatch before reading the charter back. Prior
  decisions are binding; re-deciding them wastes a cycle and drifts the project.
