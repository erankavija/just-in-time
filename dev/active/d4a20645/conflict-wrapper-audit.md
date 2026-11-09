# Read-path wrappers over a retryable conflict

Inventory for `d4a20645` REQ-02 at commit `ad4fb78f5` (merge `5b463ef09`): every error-context wrapper around a read-path capture, whether it can wrap a `RepositoryStateStoreError::RetryableConflict`, and its disposition. Line numbers are at the merge.

| Site | Wraps a conflict? | Disposition |
|---|---|---|
| `storage/configuration_store.rs` `load_configuration` | Yes | Fixed: "Failed to read configuration at …" is added only when `is_retryable_conflict` is false; a conflict passes through unchanged |
| `commands/config.rs`, `commands/claim.rs`, `commands/bootstrap.rs` callers of `load_configuration` | No wrapper of their own | Covered by the fix at `load_configuration` |
| `commands/config.rs` `capture_initialized` | No | Bare `?` |
| `storage/repository_state_store.rs` session-open path | No | Downcasts to `RepositoryStateStoreError` and keeps the type |
| `storage/json.rs` `read_leaf_nofollow` callers | No | That read raises only a leaf `Io` error, never a conflict |
| `storage/json.rs` raw filesystem reads | No | Raw `fs` I/O outside capture |
| `commands/mod.rs` `captured_records_error` | No | Maps an issue-record decode error, not a conflict |
| `commands/template.rs` missing-issue context | No | Context on an absent issue |
| `commands/archive.rs` `Option` context | No | Context on an `Option`, not a capture result |
| `commands/validate.rs` derived-plan context | No | A conflict returns `Retry` before this point |
| `commands/profile.rs` package-tree capture | No | The package capture error has no conflict variant |
| `commands/mod.rs` read driver, `storage/json.rs` and `storage/memory.rs` retry loops | No | They match the conflict to retry and add no context |

The owning test is `test_load_configuration_conflict_surfaces_its_own_message_with_exit_12` (`storage/repository_state_store_tests.rs`). Before the fix it failed with the message `Failed to read configuration at '<root>/.jit'` in place of the conflict's own `repository changed or is busy: configuration selection; retry. …`.
