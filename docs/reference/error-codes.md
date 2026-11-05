<!-- Generated from `ErrorCode::ALL`, `as_str`, `description`, and `exit_code` in `jit::output` — do not edit by hand. -->

# Machine-readable Error Codes

> **Diátaxis Type:** Reference

These are the values written to `error.code` in a machine-readable failure envelope.
Each code determines the process exit status shown in the final column. For the exit
status taxonomy and command-specific exceptions, see [Exit Codes](exit-codes.md).

| Error code | Meaning | Exit status |
| --- | --- | --- |
| `ISSUE_NOT_FOUND` | The requested issue does not exist. | `3` |
| `GATE_NOT_FOUND` | The requested gate does not exist. | `3` |
| `CYCLE_DETECTED` | The dependency would create a cycle. | `4` |
| `INVALID_ARGUMENT` | An argument or invocation is invalid. | `2` |
| `INVALID_LABEL_PATTERN` | A label query filter is not in the accepted namespace/value form. | `2` |
| `VALIDATION_FAILED` | Repository or domain validation failed. | `4` |
| `ALREADY_EXISTS` | The requested resource already exists. | `6` |
| `BLOCKED` | Unfinished dependencies block the operation. | `4` |
| `GATE_FAILED` | A quality-gate checker did not pass. | `4` |
| `IO_ERROR` | An input/output or external-system operation failed. | `10` |
| `PERMISSION_DENIED` | The operation was denied by filesystem or operating-system permissions. | `5` |
| `PARSE_ERROR` | Input data could not be parsed. | `1` |
| `REQUIRES_GIT` | The command requires Git. | `10` |
| `AMBIGUOUS_ID` | The ID prefix matches more than one candidate. | `2` |
| `INVALID_ID_PREFIX` | The ID prefix is shorter than the accepted minimum. | `2` |
| `REPOSITORY_NOT_FOUND` | No JIT repository exists at the resolved path. | `3` |
| `DOCUMENT_NOT_FOUND` | The requested document or commit does not exist. | `3` |
| `REPOSITORY_FORMAT_UNSUPPORTED` | The repository format is not the one this binary supports. | `10` |
| `DELETION_NOT_CONFIRMED` | Issue deletion lacks the required operator confirmation. | `2` |
| `LINKED_CHECKOUT_WRITE_REFUSED` | The linked checkout's write stance refuses state-mutating commands. | `2` |
| `PROFILE_CONFLICT` | Profile planning found packages being applied that define one target or declaration differently. | `4` |
| `PROFILE_UNCOMMITTED_TARGETS` | A profile application would overwrite paths that differ from HEAD; committing or discarding those changes lets it proceed. | `4` |
| `PRESET_ERROR` | A gate-preset command failed. | `3` |
| `ITEM_NOT_FOUND` | An issue item-address lookup did not resolve an item. | `1` |
| `PROFILE_ERROR` | A profile command failed without a more specific classification. | `1` |
| `SEARCH_FAILED` | The search backend failed while executing a query. | `1` |
| `RIPGREP_NOT_FOUND` | The external ripgrep search tool was not found. | `1` |
| `GENERIC_ERROR` | A command failed without a more specific public classification. | `1` |
| `recovery_failed` | Applying a leftover redo record, or a recovery routine, failed. | `1` |
| `SERVER_EXITED` | The foreground server process exited with a failure status. | `1` |
| `LOCK_TIMEOUT` | A lock wait expired while another jit process held the lock; nothing was changed. | `11` |
