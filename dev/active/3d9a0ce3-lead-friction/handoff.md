# Handoff: 3d9a0ce3 planning complete

Planning (b5ba1b61) and breakdown (a3437bab) are done. The epic is ready for an execution lead. The [plan](plan.md) and [manifest](breakdown.json) are authoritative.

## State

- 35 new tasks plus 7 live existing children (72d89161, fd9bfab8, 8673dc5e, 66986f88, f37d228e, 3754095f, da12ad66). e643ef66 is rejected as obsolete (plan OD-1).
- The profile track is serial: every task touching `commands/profile.rs`, `repository_state/profile_apply.rs` or `main.rs` is ordered.

## Cross-epic edges

| Task | Depends on | Note |
|---|---|---|
| `profile-record-commands-removal` | 756ec5ec (842b0ac6, "Keep one profile manifest version") | OD-16 |
| `profile-package-id-removal` | 2195c98f (epic) | Re-home to 2195c98f's ProfileId task once that breakdown exists (OD-14). |
| `profile-dirty-target-refusal` | 3a3b86cf (f4d9502a's git-module task) | OD-14. |
| da12ad66 | 2d10f1dd, 5f71a48d | Reject atomicity and where a reject reason is recorded. |

Adding these edges transitively reduced bb03df0a (the v1.0 tag task): its direct edges to 5f71a48d and f4d9502a were dropped, and both remain reachable through 3d9a0ce3.

## Open follow-ups

- OD-9: this checkout's drift checks (`profile/contribution_drift.rs`, `drift_report.rs`, `repository_package.rs`, `template_region.rs`) keep the `jit-dogfood` manifest and `.jit/` in sync. D-8 does not name them, and they are out of scope here. The milestone steward decides whether a follow-up is filed.
- Nearby domain-agnostic smells, not required by any criterion: the hard-coded `"plan"` document label (`commands/validate.rs:2906`) and the link checker's hard-coded permanent paths (`document/link_validator.rs:202-211`). See [investigation-children.md](investigation-children.md) §6.
