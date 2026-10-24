# Planning checkpoint — 2026-10-24

Planning node 0dc73711 is paused by user request; no rework or review is running and no leases are held.

## State

- Plan and manifest at 700a37294: 45 issues (6 stories, 39 tasks), 58 edges. Validator with warnings denied, `render --check` and the batch-create dry run pass.
- User decisions in force: charter D-22 with amended D-17/D-20 (e0e3fc4cb); REQ-21–REQ-23 and the trimmed REQ-20 on b697a3f7 (369d35a42, 10dde7f42); manifest size and task shape (plan DEC-26).
- `plan-review` gate: not yet run. Planning-lead review rounds 1–2 failed (38, 23 findings, all closed); round 4 failed (below).

## Round-4 review: FAIL, 17 blocking findings

Full text: [review-r4.md](review-r4.md). None is sizing-only; all are substantive.

| # | Finding | Sizing-only |
|---|---|---|
| 1 | Backgrounds predate a807e699 and d4373d07; `bootstrap.rs` missing from footprints; ConfigManager deletion breaks `bootstrap.rs:227` | No |
| 2 | `lock-free-read-consumers` edits the `--fix --dry-run` path 1f24e3e9 rewrites, with no edge | No |
| 3 | `capture_archive_plan` edited by unordered tasks (candidates vs container destination / legacy fallback) | No |
| 4 | `storage-format.md` read-discipline section has three unordered writers and an unowned part | No |
| 5 | `read-session` breaks integration-test implementors outside its footprint | No |
| 6 | `read-session` REQ-03 asks for a record that does not exist | No |
| 7 | `event-per-commit` stamps published events with the preview context | No |
| 8 | `read-concurrency` story claims reads it does not aggregate | No |
| 9 | `one-form-convergence` story cites REQ-01 with no child carrying it | No |
| 10 | `cost-evidence` names deleted keys and a false environment fact | No |
| 11 | `executor-config-caches-removal` migrates a method no production code calls | No |
| 12 | `captured-declaration-reads` states a behaviour change contradicting DEC-09 | No |
| 13 | `archive-candidates-one-session` REQ-03 cannot observe a read-session open | No |
| 14 | `archive-destination-no-legacy-fallback` breaks the relocated citation-test helper | No |
| 15 | `record-commit-through-capabilities` breaks two test targets outside its footprint | No |
| 16 | `control-files-publication` rests on a false location claim and an unholdable criterion | No |
| 17 | Plan prose left over from merges and removed features | No |

The review found the D-22 admission protocol sound, round-2 findings closed, DEC-01/02 external coverage correct and the three `chain-link` overrides acceptable.

## Resume

Fix the 17 findings in place (replace defective text; no correction sections), rerun the four checks, then run independent review with a single reviewer before the `plan-review` gate.
