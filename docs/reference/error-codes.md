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
| `INVALID_STATE` | A lifecycle state or transition is invalid. | `2` |
| `BLOCKED` | Unfinished dependencies block the operation. | `4` |
| `GATE_FAILED` | A quality-gate checker did not pass. | `4` |
| `IO_ERROR` | An input/output or external-system operation failed. | `10` |
| `PERMISSION_DENIED` | The operation was denied by filesystem or operating-system permissions. | `5` |
| `PARSE_ERROR` | Input data could not be parsed. | `1` |
| `CLAIM_REQUIRES_GIT` | The claim or lease operation requires Git. | `10` |
| `AMBIGUOUS_ID` | The ID prefix matches more than one candidate. | `2` |
| `INVALID_ID_PREFIX` | The ID prefix is shorter than the accepted minimum. | `2` |
| `REPOSITORY_NOT_FOUND` | No JIT repository exists at the resolved path. | `3` |
| `REPOSITORY_FORMAT_TOO_NEW` | The repository format is newer than this binary supports. | `10` |
| `STALE_BINARY` | Committed build inputs changed, working-tree build inputs are uncommitted, or build provenance records an uncommitted build input. | `10` |
| `DELETION_NOT_CONFIRMED` | Issue deletion lacks the required operator confirmation. | `2` |
| `PROFILE_NOT_FOUND` | No resolution route found the requested profile. | `3` |
| `PROFILE_CONFLICT` | Profile planning or validation found a conflict. | `4` |
| `DEPENDENCY_ERROR` | A dependency command failed. | `3` |
| `GATE_ERROR` | A gate command failed. | `6` |
| `GATE_CHECK_ERROR` | A gate-status check failed. | `3` |
| `PRESET_ERROR` | A gate-preset command failed. | `3` |
| `ITEM_NOT_FOUND` | An issue item-address lookup did not resolve an item. | `1` |
| `ITEM_COMMAND_FAILED` | An item command failed without a more specific classification. | `1` |
| `INVARIANT_COMMAND_FAILED` | An invariant command failed without a more specific classification. | `1` |
| `PROJECT_COMMAND_FAILED` | A project command failed without a more specific classification. | `1` |
| `PROFILE_ERROR` | A profile command failed without a more specific classification. | `1` |
| `SEARCH_FAILED` | The search backend failed while executing a query. | `1` |
| `RIPGREP_NOT_FOUND` | The external ripgrep search tool was not found. | `1` |
| `WORKTREE_INFO_ERROR` | Worktree identity inspection failed. | `1` |
| `WORKTREE_LIST_ERROR` | Worktree enumeration failed. | `1` |
| `HOOKS_INSTALL_ERROR` | Repository hook installation failed. | `1` |
| `GENERIC_ERROR` | A command failed without a more specific public classification. | `1` |
| `recovery_failed` | Repository recovery failed. | `1` |
| `CLAIM_ACQUIRE_ERROR` | Claim acquisition failed. | `3` |
| `CLAIM_RELEASE_ERROR` | Claim release failed. | `3` |
| `CLAIM_RENEW_ERROR` | Claim renewal failed. | `3` |
| `CLAIM_HEARTBEAT_ERROR` | Claim heartbeat failed. | `3` |
| `CLAIM_STATUS_ERROR` | Claim status inspection failed without a more specific classification. | `1` |
| `CLAIM_LIST_ERROR` | Claim enumeration failed without a more specific classification. | `1` |
| `CLAIM_FORCE_EVICT_ERROR` | Forced claim eviction failed. | `3` |
