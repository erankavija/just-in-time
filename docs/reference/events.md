<!-- Generated from `jit::domain::event_catalog` — do not edit by hand. -->

# Event Log Tags

`.jit/events.jsonl` stores one JSON object per line. Every record is tagged by a
snake-case `type` field and carries its own `id` and `timestamp`; the remaining
fields are flat on the object and vary by tag. This reference is generated from
the `Event` type in `crates/jit/src/domain/types.rs` and is also served, as the
`events` array, by `jit --schema`.

The **scope** is the state a record is about, and it decides the `issue_id` field:
an `issue`-scoped record names the one issue it concerns, while `registry`- and
`repository`-scoped records omit `issue_id` entirely, because they record a change
to shared state. The tags whose records carry no `issue_id` are exactly:

- `artifact_archive_executed` (repository)
- `gate_definition_updated` (registry)
- `gate_definition_created` (registry)
- `gate_definition_removed` (registry)
- `lifecycle_timestamps_backfilled` (repository)
- `profile_applied` (repository)
- `profile_lifecycle` (repository)

A `jit events query --issue-id <ID>` filter therefore never returns them.

| Tag | Scope | `issue_id` | Emitted when |
| --- | --- | --- | --- |
| `issue_created` | issue | yes | An issue was created. |
| `issue_claimed` | issue | yes | An issue was claimed by an assignee. |
| `issue_state_changed` | issue | yes | An issue moved from one lifecycle state to another; the record carries both states. |
| `gate_passed` | issue | yes | A quality gate on the issue was recorded as passed. |
| `gate_failed` | issue | yes | A quality gate on the issue was recorded as failed. |
| `gate_added` | issue | yes | A quality gate was added to the issue's required gates. |
| `gate_removed` | issue | yes | A quality gate was removed from the issue's required gates. |
| `issue_completed` | issue | yes | An issue was completed. |
| `issue_deleted` | issue | yes | An issue was permanently deleted. |
| `issue_released` | issue | yes | An issue was released from its assignee; the record carries the former assignee and the reason. |
| `issue_updated` | issue | yes | An issue's fields were updated; the record names the changed fields. |
| `artifact_archive_executed` | repository | no | `jit archive ... --execute` durably recorded publications, exact reference changes, and identity-guarded planned deletions before deletion attempts. |
| `dependency_reduced` | issue | yes | `jit validate --fix` removed the issue's redundant (transitively implied) dependency edges. |
| `local_rule_bypassed` | issue | yes | `--force` overrode an enforcing validation rule's error finding on an issue write; one record per bypassed rule. |
| `transition_blocked` | issue | yes | An enforcing graph rule refused a state transition; the record carries the target state and the rule. |
| `graph_rule_bypassed` | issue | yes | `--force` overrode an enforcing graph rule's error finding at a state transition; one record per bypassed rule. |
| `gate_definition_updated` | registry | no | `jit gate update` edited a registered gate. |
| `gate_definition_created` | registry | no | `jit gate define` registered a gate. |
| `gate_definition_removed` | registry | no | `jit gate remove` unregistered a gate. |
| `lifecycle_timestamps_backfilled` | repository | no | The one-time `jit migrate lifecycle-timestamps` backfill wrote derived lifecycle timestamps; the record carries the number of issues it updated. |
| `profile_applied` | repository | no | A profile package, its canonical provenance record, and this audit event reached one durable transaction commit point. |
| `profile_lifecycle` | repository | no | A profile lifecycle operation reached one durable transaction commit point; the record summarizes per-profile actions and variable source kinds without resolved values or rendered content. |

For the commands that read the log, see
[Event Log Commands](cli-commands.md#event-log-commands); for the file's place in
the `.jit/` layout, see [Storage Format](storage-format.md#event-log-format).
