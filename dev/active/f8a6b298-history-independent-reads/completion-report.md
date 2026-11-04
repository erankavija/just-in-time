# Epic Complete: Restructure storage so reads never scale with project history (f8a6b298)

**Started:** 2026-11-03 (01:48 EET)
**Completed:** 2026-11-04
**Assignee:** agent:jit-execution-lead (leads: just-in-time-8c/3e, then just-in-time-52)

## Summary

Gate runs, verdicts, archive evidence and leases are now stored as per-key
records: under each issue, by (gate, inputs digest), by archive target, and by
issue. Ordinary commands address records by key, and only named history readers
read the event log, gate-run history or the claims log. This repository and four
others moved to repository format 4 in one window.

## Metrics

| Metric | Value |
|---|---|
| Implementation tasks completed | 37 done (24 rejected in planning revisions, merged, or superseded) |
| Stories completed | 3 / 3 |
| Waves executed | 11 |
| Rework cycles | 12, in 10 issues |
| Escalations | 12, all resolved |
| Sub-agent and lead dispatches | 49, including reworks |
| Issues created during execution | 944edb32, 655273d9, 5b37a806 (external, driven by this lead), 53d19851 (merged into 2f563673); outside the epic, 2e2c8ee3 under milestone v1.0 |

## Success Criteria

- [x] REQ-01: one issue's gate-run reads open only that issue's records. Delivered by 803b107c, 2117739f, 93b7f1dd, 0851fae4, 1630015d, 944edb32 and 655273d9.
- [x] REQ-02: the latest run of a gate is found from its (issue, gate) key. Delivered by 77f3db10, 2117739f and 6c0f8739.
- [x] REQ-03: a reusable verdict is found from its (gate, digest) key. Delivered by b33ed7eb and 6c0f8739.
- [x] REQ-04: captures hold only the evaluated issue's runs. Delivered by 0bd2c797.
- [x] REQ-05: lease reads and changes touch only one lease record, and no claims-log reads happen. Delivered by c3e3e2ff, cc6ef886, 15009590, 2f563673, 614a6f97, 944edb32 and 655273d9.
- [x] REQ-06: every lease operation goes through one lock-load-change-publish sequence. Delivered by becea20a, dd53527f, 15009590 and a7314585.
- [x] REQ-07: an interrupted lease operation leaves the prior or the next lease. Delivered by 2f563673 and 7244ecb2.
- [x] REQ-08: claims use only the end-of-log probe and append one claim event. Delivered by 63de0883.
- [x] REQ-09: archive evidence is read once, with no event-log read. Delivered by f9bf70a5, db9bcf12, 7fc08dff, 5e1ba005, 944edb32 and 655273d9.
- [x] REQ-10: one event-log parser with one torn-line rule. Delivered by 84e1e27a, 97f5f2b7, 5e1ba005, 944edb32 and 655273d9.
- [x] REQ-11: a default search reads no history file. Delivered by 7a8773ca and ba4c6054.
- [x] REQ-12: nothing is derived from history and nothing compacts it. Delivered by 84e1e27a, b32a720e, 2f563673, f0a5b90a and ac8f07c2.
- [x] REQ-13: only named history readers read history. Delivered by 5b0e3d63, 5582e39e, 725af076 and ac8f07c2.
- [x] REQ-14: ordinary commands succeed over unparseable history. Delivered by 4466395d.
- [x] REQ-15: the references describe the per-key layout. Delivered by 1630015d, 09ca911e, f0a5b90a and ac8f07c2.

## Key Decisions

- The window landed and verified every remaining code task first. It then
  rebuilt the green binaries at the latest storage-relevant commit (a05567fde)
  and repeated the five-repository dry run with them.
- In the window, the jit-server stopped before install and restarted after the
  rewrite of just-in-time, with identical arguments.
- Each lead-side rework was a one-line or one-sentence fix that answered a
  verdict: two doc-review findings, one doc-review finding, a manifest drift
  attributed to 97f5f2b7, and the cutover record.
- Gate-check cargo-ci runs take the CI turnstile and the exclusive full-CI lock.
  Unlocked runs had produced exit-75 load faults.

## Escalations

Every ruling came from the steward, just-in-time-6a, acting for the user, unless
noted. Each is recorded in the plan and in the issue's `## Decisions`.

| Plan | Issue | Ruling |
|---|---|---|
| DEC-24 | 2f563673 | Absorbed lease-index removal (53d19851). |
| DEC-25 | ac8f07c2 | Per-key redo publication covers repository records; leases keep their rename-or-unlink commit point. |
| DEC-26 | ac8f07c2 | Archive-evidence paths are historical, after gf2's checker treated them as live. |
| DEC-27 | 735949ef | Story REQ-03 names the epic REQ-13 history reader. |
| DEC-28 | 5ee85aed | Story REQ-01 names the epic REQ-05 indefinite-limit exception. |

- **Lease rulings from the first lead:** the user resolved three (relayed by
  6a):
  - Stale leases are listed and marked stale.
  - dd53527f was amended, and its claims-log sequence scan was accepted as
    dd53527f D-1.
  - The wording of b697a3f7 REQ-04 was settled.
- **User decision on the cutover** (relayed by 6a): the five-repository cutover
  is accepted on the steward's independent post-window verification. It is
  recorded on 655273d9, 735949ef, 5ee85aed and dc5fa7f8.
- **Format-4 window:** the first run was denied by the outgoing lead's
  permission check. The user approved the cutover in the incoming lead's
  session, and the window ran there.
- **Holistic review ignored the user decision:** the holistic-review prompt did
  not bind user decisions, so it re-raised an accepted finding. 2e2c8ee3 gave
  every review prompt the code-review prompt's User decisions section, plus an
  M9 check that each prompt carries it.

## Issues Discovered During Execution

- **944edb32, 655273d9:** the format-4 rewrite tooling and the window, split
  from the bump by planning.
- **5b37a806:** a v1-release recovery bug, driven by this lead per user ruling.
- **53d19851:** merged into 2f563673.
- **2e2c8ee3:** under milestone v1.0, not this epic. Review prompts bind user
  decisions.

## Holistic Quality Notes

- **Story criteria restating epic criteria:** these were the main source of
  escalations. Twice a story dropped an exception the epic states. A breakdown
  should carry each epic exception into every story criterion it narrows.
- **Lessons for leads:**
  - Lead-run release work needs evidence a reviewer can inspect, or a recorded
    user decision for facts outside the checkout. That is format4-cutover.md and
    format4-run-id-check.py, beside the window tooling.
  - 655273d9 was first closed before its REQ-03 run-id check ran. The check
    passed afterwards, and it is now committed as a script.
