# Archived lifecycle semantics for artifact archival (45a140ae) — decision record

Issue: 45a140ae — Define Archived lifecycle semantics for artifact archival.

Two concepts share the word "archive": the lifecycle **state** `Archived`, and
`jit archive container`/`jit archive document`, which relocate linked artifacts
into a filesystem mirror (`archive_root`, e.g. `dev/archive`). Before this work
they were unrelated: marking a container `Archived` did not relocate its
documents and made its artifact plan ineligible (because `Archived` was not
terminal), while executing artifact archival left the container `Done`. This
record fixes the model.

## Pinned model (invoker interview 2026-07-16 — fixed scope)

### Decision 1 — Archived is terminality-preserving

**Chosen.** Archiving preserves what was true before it: an issue archived from
`Done`/`Rejected` keeps satisfying dependents; an issue archived from a
non-terminal state does not start satisfying them.

Implementation: an issue records the state it held immediately before entering
`Archived` in a new `archived_from` field. Dependency satisfaction and readiness
consult the *effective terminal state*: `Done`/`Rejected` for a literally
terminal issue, the recorded pre-archive state for an `Archived` issue, and
"none" otherwise.

**Rejected — always-terminal:** treating every `Archived` issue as terminal
would unblock the dependents of work that was archived while still unfinished.

**Rejected — never-terminal:** treating every `Archived` issue as non-terminal
would re-block the dependents of finished work the moment it was retired, so a
completed-then-archived foundation would strand everything built on it.

### Decision 2 — Coupled retirement workflow

**Chosen.** Artifact archival requires a terminal state (`Done`/`Rejected`);
successful execution transitions the container to `Archived`. Eligibility is
checked against the *effective* terminal state, so an already-`Archived`
container that was retired from a terminal state stays eligible for idempotent
reruns/reconciliation.

**Rejected — fully orthogonal:** keeping lifecycle state and artifact archival
independent is the pre-existing split this issue exists to remove; it leaves
"container `Archived` but documents un-relocated" and "documents archived but
container `Done`" as permanent, confusing states.

**Rejected — archived-first eligibility:** requiring the container to already be
`Archived` before artifact archival could run would create the exact circular
prerequisite REQ-03 forbids (you cannot reach `Archived` without archiving, and
cannot archive without being `Archived`).

### Decision 3 — Revive restores the pre-archive state exactly

**Chosen.** Reviving an `Archived` issue restores its recorded `archived_from`
state exactly (`done → archived → done`). A completed issue can never re-enter
the *active* lifecycle through the archive round-trip; this closes the
live-verified `done → archived → ready` resurrection loophole.

**Rejected — free revive + advisory warning:** letting revive land any state
(with a warning) leaves the resurrection loophole open — the whole point of the
decision is that the archive round-trip is not a backdoor into `Ready`.

**Rejected — no revive:** making `Archived` a dead end removes a legitimate
"parked by mistake / changed our mind" recovery path.

## Legal transitions

The update handler stays non-exhaustive for the active states (it does not
police, say, `backlog → gated`); the constraints below are the ones the model
*does* enforce, layered on the existing dependency/gate guards.

| From | To | Rule |
|------|----|------|
| any non-`Archived` state `S` | `Archived` | Always allowed. Records `archived_from = S`. Gate/graph-rule enforcement is bypassed (parking/retiring must not be gated), like `Rejected`. |
| `Archived` (recorded origin `O`) | `O` | Revive. Allowed; clears `archived_from`. Must still clear the target's own guards (e.g. reviving to `Done` re-checks dependencies/gates). |
| `Archived` (recorded origin `O`) | any state ≠ `O` | **Refused** with `ArchivedReviveError` — the diagnostic names `O` as the only revive target. This closes the resurrection loophole. |
| `Archived` (no recorded origin — legacy) | any state | Allowed (compatibility, see below), with an advisory warning that the pre-archive state was not recorded. |
| `Archived` | `Archived` | No-op (unchanged by the chokepoint's existing `old == target` guard). |

`archived_from` is only ever `Some` while `state == Archived`; every revive
clears it, so it never lingers on an active issue.

## Effective terminal state

`effective_terminal_state(state, archived_from)`:

- `Done` / `Rejected` → `Some(that state)` (literally terminal),
- `Archived` → `archived_from` when it is `Some(Done)`/`Some(Rejected)`, else `None`,
- anything else → `None`.

`is_effectively_terminal` = `effective_terminal_state(..).is_some()`. Dependency
satisfaction (`is_dependency_met`), readiness, `query closed`, the `is_blocked`
predicate, container rollups, archive eligibility, and classifier owner
terminality all route through this one definition so the states agree everywhere.

Rollup delivery accounting folds by origin: an issue archived from `Done` counts
as delivered, one archived from `Rejected` counts as rejected, so a
fully-delivered-then-archived container still reports 100%. The exact `by_state`
buckets are unchanged, so `Archived` remains individually visible.

## Compatibility (REQ-04)

`archived_from` is `#[serde(default, skip_serializing_if = "Option::is_none")]`:
existing issue files round-trip byte-for-byte, and an issue already in `Archived`
before this change deserializes with `archived_from = None`.

A legacy `Archived` record (`archived_from = None`) is treated as **not**
effectively terminal — exactly the behavior before this change, where `Archived`
was never terminal — so no dependent is silently unblocked by data whose
pre-archive state is unknown. This is the conservative default consistent with
Decision 1. Because their origin is unrecorded, legacy `Archived` issues keep the
prior unconstrained revive (with an advisory warning); the resurrection loophole
is closed for every issue archived under the new model, which records its origin.

Marker-backed artifact archives (`.jit-container`, the `artifact_archive_executed`
event log) are unaffected: container archival now *additionally* transitions the
container to `Archived`, but the relocation, marker, and event machinery are
unchanged. The state transition is the final durable step and is idempotent — a
rerun of an already-`Archived`-from-terminal container reconciles without
re-relocating.

## Implementation notes

- Domain (`domain/types.rs`): `Issue.archived_from` + `MinimalIssue.archived_from`;
  `effective_terminal_state` / `is_effectively_terminal` free fns and `Issue`/
  `MinimalIssue` methods; `is_dependency_met(state, archived_from)`;
  `is_blocked`, `MinimalIssue::state_symbol` archived-aware.
- Transition chokepoint (`commands/mod.rs::apply_state_transition`): stamps and
  clears `archived_from`, enforces the revive rule, bypasses graph-rule
  enforcement for `Archived` (alongside `Rejected`). The bulk-update path routes
  through the same chokepoint.
- Archive (`commands/archive.rs`): `NonTerminalTarget` blocker uses effective
  terminality; `archive_candidates` selection (`effectively_terminal_container_ids`)
  uses the same `Issue::is_effectively_terminal` predicate, so an
  Archived-from-terminal container the direct path would reconcile also surfaces
  as a candidate; `execute_archive_target` transitions the container to `Archived`
  as its final durable step; `ArtifactOwner`/`EmbeddedArtifactOwner` carry
  `archived_from` so shared-document owner terminality (independently archived
  descendants) is archived-aware.
- Diagnostics (REQ-05): the `NonTerminalTarget` blocker renders a permitted-next-
  action explanation in human output and carries it in JSON; executing an
  ineligible container fails with a message naming the same next action.
- `ArchivedReviveError` (`errors.rs`): typed, exit-4, human + JSON, remediation
  points at `jit issue update <id> --state <origin>`.
