# Dogfood: `jit apply plan` on a real container (25064508)

**Task:** `fc414353` — Dogfood apply plan on a real container (TSTB-01)
**Date:** 2026-06-24
**Container:** `25064508` — *Structured project knowledge in jit: addressable items, qualified ids, and projected invariants* (`type:epic`)

## What was done

Applied the `plan` template to a real epic with `jit apply plan 25064508` and confirmed the bracket it produced.

`25064508` carried a **legacy P-only bracket** from the removed `jit plan` command: it depended on a lone planning node (`1eb0bdfd`, `type:planning`) with no breakdown node. Per the user's decision (clean-migrate), the legacy scaffold was removed before applying:

1. `jit dep rm 25064508 1eb0bdfd` — drop the legacy `C → P` edge.
2. `JIT_ALLOW_DELETION=1 jit issue delete 1eb0bdfd` — remove the orphaned legacy planning node.
3. `jit apply plan 25064508` — instantiate a clean `plan` bracket.

## Resulting bracket (TSTB-01 confirmed)

```
25064508 (C, epic)
   └─ depends on → 5f5735c3 (B, type:breakdown)
                      ├─ labels: brackets:25064508
                      ├─ gates: coverage-preview, breakdown-review
                      └─ depends on → 0d753ba5 (P, type:planning)
                                         ├─ gates: plan-review
                                         ├─ description: auto-seeded from the template (non-empty, REQ-03)
                                         └─ depends on → 56ab0224 (moved-upstream by move-upstream-to-role)
```

Verified:
- **Spine** `C → B → P` wired; the scaffold's direct `C → B` anchor edge is the only `C` dependency (transitive reduction holds).
- **Gates** attached from the template: `P` = `plan-review`; `B` = `coverage-preview` + `breakdown-review`.
- **`brackets:25064508`** label on `B` uses the container SHORT id (convention).
- **`move-upstream-to-role`** moved `C`'s pre-apply upstream dep (`56ab0224`) onto `P`.
- **Description interpolation** — `P`'s description is non-empty and rendered from the template (closing the empty-P gap).
- `jit validate --scope 25064508` → `✓ Scope validation passed` (exit 0).

## Gaps surfaced by the dogfood (now fixed)

Dogfooding the clean-migrate of a legacy P-only container surfaced three jit gaps, all fixed as part of this work:

1. **`jit apply plan` duplicated the planning node on a legacy P-only container.** The already-applied check matched only the **breakdown** node (`find_applied_breakdown` on `brackets:<C-short-id>`), not a pre-existing **planning** node, so applying created a second `P` and the `move-upstream` transform demoted the old `P` into the new one's deps (reproduced in an isolated copy first). **Fix:** the apply precondition phase now detects a pre-existing planning-typed dependency with no breakdown node and rejects with guidance (`test_apply_rejects_legacy_planning_only_bracket`), so a legacy P-only container must be cleaned up before re-applying — exactly the migration done above.
2. **`jit issue delete` logged no event.** jit had no `IssueDeleted` event type, so deleting `1eb0bdfd` left no audit trail, violating the event-logging invariant. **Fix:** added the `IssueDeleted` event; `delete_issue` now logs it (`test_delete_issue_logs_issue_deleted_event`).
3. **`jit dep rm` logged no event.** Removing the `C → P` edge edited the container without logging. **Fix:** `remove_dependency` now logs an `issue_updated` event with `fields:["dependencies"]`, mirroring `update_issue` (`test_remove_dependency_logs_issue_updated_event`).

The two events this dogfood's pre-fix deletion missed (`1eb0bdfd` delete + the `25064508` dependency-remove) were backfilled into `events.jsonl` so the lifecycle is fully recorded.
