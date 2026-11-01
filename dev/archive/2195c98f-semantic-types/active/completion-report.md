# Epic Complete: Give every identity and closed vocabulary one semantic type (2195c98f)

**Started:** 2026-10-24
**Completed:** 2026-11-01
**Assignee:** agent:jit-execution-lead

## Summary

Identities, paths and closed vocabularies are typed and parsed once at the boundary: `IssueId`, `IssueIdPrefix`, `GateKey`, `GateRunId`, `ProfileId` and `Assignee` now reach the domain, storage, commands, the server and the web UI as semantic types.

## Metrics

| Metric | Value |
|---|---|
| Stories completed | 6 of 6 |
| Interior tasks | 83 done, 6 rejected as absorbed |
| Waves executed | 11, plus holistic-review gap tasks |
| Rework cycles | 25 across all issues |
| Escalations recorded | 15, one still open and outside this epic |
| Issues created during execution | 11, listed below |

The dispatch log in `progress.json` holds the per-issue vendor, model and reason.

## Success Criteria

Each criterion's delivering tasks carry its `satisfies:2195c98f/REQ-NN` label, so `jit issue search` lists them. The final holistic review passed all 17 hard criteria with no findings.

- [x] REQ-01 issue identity and typed storage trait
- [x] REQ-02 prefixes parsed once and resolved to an `IssueId`
- [x] REQ-03 only `IssueId` derives the short id
- [x] REQ-04 optional event issue accessor
- [x] REQ-05 event envelope
- [x] REQ-06 typed lease fields
- [x] REQ-07 record paths from `VirtualPath`
- [x] REQ-08 one reserved-target predicate
- [x] REQ-09 one profile identity type
- [x] REQ-10 one enum per closed vocabulary
- [x] REQ-11 `Assignee` for actors and filters
- [x] REQ-12 query value lists derived from enums
- [x] REQ-13 single wire spelling for event tags and plan codes
- [x] REQ-14 `Priority` ordered through `Ord`
- [x] REQ-15 graph assertions compiled at load
- [x] REQ-16 one `HierarchyConfig` constructor
- [x] REQ-17 raw strings only at boundaries; `raw-identity-sites.md` inventories them

## Wave Execution Log

`progress.json` holds per-wave status. Waves 1 to 8 delivered the staged retype behind the `IssueId` alias, wave 9 the newtype flip (7abcc22d), and waves 10 and 11 record paths, gate keys, gate-run ids and server and web short ids.

## Key Decisions

- The stalled flip worktree was salvaged after confirming no live owner, then reviewed against its criteria rather than re-implemented.
- Captured issue records decode through one path-naming decoder, `decode_issue_record`.
- `be18da9e`'s web sweep landed with `ce2bdbb0`, because code review of the server change required it; `be18da9e` then passed its own gates.
- `GateRunId` was gated after `GateKey` landed, because the two overlapped heavily.

## Escalations

`progress.json` records all of them. Final session:

- Gate-run byte identity: 4793 of 5924 stored records predate unrelated format changes. The user approved scoping byte identity to current-format records and id reproduction to the rest.
- Gate registry round trip: resolved by just-in-time-c9. `.jit/gates.toml` stays authored, and the criterion is a serialize-and-parse round trip.
- `gate-prefix` keys failed the gate-key grammar: resolved by just-in-time-c9. The option was removed (2ac34492, net -153 lines).
- Issue record key order: resolved by just-in-time-c9. Records re-serialize to the same canonical JSON, and 53 older records were left unchanged.

## Issues Discovered During Execution

`progress.json` `created_during_execution` holds earlier sessions' entries. Final session:

- 83413035 Resolved hierarchy facts carry typed issue ids (a raw parent id forced a truncation).
- 2ac34492 Criteria checks bind through check labels only (see Escalations).
- 1a3db2c3 The dependency graph carries typed node ids (pre-review sweep).
- 64420cff Commands carry assignees as Assignee (epic holistic review).
- 592d663e A profile has one identity type (epic holistic review).

Filed outside this epic:

- 9a2563fd Gate registry edits keep the authored file, under d675102c.
- 952ed1e7 Tests stop reading the live tracker, under c780514e.

## Holistic Quality Notes

- Byte-identity criteria on live data were written before anyone checked the stored data; each needed amending. Check stored data before writing such a criterion.
- `crate_source_digest`'s self-check goes stale after merging main, costing a gate cycle per landing until the lead cleans the crate first. Its cause is not diagnosed.
- A record-merge shortcut that took the branch's copy dropped two dependencies from this epic's record. They were restored at 86a3b515c, and the resolver now takes only `gates_status` from the branch.
