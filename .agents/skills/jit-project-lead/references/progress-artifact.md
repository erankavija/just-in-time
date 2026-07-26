# Progress artifact

The steward's resumable operational state for one strategic-tier container: the
wave plan plus a per-sub-strategic-container status row, so a re-invoked steward
resumes mid-drive without losing progress. This is the one-tier-up analogue of
`jit-execution-lead`'s per-epic `progress.json`, which carries per-issue rows;
here the rows are per sub-strategic container.

Unlike the charter, the progress file is operational state meaningful only while
the strategic container is active. It is fine to archive when the container
completes, so it lives under a **managed/active** path, not a permanent one.

This reference defines the file's location, JSON schema, its tie to the wave list
from `references/wave-layering.md` and `current_wave` consumer in
`references/container-dispatch.md`, its `jit doc` link, and its read-back on
resume.

## Location (config-derived)

Read from `.jit/config.toml` `[documentation]`; never hardcode a directory.

- **Active root** — `<development_root>/active`, where `development_root` is the
  `[documentation]` key of the same name. This matches the managed path the
  execution lead already writes its per-epic progress file under, one tier down.
- **Progress directory** — obtained from `jit doc dir <strategic-container-id>
  <development_root>/active`, never composed from a spelled-out pattern.
- **Progress path** — `<progress-directory>/progress.json`.

> Example: with `development_root = "notes"`, `jit doc dir <container-id>
> notes/active` prints the directory the progress file for strategic container
> `<container-id>` owns, a sibling of the execution lead's own progress
> directory; `progress.json` lands inside it. Derive the root from
> configuration; a literal directory name from an example is wrong for every
> other project.

## Schema

The wave list is produced by `references/wave-layering.md` (already emits
`waves[].containers[]` with `id`/`short_id`/`title`); this file adds the
operational fields (`status`, `wave`, `current_wave`, coherence result) that
drive and resume the dispatch loop. Rows are containers, not issues.

```json
{
  "container_id": "<full-id>",
  "container_short_id": "<short-id>",
  "charter_path": "<permanent-root>/<short-id>-charter.md",
  "current_wave": 1,
  "waves": [
    {
      "wave_number": 1,
      "containers": [
        {
          "id": "<full-id>",
          "short_id": "<short-id>",
          "title": "...",
          "wave": 1,
          "status": "pending"
        }
      ],
      "coherence_review": "not-run"
    }
  ],
  "escalations": [],
  "notes": [],
  "started_at": "<ISO-8601>"
}
```

Fields:

- **`container_id` / `container_short_id`** — the strategic-tier steward anchor.
- **`charter_path`** — the config-derived charter location
  (`references/vision-charter.md`), recorded so a resumed session finds both
  artifacts from one file.
- **`current_wave`** — the wave index the dispatch loop is on. Read by
  `references/container-dispatch.md`; advanced (`+= 1`) only after every
  container in the wave is accepted and the wave's coherence review passes.
- **`waves`** — the ordered wave list from `references/wave-layering.md`, with
  per-container operational fields added. Every direct sub-strategic child
  appears in exactly one wave's `containers` (the wave-layering coverage
  invariant), so every child has exactly one status row.
- **`containers[].status`** — one of:
  `pending` (not yet planned) →
  `planning` (`jit-planning-lead` is driving its bracket) →
  `planned` (planning and breakdown nodes are done, their gates passed, and the
  implementation frontier is ready) →
  `executing` (a fresh `jit-execution-lead` is driving implementation) →
  `accepted` (the lead returned; the container's own gates passed and its
  `[hard]` criteria are met, per container-dispatch step 8) →
  `done` (accepted **and** the wave's coherence review passed).
  Off-path values: `rework` (sent back to its lead) and `escalated` (raised to
  the steward's invoker, unresolved). Terminal skip value: `rejected` (the
  container was rejected upstream — e.g. a rejected sub-strategic child that
  stays in the roster for a complete picture; carry a note stating why). A
  `rejected` row is never dispatched and is skipped on resume exactly like
  `done`. Legacy `dispatched` rows must be reconciled against the live bracket:
  map them to `planning`, `planned`, or `executing`; never assume which phase.
- **`containers[].wave`** — the row's wave number, redundant with its position
  for direct lookup.
- **`coherence_review`** — per-wave cross-container review state
  (`references/coherence-review.md`): `not-run` → `pass` / `fail`. A wave does
  not advance while this is `not-run` or `fail`.
- **`escalations`** — open and resolved subordinate escalations the steward
  handled, one tier up from the execution lead's `escalations` array.
- **`notes`** — free-text session/handoff notes, as in the execution lead's
  progress file.

## Tie to the wave list and dispatch loop

- **Producer** — `references/wave-layering.md` computes `waves` (the ordered
  container sets). Persist its output here, then decorate each container with
  `status`/`wave` and each wave with `coherence_review`.
- **Consumer** — `references/container-dispatch.md` reads `current_wave` to pick
  the wave to dispatch, updates each container's status across the distinct
  planning and execution phases as leads return (steps 4, 5, and 8), sets the
  wave's `coherence_review` (step 9), then advances
  `current_wave` and commits the file. One tier down, this mirrors the execution
  lead advancing `current_wave` and updating per-issue statuses in its own
  `progress.json`.

## Linkage to the strategic container (REQ-03)

After first writing the file, link it to the strategic container so
`jit doc list <container-id>` surfaces both the charter and the live progress:

```bash
jit doc add <strategic-container-short-id> <progress-path> \
    --doc-type notes --label "Steward progress"
```

The link is a doc reference, not a lifecycle transition. The path is stable while
the container is active, so the link does not move as the file is updated;
re-point it if the file is later archived on container completion.

## Read-back on resume (REQ-04)

A re-invoked steward loads prior progress before dispatching anything, so no wave
is re-run and no accepted container is re-dispatched:

1. **Resolve the progress directory** with `jit doc dir <anchor-id>
   <development_root>/active`, then the progress path inside it
   (`<directory>/progress.json`).
2. **If it exists, read it in full.** Recover `current_wave`, every container's
   `status`, each wave's `coherence_review`, and open `escalations`. Resume the
   dispatch loop at `current_wave`, skipping containers already
   `done`/`accepted`/`rejected` and re-dispatching only those `pending`/`rework`.
3. **Reconcile against live jit state.** Children may have changed between
   sessions. Re-derive the wave list (`references/wave-layering.md`) and confirm
   the persisted `waves` still match the current DAG; if a child was added,
   removed, or re-linked, update the file before dispatching. A drifted wave list
   is a stop-and-escalate condition, not a silent overwrite.
4. **Read the charter too.** Use `charter_path` to load the vision and decision
   log (`references/vision-charter.md`); the two artifacts resume together.
5. **If it does not exist,** this is a first invocation: compute the wave list,
   write the file with every container `pending` and `current_wave` at the first
   wave, and link it (REQ-03) before dispatching.

## Stop and escalate

- Active root cannot be resolved (`development_root` missing or `[documentation]`
  unreadable). Stop and report.
- `jit doc dir` rejects the active root as an undeclared issue-scoped area. Stop
  and report; do not fall back to composing the directory by hand.
- The file exists but does not parse, or its `waves` no longer match the live DAG
  in a way re-derivation cannot reconcile (a child vanished mid-drive). Stop
  rather than overwrite; a blind rewrite loses recorded status and escalations.

## Red flags

- Placing the progress file under a permanent path. It is operational state and
  should archive with the container; permanent placement leaves stale progress
  files accumulating forever.
- Hardcoding `dev/` or `dev/active` instead of reading `development_root`.
- Composing the progress path as
  `<development_root>/active/<short-id>-progress.json` directly instead of
  resolving the directory with `jit doc dir` first. The directory name is
  derived by the tool, not spelled out here.
- Per-issue rows. Rows are sub-strategic containers; a container's own issues are
  the dispatched lead's progress file, one tier down — never duplicated here.
- Advancing `current_wave` before every container in the wave is `accepted` and
  the wave's `coherence_review` is `pass`.
- Resuming dispatch without reading the file back and reconciling against live
  jit state — re-dispatching an already-accepted container burns a full lead run.
